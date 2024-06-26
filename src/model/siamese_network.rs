use std::collections::HashMap;

use candle_core::Module;
use candle_core::Tensor;
use candle_nn::conv2d_no_bias;
use candle_nn::BatchNormConfig;
use candle_nn::{batch_norm, linear, BatchNorm, Conv2d, Conv2dConfig, Linear, VarBuilder};

#[derive(Clone)]
struct BasicBlock {
    conv_1: Conv2d,
    conv_2: Conv2d,
    conv_3: Conv2d,
    bn: [BatchNorm; 3],
    out_channels: usize,
}

// Block a bit like resnet with residual connection but stride 1
impl BasicBlock {
    pub fn new(
        in_channels: usize,
        out_channels: usize,
        source_channels: usize,
        vb: VarBuilder,
    ) -> Result<BasicBlock, candle_core::Error> {
        let conv_1 = conv2d_no_bias(
            in_channels,
            out_channels,
            3,
            Conv2dConfig {
                stride: 1,
                padding: 1,
                dilation: 1,
                groups: 1,
            },
            vb.pp("conv_1"),
        )?;
        let bn_1 = batch_norm(
            out_channels,
            BatchNormConfig {
                eps: 1e-5,
                remove_mean: false,
                affine: true,
                momentum: 0.1,
            },
            vb.pp("bn_1"),
        )?;
        let conv_2 = conv2d_no_bias(
            out_channels,
            out_channels,
            3,
            Conv2dConfig {
                stride: 1,
                padding: 1,
                dilation: 1,
                groups: 1,
            },
            vb.pp("conv_2"),
        )?;
        let bn_2 = batch_norm(
            out_channels,
            BatchNormConfig {
                eps: 1e-5,
                remove_mean: false,
                affine: true,
                momentum: 0.1,
            },
            vb.pp("bn_2"),
        )?;
        let conv_3 = conv2d_no_bias(
            source_channels,
            out_channels,
            1,
            Conv2dConfig {
                stride: 1,
                padding: 0,
                dilation: 1,
                groups: 1,
            },
            vb.pp("conv_3"),
        )?;
        let bn_3 = batch_norm(
            out_channels,
            BatchNormConfig {
                eps: 1e-5,
                remove_mean: false,
                affine: true,
                momentum: 0.1,
            },
            vb.pp("bn_3"),
        )?;

        Ok(BasicBlock {
            conv_1,
            conv_2,
            conv_3,
            bn: [bn_1, bn_2, bn_3],
            out_channels,
        })
    }

    pub fn forward(
        &self,
        x: &Tensor,
        base_input: &Tensor,
        train: bool,
    ) -> Result<Tensor, candle_core::Error> {
        let mut out = self.conv_1.forward(x)?;
        out = out.apply_t(&self.bn[0], train)?;
        out = out.relu()?;

        out = self.conv_2.forward(&out)?;
        out = out.apply_t(&self.bn[1], train)?;
        out = out.relu()?;

        let mut identity = self.conv_3.forward(base_input)?;
        identity = identity.apply_t(&self.bn[2], train)?;

        out = (out + identity)?;
        out = out.relu()?;

        Ok(out)
    }

    fn get_batch_norm_tensors(&self) -> Result<HashMap<String, Tensor>, candle_core::Error> {
        let mut map = HashMap::new();
        for i in 0..3 {
            map.insert(
                format!("bn_{}.running_mean", i + 1),
                self.bn[i].running_mean().copy()?,
            );
            map.insert(
                format!("bn_{}.running_var", i + 1),
                self.bn[i].running_var().copy()?,
            );
            let (weight, bias) = self.bn[i].weight_and_bias().unwrap();
            map.insert(format!("bn_{}.weight", i + 1), weight.copy()?);
            map.insert(format!("bn_{}.bias", i + 1), bias.copy()?);
        }
        Ok(map)
    }

