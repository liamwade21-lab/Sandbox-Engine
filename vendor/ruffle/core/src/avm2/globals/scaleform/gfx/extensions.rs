//! `scaleform.gfx.Extensions` native methods (Sandbox Engine patch b)

use crate::avm2::Error;
use crate::avm2::activation::Activation;
use crate::avm2::function::FunctionArgs;
use crate::avm2::parameters::ParametersExt;
use crate::avm2::value::Value;

/// Implements `Extensions.noInvisibleAdvance`'s getter
pub fn get_no_invisible_advance<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    _args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(activation.context.stage.no_invisible_advance().into())
}

/// Implements `Extensions.noInvisibleAdvance`'s setter
pub fn set_no_invisible_advance<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    activation
        .context
        .stage
        .set_no_invisible_advance(args.get_bool(0));
    Ok(Value::Undefined)
}
