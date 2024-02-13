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
    action_config.preflop_raise_sizes = vec![2.0, 3.0];
    action_config.postflop_raise_sizes = vec![0.25, 0.5, 0.66, 1.0];

    // let mut tree = game::tree::Tree::new(3, &action_config);

    // for i in 0..3 {
    //     tree.traverse(i);
    // }

    let trainer_config = TrainerConfig {
        max_iters: 500000,
        hands_per_player_per_iteration: 1000,
        update_step: 256,
        ppo_epsilon: 0.2,
        ppo_delta_1: 3.0,
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

    // Test network inference
    // let var_map = VarMap::new();
    // let vb = VarBuilder::from_varmap(&var_map, DType::F32, &Device::Cpu);

    // let max_action_per_street_cnt = 9;
    // let player_count = 3;
    // let action_abstraction_count = 7;
    // let device = Device::Cpu;

    // let card_tensor = Tensor::zeros((1, 5, 13, 4), DType::F32, &device).unwrap();

    // let action_tensor = Tensor::zeros(
    //     (
    //         1,
    //         max_action_per_street_cnt * 4,
    //         action_abstraction_count as usize,
    //         player_count as usize + 2,
    //     ),
    //     DType::F32,
    //     &device,
    // )
    // .unwrap();

    // let model = model::siamese_network::SiameseNetwork::new(
    //     player_count,
    //     action_abstraction_count,
    //     max_action_per_street_cnt,
    //     &vb,
    // );

    // model.forward(&card_tensor, &action_tensor);
}
