//! Revalidate the previewed provenance population under the same write lock
//! as its update and response projection. No custody or tax rows are written.
use super::*;
use kassiber_pyjson::BigInt;
use kassiber_setup::Sources;
use kassiber_store::rusqlite::params;
use source_funds_assembly::{asset, integer};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.links.bulk-review" | "ui.source_funds.links.bulk_review"
    )
}

fn active_rows(conn: &Connection, profile: &str) -> Result<Vec<Value>> {
    rows(conn,"SELECT t.*, w.label AS wallet_label, w.kind AS wallet_kind,w.config_json AS wallet_config_json, observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC",[profile])
}

fn scoped(conn: &Connection, profile: &str, target: &str) -> Result<Vec<Value>> {
    let mut queue = VecDeque::from([target.to_owned()]);
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        for row in rows(conn,"SELECT * FROM source_funds_links WHERE profile_id=? AND to_transaction_id=? AND state!='rejected' ORDER BY created_at ASC,id ASC",[profile,&id])? {
            if truthy(value(&row,"from_transaction_id")) {queue.push_back(text(&row,"from_transaction_id"));}
            if text(&row,"state")=="suggested" {result.push(row)}
        }
    }
    Ok(result)
}

type Allocations = HashMap<(String, String, String), (BigInt, BigInt)>;
fn stored_type(row: &Value) -> String {
    let reviewed =
        kassiber_pytext::lower(kassiber_pytext::strip(&text(row, "kind"))).replace('-', "_");
    if [
        "self_transfer",
        "exchange_transfer",
        "trade",
        "swap",
        "peg_in",
        "peg_out",
        "lightning_funding",
        "lightning_close",
        "lightning_routed",
        "lightning_swap",
        "coinjoin",
        "payjoin",
        "manual_source",
        "missing_history",
    ]
    .contains(&reviewed.as_str())
    {
        return reviewed;
    }
    let source = kassiber_pytext::lower(&text(row, "source_rail"));
    let target = kassiber_pytext::lower(&text(row, "target_rail"));
    match (source.as_str(), target.as_str()) {
        ("bitcoin", "liquid") => "peg_in",
        ("liquid", "bitcoin") => "peg_out",
        (s, "lightning") if s != "lightning" => "lightning_funding",
        ("lightning", t) if t != "lightning" => "lightning_close",
        _ if text(row, "relation_kind") == "conversion"
            || asset(value(row, "out_asset")) != asset(value(row, "in_asset")) =>
        {
            "swap"
        }
        _ => "self_transfer",
    }
    .into()
}
fn amount(row: &Value, key: &str) -> Result<BigInt> {
    if truthy(value(row, key)) {
        integer(value(row, key))
    } else {
        Ok(0.into())
    }
}
fn custody(
    conn: &Connection,
    profile: &str,
    active: &HashMap<String, &Value>,
) -> Result<Allocations> {
    let profile_row=rows(conn,"SELECT p.*,(SELECT COUNT(*) FROM transactions t WHERE t.profile_id=p.id AND t.excluded=0) AS active_count FROM profiles p WHERE p.id=?",[profile])?.remove(0);
    let mut result = Allocations::new();
    if !truthy(value(&profile_row, "last_processed_at"))
        || amount(&profile_row, "last_processed_tx_count")? != amount(&profile_row, "active_count")?
        || amount(&profile_row, "journal_input_version")?
            != amount(&profile_row, "last_processed_input_version")?
        || !crate::semantics_current(&profile_row)
    {
        return Ok(result);
    }
    for row in rows(conn,"SELECT * FROM journal_custody_projection_relations WHERE profile_id=? AND relation_kind IN ('move','conversion') AND in_transaction_id IS NOT NULL",[profile])? {
        let from=text(&row,"out_transaction_id");let to=text(&row,"in_transaction_id");
        if from==to || !active.contains_key(&from) || !active.contains_key(&to) {continue}
        let gross=amount(&row,"out_amount")?;let allocation=amount(&row,"in_amount")?;
        if gross<=0.into() || allocation<=0.into() {continue}
        let entry=result.entry((from,to,stored_type(&row))).or_default();
        entry.0+=allocation;entry.1+=gross;
    }
    // Previously reviewed compressed disclosure remains provenance evidence
    // only when the current derived native reconciliation proves it unchanged.
    for row in rows(conn,"SELECT proof.proof_json FROM journal_custody_reconciliations proof JOIN profiles profile ON profile.id=proof.profile_id JOIN custody_components component ON component.id=proof.component_id AND component.profile_id=proof.profile_id WHERE proof.profile_id=? AND component.state='active' AND component.revision=proof.component_revision AND profile.journal_input_version=proof.input_version",[profile])? {
        let proof=kassiber_pytext::loads(&text(&row,"proof_json")).map_err(|e|AppError::new("internal_error",e.to_string()))?;
        let Value::List(cells)=value(&proof,"allocations") else {continue};
        for cell in cells {
            if text(cell,"destination_kind")!="wallet" {continue}
            let from=text(cell,"source_transaction_id");let to=text(cell,"target_transaction_id");
            if !active.contains_key(&from) || !active.contains_key(&to) {continue}
            let n=amount(cell,"amount_msat")?;
            for link in rows(conn,"SELECT * FROM source_funds_links WHERE profile_id=? AND state='reviewed' AND method='custody_component' AND from_transaction_id=? AND to_transaction_id=?",[profile,&from,&to])? {
                if amount(&link,"allocation_amount")?==n && amount(&link,"from_allocation_amount")?==n
                    && asset(value(&link,"asset"))==text(cell,"asset") && asset(value(&link,"from_asset"))==text(cell,"asset") {
                    result.entry((from.clone(),to.clone(),text(&link,"link_type"))).or_insert((n.clone(),n.clone()));
                }
            }
        }
    }
    Ok(result)
}

