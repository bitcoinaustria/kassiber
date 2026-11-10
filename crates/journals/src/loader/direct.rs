//! Direct SQLite -> custody records. Ordering metadata never holds row values.
use super::*;
use kassiber_custody::snapshot::{Columns, Component, GapWorklist, Record, Snapshot};
use serde_json::Value as J;

#[derive(Default)]
struct Gate {
    columns: HashMap<&'static str, Vec<String>>,
    ordered: HashMap<&'static str, Value>,
}

/// The consumer owns its records already; the adapter only checks admission.
pub struct LoadedSnapshot {
    snapshot: Snapshot,
    gate: Gate,
}
impl LoadedSnapshot {
    /// Compare the native composition without a v1/text admission round trip.
    /// This seam is test-only until the native journal orchestrator is routed.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn project(
        &self,
    ) -> std::result::Result<kassiber_custody::Stages, kassiber_custody::Refusal> {
        kassiber_custody::run_loaded(&self.snapshot, true)
    }

    /// The loaded custody snapshot whose rows the projected stages name.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn custody(&self) -> &Snapshot {
        &self.snapshot
    }

    pub fn into_custody(self) -> Result<Snapshot> {
        self.snapshot.validate_loaded().map_err(Error::Invalid)?;
        Ok(self.snapshot)
    }
    pub fn v1(&self) -> Result<String> {
        let value =
            serde_json::to_value(&self.snapshot).map_err(|e| Error::Invalid(e.to_string()))?;
        let value = self.gate.encode(&value, "")?;
        kassiber_pyjson::dumps(&value, &Options::new().ensure_ascii(true).allow_nan(true))
            .map_err(Error::Json)
    }
}
impl Gate {
    fn native(&mut self, path: &'static str, value: Value) -> Result<J> {
        let native = convert(value.clone())?;
        self.ordered.insert(path, value);
        Ok(native)
    }
    fn encode(&self, value: &J, path: &str) -> Result<Value> {
        if let Some(ordered) = self.ordered.get(path) {
            return Ok(ordered.clone());
        }
        if kassiber_custody::codec::raw_bytes(value).is_some()
            || kassiber_custody::codec::escaped_dict(value).is_some()
        {
            return kassiber_custody::codec::from_json(value).map_err(Error::Invalid);
        }
        Ok(match value {
            J::Array(items) => Value::List(
                items
                    .iter()
                    .map(|v| self.encode(v, &format!("{path}[]")))
                    .collect::<Result<_>>()?,
            ),
            J::Object(items) => {
                let fixed: &[&str] = match path {
                    "" => &[
                        "schema_version",
                        "operation",
                        "as_of",
                        "profile",
                        "book_network",
                        "wallets",
                        "transactions",
                        "rate_rows",
                        "owned_index",
                        "ownership_warnings",
                        "components",
                        "anchors",
                        "anchor_wallets",
                        "revision_headers",
                        "receipt_sources",
                        "channel_records",
                        "loan_legs",
                        "evidence_snapshots",
                        "migration_issues",
                        "import_duplicate_quarantines",
                        "reset_hold_quarantines",
                        "gap_reviews",
                        "pair_dismissals",
                        "gap_worklist",
                    ],
                    "book_network" => &["binding", "observations", "references"],
                    "components[]" => &[
                        "header",
                        "legs",
                        "allocations",
                        "economic_terms",
                        "commitments",
                    ],
                    "gap_worklist" => &["large_book", "rows"],
                    _ => &[],
                };
                let columns: Vec<&str> = if let Some(columns) = self.columns.get(path) {
                    columns.iter().map(String::as_str).collect()
                } else if !fixed.is_empty() {
                    fixed.to_vec()
                } else {
                    items.keys().map(String::as_str).collect()
                };
                Value::Dict(
                    columns
                        .into_iter()
                        .filter_map(|key| items.get(key).map(|v| (key, v)))
                        .map(|(key, v)| {
                            let child = if path == "anchors" {
                                "anchors.*".into()
                            } else if path.is_empty() {
                                key.into()
                            } else {
                                format!("{path}.{key}")
                            };
                            Ok((Key::from(key), self.encode(v, &child)?))
                        })
                        .collect::<Result<_>>()?,
                )
            }
            _ => kassiber_custody::codec::from_json(value).map_err(Error::Invalid)?,
        })
    }
}

