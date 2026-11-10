use super::super::{field, fields, list, AnalysisIndex, Text};
use super::*;
use crate::publication::wallet::{Sources, SystemSources};
use kassiber_pyjson::Value as Py;

fn first(conn: &Connection, sql: &str, params: impl Params) -> Result<Value, AppError> {
    Ok(read(conn, sql, params)?
        .into_iter()
        .next()
        .unwrap_or(Value::Null))
}

fn source_rows(
    conn: &Connection,
    profile: &str,
    table: &str,
    keys: Option<&BTreeSet<i64>>,
) -> Result<BTreeMap<i64, Value>, AppError> {
    let mut values = Vec::new();
    if let Some(keys) = keys {
        for chunk in keys.iter().collect::<Vec<_>>().chunks(200) {
            let marks = vec!["?"; chunk.len()].join(",");
            values.extend(read(
                conn,
                &format!("SELECT rowid AS _source_key,* FROM {table} WHERE rowid IN ({marks})"),
                params_from_iter(chunk.iter().map(|v| **v)),
            )?);
        }
    } else {
        let scoped = table != "transaction_graph_cache";
        values = read(
            conn,
            &format!(
                "SELECT rowid AS _source_key,* FROM {table}{}",
                if scoped { " WHERE profile_id=?" } else { "" }
            ),
            params_from_iter(scoped.then_some(profile)),
        )?;
    }
    values
        .into_iter()
        .map(|row| Ok((integer(&row["_source_key"])?, row)))
        .collect()
}

fn relations(
    conn: &Connection,
    profile: &str,
    tables: &BTreeSet<String>,
    builder: &mut ObservationBuilder<'_>,
) -> Result<(), AppError> {
    let profile_row = if tables.contains("profiles") {
        first(conn, "SELECT * FROM profiles WHERE id=?", [profile])?
    } else {
        Value::Null
    };
    let count = if tables.contains("transactions") {
        integer(
            &first(
                conn,
                "SELECT count(*) AS n FROM transactions WHERE profile_id=? AND excluded=0",
                [profile],
            )?["n"],
        )?
    } else {
        0
    };
    let fresh = kassiber_envelope::truthy(&profile_row["last_processed_at"].py())
        && super::super::value::journal_semantics_current(&profile_row)
        && profile_row["journal_input_version"] == profile_row["last_processed_input_version"]
        && profile_row["last_processed_tx_count"] == json!(count);
    let mut groups = Vec::new();
    for table in [
        "journal_custody_decisions",
        "journal_custody_economic_relations",
    ] {
        groups.push(if tables.contains(table) {
            read(
                conn,
                &format!("SELECT * FROM {table} WHERE profile_id=?"),
                [profile],
            )?
        } else {
            vec![]
        });
    }
    let anchors: BTreeSet<_> = groups
        .iter()
        .flatten()
        .flat_map(|row| {
            ["source_transaction_id", "target_transaction_id"]
                .map(|key| row[key].as_str().map(str::to_owned))
        })
        .flatten()
        .collect();
    for record in anchors {
        if builder.records.contains_key(&record)
            || first(
                conn,
                "SELECT 1 AS n FROM transactions WHERE profile_id=? AND id=?",
                params![profile, record],
            )?
            .is_null()
        {
            continue;
        }
        let alias = first(conn,"SELECT node_id FROM chain_index_aliases WHERE profile_id=? AND observer='owner' AND alias=?",params![profile,format!("record:{record}")])?;
        if let Some(id) = alias["node_id"].as_str() {
            builder.records.insert(record, id.to_owned());
        }
    }
    builder.relations(&groups[0], &groups[1], fresh)?;
    if !fresh {
        builder.finding("custody_projection_stale",vec![],"Canonical custody relations require a current journal rebuild before they can be traversed.",true,vec![])?;
    }
    Ok(())
}

