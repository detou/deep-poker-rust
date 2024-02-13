use candle_core::{Module, Tensor};
use candle_nn::{linear, Linear, VarBuilder};

pub struct ActorNetwork {
    linear_1: Linear,
    linear_2: Linear,
}

impl ActorNetwork {
    pub fn new(
        vb: &VarBuilder,
        action_count: usize,
    ) -> Result<ActorNetwork, Box<dyn std::error::Error>> {
        let weight_dims: Vec<Vec<usize>> = vec![vec![256, 256], vec![action_count, 256]];

        Ok(ActorNetwork {
            linear_1: linear(
                weight_dims[0][1],
                weight_dims[0][0],
                vb.pp("actor_linear_1"),
            )?,
            linear_2: linear(
                weight_dims[1][1],
                weight_dims[1][0],
                vb.pp("actor_linear_2"),
            )?,
        })
    }

    pub fn forward(&self, x: &Tensor) -> Result<Tensor, Box<dyn std::error::Error>> {
        let mut y = self.linear_1.forward(x)?;
        y = y.relu()?;
        y = self.linear_2.forward(&y)?;
        y = candle_nn::ops::softmax(&y, candle_core::D::Minus1)?;
        Ok(y)
    }
}
