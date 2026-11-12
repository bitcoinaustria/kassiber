use super::*;
use std::io::Read;
use std::sync::OnceLock;

fn metadata() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        kassiber_pytext::loads(include_str!("privacy_hygiene_metadata.json")).unwrap()
    })
}

fn number(n: i64) -> Value {
    Value::Int(n.into())
}

fn normalized(v: &Value) -> String {
    kassiber_pytext::strip(&string_or_empty(v)).to_lowercase()
}

fn string_or_empty(v: &Value) -> String {
    if truthy(v) {
        py_str(v)
    } else {
        String::new()
    }
}

fn object(row: &Value) -> Value {
    kassiber_pytext::loads(&text(row, "config_json"))
        .ok()
        .filter(|v| matches!(v, Value::Dict(_)))
        .unwrap_or_else(|| dict::<&str>([]))
}

fn endpoint_host(raw: &Value) -> String {
    let raw = string_or_empty(raw);
    let raw = kassiber_pytext::strip(&raw);
    if raw.is_empty() {
        return String::new();
    }
    // `parsed.hostname or raw` when urlsplit finds no host; a urlsplit
    // ValueError answers no host at all (b1ab8bfc3).
    let fallback = || raw.to_lowercase().trim_matches(['[', ']']).to_owned();
    let invalid = String::new;
    let parse = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let parse: String = parse
        .trim_start_matches(|c| c <= '\u{20}')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let Some((scheme, rest)) = parse.split_once("://") else {
        return fallback();
    };
    if !scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return fallback();
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // `_checknetloc`: NFKC must not introduce a netloc delimiter.
    if !authority.is_ascii() && authority.contains(NFKC_DELIMITERS) {
        return invalid();
    }
    let host_port = authority.rsplit('@').next().unwrap_or_default();
    let host = if host_port.contains(['[', ']']) {
        let Some((before, tail)) = host_port.split_once('[') else {
            return invalid();
        };
        let Some((host, after)) = tail.split_once(']') else {
            return invalid();
        };
        if !before.is_empty() || (!after.is_empty() && !after.starts_with(':')) {
            return invalid();
        }
        let future = host
            .strip_prefix('v')
            .and_then(|v| v.split_once('.'))
            .is_some_and(|(version, address)| {
                !version.is_empty()
                    && version.chars().all(|c| c.is_ascii_hexdigit())
                    && !address.is_empty()
            });
        if !future && parse_v6(host).is_none() {
            return invalid();
        }
        host
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        fallback()
    } else {
        host.to_lowercase().trim_matches(['[', ']']).to_owned()
    }
}

/// `ipaddress.IPv6Address(host)`, with an optional nonempty `%scope`: whether
/// it parses, and whether it is (or maps) a loopback address.
fn parse_v6(host: &str) -> Option<bool> {
    let address = match host.split_once('%') {
        Some((address, scope)) if !scope.is_empty() && !scope.contains('%') => address,
        Some(_) => return None,
        None => host,
    };
    let v6 = address.parse::<std::net::Ipv6Addr>().ok()?;
    // IPv6Address.is_loopback reads an IPv4-mapped address's IPv4 form.
    Some(v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()))
}

/// Characters whose NFKC form contains a netloc delimiter (`_checknetloc`).
const NFKC_DELIMITERS: [char; 19] = [
    '\u{2047}', '\u{2048}', '\u{2049}', '\u{2100}', '\u{2101}', '\u{2105}', '\u{2106}', '\u{2a74}',
    '\u{fe13}', '\u{fe16}', '\u{fe55}', '\u{fe56}', '\u{fe5f}', '\u{fe6b}', '\u{ff03}', '\u{ff0f}',
    '\u{ff1a}', '\u{ff1f}', '\u{ff20}',
];

/// `_is_loopback_host`: LAN addresses and local names can reach other
/// machines, so only loopback endpoints are on the device (b1ab8bfc3).
fn loopback_host(host: &str) -> bool {
    if matches!(host, "localhost" | "::1") || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(v4) => v4.is_loopback(),
        Err(_) => parse_v6(host) == Some(true),
    }
}

fn posture(row: &Value, config: &Value, proxy: bool) -> &'static str {
    let host = endpoint_host(value(row, "url"));
    let kind = normalized(value(row, "kind"));
    if loopback_host(&host) {
        return "on_device";
    }
    if normalized(value(config, "infrastructure_owner")) == "self" {
        return "self_hosted";
    }
    if host.ends_with(".onion")
        || (proxy
            && [
                "",
                "bitcoinrpc",
                "btcpay",
                "electrum",
                "esplora",
                "liquid-esplora",
                "mempool",
            ]
            .contains(&kind.as_str()))
    {
        return "shielded";
    }
    if host.is_empty() {
        "unknown"
    } else {
        "remote"
    }
}