fn assembled_builder<'a>(
    conn: &'a Connection,
    profile: &str,
    sources: &Contributions,
) -> Result<ObservationBuilder<'a>, AppError> {
    let mut builder = ObservationBuilder::read(conn, profile)?;
    for ((table, _), row) in sources {
        apply(&mut builder, PHYSICAL[*table], row)?;
    }
    Ok(builder)
}

fn assemble(
    conn: &Connection,
    profile: &str,
    mut core: BTreeSet<String>,
    tables: &BTreeSet<String>,
    full: bool,
) -> Result<(AnalysisIndex, BTreeSet<String>), AppError> {
    if !full {
        for node in core.clone().iter().filter(|node| node.contains(":out:")) {
            core.extend(
                read(
                    conn,
                    "SELECT target FROM chain_index_edges WHERE profile_id=? AND source=?",
                    params![profile, node],
                )?
                .iter()
                .map(|row| text(&row["target"])),
            );
        }
    }
    let mut sources = contributors(conn, profile, if full { None } else { Some(&core) })?;
    let mut builder = assembled_builder(conn, profile, &sources)?;
    if full {
        core.extend(builder.nodes.keys().cloned());
    }
    for row in read(conn,"SELECT source,target FROM chain_index_edges WHERE profile_id=? AND json_extract(payload_json,'$.kind')='custody'",[profile])? { core.extend([text(&row["source"]),text(&row["target"])]); }
    relations(conn, profile, tables, &mut builder)?;
    for edge in builder
        .edges
        .values()
        .filter(|edge| edge["kind"] == "custody")
    {
        core.extend([text(&edge["source"]), text(&edge["target"])]);
    }
    let more = contributors(conn, profile, Some(&core))?;
    if more.keys().any(|key| !sources.contains_key(key)) {
        sources.extend(more);
        builder = assembled_builder(conn, profile, &sources)?;
        relations(conn, profile, tables, &mut builder)?;
    }
    let context: Vec<_> = builder
        .nodes
        .keys()
        .filter(|id| !core.contains(*id))
        .cloned()
        .collect();
    for id in context {
        let stored = first(
            conn,
            "SELECT payload_json,output_json FROM chain_index_nodes WHERE profile_id=? AND id=?",
            params![profile, id],
        )?;
        if !stored.is_null() {
            builder.restore_node(&id, parse(&stored["payload_json"])?);
            if !stored["output_json"].is_null() {
                builder.restore_output(&id, parse(&stored["output_json"])?);
            }
        }
    }
    let endpoints: BTreeSet<_> = builder
        .edges
        .values()
        .flat_map(|edge| [text(&edge["source"]), text(&edge["target"])])
        .collect();
    for id in endpoints {
        if !builder.nodes.contains_key(&id) {
            let stored = first(
                conn,
                "SELECT payload_json FROM chain_index_nodes WHERE profile_id=? AND id=?",
                params![profile, id],
            )?;
            if !stored.is_null() {
                builder.restore_node(&id, parse(&stored["payload_json"])?);
            }
        }
    }
    for (id, edge) in &mut builder.edges {
        let output = match edge["kind"].as_str() {
            Some("creates") => Some(text(&edge["target"])),
            Some("spends") => Some(text(&edge["source"])),
            _ => None,
        };
        if output.is_some_and(|output| !core.contains(&output)) {
            let stored = first(
                conn,
                "SELECT payload_json FROM chain_index_edges WHERE profile_id=? AND id=?",
                params![profile, id],
            )?;
            if !stored.is_null() {
                *edge = parse(&stored["payload_json"])?;
            }
        }
    }
    Ok((builder.finish()?, core))
}

fn encode_py(value: &Py) -> Result<String, AppError> {
    encoded(&Value::from_py(value))
}
fn optional(value: &Py) -> Result<Option<String>, AppError> {
    if value == &Py::Null {
        Ok(None)
    } else {
        encode_py(value).map(Some)
    }
}

