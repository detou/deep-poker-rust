// use candle_core::{DType, Device, Tensor};
// use candle_nn::{VarBuilder, VarMap};

use candle_core::Device;
use model::trainer_config::TrainerConfig;
use std::backtrace::Backtrace;

mod agent;
mod game;
mod model;

fn main() {
    let mut action_config = game::action::ActionConfig::new(3, 300, 20, 9);
    // 0.0 values are ignored for raises
    action_config.preflop_raise_sizes = vec![2.0, 3.0, 0.0, 0.0];
    action_config.postflop_raise_sizes = vec![0.25, 0.5, 0.66, 1.0];

    let trainer_config = TrainerConfig {
        max_iters: 500000,
        hands_per_player_per_iteration: 2048,
        update_step: 32,
        ppo_epsilon: 0.2,
        ppo_delta_1: 3.0,
        no_invalid_for_traverser: true,
    };

    let device = Device::cuda_if_available(0).unwrap();

    let mut trainer = model::trainer::Trainer::new(
        3,
        &action_config,
        &trainer_config,
        device,
        "/media/charles/CCH_BIG/deep_poker/",
    );
    if let Err(err) = trainer.train() {
        println!("Error: {}", err);

        let backtrace = Backtrace::capture();
        println!("Backtrace:\n{:?}", backtrace);
    }
}
