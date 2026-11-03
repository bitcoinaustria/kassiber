//! Local backend configuration. Credentials live only in the open book and
//! private runtime state; ordinary projections use an explicit allowlist.
use super::*;
pub use crate::backend_runtime::Config;
use crate::backend_runtime::{
    canonical, manual_mode, normalize_chain, normalize_network, tombstones, BUILTINS, KINDS,
    RESERVED,
};
use kassiber_store::rusqlite::{params, types::Value as SqlValue};
use std::collections::{BTreeMap, BTreeSet};
#[path = "backend_desktop.rs"]
mod desktop;
/// `registered_capabilities(kind).to_wire_dict()` for a Lightning adapter kind.
pub use desktop::capabilities as lightning_capabilities;
#[path = "onboarding.rs"]
mod onboarding;
#[path = "backend_output.rs"]
mod output;

fn s(value: impl AsRef<str>) -> Value {
    kassiber_envelope::s(value.as_ref())
}
trait TextKey {
    fn as_str(&self) -> Option<&str>;
}
impl TextKey for kassiber_pyjson::Key {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(v) => v.as_str(),
            _ => None,
        }
    }
}
const SYNC_KINDS: &[&str] = &[
    "bitcoinrpc",
    "custom",
    "electrum",
    "esplora",
    "liquid-esplora",
    "mempool",
];
const CONFIG_FIELDS: &[&str] = &[
    "certificate",
    "commando_peer_id",
    "cookiefile",
    "display_name",
    "insecure",
    "lightning_cli",
    "lightning_dir",
    "password",
    "rpc_file",
    "silent_payments",
    "silent_payment_scan_file",
    "silent_payment_scan_path",
    "username",
    "walletprefix",
];
const CLEAR_FIELDS: &[&str] = &[
    "auth_header",
    "token",
    "tor_proxy",
    "notes",
    "insecure",
    "certificate",
    "commando_peer_id",
    "cookiefile",
    "display_name",
    "lightning_cli",
    "lightning_dir",
    "rpc_file",
    "username",
    "password",
    "silent_payments",
    "silent_payment_scan_file",
    "silent_payment_scan_path",
    "walletprefix",
];
fn value<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
fn string(v: &Value) -> Option<String> {
    if matches!(v, Value::Null) {
        None
    } else {
        let t = py_str(v);
        let t = kassiber_pytext::strip(&t);
        (!t.is_empty()).then(|| t.to_owned())
    }
}
fn object(raw: &Value) -> Value {
    string(raw)
        .and_then(|v| kassiber_pytext::loads(&v).ok())
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or_else(|| dict::<&str>([]))
}
fn boolean(v: &Value) -> Result<Value> {
    match string(v).unwrap_or_default().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Value::Bool(true)),
        "0" | "false" | "no" | "off" => Ok(Value::Bool(false)),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {}", py_str(v)),
        )),
    }
}
fn normalize_config(config: &Value, patch: bool) -> Result<(Value, BTreeSet<String>)> {
    let mut result = dict::<&str>([]);
    let mut cleared = BTreeSet::new();
    if let Value::Dict(fields) = config {
        for (key, v) in fields {
            let Some(key) = string(&s(key.as_str().unwrap_or_default())) else {
                continue;
            };
            let key = canonical(&key);
            if RESERVED.contains(&key.as_str()) {
                continue;
            }
            if ["insecure", "silent_payments"].contains(&key.as_str()) {
                if patch && string(v).is_none() {
                    cleared.insert(key);
                    continue;
                }
                replace(
                    &mut result,
                    &key,
                    if matches!(v, Value::Null) {
                        Value::Bool(false)
                    } else {
                        boolean(v)?
                    },
                );
            } else if let Some(v) = string(v) {
                replace(&mut result, &key, s(v));
            } else if patch {
                cleared.insert(key);
            }
        }
    }
    Ok((result, cleared))
}
fn remove(v: &mut Value, key: &str) {
    if let Value::Dict(fields) = v {
        fields.retain(|(k, _)| *k != key.into());
    }
}
fn row_backend(row: &Value) -> Value {
    let mut result = dict::<&str>([]);
    for key in [
        "name",
        "kind",
        "chain",
        "network",
        "url",
        "batch_size",
        "auth_header",
        "token",
        "timeout",
        "tor_proxy",
        "notes",
    ] {
        let v = value(row, key);
        replace(
            &mut result,
            key,
            if matches!(
                key,
                "chain" | "network" | "auth_header" | "token" | "tor_proxy" | "notes"
            ) && !truthy(v)
            {
                s("")
            } else {
                v.clone()
            },
        );
    }
    replace(&mut result, "source", s("database"));
    for key in ["created_at", "updated_at"] {
        replace(&mut result, key, value(row, key).clone());
    }
    if let Value::Dict(fields) = object(value(row, "config_json")) {
        for (k, v) in fields {
            if let Some(k) = k.as_str() {
                replace(&mut result, k, v);
            }
        }
    }
    result
}
fn db_backend(conn: &Connection, name: &str, hint: &str) -> Result<Value> {
    rows(conn, "SELECT * FROM backends WHERE name=?", [name])?
        .first()
        .map(row_backend)
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                format!("Backend '{name}' not found in the database"),
            )
            .with_hint(hint)
        })
}
fn raw_backend(raw: &crate::backend_runtime::Backend) -> Value {
    dict(raw.iter().map(|(k, v)| (k.as_str(), s(v))))
}
pub(crate) fn merge(conn: &Connection, config: &mut Config) -> Result<()> {
    let removed = tombstones(conn)?
        .difference(&config.dotenv)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut view = BTreeMap::new();
    for (name, raw) in &config.runtime {
        if !removed.contains(name) || config.env_names.contains(name) {
            let mut backend = raw_backend(raw);
            if BUILTINS.contains(&name.as_str())
                && raw.get("batch_size").is_some_and(|v| v == "100")
                && !config
                    .dotenv_fields
                    .get(name)
                    .is_some_and(|fields| fields.contains("batch_size"))
                && !config
                    .env_fields
                    .get(name)
                    .is_some_and(|fields| fields.contains("batch_size"))
            {
                replace(&mut backend, "batch_size", Value::Int(100.into()));
            }
            view.insert(name.clone(), backend);
        }
    }
    for row in rows(conn, "SELECT * FROM backends", [])? {
        let name = text(&row, "name").to_lowercase();
        let fields = config.env_fields.get(&name);
        if removed.contains(&name) && fields.is_none() {
            continue;
        }
        let mut b = row_backend(&row);
        if let Some(fields) = fields {
            for field in fields {
                replace(
                    &mut b,
                    field,
                    config
                        .runtime
                        .get(&name)
                        .and_then(|r| r.get(field))
                        .map_or_else(|| s(""), s),
                );
            }
            replace(&mut b, "source", s("environment"));
        }
        view.insert(name, b);
    }
    let manual = manual_mode(conn)?;
    if config.env_default {
        if !view.contains_key(&config.default_backend) {
            if manual && removed.contains(&config.default_backend) {
                config.default_backend.clear();
            } else {
                return Err(AppError::new(
                    "config_error",
                    format!(
                        "Environment default backend '{}' is not configured",
                        config.default_backend
                    ),
                )
                .with_hint(
                    "Define that backend in the current process or remove the env override.",
                ));
            }
        }
    } else if let Some(stored) = setting(conn, "default_backend")?.filter(|v| !v.is_empty()) {
        if !view.contains_key(&stored) {
            return Err(AppError::new("config_error",format!("Stored default backend '{stored}' is not configured")).with_hint("Run `kassiber backends set-default <name>` with a valid backend, or `backends clear-default` to fall back to the env default."));
        }
        config.default_backend = stored;
    } else if manual && !view.contains_key(&config.default_backend) {
        config.default_backend.clear();
    }
    config.view = view;
    Ok(())
}
fn names(conn: &Connection) -> Result<BTreeSet<String>> {
    Ok(rows(conn, "SELECT name FROM backends ORDER BY name", [])?
        .iter()
        .map(|r| text(r, "name"))
        .collect())
}
fn shapes(conn: &Connection, config: &Config) -> Result<BTreeMap<String, (String, String)>> {
    let mut shapes = BTreeMap::new();
    for row in rows(conn, "SELECT name,chain,kind FROM backends", [])? {
        shapes.insert(
            text(&row, "name"),
            (
                string(value(&row, "chain")).unwrap_or_else(|| "bitcoin".into()),
                text(&row, "kind"),
            ),
        );
    }
    for (name, b) in &config.view {
        shapes.insert(
            name.clone(),
            (
                string(value(b, "chain")).unwrap_or_else(|| "bitcoin".into()),
                text(b, "kind"),
            ),
        );
    }
    Ok(shapes)
}
fn fallback(
    names: &BTreeSet<String>,
    shapes: &BTreeMap<String, (String, String)>,
    chain: &str,
) -> Result<String> {
    if names.is_empty() {
        return Err(AppError::new("config_error","No backends are configured").with_hint("Create a backend with `kassiber backends create`, or seed one through your dotenv bootstrap config."));
    }
    let suitable: Vec<_> = names
        .iter()
        .filter(|n| {
            shapes
                .get(*n)
                .is_some_and(|(c, k)| c == chain && SYNC_KINDS.contains(&k.to_lowercase().as_str()))
        })
        .collect();
    if suitable.is_empty() {
        return Err(AppError::new(
            "conflict",
            format!("No {chain} backend that wallets can sync against is configured"),
        )
        .with_hint(format!(
            "Create a {chain} backend that supports wallet sync first."
        )));
    }
    Ok(suitable
        .iter()
        .find(|n| !BUILTINS.contains(&n.as_str()))
        .or_else(|| suitable.iter().find(|n| n.as_str() == "mempool"))
        .unwrap_or(&suitable[0])
        .to_string())
}
fn save_tombstones(conn: &Connection, names: BTreeSet<String>) -> Result<()> {
    set_setting(
        conn,
        "bootstrap_backend_tombstones",
        &super::networks::json(
            &Value::List(names.into_iter().map(s).collect()),
            false,
            false,
        )?,
    )
}
fn kind(v: &Value) -> Result<String> {
    let k = py_str(v);
    if !KINDS.contains(&k.to_lowercase().as_str()) {
        return Err(
            AppError::new("validation", format!("Unsupported backend kind '{k}'"))
                .with_hint(format!("Choose one of: {}", KINDS.join(", "))),
        );
    }
    Ok(k.to_lowercase())
}
fn sql_value(v: &Value) -> Result<SqlValue> {
    Ok(match v {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Int(v) => SqlValue::Integer(i64::try_from(v.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Str(_) => SqlValue::Text(py_str(v)),
        _ => {
            return Err(AppError::new(
                "validation",
                "Unsupported backend field type",
            ))
        }
    })
}
fn create(conn: &Connection, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
    if name.is_empty() {
        return Err(AppError::new("validation", "Backend name is required"));
    }
    let kind = kind(value(args, "kind"))?;
    let Some(url) = string(value(args, "url")) else {
        return Err(AppError::new("validation", "Backend url is required"));
    };
    let mut chain = value(args, "chain").clone();
    if truthy(&chain) {
        chain = s(normalize_chain(&py_str(&chain))?);
    }
    let mut network = value(args, "network").clone();
    if truthy(&network) {
        network = s(normalize_network(
            string(&chain).as_deref(),
            &py_str(&network),
        )?);
    }
    positive(args)?;
    let (config, _) = normalize_config(value(args, "config"), false)?;
    if names(conn)?.contains(&name) {
        return Err(
            AppError::new("conflict", format!("Backend '{name}' already exists"))
                .with_hint("Use `kassiber backends update` to change an existing backend."),
        );
    }
    let ts = sources.now(conn)?;
    let mut values = vec![s(&name), s(&kind), chain.clone(), network, s(url)];
    for key in ["auth_header", "token", "batch_size", "timeout", "tor_proxy"] {
        values.push(value(args, key).clone());
    }
    values.extend([
        s(super::networks::json(&config, false, true)?),
        value(args, "notes").clone(),
        s(&ts),
        s(ts),
    ]);
    conn.execute("INSERT INTO backends(name,kind,chain,network,url,auth_header,token,batch_size,timeout,tor_proxy,config_json,notes,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",rusqlite::params_from_iter(values.iter().map(sql_value).collect::<Result<Vec<_>>>()?)).map_err(sql_error)?;
    let mut removed = tombstones(conn)?;
    if removed.remove(&name) {
        save_tombstones(conn, removed)?;
    }
    if manual_mode(conn)?
        && setting(conn, "default_backend")?.is_none_or(|v| v.is_empty())
        && SYNC_KINDS.contains(&kind.as_str())
        && string(&chain).unwrap_or_else(|| "bitcoin".into()) == "bitcoin"
    {
        set_setting(conn, "default_backend", &name)?;
    }
    db_backend(conn, &name, "")
}
fn positive(args: &Value) -> Result<()> {
    for (key, message) in [
        ("batch_size", "Backend batch size must be positive"),
        ("timeout", "Backend timeout must be positive"),
    ] {
        if optional(args, key).is_some_and(|v| match v {
            Value::Int(n) => n <= &0.into(),
            Value::Bool(v) => !*v,
            _ => false,
        }) {
            return Err(AppError::new("validation", message));
        }
    }
    Ok(())
}
fn update(conn: &Connection, args: &Value, sources: &dyn Sources) -> Result<Value> {
    let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
    let row = rows(conn, "SELECT * FROM backends WHERE name=?", [&name])?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new(
                "not_found",
                format!("Backend '{name}' not found in the database"),
            )
            .with_hint(
                "Only DB-backed backends can be updated. Use `kassiber backends create` first.",
            )
        })?;
    let clear = match value(args, "clear") {
        Value::List(v) => v.iter().map(py_str).collect::<BTreeSet<_>>(),
        _ => BTreeSet::new(),
    };
    let keys = [
        "kind",
        "url",
        "chain",
        "network",
        "auth_header",
        "token",
        "batch_size",
        "timeout",
        "tor_proxy",
        "config",
        "notes",
    ];
    if keys.iter().all(|k| optional(args, k).is_none()) && clear.is_empty() {
        return Err(AppError::new("validation","backends update requires at least one field to change").with_hint("Pass one or more of --kind, --url, --chain, --network, --auth-header, --token, --batch-size, --timeout, --tor-proxy, --insecure, --certificate, --cookiefile, --display-name, --username, --password, --wallet-prefix, --notes, or --clear <field>"));
    }
    let unsupported: Vec<_> = clear
        .iter()
        .filter(|f| !CLEAR_FIELDS.contains(&f.as_str()))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        let mut choices: Vec<_> = CLEAR_FIELDS
            .iter()
            .map(|k| {
                if *k == "walletprefix" {
                    "wallet-prefix".into()
                } else {
                    k.replace('_', "-")
                }
            })
            .collect();
        choices.sort();
        return Err(AppError::new(
            "validation",
            format!(
                "Unsupported backend clear field(s): {}",
                unsupported.join(", ")
            ),
        )
        .with_hint(format!("Choose one of: {}", choices.join(", "))));
    }
    let conflicts: Vec<_> = ["auth_header", "notes", "token", "tor_proxy"]
        .iter()
        .filter(|k| clear.contains(**k) && optional(args, k).is_some())
        .copied()
        .collect();
    if !conflicts.is_empty() {
        return Err(AppError::new(
            "validation",
            format!(
                "Cannot set and clear backend field(s) in the same update: {}",
                conflicts.join(", ")
            ),
        ));
    }
    let mut merged = row.clone();
    if let Some(v) = optional(args, "kind") {
        replace(&mut merged, "kind", s(kind(v)?));
    }
    if let Some(v) = optional(args, "url") {
        let Some(url) = string(v) else {
            return Err(AppError::new("validation", "Backend url cannot be empty"));
        };
        replace(&mut merged, "url", s(url));
    }
    if let Some(v) = optional(args, "chain") {
        replace(&mut merged, "chain", s(normalize_chain(&py_str(v))?));
    }
    if let Some(v) = optional(args, "network") {
        let chain = string(value(&merged, "chain"));
        replace(
            &mut merged,
            "network",
            s(normalize_network(chain.as_deref(), &py_str(v))?),
        );
    }
    positive(args)?;
    let (patch, mut clears) = normalize_config(value(args, "config"), true)?;
    clears.extend(
        clear
            .iter()
            .filter(|k| CONFIG_FIELDS.contains(&k.as_str()))
            .cloned(),
    );
    let conflicts: Vec<_> = clears
        .iter()
        .filter(|k| get(&patch, k).is_some())
        .cloned()
        .collect();
    if !conflicts.is_empty() {
        return Err(AppError::new(
            "validation",
            format!(
                "Cannot set and clear backend config field(s) in the same update: {}",
                conflicts.join(", ")
            ),
        ));
    }
    let mut config = object(value(&row, "config_json"));
    for k in clears {
        remove(&mut config, &k);
    }
    if let Value::Dict(fields) = patch {
        for (k, v) in fields {
            replace(&mut config, k.as_str().unwrap(), v);
        }
    }
    replace(
        &mut merged,
        "config_json",
        s(super::networks::json(&config, false, true)?),
    );
    for k in [
        "auth_header",
        "token",
        "batch_size",
        "timeout",
        "tor_proxy",
        "notes",
    ] {
        if clear.contains(k) {
            replace(&mut merged, k, Value::Null);
        } else if let Some(v) = optional(args, k) {
            replace(&mut merged, k, v.clone());
        }
    }
    replace(&mut merged, "updated_at", s(sources.now(conn)?));
    let columns = [
        "kind",
        "url",
        "chain",
        "network",
        "auth_header",
        "token",
        "batch_size",
        "timeout",
        "tor_proxy",
        "config_json",
        "notes",
        "updated_at",
        "name",
    ];
    conn.execute("UPDATE backends SET kind=?,url=?,chain=?,network=?,auth_header=?,token=?,batch_size=?,timeout=?,tor_proxy=?,config_json=?,notes=?,updated_at=? WHERE name=?",rusqlite::params_from_iter(columns.iter().map(|k|sql_value(value(&merged,k))).collect::<Result<Vec<_>>>()?)).map_err(sql_error)?;
    for table in ["wallet_utxos", "wallet_utxo_refreshes"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE backend_name=?"),
            [&name],
        )
        .map_err(sql_error)?;
    }
    db_backend(conn, &name, "")
}
fn wallet_refs(conn: &Connection, name: &str) -> Result<Value> {
    let mut refs = vec![];
    for row in rows(conn,"SELECT w.label AS wallet_label,p.label AS profile_label,ws.label AS workspace_label,w.config_json FROM wallets w JOIN profiles p ON p.id=w.profile_id JOIN workspaces ws ON ws.id=w.workspace_id ORDER BY ws.label,p.label,w.label",[])? {
        let config=object(value(&row,"config_json"));
        let direct=string(value(&config,"backend")).as_deref()==Some(name);
        let routes=match value(&config,"btcpay_provenance") {Value::List(v)=>v.iter().any(|r|matches!(r,Value::Dict(_))&&string(value(r,"backend")).is_some_and(|v|v.to_lowercase()==name)),_=>false};
        if direct||routes {refs.push(s(format!("{}/{}/{}",text(&row,"workspace_label"),text(&row,"profile_label"),text(&row,"wallet_label"))));}
    }
    Ok(Value::List(refs))
}
fn delete(conn: &Connection, args: &Value, config: &Config) -> Result<Value> {
    let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
    if config.dotenv.contains(&name) {
        return Err(AppError::new(
            "conflict",
            format!("Backend '{name}' is defined in your env file"),
        )
        .with_hint("Remove its KASSIBER_BACKEND_... entries from the env file and restart."));
    }
    if config.env_names.contains(&name) {
        return Err(AppError::new(
            "conflict",
            format!("Backend '{name}' is overridden by the process environment"),
        )
        .with_hint("Remove its KASSIBER_BACKEND_... process variables and restart."));
    }
    if config.env_default && config.default_backend == name {
        return Err(AppError::new(
            "conflict",
            format!("Backend '{name}' is forced as the default by the process environment"),
        )
        .with_hint("Remove KASSIBER_DEFAULT_BACKEND and restart."));
    }
    if !names(conn)?.contains(&name) {
        return Err(AppError::new("not_found",format!("Backend '{name}' not found in the database")).with_hint("Only DB-backed backends can be deleted; env-sourced backends are removed from your .env file instead."));
    }
    let refs = wallet_refs(conn, &name)?;
    if truthy(&refs) {
        let n = if let Value::List(v) = &refs {
            v.len()
        } else {
            0
        };
        return Err(AppError::new(
            "conflict",
            format!("Backend '{name}' is still used by {n} wallet(s)"),
        )
        .with_hint("Reassign the listed wallets to another backend before deleting this one.")
        .with_details(dict([("wallet_refs", refs)])));
    }
    let mut payload = dict([("name", s(&name)), ("deleted", Value::Bool(true))]);
    if setting(conn, "default_backend")?.as_deref() == Some(&name) {
        let mut remaining = names(conn)?;
        remaining.remove(&name);
        remaining.extend(config.view.keys().filter(|n| *n != &name).cloned());
        if remaining.is_empty() {
            if !manual_mode(conn)? {
                return Err(AppError::new(
                    "conflict",
                    format!("Backend '{name}' has no persistable replacement backend"),
                )
                .with_hint("Create another backend before deleting this one."));
            }
            conn.execute(
                "DELETE FROM settings WHERE key IN ('default_backend','bootstrap_default_backend')",
                [],
            )
            .map_err(sql_error)?;
        } else {
            let shapes = shapes(conn, config)?;
            let chain = shapes.get(&name).map_or("bitcoin", |(c, _)| c.as_str());
            let replacement = fallback(&remaining, &shapes, chain)?;
            set_setting(conn, "default_backend", &replacement)?;
            replace(&mut payload, "default_backend", s(replacement));
        }
    }
    conn.execute("DELETE FROM backends WHERE name=?", [&name])
        .map_err(sql_error)?;
    let mut removed = tombstones(conn)?;
    removed.insert(name.clone());
    save_tombstones(conn, removed)?;
    let routes = rows(
        conn,
        "DELETE FROM btcpay_account_routes WHERE backend_name=? RETURNING id,profile_id",
        [&name],
    )?;
    for route in &routes {
        for table in ["freshness_jobs", "freshness_source_states"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE profile_id=? AND source_key=?"),
                params![
                    text(route, "profile_id"),
                    format!("btcpay_provenance:account:{}", text(route, "id"))
                ],
            )
            .map_err(sql_error)?;
        }
    }
    if !routes.is_empty() {
        replace(
            &mut payload,
            "detached_btcpay_account_routes",
            Value::Int(routes.len().into()),
        );
    }
    Ok(payload)
}
fn set_default(conn: &Connection, args: &Value, config: &mut Config) -> Result<Value> {
    let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
    if !config.view.contains_key(&name) {
        return Err(
            AppError::new("not_found", format!("Backend '{name}' is not configured"))
                .with_hint("Use `kassiber backends list` to see available backends."),
        );
    }
    if !names(conn)?.contains(&name) {
        return Err(AppError::new("conflict",format!("Backend '{name}' only exists as an environment override and cannot be stored as the canonical default")).with_hint("Create or import that backend into SQLite first, then run `kassiber backends set-default` again."));
    }
    set_setting(conn, "default_backend", &name)?;
    config.default_backend = name.clone();
    Ok(dict([("default_backend", s(name))]))
}
fn clear_default(conn: &Connection, config: &mut Config) -> Result<Value> {
    let available = names(conn)?;
    if available.is_empty() && manual_mode(conn)? {
        conn.execute(
            "DELETE FROM settings WHERE key IN ('default_backend','bootstrap_default_backend')",
            [],
        )
        .map_err(sql_error)?;
        config.default_backend.clear();
        return Ok(dict([
            ("default_backend", Value::Null),
            ("cleared", Value::Bool(true)),
        ]));
    }
    let previous = setting(conn, "default_backend")?;
    let stored = setting(conn, "bootstrap_default_backend")?;
    let mut payload = dict::<&str>([]);
    let name = if stored.as_ref().is_some_and(|n| available.contains(n)) {
        stored.clone().unwrap()
    } else {
        let n = fallback(&available, &shapes(conn, config)?, "bitcoin")?;
        set_setting(conn, "bootstrap_default_backend", &n)?;
        replace(
            &mut payload,
            "notice",
            dict([
                ("code", s("default_backend_fallback")),
                (
                    "message",
                    s(format!(
                        "Default backend fell back to '{n}' because '{}' is no longer configured.",
                        stored.as_deref().unwrap_or("None")
                    )),
                ),
                ("previous_default", previous.map_or(Value::Null, s)),
                ("missing_default", stored.map_or(Value::Null, s)),
                ("default_backend", s(&n)),
            ]),
        );
        n
    };
    set_setting(conn, "default_backend", &name)?;
    config.default_backend = name.clone();
    let mut result = dict([("default_backend", s(name)), ("cleared", Value::Bool(true))]);
    if let Some(v) = get(&payload, "notice") {
        replace(&mut result, "notice", v.clone());
    }
    Ok(result)
}
impl Config {
    pub fn resolve_lightning(&self, conn: &Connection, name: &str) -> Result<Value> {
        let mut current = self.clone();
        merge(conn, &mut current)?;
        if let Some(backend) = current.view.get(&name.trim().to_lowercase()) {
            return Ok(backend.clone());
        }
        db_backend(
            conn,
            &name.to_lowercase(),
            "Use `kassiber backends list` to see configured backends, or `kassiber backends create` to add one.",
        )
    }

