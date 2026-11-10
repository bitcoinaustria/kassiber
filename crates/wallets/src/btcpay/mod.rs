//! BTCPay connection services shared by the CLI and the desktop daemon.
//! Local operations here never construct a network client; Greenfield reads
//! run only through `kassiber_chain::btcpay` under an explicit action grant.
pub mod args;
pub(crate) mod bark;
pub mod connect;
pub mod create;
pub(crate) mod ledger;
pub mod provenance;
pub mod setup;

use args::{opt_str, py_text};
use kassiber_chain::btcpay::{permissions, url};
use kassiber_envelope::{get, s, set, truthy, AppError};
use kassiber_pyjson::Value;

type Result<T> = std::result::Result<T, AppError>;

/// The saved backend view a guide may read: its kind and URL only.
pub struct SavedBackend {
    pub name: String,
    pub kind: String,
    pub url: Option<String>,
}

/// `btcpay key-url` (`handlers.btcpay_key_guide`). `resolve` is
/// `resolve_backend` and is called only when `--backend` is given.
pub fn key_url(args: &Value, resolve: impl FnOnce(&str) -> Result<SavedBackend>) -> Result<Value> {
    let text = |key: &str| {
        get(args, key)
            .filter(|v| truthy(v))
            .map(|v| url::from_py(&py_text(v)))
    };
    let preset = text("preset").unwrap_or_else(|| "read_only".into());
    let scope = text("store_scope");
    let mut server = text("server_url");
    if let Some(name) = text("backend") {
        let backend = resolve(&name)?;
        if backend.kind != "btcpay" {
            return Err(AppError::new(
                "validation",
                format!("Backend '{}' is not a BTCPay instance", backend.name),
            ));
        }
        server = backend.url;
    }
    let normalized = server
        .filter(|u| !u.is_empty())
        .map(|u| url::normalize_server_url(&u))
        .transpose()?;
    let mut guide = permissions::setup_guide(normalized.as_deref(), &preset, scope.as_deref())?;
    let stores = match get(args, "store_ids") {
        Some(Value::List(items)) if !items.is_empty() => Some(items.clone()),
        _ => None,
    };
    if let (Some(base), Some(stores)) = (&normalized, &stores) {
        let ids: Vec<String> = stores.iter().map(|v| url::from_py(&py_text(v))).collect();
        let link = permissions::authorize_url(base, &preset, &ids, None)?;
        set(&mut guide, "authorize_url", Value::Str(url::to_py(&link)));
        set(&mut guide, "store_ids", Value::List(stores.clone()));
    }
    set(
        &mut guide,
        "server_url",
        normalized
            .as_deref()
            .map_or(Value::Null, |u| Value::Str(url::to_py(u))),
    );
    if normalized.is_none() {
        set(
            &mut guide,
            "hint",
            s("Pass --server-url or --backend to get a pre-filled authorization link."),
        );
    }
    Ok(guide)
}

/// `ui.connections.btcpay.key_guide` (`_btcpay_key_guide_payload`): no
/// network I/O. `saved` looks up `runtime_config["backends"][name]`.
pub fn key_guide(args: &Value, saved: impl FnOnce(&str) -> Option<SavedBackend>) -> Result<Value> {
    let preset = opt_str(args, "preset")?
        .map(|p| kassiber_pytext::lower(&url::from_py(&p)))
        .unwrap_or_else(|| "read_only".into());
    if permissions::PRESETS.iter().all(|p| p.id != preset) {
        return Err(AppError::new(
            "validation",
            format!("Unknown BTCPay key preset '{preset}'"),
        )
        .with_hint("Choose one of: read_only, wallet_history."));
    }
    let mut raw = match opt_str(args, "server_url")? {
        Some(v) => Some(v),
        None => opt_str(args, "url")?,
    };
    let reference = opt_str(args, "backend")?;
    if let (Some(reference), None) = (&reference, &raw) {
        let reference = url::from_py(reference);
        let not_found = || {
            AppError::new(
                "not_found",
                format!("BTCPay instance '{reference}' is not configured"),
            )
        };
        let backend = saved(&kassiber_pytext::lower(&reference)).ok_or_else(not_found)?;
        if kassiber_pytext::lower(&backend.kind) != "btcpay" {
            return Err(not_found());
        }
        raw = backend
            .url
            .filter(|u| !u.is_empty())
            .map(|u| url::to_py(&u));
    }
    let server = raw
        .map(|u| url::normalize_server_url(&url::from_py(&u)))
        .transpose()?;
    let scope = opt_str(args, "store_scope")?.map(|v| url::from_py(&v));
    let mut guide = permissions::setup_guide(server.as_deref(), &preset, scope.as_deref())?;
    set(
        &mut guide,
        "server_url",
        server
            .as_deref()
            .map_or(Value::Null, |u| Value::Str(url::to_py(u))),
    );
    Ok(guide)
}
