use std::{
    cmp::Ordering,
    sync::{Arc, Mutex, TryLockError},
    thread,
    time::Duration,
};

use candle_core::Device;
use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use threadpool::ThreadPool;

use crate::{
    game::{action::ActionConfig, tree::Tree},
    model::poker_network::PokerNetwork,
};

use super::{agent_network::AgentNetwork, Agent};
use rand::prelude::SliceRandom;
use std::fs::File;
use std::io::Read;
use std::io::Write;

struct AgentTournament {
    network_file: String,
    elo: f32,
    iteration: u32,
    agent_network: Arc<Box<dyn Agent>>,
    hands_played: usize,
    over_max_rating: bool,
}

pub struct Tournament {
    agents: Vec<Arc<Mutex<AgentTournament>>>,
    player_count: u32,
    action_config: ActionConfig,
    device: Device,
}

impl Tournament {
    pub fn new(player_count: u32, action_config: ActionConfig, device: Device) -> Tournament {
        Tournament {
            agents: Vec::new(),
            player_count,
            action_config,
            device,
        }
    }

    pub fn add_agent(
        &mut self,
        network_file: String,
        iteration: u32,
    ) -> Result<(), candle_core::Error> {
        let mut network = PokerNetwork::new(
            self.player_count,
            self.action_config.clone(),
            self.device.clone(),
            self.device.clone(),
            false,
        )?;
        network.load_var_map(network_file.as_str())?;

        self.agents.push(Arc::new(Mutex::new(AgentTournament {
            network_file,
            elo: 1400.0,
            iteration,
            agent_network: Arc::new(Box::new(AgentNetwork::new(network))),
            hands_played: 0,
            over_max_rating: false,
        })));
        Ok(())
    }

    pub fn get_best_agents(&mut self, cnt: usize) -> Vec<Arc<Box<dyn Agent>>> {
        if self.agents.len() > cnt {
            let mut result = Vec::new();
            self.agents
                .sort_by(|a, b| b.lock().unwrap().elo.total_cmp(&a.lock().unwrap().elo));
            for i in 0..cnt {
                let agent = self.agents[i].lock().unwrap();
                println!("Taking agent: {}, elo: {}", agent.iteration, agent.elo);
                result.push(Arc::clone(&agent.agent_network));
            }
            result
        } else {
            let mut result = Vec::new();
            for agent in &self.agents {
                let agent = agent.lock().unwrap();
                result.push(Arc::clone(&agent.agent_network));
            }
            result
        }
    }