    /// Copy-on-success keeps runtime default selection consistent with rollback.
    pub fn execute(
        &mut self,
        conn: &Connection,
        op: &str,
        args: &Value,
        sources: &dyn Sources,
    ) -> Result<Value> {
        let tx = conn.unchecked_transaction().map_err(sql_error)?;
        let mut pending = self.clone();
        let result = if op.starts_with("ui.") {
            desktop::execute(&tx, op, args, &mut pending, sources)
        } else {
            execute(&tx, op, args, &mut pending, sources)
        }?;
        if op.starts_with("ui.") {
            pending.merge(&tx)?;
        }
        tx.commit().map_err(sql_error)?;
        *self = pending;
        Ok(result)
    }
}
fn execute(
    conn: &Connection,
    op: &str,
    args: &Value,
    config: &mut Config,
    sources: &dyn Sources,
) -> Result<Value> {
    match op {
        "backends.list" => output::list(config),
        "backends.kinds" => Ok(Value::List(
            KINDS.iter().map(|k| dict([("kind", s(k))])).collect(),
        )),
        "backends.get" => {
            let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
            let mut b=config.view.get(&name).cloned().map(Ok).unwrap_or_else(||db_backend(conn,&text(args,"name").to_lowercase(),"Use `kassiber backends list` to see configured backends, or `kassiber backends create` to add one."))?;
            if get(&b, "notes").is_none() {
                replace(&mut b, "notes", s(""));
            }
            replace(
                &mut b,
                "is_default",
                Value::Bool(name == config.default_backend),
            );
            output::redact(&b)
        }
        "backends.create" => output::redact(&create(conn, args, sources)?),
        "backends.update" => output::redact(&update(conn, args, sources)?),
        "backends.delete" => delete(conn, args, config),
        "backends.set-default" => set_default(conn, args, config),
        "backends.clear-default" => clear_default(conn, config),
        // This is an explicit reveal operation after the shell's database unlock.
        "backends.reveal-token" => {
            let name = kassiber_pytext::strip(&text(args, "name")).to_lowercase();
            let b=config.view.get(&name).cloned().map(Ok).unwrap_or_else(||db_backend(conn,&text(args,"name").to_lowercase(),"Use `kassiber backends list` to see configured backends, or `kassiber backends create` to add one."))?;
            Ok(dict(
                [
                    "name",
                    "kind",
                    "url",
                    "auth_header",
                    "token",
                    "username",
                    "password",
                    "config",
                ]
                .map(|k| (k, value(&b, k).clone())),
            ))
        }
        _ => Err(AppError::new(
            "not_implemented",
            "backend operation is not implemented",
        )),
    }
}
