use kassiber_envelope::{dict, get, s, truthy, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_wallets::freshness::{
    execution::Options,
    planning::{Request, Scope},
};

use crate::output::RequestId;

pub(crate) const RUN_KINDS: &[&str] = &[
    "ui.journals.process",
    "ui.wallets.sync",
    "ui.freshness.run",
    "ui.workspace.freshness.run",
];
pub(crate) const CONTROL_KINDS: &[&str] = &[
    "ui.freshness.cancel",
    "ui.freshness.pause",
    "ui.freshness.resume",
];

pub(crate) struct Invocation {
    pub(crate) request: Request,
    pub(crate) options: Options,
}

fn fallback() -> AppError {
    AppError::new(
        "native_fallback_required",
        "Freshness invocation requires the reference implementation",
    )
}

fn valid_id(id: &RequestId) -> bool {
    matches!(id, RequestId::Value(Value::Str(value)) if value.as_str().is_some_and(|v| !v.is_empty()))
}

pub(crate) fn args<'a>(
    kind: &str,
    value: Option<&'a Value>,
    empty: &'a Value,
) -> Result<&'a Value, AppError> {
    match value {
        None | Some(Value::Null) => Ok(empty),
        Some(value @ Value::Dict(_)) => Ok(value),
        Some(value) => Err(AppError::new(
            "validation",
            if kind == "ui.wallets.sync" {
                "ui.wallets.sync args must be an object"
            } else {
                "daemon args must be an object"
            },
        )
        .with_details(dict([("type", s(type_name(value)))]))),
    }
}

fn boolean(args: &Value, key: &str, kind: &str, details: bool) -> Result<Option<bool>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(value) => {
            let error = AppError::new("validation", format!("{kind} {key} must be a boolean"));
            Err(if details {
                error.with_details(dict([("type", s(type_name(value)))]))
            } else {
                error
            })
        }
    }
}

