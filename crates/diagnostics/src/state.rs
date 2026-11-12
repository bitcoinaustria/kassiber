//! Read-only aggregate projections. Raw rows never leave this module.
use crate::sanitize_text;
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::ValueRef, Connection};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, AppError>;
fn sql_error(error: kassiber_store::rusqlite::Error) -> AppError {
    AppError::new("internal_error", error.to_string())
}
fn n(value: usize) -> Value {
    Value::Int(value.into())
}
fn rows(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut statement = conn.prepare(sql).map_err(sql_error)?;
    let names = statement
        .column_names()
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>();
    let values = statement
        .query_map([], |row| {
            let mut values = Vec::new();
            for (index, name) in names.iter().enumerate() {
                let value = match row.get_ref(index)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(v) => Value::Int(v.into()),
                    ValueRef::Real(v) => Value::Float(v),
                    ValueRef::Text(v) => s(String::from_utf8_lossy(v).into_owned()),
                    ValueRef::Blob(_) => Value::Null,
                };
                values.push((name.as_str(), value));
            }
            Ok(dict(values))
        })
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    Ok(values)
}
fn val<'a>(row: &'a Value, key: &str) -> &'a Value {
    get(row, key).unwrap_or(&Value::Null)
}
fn text(row: &Value, key: &str, default: &str) -> String {
    let v = val(row, key);
    if truthy(v) {
        py_str(v)
    } else {
        default.into()
    }
}
fn count(rows: &[Value], key: &str) -> Value {
    n(rows.iter().filter(|r| truthy(val(r, key))).count())
}
fn category(v: &str) -> String {
    let v = sanitize_text(&kassiber_pytext::lower(kassiber_pytext::strip(v)));
    if v.is_empty() {
        "unknown".into()
    } else {
        v
    }
}
fn asset(v: &str) -> String {
    match kassiber_pytext::upper(v).as_str() {
        "BTC" => "BTC",
        "LBTC" => "LBTC",
        "" => "unknown",
        _ => "other",
    }
    .into()
}
fn counters(values: impl Iterator<Item = String>, key: &str) -> Value {
    let mut counts = BTreeMap::<String, usize>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    Value::List(
        counts
            .into_iter()
            .map(|(value, count)| dict([(key, s(value)), ("count", n(count))]))
            .collect(),
    )
}
fn grouped(rows: &[Value], field: &str, key: &str, default: &str) -> Value {
    counters(rows.iter().map(|r| category(&text(r, field, default))), key)
}
fn assets(rows: &[Value], key: &str) -> Value {
    counters(rows.iter().map(|r| asset(&text(r, "asset", ""))), key)
}
pub fn checks(conn: Option<&Connection>) -> Value {
    let Some(conn) = conn else {
        return dict([("database_integrity", s("unavailable"))]);
    };
    match rows(conn, "PRAGMA integrity_check") {
        Err(_) => dict([("database_integrity", s("error"))]),
        Ok(rows) if rows.len() == 1 && get(&rows[0], "integrity_check") == Some(&s("ok")) => {
            dict([("database_integrity", s("ok"))])
        }
        Ok(rows) => dict([
            ("database_integrity", s("issues")),
            ("issue_count", n(rows.len())),
        ]),
    }
}
pub fn state(conn: &Connection, backends: Value) -> Result<Value> {
    let tables = [
        "workspaces",
        "profiles",
        "accounts",
        "wallets",
        "transactions",
        "journal_entries",
        "journal_quarantines",
        "transaction_pairs",
        "direct_swap_payouts",
        "bip329_labels",
        "backends",
        "rates_cache",
        "attachments",
    ];
    let counts = dict(tables.into_iter().map(|table| {
        let value = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|v| Value::Int(v.into()))
            .unwrap_or(Value::Null);
        (table, value)
    }));
    Ok(dict([
        ("available", Value::Bool(true)),
        ("counts", counts),
        ("profiles", profiles(conn)?),
        ("wallets", wallets(conn)?),
        ("backends", backends),
        ("transactions", transactions(conn)?),
        ("journals", journals(conn)?),
        ("rates", rates(conn)?),
        ("attachments", attachments(conn)?),
        ("manual_pairs", manual_pairs(conn)?),
    ]))
}
fn profiles(conn: &Connection) -> Result<Value> {
    let rows=rows(conn,"SELECT p.id,p.tax_country,p.fiat_currency,p.gains_algorithm,p.last_processed_at,p.last_processed_tx_count,p.journal_input_version,p.last_processed_input_version,p.required_journal_semantics_version,p.journal_semantics_version,COUNT(t.id) AS active_transactions FROM profiles p LEFT JOIN transactions t ON t.profile_id=p.id AND t.excluded=0 GROUP BY p.id")?;
    let processed = rows
        .iter()
        .filter(|r| {
            kassiber_store::semantics::compatible(
                py_str(val(r, "required_journal_semantics_version"))
                    .parse()
                    .unwrap_or(-1),
                py_str(val(r, "journal_semantics_version"))
                    .parse()
                    .unwrap_or(-1),
            ) && truthy(val(r, "last_processed_at"))
                && val(r, "active_transactions") == val(r, "last_processed_tx_count")
                && val(r, "journal_input_version") == val(r, "last_processed_input_version")
        })
        .count();
    // These three fields intentionally follow Python's raw category behavior.
    let group = |key: &str| counters(rows.iter().map(|r| text(r, key, "unknown")), key);
    Ok(dict([
        ("total", n(rows.len())),
        ("processed", n(processed)),
        ("stale_or_unprocessed", n(rows.len() - processed)),
        ("by_tax_country", group("tax_country")),
        ("by_fiat_currency", group("fiat_currency")),
        ("by_gains_algorithm", group("gains_algorithm")),
    ]))
}
fn wallets(conn: &Connection) -> Result<Value> {
    let rows = rows(conn, "SELECT kind,config_json FROM wallets")?;
    let mut invalid = 0;
    let configs = rows
        .iter()
        .map(|r| {
            let raw = text(r, "config_json", "{}");
            match kassiber_pytext::loads(&raw) {
                Ok(v @ Value::Dict(_)) => v,
                _ => {
                    invalid += 1;
                    dict::<&str>([])
                }
            }
        })
        .collect::<Vec<_>>();
    let mut fields = vec![
        ("total", n(rows.len())),
        (
            "by_kind",
            counters(rows.iter().map(|r| text(r, "kind", "unknown")), "kind"),
        ),
        (
            "by_chain_network",
            counters(
                configs.iter().map(|r| {
                    format!(
                        "{}/{}",
                        category(&text(r, "chain", "unspecified")),
                        category(&text(r, "network", "unspecified"))
                    )
                }),
                "chain_network",
            ),
        ),
    ];
    for (key, field) in [
        ("descriptor_configured", "descriptor"),
        ("change_descriptor_configured", "change_descriptor"),
        ("address_lists_configured", "addresses"),
        ("source_files_configured", "source_file"),
    ] {
        fields.push((key, count(&configs, field)));
    }
    fields.push((
        "btcpay_sync_configured",
        n(configs
            .iter()
            .filter(|r| val(r, "sync_source") == &s("btcpay"))
            .count()),
    ));
    for (key, field) in [
        ("silent_payment_configured", "sp_descriptor"),
        ("explicit_backend_configured", "backend"),
    ] {
        fields.push((key, count(&configs, field)));
    }
    fields.push(("invalid_config_rows", n(invalid)));
    Ok(dict(fields))
}
fn transactions(conn: &Connection) -> Result<Value> {
    let rows = rows(
        conn,
        "SELECT direction,asset,excluded,fiat_rate,fiat_value,confirmed_at,note FROM transactions",
    )?;
    Ok(dict([
        ("total", n(rows.len())),
        ("excluded", count(&rows, "excluded")),
        (
            "priced",
            n(rows
                .iter()
                .filter(|r| {
                    val(r, "fiat_rate") != &Value::Null || val(r, "fiat_value") != &Value::Null
                })
                .count()),
        ),
        ("confirmed", count(&rows, "confirmed_at")),
        ("with_notes", count(&rows, "note")),
        (
            "by_direction",
            grouped(&rows, "direction", "direction", "unknown"),
        ),
        ("by_asset_bucket", assets(&rows, "asset_bucket")),
    ]))
}
fn journals(conn: &Connection) -> Result<Value> {
    let entries = rows(
        conn,
        "SELECT entry_type,asset,at_category FROM journal_entries",
    )?;
    let quarantines = rows(conn, "SELECT reason FROM journal_quarantines")?;
    Ok(dict([
        ("entries", n(entries.len())),
        ("quarantines", n(quarantines.len())),
        (
            "entries_by_type",
            grouped(&entries, "entry_type", "entry_type", "unknown"),
        ),
        ("entries_by_asset_bucket", assets(&entries, "asset_bucket")),
        (
            "entries_by_at_category",
            grouped(&entries, "at_category", "at_category", "none"),
        ),
        (
            "quarantines_by_reason",
            counters(
                quarantines
                    .iter()
                    .map(|r| sanitize_text(&text(r, "reason", "unknown"))),
                "reason",
            ),
        ),
    ]))
}
fn rates(conn: &Connection) -> Result<Value> {
    Ok(rows(conn,"SELECT COUNT(*) AS samples,COUNT(DISTINCT pair) AS pairs,COUNT(DISTINCT source) AS sources FROM rates_cache")?.remove(0))
}
fn attachments(conn: &Connection) -> Result<Value> {
    let rows = rows(
        conn,
        "SELECT attachment_type,source_url,stored_relpath FROM attachments",
    )?;
    Ok(dict([
        ("total", n(rows.len())),
        (
            "by_type",
            grouped(&rows, "attachment_type", "attachment_type", "unknown"),
        ),
        ("url_references", count(&rows, "source_url")),
        ("stored_files", count(&rows, "stored_relpath")),
    ]))
}
fn manual_pairs(conn: &Connection) -> Result<Value> {
    let rows=rows(conn,"SELECT term.term_kind,term.review_kind AS kind,term.tax_policy AS policy FROM custody_component_economic_terms term JOIN custody_component_legs source ON source.id=term.source_leg_id JOIN custody_component_legs target ON target.id=term.target_leg_id WHERE term.term_kind IN ('transaction_pair','direct_swap_payout') AND EXISTS (SELECT 1 FROM custody_component_transaction_memberships m WHERE m.profile_id=term.profile_id AND m.component_id=term.component_id) AND EXISTS (SELECT 1 FROM profiles p WHERE p.id=term.profile_id)")?;
    let pairs = rows
        .iter()
        .filter(|r| val(r, "term_kind") == &s("transaction_pair"))
        .count();
    Ok(dict([
        ("total", n(rows.len())),
        ("manual_pairs", n(pairs)),
        ("direct_swap_payouts", n(rows.len() - pairs)),
        ("by_kind", grouped(&rows, "kind", "kind", "unknown")),
        ("by_policy", grouped(&rows, "policy", "policy", "unknown")),
    ]))
}
/// Receives private runtime configuration and emits presence/categorical data only.
pub fn backends(config: &Value) -> Value {
    let rows = match get(config, "backends") {
        Some(Value::Dict(pairs)) => pairs
            .iter()
            .filter_map(|(_, v)| matches!(v, Value::Dict(_)).then_some(v.clone()))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let mut credentials = vec![];
    for key in ["auth_header", "cookiefile", "password", "token", "username"] {
        if !rows.is_empty() {
            credentials.push((key, count(&rows, key)));
        }
    }
    dict([
        ("total", n(rows.len())),
        (
            "default_backend_configured",
            Value::Bool(truthy(val(config, "default_backend"))),
        ),
        ("by_kind", grouped(&rows, "kind", "kind", "unknown")),
        (
            "by_chain_network",
            counters(
                rows.iter().map(|r| {
                    format!(
                        "{}/{}",
                        category(&text(r, "chain", "unspecified")),
                        category(&text(r, "network", "unspecified"))
                    )
                }),
                "chain_network",
            ),
        ),
        ("by_source", grouped(&rows, "source", "source", "unknown")),
        (
            "by_locality",
            counters(
                rows.iter().map(|r| locality(&text(r, "url", ""))),
                "locality",
            ),
        ),
        ("credential_presence", dict(credentials)),
        ("tor_proxy_configured", count(&rows, "tor_proxy")),
    ])
}
fn locality(url: &str) -> String {
    let cleaned = url
        .trim_start_matches(|c: char| u32::from(c) <= 32)
        .replace(['\r', '\n', '\t'], "");
    let url = cleaned.as_str();
    if url.is_empty() {
        return "missing".into();
    }
    // URL parsing is syntactic only; it never resolves a hostname.
    let authority = if let Some((scheme, rest)) = url.split_once("://") {
        if !scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        {
            return "unknown".into();
        }
        rest.split(['/', '?', '#']).next().unwrap_or("")
    } else if let Some(rest) = url.strip_prefix("//") {
        rest.split(['/', '?', '#']).next().unwrap_or("")
    } else {
        return "unknown".into();
    };
    if authority
        .chars()
        .any(|c| "⁇⁈⁉℀℁℅℆⩴︓︖﹕﹖﹟﹫＃／：？＠".contains(c))
    {
        return "invalid".into();
    }
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(host) = host.strip_prefix('[') {
        match host.split_once(']') {
            Some((host, _)) => {
                if let Some(rest) = host.strip_prefix('v') {
                    let valid = rest.split_once('.').is_some_and(|(version, name)| {
                        !version.is_empty()
                            && version.chars().all(|c| c.is_ascii_hexdigit())
                            && !name.is_empty()
                    });
                    if !valid {
                        return "invalid".into();
                    }
                } else if host
                    .split('%')
                    .next()
                    .unwrap_or("")
                    .parse::<std::net::Ipv6Addr>()
                    .is_err()
                {
                    return "invalid".into();
                }
                host
            }
            None => return "invalid".into(),
        }
    } else {
        if host.contains(['[', ']']) {
            return "invalid".into();
        }
        host.split(':').next().unwrap_or("")
    };
    let host = kassiber_pytext::lower(host);
    if host.is_empty() {
        return "unknown".into();
    }
    if host == "localhost" || host == "ip6-localhost" || host.ends_with(".localhost") {
        return "loopback".into();
    }
    let ip_host = host
        .split_once('%')
        .filter(|(host, scope)| host.contains(':') && !scope.is_empty() && !scope.contains('%'))
        .map_or(host.as_str(), |(ip, _)| ip);
    if let Ok(ip) = ip_host.parse::<std::net::IpAddr>() {
        if ip.is_loopback() {
            return "loopback".into();
        }
        let private = match ip {
            std::net::IpAddr::V4(ip) => private_v4(u32::from(ip)),
            std::net::IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
                || private_v6(u128::from(ip)),
                |ip| private_v4(u32::from(ip)),
            ),
        };
        return if private { "private_lan" } else { "remote" }.into();
    }
    if host.ends_with(".local") {
        "private_lan"
    } else {
        "remote"
    }
    .into()
}