fn save(
    conn: &Connection,
    profile: &str,
    index: &AnalysisIndex,
    core: &BTreeSet<String>,
) -> Result<(), AppError> {
    let public = index.observer("public")?;
    let owner = index.snapshot();
    let public = public.snapshot();
    for id in core {
        write(
            conn,
            "DELETE FROM chain_index_nodes WHERE profile_id=? AND id=?",
            params![profile, id],
        )
        .map_err(database_error)?;
        write(
            conn,
            "DELETE FROM chain_index_aliases WHERE profile_id=? AND node_id=?",
            params![profile, id],
        )
        .map_err(database_error)?;
        write(
            conn,
            "DELETE FROM chain_index_edges WHERE profile_id=? AND (source=? OR target=?)",
            params![profile, id, id],
        )
        .map_err(database_error)?;
        for finding in read(
            conn,
            "SELECT finding_id FROM chain_index_finding_nodes WHERE profile_id=? AND node_id=?",
            params![profile, id],
        )? {
            let ident = text(&finding["finding_id"]);
            let stored = first(
                conn,
                "SELECT payload_json FROM chain_index_findings WHERE profile_id=? AND id=?",
                params![profile, ident],
            )?;
            if !stored.is_null()
                && parse(&stored["payload_json"])?["node_ids"]
                    .as_array()
                    .and_then(|ids| ids.first())
                    .is_some_and(|first| !core.contains(&text(first)))
            {
                continue;
            }
            write(
                conn,
                "DELETE FROM chain_index_findings WHERE profile_id=? AND id=?",
                params![profile, ident],
            )
            .map_err(database_error)?;
            write(
                conn,
                "DELETE FROM chain_index_finding_nodes WHERE profile_id=? AND finding_id=?",
                params![profile, ident],
            )
            .map_err(database_error)?;
        }
        let node = field(field(owner, "nodes"), id);
        if node != &Py::Null {
            let values = [
                node,
                field(field(public, "nodes"), id),
                field(field(owner, "transaction_facts"), id),
                field(field(public, "transaction_facts"), id),
                field(field(owner, "output_facts"), id),
                field(field(public, "output_facts"), id),
            ]
            .into_iter()
            .map(optional)
            .collect::<Result<Vec<_>, _>>()?;
            write(
                conn,
                "INSERT INTO chain_index_nodes VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    profile,
                    id,
                    values[0],
                    values[1],
                    values[2],
                    values[3],
                    values[4],
                    values[5],
                    i64::from(list(field(owner, "profile_seeds")).contains(&Py::from(id.as_str()))),
                    field(node, "status").as_str(),
                    field(node, "kind").as_str(),
                    i64::from(kassiber_envelope::truthy(field(
                        field(field(owner, "transaction_facts"), id),
                        "complete"
                    )))
                ],
            )
            .map_err(database_error)?;
        }
    }
    for (id, edge) in fields(field(owner, "edges")) {
        let source = field(edge, "source")
            .as_str()
            .ok_or_else(super::super::invalid)?;
        let target = field(edge, "target")
            .as_str()
            .ok_or_else(super::super::invalid)?;
        if core.contains(source) || core.contains(target) {
            write(
                conn,
                "INSERT OR REPLACE INTO chain_index_edges VALUES(?,?,?,?,?,?)",
                params![
                    profile,
                    id,
                    source,
                    target,
                    encode_py(edge)?,
                    optional(field(field(public, "edges"), id))?
                ],
            )
            .map_err(database_error)?;
        }
    }
    for (observer, view) in [("owner", owner), ("public", public)] {
        let Py::Dict(subjects) = field(view, "subjects") else {
            return Err(super::super::invalid());
        };
        for (alias, ids) in subjects {
            let kassiber_pyjson::Key::Str(alias) = alias else {
                return Err(super::super::invalid());
            };
            for id in list(ids)
                .iter()
                .filter_map(Text::as_str)
                .filter(|id| core.contains(*id))
            {
                write(
                    conn,
                    "INSERT OR IGNORE INTO chain_index_aliases VALUES(?,?,?,?)",
                    params![
                        profile,
                        observer,
                        alias.as_str().ok_or_else(super::super::invalid)?,
                        id
                    ],
                )
                .map_err(database_error)?;
            }
        }
        for (id, node) in fields(field(view, "nodes")).filter(|(id, _)| core.contains(*id)) {
            let kind = field(node, "kind").as_str();
            if matches!(kind, Some("transaction" | "output")) {
                let alias = format!(
                    "{}:{}",
                    if kind == Some("transaction") {
                        "tx"
                    } else {
                        "out"
                    },
                    kassiber_envelope::py_str(field(
                        node,
                        if kind == Some("transaction") {
                            "txid"
                        } else {
                            "outpoint"
                        }
                    ))
                );
                write(
                    conn,
                    "INSERT OR IGNORE INTO chain_index_aliases VALUES(?,?,?,?)",
                    params![profile, observer, alias, id],
                )
                .map_err(database_error)?;
            }
        }
    }
    write(conn, "DELETE FROM chain_index_findings WHERE profile_id=? AND json_array_length(json_extract(payload_json,'$.node_ids'))=0",[profile]).map_err(database_error)?;
    let public_findings: BTreeMap<_, _> = list(field(public, "findings"))
        .iter()
        .map(|v| (field(v, "id").as_str().unwrap(), v))
        .collect();
    for finding in list(field(owner, "findings")) {
        let ids = list(field(finding, "node_ids"));
        if ids
            .first()
            .is_none_or(|id| core.contains(id.as_str().unwrap()))
        {
            let id = field(finding, "id").as_str().unwrap();
            write(
                conn,
                "INSERT OR REPLACE INTO chain_index_findings VALUES(?,?,?,?)",
                params![
                    profile,
                    id,
                    encode_py(finding)?,
                    optional(public_findings.get(id).copied().unwrap_or(&Py::Null))?
                ],
            )
            .map_err(database_error)?;
            for node in ids {
                write(
                    conn,
                    "INSERT OR IGNORE INTO chain_index_finding_nodes VALUES(?,?,?)",
                    params![profile, id, node.as_str().unwrap()],
                )
                .map_err(database_error)?;
            }
        }
    }
    Ok(())
}

