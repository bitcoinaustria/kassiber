//! Bull Bitcoin wallet connections: `ui.connections.bullbitcoin_wallet.create`
//! (`_create_bullbitcoin_wallet_connection_payload`) and the CLI
//! `wallets attach-bullbitcoin-wallet`. Both record where an export lives;
//! neither opens the file. An attached route only enriches rows the wallet
//! already has; the wallet's own source stays its balance source.
use crate::btcpay::args::{opt_str, req_str};
use crate::{lifecycle, Operation};
use kassiber_envelope::{dict, get, py_str, s, type_name, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::Connection;
use std::path::{Component, Path, PathBuf};

type Result<T> = std::result::Result<T, AppError>;

pub const NETWORKS: [&str; 3] = ["bitcoin", "liquid", "lightning"];
const EXPORTS_KEY: &str = "bullbitcoin_wallet_exports";

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// `normalize_bullbitcoin_wallet_network`.
pub fn network(value: &Value) -> Result<String> {
    let raw = match value {
        Value::Null => None,
        other => Some(kassiber_pytext::strip(&py_str(other)).to_owned()),
    };
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Err(validation("Bull Bitcoin wallet network cannot be empty"));
    };
    let lowered = kassiber_pytext::lower(&raw);
    let normalized = match lowered.as_str() {
        "btc" | "onchain" | "chain" => "bitcoin",
        "lbtc" | "liquidv1" => "liquid",
        "ln" => "lightning",
        other => other,
    };
    if !NETWORKS.contains(&normalized) {
        return Err(validation(format!(
            "Unsupported Bull Bitcoin wallet network '{}'",
            py_str(value)
        ))
        .with_hint("Supported networks: bitcoin, liquid, lightning."));
    }
    Ok(normalized.to_owned())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `os.path.expanduser`.
fn expand_user(raw: &str) -> PathBuf {
    if raw == "~" {
        return home().unwrap_or_else(|| raw.into());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home().map_or_else(|| raw.into(), |h| h.join(rest));
    }
    if raw.starts_with('~') {
        if let Ok(context) = kassiber_paths::Context::from_env() {
            if let Ok(expanded) = context.expand_user(Path::new(raw)) {
                return expanded;
            }
        }
    }
    PathBuf::from(raw)
}

/// `os.path.abspath(os.path.expanduser(raw))`: lexical, no symlinks.
pub fn abspath(raw: &str) -> Result<String> {
    let expanded = expand_user(raw);
    let joined = if expanded.is_absolute() {
        expanded.clone()
    } else {
        std::env::current_dir()
            .map_err(|_| AppError::new("internal_error", "Could not resolve source file"))?
            .join(&expanded)
    };
    let mut normalized = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    let text = normalized.to_string_lossy().into_owned();
    // POSIX keeps exactly two leading slashes.
    let source = expanded.to_string_lossy();
    if source.starts_with("//") && !source.starts_with("///") {
        return Ok(format!("/{text}"));
    }
    Ok(text)
}

/// `str(Path(raw).expanduser().resolve())`: symlinks resolved, missing
/// trailing components kept (`os.path.realpath`, non-strict).
pub fn resolve(raw: &str) -> Result<String> {
    let expanded = expand_user(raw);
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|_| AppError::new("internal_error", "Could not resolve source file"))?
            .join(expanded)
    };
    let mut pending: Vec<std::ffi::OsString> = joined
        .components()
        .rev()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_owned()),
            Component::ParentDir => Some("..".into()),
            _ => None,
        })
        .collect();
    let mut resolved = PathBuf::from("/");
    let mut hops = 0;
    while let Some(part) = pending.pop() {
        if part == ".." {
            resolved.pop();
            continue;
        }
        let candidate = resolved.join(&part);
        match std::fs::symlink_metadata(&candidate) {
            Ok(meta) if meta.file_type().is_symlink() => {
                hops += 1;
                if hops > 40 {
                    resolved = candidate;
                    continue;
                }
                let target = std::fs::read_link(&candidate).map_err(|_| {
                    AppError::new("internal_error", "Could not resolve source file")
                })?;
                if target.is_absolute() {
                    resolved = PathBuf::from("/");
                }
                for c in target.components().rev() {
                    match c {
                        Component::Normal(p) => pending.push(p.to_owned()),
                        Component::ParentDir => pending.push("..".into()),
                        _ => {}
                    }
                }
            }
            _ => resolved = candidate,
        }
    }
    Ok(resolved.to_string_lossy().into_owned())
}

fn networks(args: &Value) -> Result<Vec<String>> {
    match get(args, "networks").unwrap_or(&Value::Null) {
        Value::Null => match opt_str(args, "network")? {
            Some(single) => Ok(vec![network(&Value::Str(single))?]),
            None => Ok(NETWORKS.iter().map(|n| (*n).to_owned()).collect()),
        },
        Value::List(items) => {
            let mut out: Vec<String> = Vec::new();
            for item in items {
                let n = network(item)?;
                if !out.contains(&n) {
                    out.push(n);
                }
            }
            if out.is_empty() {
                return Err(validation(
                    "Select at least one Bull Bitcoin wallet network",
                ));
            }
            Ok(out)
        }
        other => Err(validation("Bull Bitcoin wallet networks must be an array")
            .with_details(dict([("type", s(type_name(other)))]))),
    }
}