pub(super) fn scalar(value: ValueRef<'_>) -> Result<J> {
    Ok(match value {
        ValueRef::Null => J::Null,
        ValueRef::Integer(value) => J::Number(value.into()),
        ValueRef::Real(value) => J::Number(
            float_storage(value)?
                .parse()
                .map_err(|_| Error::Invalid("SQLite REAL".into()))?,
        ),
        ValueRef::Text(value) => J::String(
            std::str::from_utf8(value)
                .map_err(|_| Error::Unsupported("SQLite TEXT is not UTF-8"))?
                .into(),
        ),
        ValueRef::Blob(value) => kassiber_custody::codec::to_json(&Value::Bytes(value.to_vec())),
    })
}

#[derive(Default)]
pub(super) struct Reader {
    gate: Gate,
    configs: HashMap<String, (String, bool)>,
    capacity: usize,
}
impl Reader {
    pub(super) fn rows(
        &mut self,
        conn: &Connection,
        path: &'static str,
        sql: &str,
        params: impl Params,
        safe: bool,
    ) -> Result<Vec<Record>> {
        let mut statement = conn.prepare_cached(sql)?;
        let mut seen = BTreeSet::new();
        let columns: Vec<_> = statement
            .column_names()
            .into_iter()
            .enumerate()
            .filter(|(_, name)| seen.insert(*name))
            .filter(|(_, name)| !safe || *name != "_closed_confirmation_observation")
            .map(|(i, key)| (i, key.to_owned()))
            .collect();
        let discarded: Vec<_> = (0..statement.column_count())
            .filter(|index| !columns.iter().any(|(selected, _)| selected == index))
            .collect();
        let mut order: Vec<_> = columns.iter().map(|(_, key)| key.clone()).collect();
        let has_config = safe && columns.iter().any(|(_, key)| key == "config_json");
        if has_config
            && !order
                .iter()
                .any(|key| key == "config_text_mentions_samourai")
        {
            order.push("config_text_mentions_samourai".into());
        }
        let mut columns = columns;
        columns.sort_unstable_by(|a, b| a.1.cmp(&b.1));
        let mut names: Vec<String> = columns.iter().map(|(_, key)| key.clone()).collect();
        if has_config
            && !names
                .iter()
                .any(|key| key == "config_text_mentions_samourai")
        {
            names.push("config_text_mentions_samourai".into());
            names.sort_unstable();
        }
        let mentions_index = names
            .iter()
            .position(|key| key == "config_text_mentions_samourai");
        let layout = Columns::new(names).map_err(Error::Invalid)?;
        self.gate.columns.insert(path, order);
        let mut cursor = statement.query(params)?;
        let mut result = Vec::with_capacity(self.capacity);
        while let Some(row) = cursor.next()? {
            let record = (|| -> Result<Record> {
                for index in &discarded {
                    if let ValueRef::Text(bytes) = row.get_ref(*index)? {
                        sql_value(ValueRef::Text(bytes))?;
                    }
                }
                let mut record = Vec::with_capacity(columns.len() + usize::from(has_config));
                let mut mentions = None;
                for (i, key) in &columns {
                    let raw = row.get_ref(*i)?;
                    let value =
                        if safe && ["config_json", "wallet_config_json"].contains(&key.as_str()) {
                            // Borrow SQLite TEXT for memo lookup. Do not allocate the
                            // raw private config only to replace it with sanitized facts.
                            let normalized = if let ValueRef::Text(bytes) = raw {
                                let raw = std::str::from_utf8(bytes)
                                    .map_err(|_| Error::Unsupported("SQLite TEXT is not UTF-8"))?;
                                if let Some(hit) = self.configs.get(raw) {
                                    hit.clone()
                                } else {
                                    let normalized = normalize(sql_value(ValueRef::Text(bytes))?)?;
                                    self.configs.insert(raw.into(), normalized.clone());
                                    normalized
                                }
                            } else {
                                normalize(sql_value(raw)?)?
                            };
                            if key == "config_json" {
                                mentions = Some(normalized.1);
                            }
                            J::String(normalized.0)
                        } else {
                            scalar(raw)?
                        };
                    record.push(value);
                }
                if let Some(mentions) = mentions {
                    let index = mentions_index.expect("config mention column");
                    if columns
                        .iter()
                        .any(|(_, key)| key == "config_text_mentions_samourai")
                    {
                        record[index] = J::Bool(mentions);
                    } else {
                        record.insert(index, J::Bool(mentions));
                    }
                }
                Ok(Record::new(layout.clone(), record))
            })()
            .map_err(|error| row_error(row, error))?;
            result.push(record);
        }
        Ok(result)
    }
}
fn normalize(value: Value) -> Result<(String, bool)> {
    let row = safe_row(dict([("config_json", value)]), &mut HashMap::new())?;
    Ok((
        text(get(&row, "config_json")),
        truth(get(&row, "config_text_mentions_samourai")),
    ))
}
fn string(row: &Record, key: &str) -> String {
    row.get(key)
        .map(kassiber_custody::codec::pystr)
        .unwrap_or_else(|| "None".into())
}
fn native_rows(value: Value) -> Result<Vec<Record>> {
    let J::Array(values) = convert(value)? else {
        return Err(Error::Invalid("rows".into()));
    };
    values
        .into_iter()
        .map(|v| {
            if let J::Object(row) = v {
                Ok(row.into())
            } else {
                Err(Error::Invalid("row".into()))
            }
        })
        .collect()
}