fn labels(
    conn: &Connection,
    profile: &str,
    tables: &BTreeSet<String>,
    coverage: &mut Value,
    instant: &str,
) -> Result<(), AppError> {
    let mut labels = Vec::new();
    if tables.contains("chain_analysis_labels") {
        for mut row in read(
            conn,
            "SELECT * FROM chain_analysis_labels WHERE profile_id=? AND deleted=0 ORDER BY id",
            [profile],
        )? {
            if let Ok((chain, network)) = super::super::ingestion::scope(&row) {
                row["chain"] = json!(chain);
                row["network"] = json!(network);
                labels.push(row);
            } else {
                coverage["invalid_observations"] =
                    json!(integer(&coverage["invalid_observations"])? + 1);
            }
        }
    }
    if tables.contains("chain_analysis_datasets") {
        let mut builder = ObservationBuilder::read(conn, profile)?;
        if !first(conn,"SELECT 1 AS n FROM chain_analysis_datasets WHERE profile_id=? AND status='active' LIMIT 1",[profile])?.is_null() {
            for row in read(conn,"SELECT id,payload_json,output_json FROM chain_index_nodes WHERE profile_id=?",[profile])? {
                let id=text(&row["id"]);
                builder.restore_node(&id,parse(&row["payload_json"])?);
                if !row["output_json"].is_null() {builder.restore_output(&id,parse(&row["output_json"])?);}
            }
        }
        super::super::sources::attach_datasets(conn, profile, "owner", instant, &mut builder)?;
        labels.extend(builder.labels);
        coverage["datasets"] = builder.coverage["datasets"].clone();
    }
    write(
        conn,
        "DELETE FROM chain_index_labels WHERE profile_id=?",
        [profile],
    )
    .map_err(database_error)?;
    write(
        conn,
        "DELETE FROM chain_index_label_nodes WHERE profile_id=?",
        [profile],
    )
    .map_err(database_error)?;
    for mut label in labels {
        let mut ids = Vec::new();
        for row in read(conn,"SELECT n.id,n.payload_json FROM chain_index_aliases a JOIN chain_index_nodes n ON n.profile_id=a.profile_id AND n.id=a.node_id WHERE a.profile_id=? AND a.observer='owner' AND a.alias=? ORDER BY n.id",params![profile,label["subject"].as_str().ok_or_else(super::super::invalid)?])? {
            let node=parse(&row["payload_json"])?;
            if node["chain"]==label["chain"]&&node["network"]==label["network"] {ids.push(text(&row["id"]));}
        }
        label["node_ids"] = json!(ids);
        let id = text(&label["id"]);
        write(
            conn,
            "INSERT OR REPLACE INTO chain_index_labels VALUES(?,?,?)",
            params![profile, id, encoded(&label)?],
        )
        .map_err(database_error)?;
        for id_node in ids {
            write(
                conn,
                "INSERT OR IGNORE INTO chain_index_label_nodes VALUES(?,?,?)",
                params![profile, id, id_node],
            )
            .map_err(database_error)?;
        }
    }
    Ok(())
}