fn routes(args: &Value) -> Result<Vec<(PyStr, String)>> {
    match get(args, "routes").unwrap_or(&Value::Null) {
        Value::Null => {
            let target = opt_str(args, "target_wallet")?.ok_or_else(|| {
                validation("Existing-wallet Bull Bitcoin setup requires routes")
                    .with_hint("Choose which Kassiber wallet each Bull export network maps into.")
            })?;
            Ok(networks(args)?
                .into_iter()
                .map(|n| (target.clone(), n))
                .collect())
        }
        Value::List(items) if !items.is_empty() => {
            let mut out: Vec<(PyStr, String)> = Vec::new();
            for item in items {
                if !matches!(item, Value::Dict(_)) {
                    return Err(validation(
                        "Existing-wallet Bull Bitcoin routes must be objects",
                    ));
                }
                let wallet = match opt_str(item, "wallet")? {
                    Some(w) => Some(w),
                    None => opt_str(item, "target_wallet")?,
                };
                let Some(wallet) = wallet else {
                    return Err(validation(
                        "Existing-wallet Bull Bitcoin routes require a wallet",
                    ));
                };
                let n = network(&Value::Str(req_str(
                    item,
                    "network",
                    "Bull Bitcoin network",
                )?))?;
                if !out.contains(&(wallet.clone(), n.clone())) {
                    out.push((wallet, n));
                }
            }
            Ok(out)
        }
        _ => Err(validation(
            "Existing-wallet Bull Bitcoin routes must be a non-empty array",
        )),
    }
}

fn wallet_get(conn: &Connection, profile_id: &str, reference: &str) -> Result<Value> {
    lifecycle::execute(
        conn,
        Operation::Get {
            profile_id: profile_id.into(),
            wallet: reference.into(),
        },
    )
}

fn update_routes(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    routes: Vec<Value>,
    clear: Vec<String>,
) -> Result<Value> {
    let encoded = kassiber_pyjson::dumps(&Value::List(routes), &Default::default())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    lifecycle::execute(
        conn,
        Operation::Update {
            profile_id: profile_id.into(),
            wallet: wallet_id.into(),
            label: None,
            account: None,
            config: serde_json::json!({EXPORTS_KEY: serde_json::from_str::<serde_json::Value>(&encoded).expect("routes")}),
            clear,
        },
    )
}

fn scope(conn: &Connection, args: &Value) -> Result<(String, String, String)> {
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let id = |v: &Value, key: &str| py_str(get(v, key).unwrap_or(&Value::Null));
    Ok((
        id(&workspace, "id"),
        id(&profile, "id"),
        id(&profile, "label"),
    ))
}

fn text(v: &PyStr) -> String {
    kassiber_chain::btcpay::url::from_py(v)
}