pub(crate) fn parse(kind: &str, args: &Value, id: &RequestId) -> Result<Invocation, AppError> {
    if kind == "ui.journals.process" {
        if let Value::Dict(pairs) = args {
            if !pairs.is_empty() {
                let mut unknown = pairs
                    .iter()
                    .filter_map(|(key, _)| match key {
                        Key::Str(value) => Some(value.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                unknown.sort();
                return Err(AppError::new(
                    "validation",
                    "ui.journals.process does not accept arguments",
                )
                .with_details(dict([(
                    "unknown",
                    Value::List(unknown.into_iter().map(Value::Str).collect()),
                )])));
            }
        }
        if !valid_id(id) {
            return Err(fallback());
        }
        return Ok(Invocation {
            request: Request {
                scope: Scope::ActiveProfile,
                wallet: None,
                rates: Some(false),
                journals: Some(true),
                force_full: false,
                auto_pair: true,
                wallets_sync: false,
            },
            options: Options {
                run: true,
                limit: None,
            },
        });
    }
    let wallets = kind == "ui.wallets.sync";
    let workspace = kind == "ui.workspace.freshness.run";
    if !wallets {
        let allowed: &[&str] = if workspace {
            &["workspace_id", "rates", "journals", "run", "limit"]
        } else {
            &[
                "wallet",
                "all",
                "rates",
                "journals",
                "auto_pair",
                "run",
                "limit",
                "force_full",
            ]
        };
        let Value::Dict(pairs) = args else {
            return Err(fallback());
        };
        let mut unknown = pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(key) if !allowed.iter().any(|allowed| key.as_str() == Some(allowed)) => {
                    Some(key.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        unknown.sort();
        if !unknown.is_empty() {
            return Err(AppError::new(
                "validation",
                format!("{kind} received unsupported arguments"),
            )
            .with_details(dict([(
                "unknown",
                Value::List(unknown.into_iter().map(Value::Str).collect()),
            )])));
        }
    }
    let scope = if workspace {
        let Some(Value::Str(value)) = get(args, "workspace_id") else {
            return Err(AppError::new(
                "validation",
                "ui.workspace.freshness.run requires args.workspace_id",
            ));
        };
        if kassiber_pytext::strip_py(value)
            .code_points()
            .next()
            .is_none()
        {
            return Err(AppError::new(
                "validation",
                "ui.workspace.freshness.run requires args.workspace_id",
            ));
        }
        Scope::Workspace(value.as_str().ok_or_else(fallback)?.to_owned())
    } else {
        Scope::ActiveProfile
    };
    let wallet = if workspace {
        None
    } else {
        match get(args, "wallet") {
            None | Some(Value::Null) => None,
            Some(Value::Str(value))
                if kassiber_pytext::strip_py(value)
                    .code_points()
                    .next()
                    .is_some() =>
            {
                Some(kassiber_pytext::strip(value.as_str().ok_or_else(fallback)?).to_owned())
            }
            Some(value) => {
                let error = AppError::new(
                    "validation",
                    format!("{kind} wallet must be a non-empty string"),
                );
                return Err(if wallets {
                    error.with_details(dict([("type", s(type_name(value)))]))
                } else {
                    error
                });
            }
        }
    };
    let all = if wallets {
        boolean(args, "all", kind, true)?.unwrap_or(wallet.is_none())
    } else {
        get(args, "all").map_or(wallet.is_none(), truthy)
    };
    if wallets && all && wallet.is_some() {
        return Err(AppError::new(
            "validation",
            "ui.wallets.sync wallet and all are mutually exclusive",
        ));
    }
    if !workspace && !wallets {
        boolean(args, "auto_pair", kind, false)?;
    }
    let force_full = if workspace {
        false
    } else {
        boolean(args, "force_full", kind, wallets)?.unwrap_or(false)
    };
    if wallets {
        boolean(args, "process_journals", kind, true)?;
    }
    if !valid_id(id) {
        return Err(fallback());
    }
    if !workspace && !all && wallet.is_none() {
        return Err(fallback());
    }
    let limit = if wallets {
        None
    } else {
        match get(args, "limit").filter(|value| truthy(value)) {
            None => None,
            Some(Value::Int(value)) => Some(i64::try_from(value.clone()).map_err(|_| fallback())?),
            Some(Value::Bool(value)) => Some(i64::from(*value)),
            Some(_) => return Err(fallback()),
        }
    };
    Ok(Invocation {
        request: Request {
            scope,
            wallet: if all { None } else { wallet },
            rates: if wallets {
                Some(false)
            } else {
                get(args, "rates")
                    .filter(|v| !matches!(v, Value::Null))
                    .map(truthy)
            },
            journals: get(
                args,
                if wallets {
                    "process_journals"
                } else {
                    "journals"
                },
            )
            .filter(|v| !matches!(v, Value::Null))
            .map(truthy),
            force_full,
            auto_pair: wallets || get(args, "auto_pair").is_some_and(truthy),
            wallets_sync: wallets,
        },
        options: Options {
            run: wallets || get(args, "run").is_none_or(truthy),
            limit,
        },
    })
}

#[cfg(feature = "test-kinds")]
pub(crate) fn supported(
    daemon: &mut crate::Daemon,
    kind: &str,
    raw_args: Option<&Value>,
    id: &RequestId,
) -> Result<bool, AppError> {
    if daemon.freshness_requests.running() {
        return Err(super::busy());
    }
    if !valid_id(id) {
        return Ok(false);
    }
    let mut selected_args = raw_args.cloned();
    if let Some(Value::Dict(pairs)) = selected_args.as_mut() {
        pairs.retain(|(key, _)| *key != Key::from("expected_scope"));
    }
    let empty = dict::<&str>([]);
    let parsed = args(kind, selected_args.as_ref(), &empty).and_then(|args| parse(kind, args, id));
    if parsed
        .as_ref()
        .is_err_and(|error| error.code == "native_fallback_required")
    {
        return Ok(false);
    }
    if daemon.book.is_none() {
        return Ok(false);
    }
    // Desktop processing resolves an incomplete context like Python's plain
    // fallback and needs no source plan, so every open book is native.
    if kind == "ui.journals.process" {
        return Ok(true);
    }
    if kind != "ui.workspace.freshness.run" {
        let context = daemon.book_mut()?.context()?;
        if context.workspace_id.is_empty() || context.profile_id.is_empty() {
            return Ok(false);
        }
    }
    if let Some(value) = raw_args.filter(|value| get(value, "expected_scope").is_some()) {
        if !matches!(daemon.check_expected_scope(Some(value), id), Ok(None)) {
            return Ok(true);
        }
    }
    let invocation = match parsed {
        Ok(invocation) => invocation,
        Err(error) => return Ok(error.code != "native_fallback_required"),
    };
    match daemon.book_mut()?.freshness_plan(&invocation.request) {
        Ok(plan) => {
            if kind == "ui.wallets.sync" && !plan.supports_wallets_sync() {
                return Ok(false);
            }
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

pub(crate) fn handle(
    daemon: &mut crate::Daemon,
    kind: &str,
    raw_args: Option<&Value>,
    id: &RequestId,
) -> crate::Handled {
    if let Some(locked) = daemon.open_lazily(id)? {
        return Ok((Some(locked), false));
    }
    let mut selected_args = raw_args.cloned();
    if let Some(value) = selected_args
        .as_mut()
        .filter(|value| get(value, "expected_scope").is_some())
    {
        if let Some(locked) = daemon.check_expected_scope(Some(value), id)? {
            return Ok((Some(locked), false));
        }
        if let Value::Dict(pairs) = value {
            pairs.retain(|(key, _)| *key != Key::from("expected_scope"));
        }
    }
    let raw_args = selected_args.as_ref();
    let empty = dict::<&str>([]);
    let args = if kind == "ui.journals.process" {
        match raw_args {
            None | Some(Value::Null) => &empty,
            Some(value @ Value::Dict(_)) => value,
            Some(value) => {
                return Err(AppError::new(
                    "validation",
                    "ui.journals.process does not accept arguments",
                )
                .with_details(dict([("type", s(type_name(value)))]))
                .into())
            }
        }
    } else {
        args(kind, raw_args, &empty)?
    };
    if CONTROL_KINDS.contains(&kind) {
        let value = daemon
            .book_mut()?
            .freshness_control(kind.rsplit('.').next().unwrap_or(""), args)?;
        return Ok((
            Some(crate::output::with_request_id(
                kassiber_envelope::success(kind, value),
                id,
            )),
            false,
        ));
    }
    let invocation = parse(kind, args, id)?;
    if kind == "ui.wallets.sync" {
        let context = daemon.book_mut()?.context()?;
        if context.workspace_id.is_empty() || context.profile_id.is_empty() {
            return Ok((
                Some(crate::output::with_request_id(
                    kassiber_envelope::success(kind, dict([("results", Value::List(vec![]))])),
                    id,
                )),
                false,
            ));
        }
    }
    daemon.start_freshness(kind, id, &invocation.request, invocation.options, &empty)?;
    Ok((None, false))
}
