//! `scaleform.gfx.TextFieldEx` native methods (Sandbox Engine patch b; the behaviour is
//! `display_object/scaleform_text.rs`)

use crate::avm2::Error;
use crate::avm2::activation::Activation;
use crate::avm2::function::FunctionArgs;
use crate::avm2::parameters::ParametersExt;
use crate::avm2::value::Value;
use crate::display_object::{EditText, ScaleformText, TextAutoSize, VerticalAlign};
use crate::string::AvmString;

/// The text field passed first, if it is one.
fn text_field<'gc>(args: &FunctionArgs<'_, 'gc>) -> Option<EditText<'gc>> {
    args.try_get_object(0)?.as_display_object()?.as_edit_text()
}

/// Changes one Scaleform setting of the field passed first.
fn change<'gc>(
    activation: &mut Activation<'_, 'gc>,
    args: &FunctionArgs<'_, 'gc>,
    edit: impl FnOnce(&mut ScaleformText),
) -> Result<Value<'gc>, Error<'gc>> {
    if let Some(field) = text_field(args) {
        let mut settings = field.scaleform();
        edit(&mut settings);
        field.set_scaleform(settings, activation.context);
    }
    Ok(Value::Undefined)
}

pub fn set_vertical_align<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let name = args.get_string(activation, 1).to_string();
    let Some(align) = VerticalAlign::from_name(&name) else {
        return Ok(Value::Undefined);
    };
    change(activation, &args, |s| s.vertical_align = align)
}

pub fn get_vertical_align<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let name = text_field(&args).map_or("none", |field| field.scaleform().vertical_align.name());
    Ok(AvmString::new_utf8(activation.gc(), name).into())
}

pub fn set_text_auto_size<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let name = args.get_string(activation, 1).to_string();
    let Some(mode) = TextAutoSize::from_name(&name) else {
        return Ok(Value::Undefined);
    };
    change(activation, &args, |s| s.auto_size = mode)
}

pub fn get_text_auto_size<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let name = text_field(&args).map_or("none", |field| field.scaleform().auto_size.name());
    Ok(AvmString::new_utf8(activation.gc(), name).into())
}

pub fn set_no_translate<'gc>(
    activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    let value = args.get_bool(1);
    change(activation, &args, |s| s.no_translate = value)
}

pub fn get_no_translate<'gc>(
    _activation: &mut Activation<'_, 'gc>,
    _this: Value<'gc>,
    args: FunctionArgs<'_, 'gc>,
) -> Result<Value<'gc>, Error<'gc>> {
    Ok(text_field(&args).is_some_and(|field| field.scaleform().no_translate).into())
}