fn postures() -> Value {
    dict(["on_device", "self_hosted", "shielded", "remote", "unknown"].map(|k| (k, number(0))))
}

fn increment(v: &mut Value, key: &str) {
    replace(v, key, number(int(v, key) + 1));
}

fn network(conn: &Connection) -> Result<Value> {
    let mut counts = postures();
    let mut surfaces =
        dict(["bitcoin", "liquid", "lightning", "market", "other"].map(|k| (k, postures())));
    let mut total = 0;
    let mut credentials = 0;
    let mut proxies = 0;
    for row in rows(conn, "SELECT kind,chain,network,url,auth_header,token,tor_proxy,config_json FROM backends ORDER BY name ASC", [])? {
        total += 1;
        let config = object(&row);
        let proxy = !kassiber_pytext::strip(&string_or_empty(value(&row, "tor_proxy"))).is_empty();
        proxies += i64::from(proxy);
        credentials += i64::from(truthy(value(&row, "auth_header")) || truthy(value(&row, "token")) || ["certificate", "cookiefile", "commando_peer_id", "lightning_cli", "lightning_dir", "password", "rpc_file", "username"].iter().any(|k| truthy(value(&config, k))));
        let posture = posture(&row, &config, proxy);
        increment(&mut counts, posture);
        let kind = normalized(value(&row, "kind"));
        let chain = normalized(value(&row, "chain"));
        let surface = if ["lnd", "coreln", "nwc"].contains(&kind.as_str()) { "lightning" }
            else if ["coinbase-exchange", "coingecko", "mempool-rates"].contains(&kind.as_str()) { "market" }
            else if chain == "liquid" || kind == "liquid-esplora" { "liquid" }
            else if chain == "bitcoin" || ["bitcoinrpc", "btcpay", "electrum", "esplora", "mempool"].contains(&kind.as_str()) { "bitcoin" }
            else { "other" };
        let mut surface_counts = value(&surfaces, surface).clone();
        increment(&mut surface_counts, posture);
        replace(&mut surfaces, surface, surface_counts);
    }
    Ok(dict([
        ("backend_count", number(total)),
        ("posture_counts", counts),
        ("by_surface", surfaces),
        ("credentialed_backend_count", number(credentials)),
        ("proxy_configured_backend_count", number(proxies)),
        ("evidence_level", s("derived")),
    ]))
}

fn ai(conn: &Connection) -> Result<Value> {
    let mut facts = dict(
        [
            "provider_count",
            "local_provider_count",
            "remote_provider_count",
            "tee_provider_count",
            "cli_provider_count",
            "credentialed_provider_count",
            "unacknowledged_off_device_provider_count",
        ]
        .map(|k| (k, number(0))),
    );
    for row in rows(
        conn,
        "SELECT name,base_url,api_key,kind,acknowledged_at FROM ai_providers ORDER BY name ASC",
        [],
    )? {
        increment(&mut facts, "provider_count");
        let kind = normalized(value(&row, "kind"));
        let url = normalized(value(&row, "base_url"));
        let key = if [
            "claude-cli://default",
            "codex-cli://default",
            "opencode-cli://default",
            "copilot-cli://default",
        ]
        .contains(&url.as_str())
        {
            "cli_provider_count"
        } else {
            match kind.as_str() {
                "local" => "local_provider_count",
                "tee" => "tee_provider_count",
                _ => "remote_provider_count",
            }
        };
        increment(&mut facts, key);
        if truthy(value(&row, "api_key")) {
            increment(&mut facts, "credentialed_provider_count");
        }
        if kind != "local" && !truthy(value(&row, "acknowledged_at")) {
            increment(&mut facts, "unacknowledged_off_device_provider_count");
        }
    }
    replace(&mut facts, "evidence_level", s("exact"));
    Ok(facts)
}