fn eligible(
    conn: &Connection,
    profile: &str,
    considered: &[&Value],
    frozen: Option<&source_funds_suggest::Inputs<'_>>,
) -> Result<Vec<Value>> {
    if considered.is_empty() {
        return Ok(vec![]);
    }
    let loaded_active;
    let active = if let Some(inputs) = frozen {
        &inputs.report.active
    } else {
        loaded_active = active_rows(conn, profile)?;
        &loaded_active
    };
    let by_id: HashMap<_, _> = active.iter().map(|row| (text(row, "id"), row)).collect();
    let custody = if let Some(inputs) = frozen {
        inputs
            .report
            .allocations
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    } else {
        custody(conn, profile, &by_id)?
    };
    let structural = if considered
        .iter()
        .any(|row| text(row, "method") == "utxo_spend")
    {
        if let Some(inputs) = frozen {
            inputs
                .structural
                .iter()
                .map(|p| source_funds_assembly::Pair {
                    from: p.from.clone(),
                    to: p.to.clone(),
                    allocation: p.allocation.into(),
                    gross: p.from_allocation.into(),
                })
                .collect()
        } else {
            source_funds_assembly::derive(conn, profile, active)?
        }
    } else {
        vec![]
    };
    let mut result = vec![];
    for row in considered {
        let method = text(row, "method");
        if !["custody_component", "utxo_spend"].contains(&method.as_str())
            || !["exact", "strong"].contains(&text(row, "confidence").as_str())
            || matches!(value(row, "allocation_amount"), Value::Null)
            || truthy(value(row, "uses_chain_observation"))
        {
            continue;
        }
        let from = text(row, "from_transaction_id");
        let to = text(row, "to_transaction_id");
        if !from.is_empty() && !rows(conn,"SELECT 1 FROM source_funds_links WHERE profile_id=? AND from_transaction_id=? AND to_transaction_id=? AND state='rejected' LIMIT 1",[profile,&from,&to])?.is_empty() {continue}
        let (Some(from_tx), Some(to_tx)) = (by_id.get(&from), by_id.get(&to)) else {
            continue;
        };
        let deterministic = if method == "custody_component" {
            if let Some((allocation, gross)) = custody.get(&(from, to, text(row, "link_type"))) {
                *allocation == amount(row, "allocation_amount")?
                    && *gross == amount(row, "from_allocation_amount")?
                    && asset(value(row, "asset")) == asset(value(to_tx, "asset"))
                    && asset(value(row, "from_asset")) == asset(value(from_tx, "asset"))
            } else {
                false
            }
        } else if let Some(pair) = structural.iter().find(|p| p.from == from && p.to == to) {
            pair.allocation == amount(row, "allocation_amount")?
                && pair.gross == amount(row, "from_allocation_amount")?
                && text(row, "confidence") == "exact"
        } else {
            false
        };
        if deterministic && source_funds_links::validate(row, from_tx, &Value::Null, to_tx).is_ok()
        {
            result.push((*row).clone())
        }
    }
    Ok(result)
}