// CPython 3.11.17 ipaddress registry ranges, frozen with locality oracle vectors.
fn private_v4(ip: u32) -> bool {
    const PRIVATE: &[(u32, u32)] = &[
        (0u32, 16777215u32),
        (167772160u32, 184549375u32),
        (2130706432u32, 2147483647u32),
        (2851995648u32, 2852061183u32),
        (2886729728u32, 2887778303u32),
        (3221225472u32, 3221225727u32),
        (3221225642u32, 3221225643u32),
        (3221225984u32, 3221226239u32),
        (3232235520u32, 3232301055u32),
        (3323068416u32, 3323199487u32),
        (3325256704u32, 3325256959u32),
        (3405803776u32, 3405804031u32),
        (4026531840u32, 4294967295u32),
        (4294967295u32, 4294967295u32),
    ];
    const EXCEPTIONS: &[(u32, u32)] = &[
        (3221225481u32, 3221225481u32),
        (3221225482u32, 3221225482u32),
    ];
    PRIVATE
        .iter()
        .any(|(start, end)| (*start..=*end).contains(&ip))
        && !EXCEPTIONS
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&ip))
}

// CPython 3.11.17 ipaddress registry ranges, frozen with locality oracle vectors.
fn private_v6(ip: u128) -> bool {
    const PRIVATE: &[(u128, u128)] = &[
        (1u128, 1u128),
        (0u128, 0u128),
        (281470681743360u128, 281474976710655u128),
        (
            524413980668812575603097810486951936u128,
            524413980670021501422712439661658111u128,
        ),
        (
            1329227995784915872903807060280344576u128,
            1329227995784915891350551133989896191u128,
        ),
        (
            42540488161975842760550356425300246528u128,
            42540528726795050063891204319802818559u128,
        ),
        (
            42540766411282592856903984951653826560u128,
            42540766490510755371168322545197776895u128,
        ),
        (
            42545680458834377588178886921629466624u128,
            42550872755692912415807417417958686719u128,
        ),
        (
            85065399433376081038215121361612832768u128,
            85065723951929739464941904517633409023u128,
        ),
        (
            334965454937798799971759379190646833152u128,
            337623910929368631717566993311207522303u128,
        ),
        (
            338288524927261089654018896841347694592u128,
            338620831926207318622244848606417780735u128,
        ),
    ];
    const EXCEPTIONS: &[(u128, u128)] = &[
        (
            42540488241204005274814694018844196865u128,
            42540488241204005274814694018844196865u128,
        ),
        (
            42540488241204005274814694018844196866u128,
            42540488241204005274814694018844196866u128,
        ),
        (
            42540488399660330303343369205932097536u128,
            42540488478888492817607706799476047871u128,
        ),
        (
            42540488479219738492182115193345540096u128,
            42540488479220947418001729822520246271u128,
        ),
        (
            42540490697277043217009159418706657280u128,
            42540491964927643445238560915409862655u128,
        ),
        (
            42540491964927643445238560915409862656u128,
            42540493232578243673467962412113068031u128,
        ),
    ];
    PRIVATE
        .iter()
        .any(|(start, end)| (*start..=*end).contains(&ip))
        && !EXCEPTIONS
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&ip))
}