fn next_expiry(
    conn: &Connection,
    profile: &str,
    tables: &BTreeSet<String>,
    instant: &str,
) -> Result<Option<String>, AppError> {
    if !tables.contains("chain_analysis_datasets") {
        return Ok(None);
    }
    let mut values = BTreeSet::new();
    for field in ["valid_from", "valid_until"] {
        let row=first(conn,&format!("SELECT min(c.{field}) AS value FROM chain_analysis_dataset_claims c JOIN chain_analysis_datasets d ON d.id=c.dataset_id WHERE d.profile_id=? AND d.status='active' AND c.{field}>?"),params![profile,instant])?;
        if let Some(value) = row["value"].as_str().filter(|v| !v.is_empty()) {
            values.insert(value.to_owned());
        }
    }
    for row in read(
        conn,
        "SELECT manifest_json FROM chain_analysis_datasets WHERE profile_id=? AND status='active'",
        [profile],
    )? {
        let manifest = parse(&row["manifest_json"])?;
        for field in ["valid_from", "valid_until"] {
            if let Some(value) = manifest[field]
                .as_str()
                .filter(|value| !value.is_empty() && *value > instant)
            {
                values.insert(value.to_owned());
            }
        }
    }
    Ok(values.into_iter().next())
}

pub fn synchronize(conn: &Connection, profile: &str, rebuild: bool) -> Result<Py, AppError> {
    synchronize_with_sources(conn, profile, rebuild, &mut SystemSources::default())
}