fn confirmation_quantity_refusals(rows: &[Record]) -> Result<()> {
    for row in rows {
        let version = row.get("observation_authority_version").unwrap_or(&J::Null);
        if version.is_null() {
            continue;
        }
        let version = kassiber_custody::codec::from_json(version).map_err(Error::Invalid)?;
        if ownership::legacy_integer(&version)? != Some(1.into()) {
            continue;
        }
        let raw = row.get("raw_json").unwrap_or(&J::Null);
        let hash = kassiber_custody::evidence::graph_hash(raw)
            .map_err(|error| Error::Invalid(error.to_string()))?
            .0;
        let observed = row.get("observation_graph_hash").unwrap_or(&J::Null);
        if !kassiber_custody::codec::truth(observed)
            || kassiber_custody::codec::pystr(observed) != hash
        {
            continue;
        }
        for field in ["amount", "fee"] {
            let value = row.get(field).unwrap_or(&J::Null);
            if kassiber_custody::codec::truth(value) {
                let value = kassiber_custody::codec::from_json(value).map_err(Error::Invalid)?;
                ownership::outpoint_integer(&value)?;
            }
        }
    }
    Ok(())
}

/// Reads pending writes under the caller's write lock; never repairs or commits.
#[allow(clippy::field_reassign_with_default)] // Snapshot owns a private memo field.
pub fn load(conn: &Connection, profile: Value, as_of: &str) -> Result<LoadedSnapshot> {
    if conn.is_autocommit()
        || conn.transaction_state(Some("main"))? != rusqlite::TransactionState::Write
    {
        return Err(Error::TransactionRequired);
    }
    load_read_snapshot(conn, profile, as_of)
}

