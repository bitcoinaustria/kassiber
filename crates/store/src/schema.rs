use crate::{map_sqlite_error, Error, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
const MANIFEST: &str = include_str!("../fixtures/current-schema.json");

fn objects(conn: &Connection) -> Result<Value> {
    let mut statement = conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name").map_err(map_sqlite_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok(json!([
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?
            ]))
        })
        .map_err(map_sqlite_error)?;
    let rows: std::result::Result<Vec<_>, _> = rows.collect();
    Ok(Value::Array(rows.map_err(map_sqlite_error)?))
}
pub fn fingerprint(conn: &Connection) -> Result<String> {
    Ok(
        Sha256::digest(serde_json::to_vec(&objects(conn)?).expect("schema JSON"))
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}
pub fn verify(conn: &Connection) -> Result<()> {
    let expected: Value = serde_json::from_str(MANIFEST).expect("checked-in schema manifest");
    // Identical SQLite definitions imply identical parsed structure. Avoid
    // creating a second database and walking every pragma on the usual fresh
    // schema. Historical ALTER layouts still take the full normalized gate.
    if objects(conn)? != expected["objects"] {
        crate::schema_structure::verify(conn, &expected)?;
    }
    let mut stmt = conn
        .prepare("SELECT id,migration_name,schema_version FROM schema_migration_audits ORDER BY id")
        .map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    let mut found = BTreeSet::new();
    for row in rows {
        let (id, name, version) = row.map_err(map_sqlite_error)?;
        let known = expected["required_audits"]
            .as_array()
            .expect("audit list")
            .iter()
            .any(|v| v[0].as_str() == Some(&id));
        if id != name
            || version != 1
            || (!known
                && ![
                    "custody-durable-evidence-v1",
                    "device-sync-removal-v1",
                    "at-fee-consumed-regime-v1",
                ]
                .contains(&id.as_str()))
        {
            return Err(Error::migration("migration_audits"));
        }
        found.insert(id);
    }
    if expected["required_audits"]
        .as_array()
        .expect("audit list")
        .iter()
        .any(|v| !found.contains(v[0].as_str().expect("audit id")))
    {
        return Err(Error::migration("migration_audits"));
    }
    for chunk in include_str!("semantic-probes.sql")
        .split("-- probe: ")
        .skip(1)
    {
        let (name, sql) = chunk.split_once('\n').expect("named probe");
        if conn
            .query_row(sql, [], |r| r.get::<_, bool>(0))
            .map_err(map_sqlite_error)?
        {
            return Err(migration_detail(name));
        }
    }
    verify_json_backfills(conn)?;
    verify_asset_backfills(conn)?;
    verify_gap_relations(conn)
}
fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}
fn parse_compat_json(raw: Option<&str>) -> Result<Value> {
    let raw = raw.filter(|v| !v.is_empty()).unwrap_or("{}");
    // serde_json's default parser can round a large integer through f64.
    // Python retains it as an int, which can become a valid 64-digit txid.
    // R1 must refuse unsupported values rather than lose a migration input.
    let mut quoted = false;
    let mut escaped = false;
    let mut position = 0;
    let bytes = raw.as_bytes();
    while position < bytes.len() {
        let byte = bytes[position];
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            position += 1;
        } else if byte == b'"' {
            quoted = true;
            position += 1;
        } else if byte == b'-' || byte.is_ascii_digit() {
            let start = position;
            position += 1;
            while position < bytes.len()
                && matches!(
                    bytes[position],
                    b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
                )
            {
                position += 1;
            }
            let token = &raw[start..position];
            let floating = token.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E'));
            // The shell enables serde_json/arbitrary_precision in the shared
            // workspace. Do not let that feature silently broaden admission:
            // Python decodes overflowing exponents as infinity, which this
            // conservative projection cannot represent faithfully.
            if (floating && token.parse::<f64>().is_ok_and(|n| !n.is_finite()))
                || (!floating && token.parse::<i64>().is_err() && token.parse::<u64>().is_err())
            {
                return Err(unsupported_json());
            }
        } else {
            position += 1;
        }
    }
    serde_json::from_str(raw).map_err(|_| unsupported_json())
}
fn unsupported_json() -> Error {
    Error::new(
        crate::ErrorKind::UnsupportedSchemaLayout,
        "stored JSON requires Python-compatible parsing before Rust admission",
    )
    .details(json!({"reason":"unsupported_python_json"}))
}
fn asset_code(value: &str) -> String {
    let text = value.trim_matches(python_whitespace);
    if crate::valid_hex(&text.to_lowercase(), 64) {
        return text.to_lowercase();
    }
    if ["l-btc", "lbtc"].contains(&text.to_lowercase().as_str()) {
        "LBTC".into()
    } else {
        text.to_uppercase()
    }
}
fn verify_asset_backfills(conn: &Connection) -> Result<()> {
    let mut fees=conn.prepare("SELECT p.kind,o.asset,i.asset FROM transaction_pairs p JOIN transactions o ON o.id=p.out_transaction_id JOIN transactions i ON i.id=p.in_transaction_id WHERE p.swap_fee_msat IS NOT NULL AND p.deleted_at IS NULL").map_err(map_sqlite_error)?;
    for row in fees
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(map_sqlite_error)?
    {
        let (kind, outgoing, incoming) = row.map_err(map_sqlite_error)?;
        let kind = kind.unwrap_or_default();
        if asset_code(&outgoing) == asset_code(&incoming)
            && ![
                "chain-swap",
                "peg-in",
                "peg-out",
                "reverse-submarine-swap",
                "submarine-swap",
                "swap-refund",
            ]
            .contains(&kind.trim_matches(python_whitespace).to_lowercase().as_str())
        {
            return Err(Error::migration("stale_same_asset_fee"));
        }
    }
    const POLICY: [&str; 3] = [
        "6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d",
        "144c654344aa716d6f3abcc1ca90e5641e4e2a7f633bc09fe3baf64585819a49",
        "5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225",
    ];
    let policy_pending: bool = conn
        .query_row(
            "WITH assets AS MATERIALIZED (SELECT DISTINCT asset FROM transactions) SELECT EXISTS(SELECT 1 FROM assets WHERE lower(asset) IN (?1,?2,?3))",
            POLICY,
            |r| r.get(0),
        )
        .map_err(map_sqlite_error)?;
    if policy_pending {
        return Err(Error::migration("liquid_observation_backfill"));
    }
    // Prove the joined Liquid worklist is empty using a covering asset scan,
    // rather than visiting every wide transaction row on ordinary BTC books.
    let liquid: bool = conn.query_row(
        "WITH assets AS MATERIALIZED (SELECT DISTINCT asset FROM transactions) SELECT EXISTS(SELECT 1 FROM assets WHERE upper(replace(asset,'-',''))='LBTC')",
        [], |r| r.get(0),
    ).map_err(map_sqlite_error)?;
    if !liquid {
        return Ok(());
    }
    let mut stmt=conn.prepare("SELECT t.raw_json,w.config_json,t.external_id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE upper(replace(t.asset,'-',''))='LBTC'").map_err(map_sqlite_error)?;
    for row in stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(map_sqlite_error)?
    {
        let (raw, config, external) = row.map_err(map_sqlite_error)?;
        let raw: Value = parse_compat_json(raw.as_deref())?;
        let config: Value = parse_compat_json(config.as_deref())?;
        if !raw.is_object() || !config.is_object() {
            continue;
        }
        if !["liquid", "lbtc", "elements"].contains(
            &value_text(&config["chain"])?
                .trim_matches(python_whitespace)
                .to_lowercase()
                .as_str(),
        ) {
            continue;
        }
        let Some(network) = liquid_network(&value_text(&config["network"])?) else {
            continue;
        };
        let configured = asset_code(&value_text(&config["policy_asset"])?);
        let expected_asset = if configured.is_empty() || configured == "LBTC" {
            POLICY[match network {
                "liquidv1" => 0,
                "liquidtestnet" => 1,
                _ => 2,
            }]
        } else {
            configured.as_str()
        };
        if expected_asset.len() != 64 {
            continue;
        }
        let raw_txid = value_text(&raw["txid"])?;
        let txid = if raw_txid.is_empty() {
            external.as_deref().unwrap_or("")
        } else {
            raw_txid.as_str()
        }
        .trim_matches(python_whitespace)
        .to_lowercase();
        if !crate::valid_hex(&txid, 64) {
            continue;
        }
        let chain = value_text(&raw["chain"])?
            .trim_matches(python_whitespace)
            .to_lowercase();
        let existing_network = value_text(&raw["network"])?;
        let existing_asset_id = asset_code(&value_text(&raw["asset_id"])?);
        let existing_asset = asset_code(&value_text(&raw["asset"])?);
        if (!chain.is_empty() && !["liquid", "lbtc", "elements"].contains(&chain.as_str()))
            || (!existing_network.trim_matches(python_whitespace).is_empty()
                && liquid_network(&existing_network) != Some(network))
            || (!existing_asset_id.is_empty() && existing_asset_id != expected_asset)
            || (!existing_asset.is_empty()
                && existing_asset != "LBTC"
                && existing_asset != expected_asset)
        {
            continue;
        }
        if raw["txid"] != txid
            || raw["chain"] != "liquid"
            || raw["network"] != network
            || raw["asset_id"] != expected_asset
            || raw["asset"] != "LBTC"
        {
            return Err(Error::migration("liquid_observation_backfill"));
        }
    }
    Ok(())
}
fn value_text(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Null => String::new(),
        Value::Bool(false) => String::new(),
        Value::Number(n) if n.as_f64() == Some(0.0) => String::new(),
        Value::Array(v) if v.is_empty() => String::new(),
        Value::Object(v) if v.is_empty() => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => return Err(unsupported_json()),
        _ => value.to_string(),
    })
}
fn liquid_network(value: &str) -> Option<&'static str> {
    match value
        .trim_matches(python_whitespace)
        .to_lowercase()
        .as_str()
    {
        "" | "liquid" | "liquidv1" | "main" | "mainnet" => Some("liquidv1"),
        "liquidtestnet" | "test" | "testnet" => Some("liquidtestnet"),
        "elements" | "elementsregtest" | "regtest" => Some("elementsregtest"),
        _ => None,
    }
}
fn verify_json_backfills(conn: &Connection) -> Result<()> {
    for (sql,key,array) in [
        ("SELECT config_json FROM wallets WHERE config_json LIKE '%\"ownership_history\"%'","ownership_history",true),
        ("SELECT raw_json FROM transactions WHERE payment_hash IS NULL AND raw_json LIKE '%payment_hash%'","payment_hash",false),
    ] {
        let mut stmt=conn.prepare(sql).map_err(map_sqlite_error)?;
        for row in stmt.query_map([],|r|r.get::<_,Option<String>>(0)).map_err(map_sqlite_error)? {
            let raw=row.map_err(map_sqlite_error)?;
            let payload:Value=parse_compat_json(raw.as_deref())?;
            if (array && payload[key].is_array()) || (!array && payload[key].as_str().is_some_and(|s|payment_hash_backfill_candidate(&s.trim_matches(python_whitespace).to_lowercase()))) {
                return Err(migration_detail(if array {"inline_ownership_history"}else{"payment_hash_backfill"}));
            }
        }
    }
    Ok(())
}
fn payment_hash_backfill_candidate(text: &str) -> bool {
    if text.len() != 64 {
        return false;
    }
    let mut nibble = false;
    for byte in text.bytes() {
        if byte.is_ascii_hexdigit() {
            nibble = !nibble;
        } else if !(byte.is_ascii_whitespace() || byte == b'\x0b') || nibble {
            return false;
        }
    }
    !nibble
}
fn migration_detail(name: &str) -> Error {
    Error::migration("semantic_compatibility")
        .details(json!({"reason":"semantic_compatibility","probe":name}))
}
fn verify_gap_relations(conn: &Connection) -> Result<()> {
    // Derive only the durable IDs Python's low-level backfill reads. It never
    // grants accounting authority. Candidate recovery remains Python-only.
    let mut stmt = conn.prepare("SELECT id,workspace_id,profile_id,component_id,snapshot_json,action,event_kind FROM custody_gap_reviews").map_err(map_sqlite_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(map_sqlite_error)?;
    for row in rows {
        let (review, workspace, profile, component, snapshot, action, event) =
            row.map_err(map_sqlite_error)?;
        let mut relations = BTreeSet::<(String, String)>::new();
        let parsed: Value = parse_compat_json(snapshot.as_deref())?;
        for (role, key) in [("source", "source_ids"), ("return", "return_ids")] {
            if let Some(values) = parsed.get(key).and_then(Value::as_array) {
                for id in values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    relations.insert((role.into(), id.into()));
                }
            }
        }
        if let Some(component) = component.as_ref().filter(|s| !s.is_empty()) {
            let mut legs = conn.prepare("SELECT role,coalesce(anchor_transaction_id,transaction_id) FROM custody_component_legs WHERE component_id=? AND workspace_id=? AND profile_id=? AND role IN ('source','fee','destination') AND coalesce(anchor_transaction_id,transaction_id) IS NOT NULL").map_err(map_sqlite_error)?;
            for leg in legs
                .query_map([component, &workspace, &profile], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(map_sqlite_error)?
            {
                let (role, id) = leg.map_err(map_sqlite_error)?;
                relations.insert((
                    if role == "destination" {
                        "return"
                    } else {
                        "source"
                    }
                    .into(),
                    id,
                ));
            }
        }
        let mut existing = conn
            .prepare(
                "SELECT role,transaction_id FROM custody_gap_review_transactions WHERE review_id=?",
            )
            .map_err(map_sqlite_error)?;
        let existing: BTreeSet<_> = existing
            .query_map([&review], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(map_sqlite_error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(map_sqlite_error)?;
        for (role, id) in &relations {
            let collision: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM transactions WHERE id=? AND (workspace_id!=? OR profile_id!=?))", [id,&workspace,&profile], |r| r.get(0)).map_err(map_sqlite_error)?;
            if !collision && !existing.contains(&(role.clone(), id.clone())) {
                return Err(Error::migration("gap_review_relations"));
            }
        }
        let header: Option<(i64,i64)> = conn.query_row("SELECT expected_source_count,expected_return_count FROM custody_gap_review_relation_sets WHERE review_id=?", [&review], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(map_sqlite_error)?;
        if !relations.is_empty() && header.is_none() {
            return Err(Error::migration("gap_review_relation_set"));
        }
        if component.is_none() && action == "dismissed" && event == "review_decision" {
            let count = |role: &str| existing.iter().filter(|(r, _)| r == role).count() as i64;
            if header != Some((count("source"), count("return"))) {
                let observations:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM transactions WHERE profile_id=? AND coalesce(excluded,0)=0)",[&profile],|r|r.get(0)).map_err(map_sqlite_error)?;
                if observations {
                    // Exact recovery depends on the custody candidate matcher.
                    // Do not claim Python migration can repair an unmatched
                    // historical fingerprint, or silently admit a recoverable one.
                    return Err(Error::new(
                        crate::ErrorKind::UnsupportedSchemaLayout,
                        "legacy gap candidate recovery requires a ported custody matcher",
                    )
                    .details(json!({"reason":"legacy_gap_candidate_recovery"})));
                }
            }
        }
    }
    Ok(())
}
use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_refuses_lossy_integers_and_python_collection_strings() {
        assert!(parse_compat_json(Some(r#"{"signed":-9223372036854775808,"unsigned":18446744073709551615,"quoted":"1111111111111111111111111111111111111111111111111111111111111111"}"#)).is_ok());
        for raw in [
            r#"{"txid":1111111111111111111111111111111111111111111111111111111111111111}"#,
            r#"{"value":-18446744073709551616}"#,
            r#"{"value":1e400}"#,
            r#"{"value":-1e400}"#,
        ] {
            assert_eq!(
                parse_compat_json(Some(raw)).unwrap_err().kind,
                crate::ErrorKind::UnsupportedSchemaLayout
            );
        }
        assert!(value_text(&json!(["a", "b"])).is_err());
        assert!(value_text(&json!({"a":1})).is_err());
        assert_eq!(value_text(&json!([])).unwrap(), "");
        assert_eq!(value_text(&json!(0)).unwrap(), "");
    }
    #[test]
    fn payment_hash_admission_matches_all_python_fromhex_whitespace() {
        for byte in *b" \t\n\r\x0b\x0c" {
            let text = format!(
                "{}{}{}{}",
                "ab".repeat(15),
                char::from(byte),
                char::from(byte),
                "cd".repeat(16)
            );
            assert!(payment_hash_backfill_candidate(&text));
        }
        assert!(!payment_hash_backfill_candidate(&format!(
            "a b{}",
            "f".repeat(61)
        )));
    }
    fn current_schema() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        for kind in ["table", "index", "view", "trigger"] {
            for object in manifest["objects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| o[0] == kind)
            {
                if let Some(sql) = object[3].as_str() {
                    conn.execute_batch(sql).unwrap();
                }
            }
        }
        for audit in manifest["required_audits"].as_array().unwrap() {
            conn.execute(
                "INSERT INTO schema_migration_audits VALUES(?1,?1,1,'{}','synthetic')",
                [audit[0].as_str().unwrap()],
            )
            .unwrap();
        }
        conn.execute("INSERT INTO chain_index_clock VALUES(1,0)", [])
            .unwrap();
        conn
    }
    #[test]
    fn exact_objects_and_audits_gate_current_schema() {
        let conn = current_schema();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(objects(&conn).unwrap(), manifest["objects"]);
        verify(&conn).unwrap();
        let fingerprint_before = fingerprint(&conn).unwrap();
        conn.execute(
            "DELETE FROM schema_migration_audits WHERE migration_name='kassiber-tax-engine-v1'",
            [],
        )
        .unwrap();
        assert_eq!(
            verify(&conn).unwrap_err().kind,
            crate::ErrorKind::RequiresPythonMigration
        );
        assert_eq!(fingerprint(&conn).unwrap(), fingerprint_before);
        conn.execute_batch("DROP TRIGGER chain_index_track_transactions_insert")
            .unwrap();
        assert_ne!(fingerprint(&conn).unwrap(), fingerprint_before);
    }
    #[test]
    fn newer_unknown_layouts_and_audits_are_refused() {
        for drift in [
            "CREATE TABLE future_storage(value INTEGER)",
            "ALTER TABLE transactions ADD COLUMN future_column TEXT",
            "CREATE INDEX idx_future_transactions ON transactions(asset)",
            "CREATE TRIGGER trg_future BEFORE INSERT ON ai_provider_checks BEGIN SELECT 1; END",
            "CREATE VIEW future_view AS SELECT 1",
            "INSERT INTO schema_migration_audits VALUES('future-migration-v1','future-migration-v1',1,'{}','fixed')",
            "INSERT INTO schema_migration_audits VALUES('at-fee-consumed-regime-v2','at-fee-consumed-regime-v2',1,'{}','fixed')",
        ] {
            let conn = current_schema();
            verify(&conn).unwrap();
            conn.execute_batch(drift).unwrap();
            assert_eq!(
                verify(&conn).unwrap_err().kind,
                crate::ErrorKind::RequiresPythonMigration,
                "{drift}"
            );
        }
        // The retired audits Python may still hold remain admitted.
        let conn = current_schema();
        conn.execute_batch("INSERT INTO schema_migration_audits VALUES('at-fee-consumed-regime-v1','at-fee-consumed-regime-v1',1,'{}','fixed')").unwrap();
        verify(&conn).unwrap();
    }
    #[test]
    fn historical_layout_uses_normalized_gate_and_still_refuses_constraint_drift() {
        let conn = current_schema();
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        conn.execute_batch("ALTER TABLE transaction_edit_events ADD COLUMN sync_context_json TEXT")
            .unwrap();
        assert_ne!(objects(&conn).unwrap(), manifest["objects"]);
        verify(&conn).unwrap();
        conn.execute_batch("CREATE INDEX unexpected_index ON transactions(asset)")
            .unwrap();
        assert_eq!(
            verify(&conn).unwrap_err().kind,
            crate::ErrorKind::RequiresPythonMigration
        );
    }
}