fn wallets(conn: &Connection, profile: &str) -> Result<Value> {
    let mut facts = dict(
        [
            "wallet_count",
            "watch_only_material_wallet_count",
            "backend_linked_wallet_count",
            "descriptor_wallet_count",
            "xpub_wallet_count",
            "address_wallet_count",
            "silent_payment_wallet_count",
            "source_file_wallet_count",
        ]
        .map(|k| (k, number(0))),
    );
    for row in rows(
        conn,
        "SELECT kind,config_json FROM wallets WHERE profile_id=? ORDER BY kind ASC",
        [profile],
    )? {
        increment(&mut facts, "wallet_count");
        let config = object(&row);
        for (field, keys) in [
            ("backend_linked_wallet_count", &["backend"][..]),
            (
                "descriptor_wallet_count",
                &["descriptor", "change_descriptor"][..],
            ),
            ("xpub_wallet_count", &["xpub"][..]),
            ("address_wallet_count", &["addresses"][..]),
            ("silent_payment_wallet_count", &["sp_descriptor"][..]),
            ("source_file_wallet_count", &["source_file"][..]),
            (
                "watch_only_material_wallet_count",
                &[
                    "addresses",
                    "change_descriptor",
                    "descriptor",
                    "samourai",
                    "sp_descriptor",
                    "xpub",
                ][..],
            ),
        ] {
            if keys.iter().any(|k| truthy(value(&config, k))) {
                increment(&mut facts, field);
            }
        }
    }
    replace(&mut facts, "evidence_level", s("exact"));
    Ok(facts)
}

fn database(conn: &Connection) -> Value {
    let fact = || -> Option<(&str, &str)> {
        let path: String = conn
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .ok()?;
        if path.is_empty() {
            return None;
        }
        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(_) => return None,
        };
        if size == 0 {
            return Some(("missing_or_empty", "exact"));
        }
        let mut header = [0; 16];
        let plain = std::fs::File::open(&path)
            .and_then(|mut f| f.read_exact(&mut header))
            .is_ok()
            && &header == b"SQLite format 3\0";
        Some((if plain { "plaintext" } else { "encrypted_like" }, "exact"))
    };
    let (status, evidence) = fact().unwrap_or(("unknown", "unknown"));
    dict([("status", s(status)), ("evidence_level", s(evidence))])
}

fn finding(findings: &mut Vec<Value>, id: &str, count: i64, evidence: Value) {
    let item = value(value(metadata(), "findings"), id);
    findings.push(dict([
        ("id", s(id)),
        ("category", value(item, "category").clone()),
        ("severity", value(item, "severity").clone()),
        ("title", value(item, "title").clone()),
        (
            "detail",
            s(text(item, "detail").replace("{count}", &count.to_string())),
        ),
        ("evidence_level", value(item, "evidence_level").clone()),
        ("evidence", evidence),
        ("recommendation", value(item, "recommendation").clone()),
    ]));
}

