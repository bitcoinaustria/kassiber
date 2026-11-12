use super::value::{json, Value};
use super::{ingestion::ObservationBuilder, AnalysisIndex};
use crate::{database_error, publication::row::rows};
use kassiber_envelope::AppError;
use kassiber_pyjson::Options;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection, Params};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SUBJECTS: usize = 100_000;
pub const MAX_SCRIPT_BYTES: usize = 10_000;
const MATCH_LIMIT: usize = 5_000;

fn text(value: &Value) -> String {
    match value {
        Value::String(v) => v
            .code_points()
            .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
            .collect(),
        _ => kassiber_envelope::py_str(&value.py()),
    }
}

fn truth(value: &Value) -> bool {
    kassiber_envelope::truthy(&value.py())
}

fn read(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>, AppError> {
    rows(conn, sql, params)?
        .into_iter()
        .map(|row| Ok(Value::from_py(&row)))
        .collect()
}

fn parse(value: &Value) -> Result<Value, AppError> {
    let parsed = kassiber_pytext::loads(value.as_str().ok_or_else(super::invalid)?)
        .map_err(|_| super::invalid())?;
    let parsed = Value::from_py(&parsed);
    if parsed.is_object() {
        Ok(parsed)
    } else {
        Err(super::invalid())
    }
}

fn source_rows(
    conn: &Connection,
    profile: &str,
    table: &str,
    tables: &BTreeSet<String>,
    builder: &mut ObservationBuilder<'_>,
) -> Result<Vec<Value>, AppError> {
    if !tables.contains(table) {
        builder.coverage["missing_tables"]
            .as_array_mut()
            .unwrap()
            .push(json!(table));
        return Ok(vec![]);
    }
    let values = if table == "transaction_graph_cache" {
        read(conn, "SELECT * FROM transaction_graph_cache", [])?
    } else {
        read(
            conn,
            &format!("SELECT * FROM {table} WHERE profile_id = ?"),
            [profile],
        )?
    };
    builder.coverage["source_rows"][table] = json!(values.len());
    Ok(values)
}

pub fn build_index(
    conn: &Connection,
    profile: &str,
    observer: &str,
) -> Result<AnalysisIndex, AppError> {
    use crate::publication::wallet::{Sources, SystemSources};
    build_index_at(conn, profile, observer, &SystemSources::default().now())
}

pub fn build_index_at(
    conn: &Connection,
    profile: &str,
    observer: &str,
    instant: &str,
) -> Result<AnalysisIndex, AppError> {
    let outermost = conn.is_autocommit();
    conn.execute_batch("SAVEPOINT chain_analysis_read")
        .map_err(database_error)?;
    let result = capture(conn, profile, observer, instant);
    kassiber_store::debug_assert_no_write(conn, outermost, "chain_analysis_read");
    conn.execute_batch("RELEASE SAVEPOINT chain_analysis_read")
        .map_err(database_error)?;
    result
}

fn capture(
    conn: &Connection,
    profile: &str,
    observer: &str,
    instant: &str,
) -> Result<AnalysisIndex, AppError> {
    let mut builder = ObservationBuilder::read(conn, profile)?;
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(database_error)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(database_error)?;
    let wallets: BTreeMap<_, _> = source_rows(conn, profile, "wallets", &tables, &mut builder)?
        .into_iter()
        .map(|row| (text(&row["id"]), row))
        .collect();
    let transactions = source_rows(conn, profile, "transactions", &tables, &mut builder)?;
    for stored in &transactions {
        let mut row = stored.clone();
        let wallet = wallets
            .get(&text(&row["wallet_id"]))
            .unwrap_or(&Value::Null);
        row["wallet_kind"] = wallet["kind"].clone();
        row["wallet_config_json"] = wallet["config_json"].clone();
        builder.transaction(&row, false)?;
    }
    for row in source_rows(
        conn,
        profile,
        "transaction_graph_cache",
        &tables,
        &mut builder,
    )? {
        builder.reference(&row, false)?;
    }
    for row in source_rows(conn, profile, "wallet_utxos", &tables, &mut builder)? {
        builder.inventory(&row)?;
    }
    if tables.contains("chain_analysis_observations") {
        for row in source_rows(
            conn,
            profile,
            "chain_analysis_observations",
            &tables,
            &mut builder,
        )? {
            builder.reference(&row, true)?;
        }
    }
    if tables.contains("chain_analysis_reference_assertions") {
        let assertions = source_rows(
            conn,
            profile,
            "chain_analysis_reference_assertions",
            &tables,
            &mut builder,
        )?;
        builder.coverage["reference_reconciling_count"] =
            json!(assertions.iter().filter(|row| row["active"] == 2).count());
        for row in assertions {
            if row["active"] == 1 {
                builder.reference(&row, true)?;
            }
        }
    }
    if tables.contains("chain_analysis_labels") {
        for mut row in source_rows(
            conn,
            profile,
            "chain_analysis_labels",
            &tables,
            &mut builder,
        )? {
            if truth(&row["deleted"]) {
                continue;
            }
            if let Ok((chain, network)) = super::ingestion::scope(&row) {
                row["chain"] = json!(chain);
                row["network"] = json!(network);
                builder.labels.push(row);
            } else {
                builder.coverage["invalid_observations"] =
                    json!(builder.coverage["invalid_observations"].as_u64().unwrap() + 1);
            }
        }
        builder.labels.sort_by_key(|row| text(&row["id"]));
    }
    if tables.contains("chain_analysis_datasets") {
        attach_datasets(conn, profile, observer, instant, &mut builder)?;
    }
    let profiles = if tables.contains("profiles") {
        read(conn, "SELECT * FROM profiles WHERE id = ?", [profile])?
    } else {
        vec![]
    };
    let profile_row = profiles.first().unwrap_or(&Value::Null);
    let fresh = truth(&profile_row["last_processed_at"])
        && super::value::journal_semantics_current(profile_row)
        && profile_row["journal_input_version"] == profile_row["last_processed_input_version"]
        && profile_row["last_processed_tx_count"]
            == json!(transactions
                .iter()
                .filter(|row| !truth(&row["excluded"]))
                .count());
    let decisions = source_rows(
        conn,
        profile,
        "journal_custody_decisions",
        &tables,
        &mut builder,
    )?;
    let economics = source_rows(
        conn,
        profile,
        "journal_custody_economic_relations",
        &tables,
        &mut builder,
    )?;
    builder.relations(&decisions, &economics, fresh)?;
    if !fresh {
        builder.finding("custody_projection_stale", vec![], "Canonical custody relations require a current journal rebuild before they can be traversed.", true, vec![])?;
    }
    builder.finish()
}

fn partition() -> Value {
    json!({"match_count":0,"active_dataset_count":0,"inactive_by_date_count":0,"lookup_queries":0,"match_limit":MATCH_LIMIT,"truncated":false})
}

fn add(value: &mut Value, key: &str) {
    value[key] = json!(value[key].as_u64().unwrap() + 1);
}

pub(super) fn attach_datasets(
    conn: &Connection,
    profile: &str,
    observer: &str,
    instant: &str,
    builder: &mut ObservationBuilder<'_>,
) -> Result<(), AppError> {
    if !["public", "owner", "disclosed"].contains(&observer) {
        return Err(AppError::new(
            "validation",
            "Invalid dataset observer or match limit",
        ));
    }
    let mut observed = BTreeSet::new();
    let mut aliases = Vec::new();
    let mut skipped = 0;
    for (id, node) in &builder.nodes {
        let (Some(chain @ ("bitcoin" | "liquid")), Some(network)) =
            (node["chain"].as_str(), node["network"].as_str())
        else {
            continue;
        };
        let subject = match node["kind"].as_str() {
            Some("transaction") => format!("tx:{}", text(&node["txid"])),
            Some("output") => {
                if let Some(script) = builder
                    .output_facts
                    .get(id)
                    .and_then(|fact| fact["script"].as_str())
                    .filter(|s| !s.is_empty())
                {
                    if script.len() <= MAX_SCRIPT_BYTES * 2 {
                        observed.insert((
                            chain.to_owned(),
                            network.to_owned(),
                            format!("script:{script}"),
                        ));
                    } else {
                        skipped += 1;
                    }
                }
                format!("out:{}", text(&node["outpoint"]))
            }
            _ => continue,
        };
        observed.insert((chain.to_owned(), network.to_owned(), subject.clone()));
        aliases.push((subject, id.clone()));
    }
    for (alias, id) in aliases {
        builder.alias(&json!(alias), &id);
    }
    let mut grouped = BTreeMap::<(String, String), Vec<String>>::new();
    for (chain, network, subject) in observed.iter().take(MAX_SUBJECTS) {
        if subject.strip_prefix("script:").is_some_and(|script| {
            script.is_empty()
                || !script.len().is_multiple_of(2)
                || !script.bytes().all(|b| b.is_ascii_hexdigit())
        }) || subject.strip_prefix("out:").is_some_and(|outpoint| {
            outpoint
                .rsplit_once(':')
                .is_none_or(|(_, index)| index.len() > 10 || index.parse::<u32>().is_err())
        }) {
            return Err(AppError::new(
                "validation",
                "Attribution subject must be a valid address, transaction, output or script in the selected domain",
            ));
        }
        grouped
            .entry((chain.clone(), network.clone()))
            .or_default()
            .push(subject.clone());
    }
    let clause = if observer == "public" {
        " AND visibility='public'"
    } else {
        ""
    };
    let mut packs = read(conn, &format!("SELECT * FROM chain_analysis_datasets WHERE profile_id=? AND status='active'{clause} ORDER BY id"), [profile])?;
    for pack in &mut packs {
        let manifest = parse(&pack["manifest_json"])?;
        if ["valid_from", "valid_until"]
            .iter()
            .any(|key| !matches!(manifest.get(key), Some(Value::Null | Value::String(_))))
            || ["source", "attribution_method", "license"]
                .iter()
                .any(|key| manifest.get(key).is_none())
        {
            return Err(super::invalid());
        }
        let pack = pack.as_object_mut().unwrap();
        pack.remove("profile_id");
        pack.remove("manifest_json");
        pack.insert("manifest".into(), manifest);
        pack.insert("accounting_authority".into(), json!(false));
    }
    let encoded = kassiber_pyjson::dumps(
        &json!(packs).py(),
        &Options::new()
            .sort_keys(true)
            .compact()
            .ensure_ascii(false)
            .allow_nan(false),
    )
    .map_err(|_| super::invalid())?;
    let state = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let visibilities = if observer == "public" {
        &["public"][..]
    } else {
        &["public", "private"][..]
    };
    let mut partitions = BTreeMap::<&str, Value>::new();
    let mut found = BTreeMap::<&str, Vec<Value>>::new();
    for visibility in visibilities {
        partitions.insert(visibility, partition());
        found.insert(visibility, vec![]);
    }
    for pack in &packs {
        let visibility = pack["visibility"].as_str().ok_or_else(super::invalid)?;
        let partition = partitions.get_mut(visibility).ok_or_else(super::invalid)?;
        add(partition, "active_dataset_count");
        let manifest = &pack["manifest"];
        if manifest["valid_from"]
            .as_str()
            .is_some_and(|v| !v.is_empty() && instant < v)
            || manifest["valid_until"]
                .as_str()
                .is_some_and(|v| !v.is_empty() && instant >= v)
        {
            add(partition, "inactive_by_date_count");
            continue;
        }
        if partition["truncated"] == true {
            continue;
        }
        let keys = grouped.get(&(text(&pack["chain"]), text(&pack["network"])));
        let matches = found.get_mut(visibility).unwrap();
        for chunk in keys.into_iter().flatten().collect::<Vec<_>>().chunks(200) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut params = vec![Sql::Text(text(&pack["id"]))];
            params.extend(chunk.iter().map(|v| Sql::Text((*v).clone())));
            params.extend([
                Sql::Text(instant.to_owned()),
                Sql::Text(instant.to_owned()),
                Sql::Integer((MATCH_LIMIT + 1 - matches.len()) as i64),
            ]);
            add(partition, "lookup_queries");
            for row in read(conn, &format!("SELECT * FROM chain_analysis_dataset_claims INDEXED BY idx_chain_analysis_dataset_subject WHERE dataset_id=? AND subject IN ({marks}) AND (valid_from IS NULL OR valid_from<=?) AND (valid_until IS NULL OR valid_until>?) ORDER BY subject,record_number LIMIT ?"), params_from_iter(params))? {
                let source = parse(&row["source_record_json"])?;
                matches.push(json!({"id":format!("dataset:{}:{}",text(&pack["id"]),text(&row["record_number"])),"revision":1,"dataset_id":pack["id"],"dataset_status":pack["status"],"chain":pack["chain"],"network":pack["network"],"subject":row["subject"],"label":row["label"],"category":row["category"],"source":source.get("source").unwrap_or(&manifest["source"]),"confidence":"imported","cluster_defining":false,"visibility":pack["visibility"],"attribution_method":source.get("attribution_method").unwrap_or(&manifest["attribution_method"]),"source_record":source,"valid_from":row["valid_from"],"valid_until":row["valid_until"],"dataset_version":pack["version"],"content_sha256":pack["content_sha256"],"license":manifest["license"],"evidence_level":"sourced_claim","accounting_authority":false}));
            }
            if matches.len() > MATCH_LIMIT {
                partition["truncated"] = json!(true);
                break;
            }
        }
    }
    let mut count = 0;
    for visibility in visibilities {
        let matches = &found[visibility];
        let size = MATCH_LIMIT.min(matches.len());
        partitions.get_mut(visibility).unwrap()["match_count"] = json!(size);
        count += size;
        builder
            .labels
            .extend(matches.iter().take(MATCH_LIMIT).cloned());
    }
    let total = |key: &str| {
        partitions
            .values()
            .map(|p| p[key].as_u64().unwrap())
            .sum::<u64>()
    };
    builder.coverage["datasets"] = json!({"state_digest":state,"subject_count":grouped.values().map(Vec::len).sum::<usize>(),"match_count":count,"active_dataset_count":packs.len(),"inactive_by_date_count":total("inactive_by_date_count"),"lookup_queries":total("lookup_queries"),"match_limit":MATCH_LIMIT,"match_limit_per_visibility":true,"truncated":partitions.values().any(|p|p["truncated"]==true),"visibility_coverage":partitions,"complete_chain_coverage":false,"authority":"imported_attribution_claims","subject_limit":MAX_SUBJECTS,"subjects_truncated":observed.len()>MAX_SUBJECTS,"scripts_skipped":skipped});
    Ok(())
}