pub fn synchronize_with_sources(
    conn: &Connection,
    profile: &str,
    rebuild: bool,
    sources: &mut dyn Sources,
) -> Result<Py, AppError> {
    crate::publication::row::require_transaction(conn)?;
    let tables = install(conn)?;
    let mut state = first(
        conn,
        "SELECT * FROM chain_index_state WHERE profile_id=?",
        [profile],
    )?;
    let watermark = integer(
        &first(
            conn,
            "SELECT revision FROM chain_index_clock WHERE id=1",
            [],
        )?["revision"],
    )?;
    let instant = sources.now();
    let full = rebuild || state.is_null() || state["version"] != json!(VERSION);
    let expired = state["next_expiry"]
        .as_str()
        .is_some_and(|value| !value.is_empty() && value <= instant.as_str());
    if !full && state["watermark"] == json!(watermark) && !expired {
        return Ok(state.py());
    }
    let mut dirty = BTreeMap::<String, BTreeSet<i64>>::new();
    if !full {
        for row in read(
            conn,
            "SELECT source_table,source_key FROM chain_index_dirty WHERE revision>?",
            [integer(&state["watermark"])?],
        )? {
            dirty
                .entry(text(&row["source_table"]))
                .or_default()
                .insert(integer(&row["source_key"])?);
        }
    }
    if dirty.contains_key("wallets") || dirty.contains_key("book_network_bindings") {
        for row in read(
            conn,
            "SELECT rowid AS key FROM transactions WHERE profile_id=?",
            [profile],
        )? {
            dirty
                .entry("transactions".into())
                .or_default()
                .insert(integer(&row["key"])?);
        }
    }
    if dirty.contains_key("book_network_bindings") {
        for table in PHYSICAL {
            dirty.entry(table.into()).or_default().insert(-1);
        }
    }
    if !full && dirty.contains_key("chain_analysis_reference_assertions") {
        let keys = dirty["chain_analysis_reference_assertions"].clone();
        let mut ids: BTreeSet<_> = source_rows(
            conn,
            profile,
            "chain_analysis_reference_assertions",
            if keys.contains(&-1) {
                None
            } else {
                Some(&keys)
            },
        )?
        .values()
        .map(|row| text(&row["txid"]))
        .collect();
        for key in keys {
            let old=first(conn,"SELECT payload_json FROM chain_index_sources WHERE profile_id=? AND source_table='chain_analysis_reference_assertions' AND source_key=?",params![profile,key])?;
            if !old.is_null() {
                ids.insert(text(&parse(&old["payload_json"])?["txid"]));
            }
        }
        for txid in ids {
            for row in read(conn,"SELECT DISTINCT s.source_table,s.source_key FROM chain_index_aliases a JOIN chain_index_source_nodes s ON s.profile_id=a.profile_id AND s.node_id=a.node_id WHERE a.profile_id=? AND a.observer='owner' AND a.alias=?",params![profile,txid])? {dirty.entry(text(&row["source_table"])).or_default().insert(integer(&row["source_key"])?);}
        }
    }
    if !full
        && !expired
        && dirty.len() == 1
        && dirty.contains_key("chain_analysis_acquisition_grants")
    {
        let revision = integer(&state["revision"])? + 1;
        let snapshot =
            super::super::digest(&json!([state["instance_id"], profile, VERSION, revision]).py())?;
        write(
            conn,
            "UPDATE chain_index_state SET watermark=?,revision=?,snapshot_id=? WHERE profile_id=?",
            params![watermark, revision, snapshot, profile],
        )
        .map_err(database_error)?;
        return Ok(first(
            conn,
            "SELECT * FROM chain_index_state WHERE profile_id=?",
            [profile],
        )?
        .py());
    }
    let mut core = BTreeSet::new();
    if full {
        for table in [
            "chain_index_sources",
            "chain_index_source_nodes",
            "chain_index_nodes",
            "chain_index_edges",
            "chain_index_aliases",
            "chain_index_findings",
            "chain_index_finding_nodes",
        ] {
            write(
                conn,
                &format!("DELETE FROM {table} WHERE profile_id=?"),
                [profile],
            )
            .map_err(database_error)?;
        }
    }
    let mut changed = full || expired || OVERLAYS.iter().any(|table| dirty.contains_key(*table));
    for table in PHYSICAL {
        if !tables.contains(table) {
            continue;
        }
        let keys = dirty.get(table).cloned().unwrap_or_default();
        let keys = if full || keys.contains(&-1) {
            None
        } else {
            Some(&keys)
        };
        if keys.is_some_and(BTreeSet::is_empty) {
            continue;
        }
        let rows = source_rows(conn, profile, table, keys)?;
        let mut selected = keys
            .cloned()
            .unwrap_or_else(|| rows.keys().copied().collect());
        if keys.is_none() {
            for row in read(
                conn,
                "SELECT source_key FROM chain_index_sources WHERE profile_id=? AND source_table=?",
                params![profile, table],
            )? {
                selected.insert(integer(&row["source_key"])?);
            }
        }
        changed |= !selected.is_empty();
        for key in selected {
            core.extend(contribute(
                conn,
                profile,
                table,
                key,
                rows.get(&key).cloned(),
            )?);
        }
    }
    if !changed {
        write(
            conn,
            "UPDATE chain_index_state SET watermark=? WHERE profile_id=?",
            params![watermark, profile],
        )
        .map_err(database_error)?;
        state["watermark"] = json!(watermark);
        return Ok(state.py());
    }
    let (index, core) = assemble(conn, profile, core, &tables, full)?;
    save(conn, profile, &index, &core)?;
    let mut coverage = Value::from_py(field(index.snapshot(), "coverage"));
    coverage["source_rows"] = json!({});
    for table in ["wallets"].into_iter().chain(PHYSICAL).chain([
        "chain_analysis_labels",
        "journal_custody_decisions",
        "journal_custody_economic_relations",
    ]) {
        if tables.contains(table) {
            let scoped = table != "transaction_graph_cache";
            coverage["source_rows"][table] = first(
                conn,
                &format!(
                    "SELECT count(*) AS n FROM {table}{}",
                    if scoped { " WHERE profile_id=?" } else { "" }
                ),
                params_from_iter(scoped.then_some(profile)),
            )?["n"]
                .clone();
        }
    }
    coverage["missing_tables"] = json!([
        "wallets",
        "transactions",
        "transaction_graph_cache",
        "wallet_utxos",
        "journal_custody_decisions",
        "journal_custody_economic_relations"
    ]
    .into_iter()
    .filter(|table| !tables.contains(*table))
    .collect::<Vec<_>>());
    let totals=first(conn,"SELECT coalesce(sum(invalid_count),0) AS invalid,coalesce(sum(rejected_count),0) AS rejected FROM chain_index_sources WHERE profile_id=?",[profile])?;
    coverage["invalid_observations"] = totals["invalid"].clone();
    coverage["cache_rejected"] = totals["rejected"].clone();
    let counts=first(conn,"SELECT count(*) AS node_count,sum(status='missing') AS missing_node_count,sum(status='conflicting') AS conflicting_node_count,sum(status='reference') AS reference_node_count,sum(complete) AS complete_transaction_count FROM chain_index_nodes WHERE profile_id=?",[profile])?;
    for key in [
        "node_count",
        "missing_node_count",
        "conflicting_node_count",
        "reference_node_count",
        "complete_transaction_count",
    ] {
        coverage[key] = if counts[key].is_null() {
            json!(0)
        } else {
            counts[key].clone()
        };
    }
    coverage["edge_count"] = first(
        conn,
        "SELECT count(*) AS n FROM chain_index_edges WHERE profile_id=?",
        [profile],
    )?["n"]
        .clone();
    if tables.contains("chain_analysis_reference_assertions") {
        coverage["reference_reconciling_count"]=first(conn,"SELECT count(*) AS n FROM chain_analysis_reference_assertions WHERE profile_id=? AND active=2",[profile])?["n"].clone();
    }
    labels(conn, profile, &tables, &mut coverage, &instant)?;
    let revision = if state.is_null() {
        0
    } else {
        integer(&state["revision"])?
    } + i64::from(
        !rebuild || state.is_null() || state["watermark"] != json!(watermark) || expired,
    );
    let identity = if state.is_null() {
        sources.id().replace('-', "")
    } else {
        text(&state["instance_id"])
    };
    let snapshot = super::super::digest(&json!([identity, profile, VERSION, revision]).py())?;
    let expiry = next_expiry(conn, profile, &tables, &instant)?;
    write(
        conn,
        "INSERT OR REPLACE INTO chain_index_state VALUES(?,?,?,?,?,?,?,?)",
        params![
            profile,
            identity,
            VERSION,
            watermark,
            revision,
            snapshot,
            encoded(&coverage)?,
            expiry
        ],
    )
    .map_err(database_error)?;
    Ok(first(
        conn,
        "SELECT * FROM chain_index_state WHERE profile_id=?",
        [profile],
    )?
    .py())
}