pub fn report(conn: &Connection, workspace: &Value, profile: &Value) -> Result<Value> {
    let database = database(conn);
    let mut findings = Vec::new();
    match text(&database, "status").as_str() {
        "plaintext" => finding(
            &mut findings,
            "database_plaintext",
            0,
            dict([("database_encryption", s("plaintext"))]),
        ),
        "unknown" => finding(
            &mut findings,
            "database_encryption_unknown",
            0,
            dict([("database_encryption", s("unknown"))]),
        ),
        _ => {}
    }
    let network = network(conn)?;
    let remote = int(value(&network, "posture_counts"), "remote");
    if remote > 0 {
        finding(
            &mut findings,
            "remote_backend_endpoints",
            remote,
            dict([("remote_backend_count", number(remote))]),
        );
    }
    let ai = ai(conn)?;
    let off_device = int(&ai, "remote_provider_count")
        + int(&ai, "tee_provider_count")
        + int(&ai, "cli_provider_count");
    let unacknowledged = int(&ai, "unacknowledged_off_device_provider_count");
    if unacknowledged > 0 {
        finding(
            &mut findings,
            "unacknowledged_off_device_ai_provider",
            unacknowledged,
            dict([(
                "unacknowledged_off_device_provider_count",
                number(unacknowledged),
            )]),
        );
    } else if off_device > 0 {
        finding(
            &mut findings,
            "off_device_ai_provider_configured",
            off_device,
            dict([("off_device_ai_provider_count", number(off_device))]),
        );
    }
    let wallets = wallets(conn, &text(profile, "id"))?;
    let watch = int(&wallets, "watch_only_material_wallet_count");
    if watch > 0 {
        finding(
            &mut findings,
            "watch_only_wallet_material_present",
            watch,
            dict([("watch_only_material_wallet_count", number(watch))]),
        );
    }
    let Value::List(initial) = value(metadata(), "limitations") else {
        unreachable!()
    };
    let mut limitations = initial.clone();
    let Value::List(queries) = value(metadata(), "queries") else {
        unreachable!()
    };
    let mut counts = dict::<&str>([]);
    for query in queries {
        let count = match conn.query_row(&text(query, "sql"), [text(profile, "id")], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(n) => n,
            Err(e) => {
                let e = match e {
                    kassiber_store::rusqlite::Error::SqlInputError { error, msg, .. } => {
                        kassiber_store::rusqlite::Error::SqliteFailure(error, Some(msg))
                    }
                    other => other,
                };
                if !sql::optional_cache_error(&e) {
                    return Err(sql_error(e));
                }
                limitations.push(dict([
                    ("code", value(query, "code").clone()),
                    ("message", s("A local database count could not be read.")),
                    ("evidence_level", s("unknown")),
                    ("details", dict([("error", s(e.to_string()))])),
                ]));
                // A failed count stays unknown, never an exact zero (a17cc6b8c).
                replace(&mut counts, &text(query, "name"), Value::Null);
                continue;
            }
        };
        replace(&mut counts, &text(query, "name"), number(count));
    }
    // A count read, or None when its query failed.
    let count = |name: &str| match value(&counts, name) {
        Value::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    };
    let nullable = |n: Option<i64>| n.map_or(Value::Null, number);
    // `_count_evidence_level`.
    let evidence = |names: &[&str]| {
        s(if names.iter().any(|name| count(name).is_none()) {
            "unknown"
        } else {
            "exact"
        })
    };
    for (name, id, evidence) in [
        (
            "privacy_boundary_transactions",
            "privacy_boundary_transactions_present",
            "privacy_boundary_transaction_count",
        ),
        (
            "raw_json_transactions",
            "import_raw_json_present",
            "transaction_raw_json_count",
        ),
    ] {
        if let Some(n) = count(name).filter(|n| *n != 0) {
            finding(&mut findings, id, n, dict([(evidence, number(n))]));
        }
    }
    let inventory = dict([
        ("utxo_count", value(&counts, "utxo_count").clone()),
        (
            "active_utxo_count",
            value(&counts, "active_utxo_count").clone(),
        ),
        (
            "utxos_with_address_count",
            value(&counts, "utxos_with_address").clone(),
        ),
        (
            "utxos_with_script_count",
            value(&counts, "utxos_with_script").clone(),
        ),
        (
            "utxos_with_derivation_metadata_count",
            value(&counts, "utxos_with_derivation").clone(),
        ),
        (
            "evidence_level",
            evidence(&[
                "utxo_count",
                "active_utxo_count",
                "utxos_with_address",
                "utxos_with_script",
                "utxos_with_derivation",
            ]),
        ),
    ]);
    let location = [
        "utxos_with_address",
        "utxos_with_script",
        "utxos_with_derivation",
    ];
    if location
        .iter()
        .any(|name| count(name).is_some_and(|n| n != 0))
    {
        finding(
            &mut findings,
            "local_utxo_location_metadata_present",
            0,
            dict(
                [
                    "utxos_with_address_count",
                    "utxos_with_script_count",
                    "utxos_with_derivation_metadata_count",
                ]
                .map(|k| (k, value(&inventory, k).clone())),
            ),
        );
        if let Some(Value::Dict(fields)) = findings.last_mut() {
            for (key, item) in fields.iter_mut() {
                if matches!(key, kassiber_pyjson::Key::Str(k) if k.as_str() == Some("evidence_level"))
                {
                    *item = evidence(&location);
                }
            }
        }
    }
    let quarantines = count("privacy_quarantine_count");
    if let Some(n) = quarantines.filter(|n| *n != 0) {
        finding(
            &mut findings,
            "privacy_quarantines_present",
            n,
            dict([("privacy_quarantine_count", number(n))]),
        );
    }
    let active = count("active_transactions");
    // Without an active count the reference evaluates no freshness.
    let status = match active {
        None => "unknown",
        Some(active) => {
            let processed_count =
                gain_explanation::profile_number(profile, "last_processed_tx_count")?;
            let version = gain_explanation::profile_number(profile, "journal_input_version")?;
            let processed_version =
                gain_explanation::profile_number(profile, "last_processed_input_version")?;
            if active == 0 {
                "no_transactions"
            } else if !truthy(value(profile, "last_processed_at")) {
                "not_processed"
            } else if processed_count != number(active)
                || version != processed_version
                || !crate::semantics_current(profile)
            {
                "stale"
            } else {
                "current"
            }
        }
    };
    let journals = dict([
        ("status", s(status)),
        (
            "needs_processing",
            if active.is_none() {
                Value::Null
            } else {
                Value::Bool(matches!(status, "not_processed" | "stale"))
            },
        ),
        ("active_transaction_count", nullable(active)),
        (
            "journal_entry_count",
            value(&counts, "journal_entry_count").clone(),
        ),
        (
            "quarantine_count",
            value(&counts, "quarantine_count").clone(),
        ),
        (
            "evidence_level",
            evidence(&[
                "active_transactions",
                "journal_entry_count",
                "quarantine_count",
                "privacy_quarantine_count",
            ]),
        ),
        ("privacy_quarantine_count", nullable(quarantines)),
    ]);
    let highest = findings
        .iter()
        .map(|f| text(f, "severity"))
        .max_by_key(|v| match v.as_str() {
            "alert" => 3,
            "warning" => 2,
            "info" => 1,
            _ => 0,
        })
        .unwrap_or_else(|| "none".into());
    Ok(dict([
        ("payload_schema_version", number(1)),
        ("redaction", s("ai_export_safe")),
        ("local_only", Value::Bool(true)),
        ("read_only", Value::Bool(true)),
        (
            "scope",
            dict([
                ("workspace", s("active")),
                ("profile", s("active")),
                (
                    "workspace_selected",
                    Value::Bool(truthy(value(workspace, "id"))),
                ),
                (
                    "profile_selected",
                    Value::Bool(truthy(value(profile, "id"))),
                ),
            ]),
        ),
        (
            "summary",
            dict([
                ("finding_count", number(findings.len() as i64)),
                ("highest_severity", s(highest)),
                ("remote_backend_count", number(remote)),
                ("off_device_ai_provider_count", number(off_device)),
                ("wallet_count", value(&wallets, "wallet_count").clone()),
                ("watch_only_material_wallet_count", number(watch)),
                ("active_transaction_count", nullable(active)),
                (
                    "privacy_boundary_transaction_count",
                    value(&counts, "privacy_boundary_transactions").clone(),
                ),
                ("privacy_quarantine_count", nullable(quarantines)),
            ]),
        ),
        (
            "facts",
            dict([
                ("database", database),
                ("network", network),
                ("ai", ai),
                ("wallets", wallets),
                (
                    "transactions",
                    dict([
                        ("active_transaction_count", nullable(active)),
                        (
                            "excluded_transaction_count",
                            value(&counts, "excluded_transactions").clone(),
                        ),
                        (
                            "transaction_raw_json_count",
                            value(&counts, "raw_json_transactions").clone(),
                        ),
                        (
                            "privacy_boundary_transaction_count",
                            value(&counts, "privacy_boundary_transactions").clone(),
                        ),
                        (
                            "evidence_level",
                            evidence(&[
                                "active_transactions",
                                "excluded_transactions",
                                "raw_json_transactions",
                                "privacy_boundary_transactions",
                            ]),
                        ),
                    ]),
                ),
                ("inventory", inventory),
                ("journals", journals),
            ]),
        ),
        ("findings", Value::List(findings)),
        ("limitations", Value::List(limitations)),
    ]))
}