#[allow(clippy::field_reassign_with_default)]
pub fn load_read_snapshot(
    conn: &Connection,
    profile: Value,
    as_of: &str,
) -> Result<LoadedSnapshot> {
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            kassiber_custody::journal_support::clear_memos();
        }
    }
    let _memos = Memos;
    if conn.is_autocommit() {
        return Err(Error::TransactionRequired);
    }
    let profile_id = text(get(&profile, "id"));
    let count: usize = conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
        [&profile_id],
        |r| r.get::<_, i64>(0),
    )? as usize;
    let mut reader = Reader {
        capacity: count,
        ..Default::default()
    };
    let network = network(conn, &profile_id, &mut reader)?;
    let transactions = match reader.rows(
        conn,
        "transactions[]",
        TRANSACTIONS.as_str(),
        [&profile_id],
        true,
    ) {
        Err(error) if missing_column(&error) => reader.rows(
            conn,
            "transactions[]",
            LEGACY_TRANSACTIONS.as_str(),
            [&profile_id],
            true,
        ),
        rows => rows,
    }?;
    confirmation_quantity_refusals(&transactions)?;
    reader.capacity = 0;
    let wallets = reader.rows(conn,"wallets[]","SELECT w.id,w.label,w.kind,w.config_json,w.account_id AS wallet_account_id,COALESCE(a.code,'treasury') AS account_code,COALESCE(a.label,'Treasury') AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=?",[&profile_id],true)?;
    let (owned, warnings) = ownership::load(conn, &profile_id, &transactions)?;
    let mut components = vec![];
    let mut anchor_ids = BTreeSet::new();
    let mut wallet_ids = BTreeSet::new();
    let mut revision_ids = BTreeSet::new();
    for header in reader.rows(conn,"components[].header","SELECT * FROM custody_components WHERE profile_id=? AND state='active' ORDER BY created_at,revision,id",[&profile_id],false)? {
        let id = string(&header,"id");
        let legs = reader.rows(conn,"components[].legs[]","SELECT * FROM custody_component_legs WHERE component_id=? ORDER BY ordinal,id",[&id],false)?;
        for leg in &legs {
            for key in ["transaction_id","anchor_transaction_id"] { if leg.get(key).is_some_and(|v| !v.is_null()) { anchor_ids.insert(string(leg,key)); } }
            if leg.get("wallet_id").is_some_and(|v| !v.is_null()) { wallet_ids.insert(string(leg,"wallet_id")); }
        }
        for key in ["supersedes_component_id","superseded_by_component_id"] { if let Some(value) = header.get(key).filter(|v| !v.is_null()) { revision_ids.insert(revisions::Reference::from_json(value)?); } }
        components.push(Component {
            header, legs,
            allocations: reader.rows(conn,"components[].allocations[]","SELECT * FROM custody_component_allocations WHERE component_id=? ORDER BY ordinal,id",[&id],false)?,
            economic_terms: reader.rows(conn,"components[].economic_terms[]","SELECT * FROM custody_component_economic_terms WHERE component_id=? ORDER BY ordinal,id",[&id],false)?,
            commitments: reader.rows(conn,"components[].commitments[]","SELECT * FROM custody_component_evidence_commitments WHERE component_id=? AND profile_id=? ORDER BY ordinal,id",[&id,&profile_id],false)?,
        });
    }
    let mut anchors = serde_json::Map::new();
    for id in anchor_ids {
        let row=reader.rows(conn,"anchors.*","SELECT t.*,w.kind AS wallet_kind,w.config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.id=?",[&id],true)?.into_iter().next().map(|row| J::Object(row.into_map())).unwrap_or(J::Null);
        anchors.insert(id, row);
    }
    let mut anchor_wallets = vec![];
    for id in wallet_ids {
        anchor_wallets.extend(reader.rows(
            conn,
            "anchor_wallets[]",
            "SELECT id,kind,config_json FROM wallets WHERE id=?",
            [&id],
            true,
        )?);
    }
    let mut revision_headers = vec![];
    // INTEGER and REAL share a sortable family, unlike TEXT and BLOB.
    let mixed_revisions = if revision_ids.iter().any(revisions::Reference::is_text) {
        revision_ids.iter().any(|id| !id.is_text())
    } else if revision_ids.iter().any(revisions::Reference::is_bytes) {
        revision_ids.iter().any(|id| !id.is_bytes())
    } else {
        false
    };
    if mixed_revisions {
        return Err(app_refusal(kassiber_envelope::AppError::new(
            "validation",
            "Custody revision references contain incompatible types",
        )));
    }
    for id in revision_ids {
        revision_headers.extend(reader.rows(
            conn,
            "revision_headers[]",
            "SELECT * FROM custody_components WHERE id=?",
            [&id],
            false,
        )?);
    }
    let channel_records = reader.rows(conn,"channel_records[]","SELECT r.txid,r.outpoint,r.tag,r.wallet_id,r.channel_id,r.amount_msat,r.raw_json,w.config_json,b.chain,b.network FROM lightning_node_records r JOIN wallets w ON w.id=r.wallet_id LEFT JOIN backends b ON b.name=r.backend_name WHERE r.profile_id=? AND r.record_type='channel'",[&profile_id],true)?;
    let gap_worklist = if count <= 50_000 {
        reader.capacity = count;
        let rows = reader.rows(conn,"gap_worklist.rows[]","SELECT t.id,t.profile_id,t.wallet_id,w.label AS wallet_label,w.kind AS wallet_kind,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.excluded,t.kind,t.kind_override,t.privacy_boundary,t.external_id,t.raw_json,CASE WHEN w.profile_id=t.profile_id THEN w.config_json ELSE '{}' END AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id",[&profile_id],true)?;
        reader.capacity = 0;
        GapWorklist {
            large_book: false,
            rows,
        }
    } else {
        let worklist = worklist::load(conn, &profile_id, &mut HashMap::new())?;
        let worklist = reader.gate.native("gap_worklist", worklist)?;
        serde_json::from_value(worklist).map_err(|e| Error::Invalid(e.to_string()))?
    };
    // `_latest_first_rows`: a narrow table without the source order's
    // columns keeps the legacy order, then drops the exact prices; one
    // without a profiles table or a currency column keeps every row.
    let attempts: Vec<String> = [true, false]
        .into_iter()
        .flat_map(|filtered| (0..3).map(move |shape| super::rate_rows_sql(shape, filtered)))
        .collect();
    let mut rate_rows = Err(Error::Invalid(String::new()));
    for (index, sql) in attempts.iter().enumerate() {
        rate_rows = reader.rows(conn, "rate_rows[]", sql, [&profile_id], false);
        match &rate_rows {
            Err(error) if index + 1 < attempts.len() && operational(error) => continue,
            _ => break,
        }
    }
    let rate_rows = rate_rows?;
    let gap_reviews = reader.rows(conn,"gap_reviews[]","SELECT r.* FROM custody_gap_reviews r JOIN (SELECT gap_id,MAX(revision) AS revision FROM custody_gap_reviews WHERE profile_id=? GROUP BY gap_id) latest ON latest.gap_id=r.gap_id AND latest.revision=r.revision WHERE r.profile_id=? ORDER BY r.gap_id,r.created_at,r.id",[&profile_id,&profile_id],false).or_else(|error| if operational(&error) { Ok(vec![]) } else { Err(error) })?;
    let owned_index = reader.gate.native("owned_index", owned)?;
    let profile = reader
        .gate
        .native("profile", profile)?
        .as_object()
        .cloned()
        .ok_or(Error::Invalid("profile".into()))?;
    let mut snapshot = Snapshot::default();
    snapshot.schema_version = 1;
    snapshot.operation = "custody".into();
    snapshot.as_of = as_of.into();
    snapshot.profile = profile.into();
    snapshot.book_network = network;
    snapshot.wallets = wallets;
    snapshot.transactions = transactions;
    snapshot.rate_rows = rate_rows;
    snapshot.owned_index = (!owned_index.is_null()).then_some(owned_index);
    snapshot.ownership_warnings = warnings.iter().map(text).collect();
    snapshot.components = components;
    snapshot.anchors = anchors;
    snapshot.anchor_wallets = anchor_wallets;
    snapshot.revision_headers = revision_headers;
    snapshot.channel_records = channel_records;
    snapshot.loan_legs = reader.rows(
        conn,
        "loan_legs[]",
        "SELECT transaction_id,role FROM loan_legs WHERE profile_id=? AND deleted_at IS NULL",
        [&profile_id],
        false,
    )?;
    snapshot.evidence_snapshots = reader.rows(conn,"evidence_snapshots[]","SELECT subject_id,detail_hash,quantity_hash,payload_json,created_at FROM custody_authored_evidence_snapshots WHERE profile_id=? AND subject_kind='custody_component' ORDER BY subject_id,quantity_hash,detail_hash",[&profile_id],false)?;
    snapshot.migration_issues = reader.rows(conn,"migration_issues[]","SELECT * FROM custody_authored_migration_issues WHERE profile_id=? AND resolved_at IS NULL ORDER BY created_at,id",[&profile_id],false)?;
    // Held ID-less import duplicates, derived like the gap worklist. Every
    // book the store opens has the 8f identity tables (the ordered pass adds
    // them and verification refuses a book without them); only an older
    // Python fixture schema lacks them, and gets that Python's snapshot.
    let identity_schema = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='synthetic_import_duplicate_reviews')",
        [],
        |r| r.get::<_, bool>(0),
    )?;
    if identity_schema {
        let duplicates = Value::List(
            kassiber_wallets::import_identity::duplicate_review_quarantines(conn, &profile_id)
                .map_err(app_refusal)?,
        );
        snapshot.import_duplicate_quarantines = Some(native_rows(duplicates.clone())?);
        reader
            .gate
            .ordered
            .insert("import_duplicate_quarantines", duplicates);
    }
    // Kept reset decisions that could not bind; only while the book keeps
    // reset records, as Python adds the key.
    if kassiber_setup::book_reset::has_state(conn, &profile_id).map_err(app_refusal)? {
        let holds = Value::List(
            kassiber_setup::book_reset::held_quarantines(conn, &profile_id).map_err(app_refusal)?,
        );
        snapshot.reset_hold_quarantines = Some(native_rows(holds.clone())?);
        reader.gate.ordered.insert("reset_hold_quarantines", holds);
    }
    snapshot.gap_reviews = gap_reviews;
    snapshot.pair_dismissals = reader.rows(
        conn,
        "pair_dismissals[]",
        "SELECT * FROM transaction_pair_dismissals WHERE profile_id=?",
        [&profile_id],
        false,
    )?;
    snapshot.gap_worklist = gap_worklist;
    let sources = match receipts::load(conn, &profile_id, snapshot.transactions.iter()) {
        Ok(sources) => sources,
        Err(error) => {
            if let Err(prior) = kassiber_custody::receipt_source_error_precedence(&snapshot) {
                let family =
                    if ["ValueError", "TypeError", "KeyError"].contains(&prior.code.as_str()) {
                        prior.code.as_str()
                    } else {
                        "AppError"
                    };
                let mut refusal = crate::Refusal::new(family, prior.message);
                refusal.hint = prior.hint;
                refusal.details = prior.details;
                if family == "AppError" {
                    refusal.code = Some(prior.code);
                }
                return Err(Error::Reference(refusal));
            }
            return Err(error);
        }
    };
    snapshot.receipt_sources = native_rows(list(sources.clone()))?;
    reader.gate.ordered.insert("receipt_sources", list(sources));
    Ok(LoadedSnapshot {
        snapshot,
        gate: reader.gate,
    })
}