/// `ui.connections.bullbitcoin_wallet.create`.
pub fn create(conn: &Connection, args: &Value) -> Result<Value> {
    let label = text(&req_str(args, "label", "Connection label")?);
    let source = match opt_str(args, "source_file")? {
        Some(v) => Some(v),
        None => opt_str(args, "file")?,
    };
    let source = source.ok_or_else(|| {
        validation("Bull Bitcoin wallet setup requires a CSV file")
            .with_hint("Choose the unified wallet export CSV from Bull Bitcoin Wallet.")
    })?;
    let source_file = resolve(&text(&source))?;
    let mode = opt_str(args, "mode")?
        .map(|m| kassiber_pytext::lower(kassiber_pytext::strip(&text(&m))))
        .unwrap_or_else(|| "wallet_sources".into());
    if ![
        "wallet_sources",
        "existing_wallets",
        "map_existing",
        "provenance",
    ]
    .contains(&mode.as_str())
    {
        return Err(validation(format!(
            "Unsupported Bull Bitcoin wallet setup mode '{mode}'"
        )));
    }
    let empty = dict::<&str>([]);
    if mode != "wallet_sources" {
        let chosen = routes(args)?;
        let (_, profile, _) = scope(conn, &empty)?;
        for (wallet, _) in &chosen {
            wallet_get(conn, &profile, &text(wallet))?;
        }
        let chosen = routes(args)?;
        let mut updated = Vec::new();
        for (wallet_ref, n) in &chosen {
            let wallet = wallet_get(conn, &profile, &text(wallet_ref))?;
            let mut existing =
                match get(get(&wallet, "config").unwrap_or(&Value::Null), EXPORTS_KEY) {
                    Some(Value::List(items)) => items.clone(),
                    _ => Vec::new(),
                };
            let next = dict([
                ("source_file", s(source_file.as_str())),
                ("network", s(n.as_str())),
            ]);
            if !existing.contains(&next) {
                existing.push(next);
            }
            updated.push(update_routes(
                conn,
                &profile,
                &py_str(get(&wallet, "id").unwrap_or(&Value::Null)),
                existing,
                Vec::new(),
            )?);
        }
        return Ok(dict([
            ("mode", s("existing_wallets")),
            ("source_file", s(source_file.as_str())),
            ("wallet", updated.first().cloned().unwrap_or(Value::Null)),
            ("wallets", Value::List(updated)),
            (
                "routes",
                Value::List(
                    chosen
                        .into_iter()
                        .map(|(w, n)| dict([("wallet", Value::Str(w)), ("network", s(n))]))
                        .collect(),
                ),
            ),
        ]));
    }
    let (workspace, profile, profile_label) = scope(conn, &empty)?;
    let chosen = networks(args)?;
    let labels: Vec<String> = if chosen.len() == 1 {
        vec![label.clone()]
    } else {
        chosen
            .iter()
            .map(|n| {
                let name = match n.as_str() {
                    "bitcoin" => "Bitcoin",
                    "liquid" => "Liquid",
                    _ => "Lightning",
                };
                format!("{label} - {name}")
            })
            .collect()
    };
    let mut taken = Vec::new();
    for candidate in &labels {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM wallets WHERE profile_id = ? AND label = ?",
                [profile.as_str(), candidate.as_str()],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists && !taken.contains(candidate) {
            taken.push(candidate.clone());
        }
    }
    if !taken.is_empty() {
        taken.sort();
        return Err(AppError::new(
            "conflict",
            format!(
                "Wallet '{}' already exists in profile '{profile_label}'",
                taken[0]
            ),
        )
        .with_hint("Choose a different connection label.")
        .with_details(dict([(
            "existing_labels",
            Value::List(taken.into_iter().map(s).collect()),
        )])));
    }
    let mut wallets = Vec::new();
    for (label, n) in labels.iter().zip(&chosen) {
        let mut config = serde_json::json!({
            "source_file": source_file,
            "source_format": "bullbitcoin_wallet_csv",
            "bullbitcoin_wallet_network": n,
        });
        if n == "bitcoin" || n == "liquid" {
            config["chain"] = n.as_str().into();
        }
        wallets.push(lifecycle::execute(
            conn,
            Operation::Create {
                workspace_id: workspace.clone(),
                profile_id: profile.clone(),
                label: label.clone(),
                kind: "bullbitcoin".into(),
                account: None,
                config,
            },
        )?);
    }
    Ok(dict([
        ("mode", s("wallet_sources")),
        ("source_file", s(source_file)),
        ("wallet", wallets[0].clone()),
        ("wallets", Value::List(wallets)),
        ("networks", Value::List(chosen.into_iter().map(s).collect())),
    ]))
}

/// `wallets attach-bullbitcoin-wallet` (`attach_bullbitcoin_wallet_export_to_wallet`).
pub fn attach(conn: &Connection, args: &Value) -> Result<Value> {
    let n = network(get(args, "network").unwrap_or(&Value::Null))?;
    let file = abspath(&py_str(get(args, "file").unwrap_or(&Value::Null)))?;
    let (_, profile, _) = scope(conn, args)?;
    let reference = py_str(get(args, "wallet").unwrap_or(&Value::Null));
    let wallet = crate::resolve(conn, &profile, &reference)?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id = ?",
            [&wallet.id],
            |r| r.get(0),
        )
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let config = kassiber_pytext::loads(raw.as_deref().unwrap_or("{}")).unwrap_or(Value::Null);
    let mut routes: Vec<Value> = Vec::new();
    if let Value::Dict(_) = config {
        match get(&config, EXPORTS_KEY) {
            None | Some(Value::Null) => {}
            Some(Value::List(items)) => {
                for item in items {
                    if !matches!(item, Value::Dict(_)) {
                        return Err(validation(
                            "Bull Bitcoin wallet export routes must be objects",
                        ));
                    }
                    let source = get(item, "source_file")
                        .filter(|v| !matches!(v, Value::Null))
                        .map(|v| kassiber_pytext::strip(&py_str(v)).to_owned())
                        .filter(|v| !v.is_empty())
                        .ok_or_else(|| {
                            validation("Bull Bitcoin wallet export routes require source_file")
                        })?;
                    let route = dict([
                        ("source_file", s(abspath(&source)?)),
                        (
                            "network",
                            s(network(get(item, "network").unwrap_or(&Value::Null))?),
                        ),
                    ]);
                    if !routes.contains(&route) {
                        routes.push(route);
                    }
                }
            }
            Some(_) => {
                return Err(validation(
                    "Bull Bitcoin wallet export config must be a list",
                ))
            }
        }
    }
    let next = dict([
        ("source_file", s(file.as_str())),
        ("network", s(n.as_str())),
    ]);
    if !routes.contains(&next) {
        routes.push(next);
    }
    let updated = update_routes(conn, &profile, &wallet.id, routes.clone(), Vec::new())?;
    Ok(dict([
        ("wallet", updated),
        ("source_file", s(file)),
        ("network", s(n)),
        ("routes", Value::List(routes)),
    ]))
}