    pub fn get_agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn play(&mut self, total_hands: usize) {
        // Dynamically select agents based on Elo for each game
        self.agents
            .sort_by(|a, b| b.lock().unwrap().elo.total_cmp(&a.lock().unwrap().elo));

        let agents_tournament_base = Arc::new(Mutex::new(self.agents.clone()));
        let n_workers = num_cpus::get() * 3 / 4;
        let thread_pool = ThreadPool::new(n_workers);
        let elo_k = 40.0;

        let batch_size = (self.agents.len() as f32 / n_workers as f32).ceil() as usize;
        let rounds = total_hands / self.agents.len();

        let progress_bar = Arc::new(ProgressBar::new(rounds as u64 * self.agents.len() as u64));
        progress_bar.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos:>7}/{len:7} ({eta})").unwrap()
        .progress_chars("#>-"));

        for worker in 0..n_workers {
            let player_count = self.player_count;
            let agents_length = self.agents.len();
            let start = worker * batch_size;
            let end = ((worker + 1) * batch_size).min(agents_length);
            let action_config = self.action_config.clone();
            let device = self.device.clone();
            let agents_tournament = Arc::clone(&agents_tournament_base);
            let pb = Arc::clone(&progress_bar);

            thread_pool.execute(move || {
                let mut rng = rand::thread_rng();

                // Run the loop N times
                for _ in 0..rounds {
                    for agent_index in start..end {
                        pb.inc(1);
                        let mut agents_game: Vec<(usize, Arc<Mutex<AgentTournament>>)>;

                        // Lock the tournament agents to prepare the match
                        // Choose agents with closer Elo for fairer matches
                        {
                            let agents_locked = agents_tournament.lock().unwrap();
                            let current_agent = Arc::clone(&agents_locked[agent_index]);
                            let chosen_elo = current_agent.lock().unwrap().elo;

                            // Step 1: Sort the remaining players by their elo difference to the chosen player
                            let mut elo_differences = agents_locked
                                .iter()
                                .enumerate()
                                .filter_map(|(index, agent)| {
                                    if index != agent_index {
                                        let agent = agent.lock().unwrap();
                                        Some((index, (chosen_elo - agent.elo).abs()))
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>();

                            // Randomize elo differences to avoid always choosing the same players, especially at the start
                            elo_differences.shuffle(&mut rng);

                            // Sort by elo difference
                            elo_differences
                                .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));

                            // Step 2: Take N-1 agents with the closest elo difference, then add current agent
                            agents_game = elo_differences
                                .iter()
                                .take((player_count - 1) as usize)
                                .map(|&(index, _)| (index, Arc::clone(&agents_locked[index])))
                                .collect::<Vec<_>>();

                            agents_game.push((agent_index, current_agent));
                        }

                        // Assign each agent a random index between 0 and player_count
                        let mut tree_agents = Vec::new();
                        let mut indexes = Vec::new();
                        {
                            let mut indexes_sorted = (0..player_count as usize).collect::<Vec<_>>();
                            for _ in 0..player_count {
                                let mut rng = rand::thread_rng();
                                let vec_index = rng.gen_range(0..indexes_sorted.len());
                                let agent_index = indexes_sorted[vec_index];
                                let agent = agents_game[agent_index].1.lock().unwrap();
                                tree_agents.push(Arc::clone(&agent.agent_network));
                                indexes.push(agent_index);
                                indexes_sorted.remove(vec_index);
                            }
                        }

                        // Play one game
                        let mut total_won = vec![0.0f32; player_count as usize];
                        let mut tree = Tree::new(player_count, &action_config);
                        {
                            let rewards = match tree.play_one_hand(&tree_agents, &device, true) {
                                Ok(r) => r,
                                Err(error) => panic!("ERROR in play_one_hand: {:?}", error),
                            };
                            for p in 0..player_count as usize {
                                total_won[indexes[p]] += rewards[p];
                            }
                        }

                        // Lock players & update ELO
                        {
                            let mut agents_locked = vec![];
                            // We need to lock all players at once to avoid deadlocks, or wait if one fails
                            loop {
                                for arc_mutex in &agents_game {
                                    match arc_mutex.1.try_lock() {
                                        Ok(guard) => agents_locked.push(guard),
                                        Err(TryLockError::WouldBlock) => {
                                            agents_locked.clear(); // Release any acquired locks
                                            thread::sleep(Duration::from_millis(1)); // Prevent busy-waiting
                                            break;
                                        }
                                        Err(_) => panic!("Mutex is poisoned"),
                                    }
                                }
                                if agents_locked.len() == agents_game.len() {
                                    break; // Successfully locked all Mutexes
                                }
                            }

                            // Check each pair of players to determine winner(s) and update ELO
                            let mut elo_diff = vec![0.0f32; player_count as usize];
                            for i in 0..player_count as usize {
                                for j in i + 1..player_count as usize {
                                    // // We dont want ELO to increase for negative rewards, or decrease for positive rewards
                                    // if (total_won[i] < -1e-4 && total_won[j] < -1e-4)
                                    //     || (total_won[i] > 1e-4 && total_won[j] > 1e-4)
                                    // {
                                    //     continue;
                                    // }

                                    let player_i_result =
                                        if (total_won[i] - total_won[j]).abs() < 1e-4 {
                                            0.5
                                        } else if total_won[i] > total_won[j] {
                                            1.0
                                        } else {
                                            0.0
                                        };

                                    let prob_i = Self::calculate_expected_win_prob(
                                        agents_locked[i].elo,
                                        agents_locked[j].elo,
                                    );
                                    let prob_j = Self::calculate_expected_win_prob(
                                        agents_locked[j].elo,
                                        agents_locked[i].elo,
                                    );

                                    // Adjust K-factor based on the possible reward in the game
                                    let max_to_win = action_config.buy_in as f32;

                                    let min_value = if (total_won[i] < -1e-4
                                        && total_won[j] < -1e-4)
                                        || (total_won[i] > 1e-4 && total_won[j] > 1e-4)
                                    {
                                        (total_won[i] - total_won[j]).abs()
                                    } else {
                                        total_won[i].abs().min(total_won[j].abs())
                                    };

                                    let dyn_elo_k = elo_k * min_value / max_to_win;

                                    let dyn_elo_k_i = if agents_locked[i].over_max_rating {
                                        dyn_elo_k * 0.25
                                    } else if agents_locked[i].hands_played > 100000 {
                                        dyn_elo_k * 0.5
                                    } else {
                                        dyn_elo_k
                                    };

                                    let dyn_elo_k_j = if agents_locked[j].over_max_rating {
                                        dyn_elo_k * 0.25
                                    } else if agents_locked[j].hands_played > 100000 {
                                        dyn_elo_k * 0.5
                                    } else {
                                        dyn_elo_k
                                    };

                                    let elo_diff_i = Self::calculate_elo_diff(
                                        prob_i,
                                        player_i_result,
                                        dyn_elo_k_i,
                                    );
                                    let elo_diff_j = Self::calculate_elo_diff(
                                        prob_j,
                                        1.0 - player_i_result,
                                        dyn_elo_k_j,
                                    );

                                    elo_diff[i] += elo_diff_i;
                                    elo_diff[j] += elo_diff_j;
                                }
                            }

                            // Update ELO
                            for i in 0..player_count as usize {
                                agents_locked[i].elo += elo_diff[i];
                                agents_locked[i].hands_played += 1;
                                if agents_locked[i].elo > 2400.0 {
                                    agents_locked[i].over_max_rating = true;
                                }
                            }
                        }
                    }
                }
            });
        }

        thread_pool.join();

        // Remove all agents with a negative ELO
        self.agents.retain(|agent| agent.lock().unwrap().elo >= 0.0);

        // Sort agents by ELO and print them
        self.agents
            .sort_by(|a, b| b.lock().unwrap().elo.total_cmp(&a.lock().unwrap().elo));

        for agent in self.agents.iter() {
            let ag = agent.lock().unwrap();
            println!("Agent: {} - ELO: {}", ag.iteration, ag.elo);
        }
    }

    fn calculate_expected_win_prob(elo_a: f32, elo_b: f32) -> f32 {
        let elo_diff = elo_b - elo_a;
        1.0 / (1.0 + 10.0f32.powf(elo_diff / 400.0))
    }

    fn calculate_elo_diff(expected_win_prob: f32, actual_win: f32, k: f32) -> f32 {
        k * (actual_win - expected_win_prob)
    }

    pub fn save_state<P: AsRef<std::path::Path>>(&self, path: P) {
        let mut file = File::create(path).expect("Failed to create file");

        for agent in &self.agents {
            let agent = agent.lock().unwrap();
            let line = format!(
                "{};{};{};{};{}\n",
                agent.iteration,
                agent.elo,
                agent.network_file,
                agent.hands_played,
                agent.over_max_rating
            );
            file.write_all(line.as_bytes())
                .expect("Failed to write to file");
        }
    }

    pub fn load_state<P: AsRef<std::path::Path>>(&mut self, path: P) {
        let mut file = File::open(path).expect("Failed to open file");
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .expect("Failed to read file");

        for line in contents.lines() {
            let parts: Vec<&str> = line.split(';').collect();
            let iteration = parts[0].parse::<u32>().unwrap();
            let elo = parts[1].parse::<f32>().unwrap();
            let network_file = parts[2].to_string();
            let hands_played = if parts.len() > 3 {
                parts[3].parse::<usize>().unwrap()
            } else {
                0
            };
            let over_max_rating = if parts.len() > 4 {
                parts[4].parse::<bool>().unwrap()
            } else {
                false
            };
            self.add_agent(network_file, iteration).unwrap();
            let agent = self.agents.last().unwrap();
            let mut agent = agent.lock().unwrap();
            agent.elo = elo;
            agent.hands_played = hands_played;
            agent.over_max_rating = over_max_rating;
        }
    }
}
