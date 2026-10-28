//! Local descriptor preview; network-backed detection remains disabled.
use crate::{
    output::{with_request_id, RequestId},
    Daemon, Handled,
};
use kassiber_envelope::{dict, get, s, set, success, type_name, AppError};
use kassiber_pyjson::{dumps, Value};
pub const KINDS: &[&str] = &["ui.wallets.preview_descriptor"];

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let mut args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(v @ Value::Dict(_)) => v.clone(),
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))]))
                .into())
        }
    };
    for key in ["descriptor", "change_descriptor", "wallet_material"] {
        let value = crate::imports::optional_string(&args, key)?;
        set(&mut args, key, value.map_or(Value::Null, s));
    }
    if get(&args, "wallet_material").is_some_and(|v| !matches!(v, Value::Null)) {
        let value = crate::imports::optional_string(&args, "script_type")?;
        set(&mut args, "script_type", value.map_or(Value::Null, s));
        if let Some(value) = get(&args, "script_types") {
            if !matches!(value, Value::Null | Value::Str(_) | Value::List(_)) {
                return Err(AppError::new(
                    "validation",
                    "script_types must be an array of script type names",
                )
                .with_details(dict([("type", s(type_name(value)))]))
                .into());
            }
        }
    }
    for key in ["chain", "network"] {
        let value = crate::imports::optional_string(&args, key)?;
        set(&mut args, key, value.map_or(Value::Null, s));
    }
    let args =
        serde_json::from_str(&dumps(&args, &Default::default()).map_err(|_| {
            AppError::new("internal_error", "Invalid descriptor preview arguments")
        })?)
        .map_err(|_| AppError::new("internal_error", "Invalid descriptor preview arguments"))?;
    let payload = kassiber_wallets::preview(&args)?;
    Ok((
        Some(with_request_id(success(kind, payload), request_id)),
        false,
    ))
}