fn network(conn: &Connection, profile: &str, reader: &mut Reader) -> Result<Option<J>> {
    if !table_exists(conn, "book_network_bindings")? {
        return Ok(None);
    }
    let binding = reader
        .rows(
            conn,
            "book_network.binding",
            "SELECT * FROM book_network_bindings WHERE profile_id=?",
            [profile],
            false,
        )?
        .into_iter()
        .next()
        .map(|row| J::Object(row.into_map()))
        .unwrap_or(J::Null);
    let observations = reader.rows(conn,"book_network.observations[]","SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id",[profile],true)?;
    let mut references = vec![];
    for table in [
        "chain_analysis_observations",
        "chain_observer_instances",
        "chain_observation_provenance",
        "wallet_policy_epochs",
        "wallet_utxos",
    ] {
        if !table_exists(conn, table)? {
            continue;
        }
        let columns = query(conn, &format!("PRAGMA table_info(\"{table}\")"), [])?;
        if !["profile_id", "chain", "network"]
            .iter()
            .all(|name| columns.iter().any(|row| text(get(row, "name")) == *name))
        {
            continue;
        }
        for source in query(
            conn,
            &format!("SELECT * FROM \"{table}\" WHERE profile_id=?"),
            [profile],
        )? {
            let mut item = dict([
                ("table", Value::from(table)),
                ("chain", get(&source, "chain").clone()),
                ("network", get(&source, "network").clone()),
                ("payload_json", get(&source, "payload_json").clone()),
            ]);
            if table == "wallet_policy_epochs" {
                let private = parsed(get(&source, "private_material_json"));
                set(
                    &mut item,
                    "private_material_present",
                    Value::Bool(matches!(private, Value::Dict(_)) && truth(&private)),
                );
                set(
                    &mut item,
                    "source_present",
                    Value::Bool(
                        !query(
                            conn,
                            "SELECT 1 FROM wallet_policy_sources WHERE epoch_id=? LIMIT 1",
                            [text(get(&source, "id"))],
                        )?
                        .is_empty(),
                    ),
                );
            }
            references.push(item);
        }
    }
    let references = reader
        .gate
        .native("book_network.references", list(references))?;
    Ok(Some(J::Object(
        [
            ("binding".into(), binding),
            (
                "observations".into(),
                J::Array(
                    observations
                        .into_iter()
                        .map(|row| J::Object(row.into_map()))
                        .collect(),
                ),
            ),
            ("references".into(), references),
        ]
        .into_iter()
        .collect(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_loader_adapter_preserves_order_and_refuses_invalid_shapes() {
        assert!(native_rows(Value::Null).is_err());
        assert!(native_rows(list(vec![Value::from(1)])).is_err());
        assert_eq!(
            native_rows(list(vec![dict([("id", Value::from("a"))])])).unwrap()[0]["id"],
            J::from("a")
        );
        let row: Record = serde_json::json!({"id": 7})
            .as_object()
            .unwrap()
            .clone()
            .into();
        assert_eq!(string(&row, "id"), "7");
        assert_eq!(string(&row, "missing"), "None");
        let mut gate = Gate::default();
        let original = dict([("z", Value::from(1)), ("a", Value::from(2))]);
        assert_eq!(
            gate.native("profile", original.clone()).unwrap(),
            serde_json::json!({"a": 2, "z": 1})
        );
        assert_eq!(
            gate.encode(&serde_json::json!({"a": 2, "z": 1}), "profile")
                .unwrap(),
            original
        );
        assert_eq!(
            gate.encode(
                &serde_json::json!({"z": [null, true, 1.5, "x"], "a": 2}),
                "other"
            )
            .unwrap(),
            dict([
                ("a", Value::from(2)),
                (
                    "z",
                    list(vec![
                        Value::Null,
                        Value::Bool(true),
                        Value::Float(1.5),
                        Value::from("x")
                    ])
                )
            ])
        );
        for raw in [
            Value::Null,
            Value::from("invalid"),
            Value::from("[]"),
            dict([]),
        ] {
            assert_eq!(normalize(raw).unwrap(), ("{}".into(), false));
        }
        assert_eq!(
            normalize(dict([
                ("chain", Value::from("bitcoin")),
                ("secret", Value::from("samourai"))
            ]))
            .unwrap(),
            (r#"{"chain": "bitcoin"}"#.into(), true)
        );
        assert_eq!(normalize(Value::from(r#"{"chain":"bitcoin","secret":"samourai","samourai":{"role":"postmix","group_id":"g","section":"s","secret":"drop"}}"#)).unwrap(), (r#"{"chain": "bitcoin", "samourai": {"role": "postmix", "group_id": "g", "section": "s"}}"#.into(), true));
        assert_eq!(
            normalize(Value::from(r#"{"samourai": 7}"#)).unwrap(),
            (r#"{"samourai": null}"#.into(), true)
        );
    }

    #[test]
    fn native_loader_filters_only_safe_confirmation_columns() {
        let conn = Connection::open_in_memory().unwrap();
        let mut reader = Reader::default();
        for value in ["NULL", "7", "1.5", "'legacy'", "x'ff00'"] {
            let rows = reader
                .rows(
                    &conn,
                    "rows[]",
                    &format!(
                        "SELECT {value} AS _closed_confirmation_observation, 'kept' AS ordinary"
                    ),
                    [],
                    true,
                )
                .unwrap();
            assert_eq!(rows[0].iter().count(), 1);
            assert_eq!(rows[0]["ordinary"], J::from("kept"));
            assert_eq!(reader.gate.columns["rows[]"], ["ordinary"]);
        }
        let rows = reader
            .rows(
                &conn,
                "rows[]",
                "SELECT 'retained' AS _closed_confirmation_observation",
                [],
                false,
            )
            .unwrap();
        assert_eq!(
            rows[0]["_closed_confirmation_observation"],
            J::from("retained")
        );
        assert_eq!(
            reader.gate.columns["rows[]"],
            ["_closed_confirmation_observation"]
        );
        assert!(matches!(
            reader.rows(
                &conn,
                "rows[]",
                "SELECT CAST(x'ff' AS TEXT) AS _closed_confirmation_observation",
                [],
                true,
            ),
            Err(Error::Reference(_))
        ));
        let safe = safe_row(
            dict([
                ("_closed_confirmation_observation", Value::from("discarded")),
                ("ordinary", Value::from("kept")),
            ]),
            &mut HashMap::new(),
        )
        .unwrap();
        assert_eq!(safe, dict([("ordinary", Value::from("kept"))]));
    }

    #[test]
    fn native_loader_reads_direct_columns_without_conversion() {
        let conn = Connection::open_in_memory().unwrap();
        let mut reader = Reader::default();
        let rows = reader.rows(&conn,"rows[]","SELECT NULL AS n,9223372036854775807 AS i,1.0 AS f,'钱' AS s,1 AS duplicate,2 AS duplicate",[],false).unwrap();
        assert_eq!(rows[0]["i"].as_i64(), Some(i64::MAX));
        assert_eq!(rows[0]["f"].to_string(), "1.0");
        assert_eq!(rows[0]["duplicate"], J::from(1));
        let value = reader
            .gate
            .encode(
                &J::Array(
                    rows.into_iter()
                        .map(|row| J::Object(row.into_map()))
                        .collect(),
                ),
                "rows",
            )
            .unwrap();
        assert_eq!(kassiber_pyjson::dumps(&value,&Options::new()).unwrap(),"[{\"n\": null, \"i\": 9223372036854775807, \"f\": 1.0, \"s\": \"\\u94b1\", \"duplicate\": 1}]");
        let blobs = reader
            .rows(&conn, "blobs[]", "SELECT x'ff00' AS b", [], false)
            .unwrap();
        assert_eq!(
            kassiber_custody::codec::from_json(&blobs[0]["b"]).unwrap(),
            Value::Bytes(vec![255, 0])
        );
        let blob = reader.gate.encode(&blobs[0]["b"], "blobs[].b").unwrap();
        assert_eq!(
            kassiber_pyjson::dumps(&blob, &Options::new()),
            Err(kassiber_pyjson::Error::BytesNotSerializable)
        );
        for key in ["$kassiber_bytes", "$kassiber_dict"] {
            let literal = dict([(key, Value::from("ff00"))]);
            let stored = convert(literal.clone()).unwrap();
            assert_eq!(
                kassiber_custody::codec::from_json(&stored).unwrap(),
                literal
            );
            assert_eq!(reader.gate.encode(&stored, "literal").unwrap(), literal);
        }
        let rows = reader
            .rows(&conn, "rows[]", "SELECT 1e999 AS f", [], false)
            .unwrap();
        assert_eq!(
            kassiber_custody::codec::from_json(&rows[0]["f"]).unwrap(),
            Value::Float(f64::INFINITY)
        );
        assert!(matches!(
            reader.rows(
                &conn,
                "rows[]",
                "SELECT CAST(x'ff' AS TEXT) AS t",
                [],
                false
            ),
            Err(Error::Reference(_))
        ));
        // Both a cold and a memoized configuration use identical sanitized facts.
        let rows=reader.rows(&conn,"rows[]","SELECT '{\"chain\":\"bitcoin\",\"private\":\"samourai\"}' AS config_json UNION ALL SELECT '{\"chain\":\"bitcoin\",\"private\":\"samourai\"}'",[],true).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], rows[1]);
        assert_eq!(rows[0]["config_json"], J::from("{\"chain\": \"bitcoin\"}"));
        assert_eq!(rows[0]["config_text_mentions_samourai"], J::Bool(true));
        assert_eq!(reader.configs.len(), 1);
        let rows = reader
            .rows(
                &conn,
                "rows[]",
                "SELECT NULL AS wallet_config_json",
                [],
                true,
            )
            .unwrap();
        assert_eq!(rows[0]["wallet_config_json"], J::from("{}"));
        assert!(!rows[0].contains_key("config_text_mentions_samourai"));
        let rows = reader.rows(&conn,"rows[]","SELECT '{\"private\":\"samourai\"}' AS config_json,0 AS config_text_mentions_samourai",[],true).unwrap();
        assert_eq!(rows[0]["config_text_mentions_samourai"], J::Bool(true));
        assert!(matches!(
            reader.rows(
                &conn,
                "rows[]",
                "SELECT CAST(x'ff' AS TEXT) AS config_json",
                [],
                true
            ),
            Err(Error::Reference(_))
        ));
    }
}