    fn set_batch_norm_tensors(
        &mut self,
        tensors: HashMap<String, Tensor>,
    ) -> Result<(), candle_core::Error> {
        for i in 0..3 {
            self.bn[i] = BatchNorm::new(
                self.out_channels,
                tensors[&format!("bn_{}.running_mean", i + 1)].clone(),
                tensors[&format!("bn_{}.running_var", i + 1)].clone(),
                tensors[&format!("bn_{}.weight", i + 1)].clone(),
                tensors[&format!("bn_{}.bias", i + 1)].clone(),
                1e-5,
            )?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SiameseTwin {
    conv_block_1: BasicBlock,
    conv_block_2: BasicBlock,
}

impl SiameseTwin {
    pub fn new(size: &[usize], vb: VarBuilder) -> Result<SiameseTwin, candle_core::Error> {
        let conv_block_1 = BasicBlock::new(size[0], size[1], size[0], vb.pp("twin_1"))?;
        let conv_block_2 = BasicBlock::new(size[1], size[2], size[0], vb.pp("twin_2"))?;

        Ok(SiameseTwin {
            conv_block_1,
            conv_block_2,
        })
    }

    pub fn forward(&self, x: &Tensor, train: bool) -> Result<Tensor, candle_core::Error> {
        let mut out = self.conv_block_1.forward(x, x, train)?;
        out = self.conv_block_2.forward(&out, x, train)?;
        out = out.avg_pool2d((1, 1))?;
        Ok(out)
    }

    fn get_batch_norm_tensors(&self) -> Result<HashMap<String, Tensor>, candle_core::Error> {
        let mut map = HashMap::new();

        let tensors1 = self.conv_block_1.get_batch_norm_tensors()?;
        for (k, v) in tensors1 {
            map.insert(format!("twin_1.{}", k), v);
        }
        let tensors2 = self.conv_block_2.get_batch_norm_tensors()?;
        for (k, v) in tensors2 {
            map.insert(format!("twin_2.{}", k), v);
        }

        Ok(map)
    }

    fn set_batch_norm_tensors(
        &mut self,
        tensors: HashMap<String, Tensor>,
    ) -> Result<(), candle_core::Error> {
        let mut tensors1 = HashMap::new();
        let mut tensors2 = HashMap::new();
        for (k, v) in tensors {
            if let Some(stripped) = k.strip_prefix("twin_1.") {
                tensors1.insert(stripped.to_string(), v);
            } else if let Some(stripped) = k.strip_prefix("twin_2.") {
                tensors2.insert(stripped.to_string(), v);
            }
        }

        self.conv_block_1.set_batch_norm_tensors(tensors1)?;
        self.conv_block_2.set_batch_norm_tensors(tensors2)?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct SiameseNetwork {
    card_twin: SiameseTwin,
    action_twin: SiameseTwin,
    merge_layer: Linear,
    output_layer: Linear,
}

impl SiameseNetwork {
    pub fn new(
        player_count: u32,
        action_abstraction_count: u32,
        max_action_per_street_cnt: usize,
        vb: VarBuilder,
    ) -> Result<SiameseNetwork, candle_core::Error> {
        let features_size = [48, 96];

        let card_input_size = (13, 4);
        let card_output_size = card_input_size.0 * card_input_size.1 * features_size[1];

        let action_input_size = (action_abstraction_count as usize, player_count as usize + 2);
        let action_output_size = action_input_size.0 * action_input_size.1 * features_size[1];

        let card_twin =
            SiameseTwin::new(&[6, features_size[0], features_size[1]], vb.pp("card_twin"))?;
        let action_twin = SiameseTwin::new(
            &[
                max_action_per_street_cnt * 4,
                features_size[0],
                features_size[1],
            ],
            vb.pp("action_twin"),
        )?;

        let merge_layer = linear(card_output_size + action_output_size, 512, vb.pp("merge"))?;

        let output_layer = linear(512, 512, vb.pp("output"))?;

        Ok(SiameseNetwork {
            card_twin,
            action_twin,
            merge_layer,
            output_layer,
        })
    }

    pub fn forward(
        &self,
        card_tensor: &Tensor,
        action_tensor: &Tensor,
        train: bool,
    ) -> Result<Tensor, candle_core::Error> {
        // Card Output
        let mut card_t = self.card_twin.forward(card_tensor, train)?;
        card_t = card_t.flatten(1, 3)?;

        let mut action_t = self.action_twin.forward(action_tensor, train)?;
        action_t = action_t.flatten(1, 3)?;

        let merged = Tensor::cat(&[&card_t, &action_t], 1)?;
        let mut output = self.merge_layer.forward(&merged)?;
        output = output.relu()?;
        output = self.output_layer.forward(&output)?;
        output = output.relu()?;

        Ok(output)
    }

    pub fn get_batch_norm_tensors(&self) -> Result<HashMap<String, Tensor>, candle_core::Error> {
        let mut map = HashMap::new();

        let card_tensors = self.card_twin.get_batch_norm_tensors()?;
        for (k, v) in card_tensors {
            map.insert(format!("card_twin.{}", k), v);
        }
        let action_tensors = self.action_twin.get_batch_norm_tensors()?;
        for (k, v) in action_tensors {
            map.insert(format!("action_twin.{}", k), v);
        }

        Ok(map)
    }

    pub fn set_batch_norm_tensors(
        &mut self,
        tensors: HashMap<String, Tensor>,
    ) -> Result<(), candle_core::Error> {
        let mut card_tensors = HashMap::new();
        let mut action_tensors = HashMap::new();
        for (k, v) in tensors {
            if let Some(stripped) = k.strip_prefix("card_twin.") {
                card_tensors.insert(stripped.to_string(), v);
            } else if let Some(stripped) = k.strip_prefix("action_twin.") {
                action_tensors.insert(stripped.to_string(), v);
            }
        }

        self.card_twin.set_batch_norm_tensors(card_tensors)?;
        self.action_twin.set_batch_norm_tensors(action_tensors)?;

        Ok(())
    }
}