pub(super) fn eligible_ids(conn: &Connection, profile: &str, target: &str) -> Result<Vec<String>> {
    let population = scoped(conn, profile, target)?;
    let considered: Vec<_> = population.iter().collect();
    Ok(eligible(conn, profile, &considered, None)?
        .iter()
        .map(|row| text(row, "id"))
        .collect())
}

pub fn execute(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    execute_with_inputs(conn, command, raw, sources, None)
}

pub(super) fn execute_with_inputs(
    conn: &Connection,
    command: &str,
    raw: &Value,
    sources: &dyn Sources,
    frozen: Option<&source_funds_suggest::Inputs<'_>>,
) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let args = match raw {
        Value::Null if desktop => &empty,
        Value::Dict(_) => raw,
        _ => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(kassiber_envelope::type_name(raw)))])))
        }
    };
    let mut target = value(args, "target_transaction");
    if desktop && !truthy(target) {
        target = value(args, "target_transaction_ref")
    }
    let target = if desktop {
        let Value::Str(raw) = target else {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.links.bulk_review requires args.target_transaction",
            ));
        };
        let raw = raw.as_str().unwrap_or("");
        let raw = kassiber_pytext::strip(raw);
        if raw.is_empty() {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.links.bulk_review requires args.target_transaction",
            ));
        }
        raw.to_owned()
    } else {
        py_str(target)
    };
    let selected = if desktop {
        match value(args,"link_ids") {
            Value::Null=>None,
            Value::List(ids) if ids.iter().all(|id| matches!(id,Value::Str(v) if v.as_str().is_some_and(|s|!kassiber_pytext::strip(s).is_empty())))=>Some(ids.iter().map(|id|kassiber_pytext::strip(&py_str(id)).to_owned()).collect::<Vec<_>>()),
            _=>return Err(AppError::new("validation","ui.source_funds.links.bulk_review args.link_ids must be a list of link ids")),
        }
    } else {
        None
    };
    crate::export::admitted(conn, || {
        let (_, profile) =
            kassiber_setup::references::scope(conn, if desktop { &empty } else { args })?;
        let profile = text(&profile, "id");
        let target = source_funds_reads::resolve_transaction(conn, &profile, &target)?;
        let population = scoped(conn, &profile, &target)?;
        if selected.as_ref().is_some_and(|ids| ids.len() > 500) {
            return Err(AppError::new(
                "validation",
                "source-funds bulk review accepts a bounded list of link ids",
            ));
        }
        let considered: Vec<_> = population
            .iter()
            .filter(|row| {
                selected
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&text(row, "id")))
            })
            .collect();
        let reviewable = eligible(conn, &profile, &considered, frozen)?;
        let now = sources.now(conn)?;
        let dry_run = !desktop && truthy(value(args, "dry_run"));
        if dry_run {
            kassiber_store::begin_write_savepoint(conn, "source_funds_dry_run")
                .map_err(sql_error)?
        }
        let operation = (|| {
            for row in &reviewable {
                conn.execute("UPDATE source_funds_links SET state='reviewed',allocation_policy='explicit',updated_at=? WHERE id=?",params![now,text(row,"id")]).map_err(sql_error)?;
            }
            let mut links = Vec::new();
            for row in &reviewable {
                for row in rows(
                    conn,
                    "SELECT * FROM source_funds_links WHERE id=?",
                    [text(row, "id")],
                )? {
                    links.push(source_funds_reads::link(conn, &row)?)
                }
            }
            Ok(dict([("reviewed",Value::Int(reviewable.len().into())),("skipped",Value::Int((population.len()-reviewable.len()).into())),("target_transaction_id",s(target)),("links",Value::List(links)),("policy",s("Bulk review only accepts exact links re-verified against the current stored custody projection. Provider ids, privacy hints, time/amount matches, chain observations, and rejected pairs remain manual."))]))
        })();
        if dry_run {
            conn.execute_batch("ROLLBACK TO source_funds_dry_run; RELEASE source_funds_dry_run")
                .map_err(sql_error)?
        }
        operation.map(|mut result| {
            if dry_run {
                kassiber_envelope::set(&mut result, "dry_run", Value::Bool(true));
            }
            result
        })
    })
}
