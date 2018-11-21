use tract_core::model::ModelDsl;
use tract_core::ops::nn::ConvUnary;
use tract_core::ops::prelude::*;
use tract_core::context::Context;
use tract_core::optim::OptimizerPass;
use tract_core::*;

#[derive(Debug)]
pub struct TensorflowContext;

impl Context for TensorflowContext {
    fn optimizer_passes(&self) -> Vec<Box<OptimizerPass>> {
        let dflt = tract_core::context::DefaultContext;
        let mut passes = dflt.optimizer_passes();
        passes.push(Box::new(UntensorflowConv));
        passes
    }
}

struct UntensorflowConv;
impl OptimizerPass for UntensorflowConv {
    fn pass(&self, model: &mut Model) -> TractResult<bool> {
        let mut done_something = false;
        done_something = done_something || undo_all_conv1d_as_conv2d(model)?;
        done_something = done_something || undo_all_space_to_batch(model)?;
        Ok(done_something)
    }
}

macro_rules! some_or_ok_false {
    ($option:expr) => {
        match $option {
            Some(prec) => prec,
            None => return Ok(false),
        }
    };
}

fn undo_all_conv1d_as_conv2d(model: &mut Model) -> TractResult<bool> {
    let convs: Vec<usize> = model
        .eval_order()?
        .into_iter()
        .filter(|&node| model.node(node).op_is::<ConvUnary>())
        .collect();
    convs.into_iter().try_fold(
        false,
        |acc, cv| Ok(acc || undo_conv1d_as_conv2d(model, cv)?),
    )
}

fn undo_conv1d_as_conv2d(model: &mut Model, node_id: usize) -> TractResult<bool> {
    use tract_core::ops::array::{AddDims, RmDims};
    let new_op = {
        let prec_node = some_or_ok_false!(model.single_prec(node_id)?);
        let add_dim_op = some_or_ok_false!(prec_node.op_as::<AddDims>());
        let succ_node = some_or_ok_false!(model.single_succ(node_id)?);
        let rm_dim_op = some_or_ok_false!(succ_node.op_as::<RmDims>());
        let conv_op = some_or_ok_false!(model.node(node_id).op_as::<ConvUnary>());
        if add_dim_op.axes.len() == 1 && rm_dim_op.axes == add_dim_op.axes {
            let axis = add_dim_op.axes[0];
            conv_op.rm_dummy_axis(axis)?
        } else {
            None
        }
    };
    if let Some(new_op) = new_op {
        let name = model.node(node_id).name.clone();
        model.replace_nodes(node_id, 1, 1, vec![(name, Box::new(new_op))])?;
    }
    Ok(false)
}

fn undo_all_space_to_batch(model: &mut Model) -> TractResult<bool> {
    let convs: Vec<usize> = model
        .eval_order()?
        .into_iter()
        .filter(|&node| model.node(node).op_is::<ConvUnary>())
        .collect();
    convs
        .into_iter()
        .try_fold(false, |acc, cv| Ok(acc || undo_space_to_batch(model, cv)?))
}

fn undo_space_to_batch(model: &mut Model, node_id: usize) -> TractResult<bool> {
    use ops::nn::s2b::unary::SpaceToBatchUnary;
    let new_op = {
        let prec_node = some_or_ok_false!(model.single_prec(node_id)?);
        let s2b_op = some_or_ok_false!(prec_node.op_as::<SpaceToBatchUnary>());
        let succ_node = some_or_ok_false!(model.single_succ(node_id)?);
        let conv_op = some_or_ok_false!(model.node(node_id).op_as::<ConvUnary>());
        let new_op = ConvUnary {
            data_fmt: conv_op.data_fmt,
            kernel_is_hwio: conv_op.kernel_is_hwio,
            padding: conv_op.padding.clone(), // FIXME
            dilations: s2b_op.block_shape.iter().map(|&i| i as usize).collect(),
            strides: conv_op.strides.clone(),
            kernel: conv_op.kernel.clone(),
            bias: conv_op.bias.clone(),
            full_input_shape: model
                .fact(prec_node.inputs[0])?
                .shape
                .concretize()
                .ok_or("Optimizing an unalized network")?,
            full_output_shape: succ_node.outputs[0]
                .fact
                .shape
                .concretize()
                .ok_or("Optimizing an unalized network")?,
            group: conv_op.group,
        };
        Some(new_op)
    };
    if let Some(new_op) = new_op {
        let name = model.node(node_id).name.clone();
        model.replace_nodes(node_id, 1, 1, vec![(name, Box::new(new_op))])?;
    }
    Ok(false)
}