pub fn table(payload: &Value) -> Value {
    let Value::List(findings) = value(payload, "findings") else {
        unreachable!()
    };
    Value::List(
        findings
            .iter()
            .map(|f| {
                dict([
                    ("severity", value(f, "severity").clone()),
                    ("category", value(f, "category").clone()),
                    ("finding", value(f, "id").clone()),
                    ("evidence_level", value(f, "evidence_level").clone()),
                    ("detail", value(f, "detail").clone()),
                ])
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_hygiene_storage_headers_preserve_unknown_empty_and_encrypted_states() {
        let memory = Connection::open_in_memory().unwrap();
        assert_eq!(
            database(&memory),
            dict([("status", s("unknown")), ("evidence_level", s("unknown"))])
        );
        let root = tempfile::tempdir().unwrap();
        let empty = Connection::open(root.path().join("empty.sqlite")).unwrap();
        assert_eq!(
            database(&empty),
            dict([
                ("status", s("missing_or_empty")),
                ("evidence_level", s("exact"))
            ])
        );
        let encrypted = Connection::open(root.path().join("encrypted.sqlite")).unwrap();
        encrypted
            .execute_batch("PRAGMA key='synthetic database passphrase';")
            .unwrap();
        encrypted
            .execute_batch(include_str!("../tests/data/privacy_hygiene_seed.sql"))
            .unwrap();
        encrypted
            .execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        assert_eq!(
            database(&encrypted),
            dict([
                ("status", s("encrypted_like")),
                ("evidence_level", s("exact"))
            ])
        );
        let report =
            crate::execute(&encrypted, "reports.privacy-hygiene", &dict::<&str>([])).unwrap();
        assert_eq!(value(&report, "findings"), &Value::List(vec![]));
        assert_eq!(text(value(&report, "summary"), "highest_severity"), "none");
        assert_eq!(table(&report), Value::List(vec![]));
    }
}
