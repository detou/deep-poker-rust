use super::Agent;
use crate::game::hand_state::HandState;

use rand::Rng;

pub struct AgentRandom {}

impl<'a> Agent<'a> for AgentRandom {
    fn choose_action(
        &self,
        _hand_state: &HandState,
        valid_actions_mask: &[bool],
        _street: u8,
        _action_config: &crate::game::action::ActionConfig,
        _device: &candle_core::Device,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut rng = rand::thread_rng();
        let mut action_index = rng.gen_range(0..valid_actions_mask.len());

        while !valid_actions_mask[action_index] {
            action_index = rng.gen_range(0..valid_actions_mask.len());
        }

        Ok(action_index)
    }
}
