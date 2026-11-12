use super::source_funds_disclosure as disclosure;
use super::source_funds_projection::{amount, integer, normalized_asset as asset, ratio};
use super::*;
use kassiber_pyjson::BigInt;
use kassiber_pytext::strip;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

fn n(value: &Value) -> Result<BigInt> {
    if truthy(value) {
        integer(value)
    } else {
        Ok(0.into())
    }
}
fn q(value: impl Into<BigInt>) -> Value {
    Value::Int(value.into())
}
fn fallback(row: &Value, key: &str) -> String {
    get(row, key)
        .filter(|v| truthy(v))
        .map(py_str)
        .unwrap_or_default()
}
fn planned_text(row: &Value, key: &str) -> Value {
    match value(row, key) {
        Value::Str(text) => Value::Str(kassiber_pytext::strip_py(text)),
        _ => s(strip(&fallback(row, key))),
    }
}
fn json(row: &Value) -> serde_json::Value {
    kassiber_custody::codec::to_json(row)
}
fn list(row: &Value, key: &str) -> Vec<Value> {
    match value(row, key) {
        Value::List(v) => v.clone(),
        _ => vec![],
    }
}

pub fn transaction(conn: &Connection, profile: &str, id: &str) -> Result<Option<Value>> {
    Ok(rows(conn, "SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.id=?", [profile, id])?.into_iter().next())
}

pub fn resolve_transaction(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    let id = super::source_funds_reads::resolve_transaction(conn, profile, reference)?;
    transaction(conn, profile, &id)?
        .ok_or_else(|| AppError::new("internal_error", "transaction wallet missing"))
}

pub struct Inputs<'a> {
    connection: &'a Connection,
    profile_id: String,
    pub(super) active: Vec<Value>,
    pub(super) allocations: BTreeMap<(String, String, String), (BigInt, BigInt)>,
    pub(super) allocation_details: BTreeMap<(String, String, String), (BTreeSet<String>, bool)>,
}

pub(super) fn active_rows(conn: &Connection, profile_id: &str) -> Result<Vec<Value>> {
    rows(conn, "SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.config_json AS wallet_config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC", [profile_id])
}

pub fn load_inputs<'a>(conn: &'a Connection, profile: &Value) -> Result<Inputs<'a>> {
    load_inputs_with_active(conn, profile, active_rows(conn, &text(profile, "id"))?)
}

pub(super) fn load_inputs_with_active<'a>(
    conn: &'a Connection,
    profile: &Value,
    active: Vec<Value>,
) -> Result<Inputs<'a>> {
    let profile_id = text(profile, "id");
    let current = truthy(value(profile, "last_processed_at"))
        && n(value(profile, "last_processed_tx_count"))? == BigInt::from(active.len())
        && n(value(profile, "journal_input_version"))?
            == n(value(profile, "last_processed_input_version"))?
        && crate::semantics_current(profile);
    let mut allocations: BTreeMap<_, (BigInt, BigInt)> = BTreeMap::new();
    let mut allocation_details: BTreeMap<_, (BTreeSet<String>, bool)> = BTreeMap::new();
    let ids: HashSet<_> = active.iter().map(|r| text(r, "id")).collect();
    if current {
        for row in rows(conn, "SELECT * FROM journal_custody_projection_relations WHERE profile_id=? AND relation_kind IN ('move','conversion') AND in_transaction_id IS NOT NULL", [&profile_id])? {
            let from = fallback(&row,"out_transaction_id");
            let to = fallback(&row,"in_transaction_id");
            let source = n(value(&row,"out_amount"))?;
            let target = n(value(&row,"in_amount"))?;
            if from.is_empty() || to.is_empty() || from==to || !ids.contains(&from) || !ids.contains(&to) || source<=0.into() || target<=0.into() { continue }
            let reviewed = kassiber_pytext::lower(strip(&fallback(&row,"kind"))).replace('-',"_");
            let source_rail = kassiber_pytext::lower(&fallback(&row,"source_rail"));
            let target_rail = kassiber_pytext::lower(&fallback(&row,"target_rail"));
            let kind = if LINK_TYPES.contains(&reviewed.as_str()) { reviewed.as_str() }
            else if source_rail=="bitcoin" && target_rail=="liquid" { "peg_in" }
            else if source_rail=="liquid" && target_rail=="bitcoin" { "peg_out" }
            else if target_rail=="lightning" && source_rail!="lightning" { "lightning_funding" }
            else if source_rail=="lightning" && target_rail!="lightning" { "lightning_close" }
            else if text(&row,"relation_kind")=="conversion" || asset(value(&row,"out_asset"))!=asset(value(&row,"in_asset")) { "swap" } else { "self_transfer" };
            allocation_details.entry((from.clone(),to.clone(),kind.into())).or_default().0.insert(text(&row,"id"));
            let entry = allocations.entry((from,to,kind.into())).or_insert((0.into(),0.into()));
            entry.0 += target;
            entry.1 += source;
        }
        let links = rows(conn,"SELECT * FROM source_funds_links WHERE profile_id=? AND state='reviewed' AND method='custody_component'",[&profile_id])?;
        for proof in rows(conn,"SELECT proof.proof_json FROM journal_custody_reconciliations proof JOIN profiles profile ON profile.id=proof.profile_id JOIN custody_components component ON component.id=proof.component_id AND component.profile_id=proof.profile_id WHERE proof.profile_id=? AND component.state='active' AND component.revision=proof.component_revision AND profile.journal_input_version=proof.input_version",[&profile_id])? {
            let proof = kassiber_pytext::loads(&text(&proof,"proof_json")).map_err(|e| AppError::new("internal_error",e.to_string()))?;
            for cell in list(&proof,"allocations") {
                let from=text(&cell,"source_transaction_id"); let to=text(&cell,"target_transaction_id");
                if text(&cell,"destination_kind")!="wallet" || !ids.contains(&from) || !ids.contains(&to) { continue }
                let amount=integer(value(&cell,"amount_msat"))?;
                for link in &links {
                    if text(link,"from_transaction_id")==from && text(link,"to_transaction_id")==to && n(value(link,"allocation_amount"))?==amount && n(value(link,"from_allocation_amount"))?==amount && asset(value(link,"asset"))==text(&cell,"asset") && asset(value(link,"from_asset"))==text(&cell,"asset") {
                        let key = (from.clone(),to.clone(),text(link,"link_type"));
                        if !allocations.contains_key(&key) {
                            allocation_details.insert(key.clone(), (BTreeSet::new(), true));
                        }
                        allocations.entry(key).or_insert((amount.clone(),amount.clone()));
                    }
                }
            }
        }
    }
    Ok(Inputs {
        connection: conn,
        profile_id,
        active,
        allocations,
        allocation_details,
    })
}

const LINK_TYPES: &[&str] = &[
    "manual_source",
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
    "missing_history",
];

fn finding(
    findings: &mut Vec<Value>,
    code: &str,
    severity: &str,
    message: &str,
    reference: &str,
    quantity: Option<(&BigInt, &str)>,
) -> Result<()> {
    let mut item = dict([
        ("code", s(code)),
        ("severity", s(severity)),
        ("message", s(message)),
        ("ref", s(reference)),
    ]);
    if let Some((quantity, asset)) = quantity {
        replace(&mut item, "amount", amount(quantity)?);
        replace(&mut item, "amount_msat", q(quantity.clone()));
        replace(&mut item, "asset", s(asset));
    }
    if !findings.contains(&item) {
        findings.push(item);
    }
    Ok(())
}

fn size_truncation(findings: &mut Vec<Value>, target_id: &str) -> Result<()> {
    finding(findings,"path_truncated","blocker","Source-funds graph exceeded the per-report node/edge cap; review the upstream suggestions or split the disclosure into a narrower target before retrying.",target_id,None)
}

fn attachments(conn: &Connection, table: &str, column: &str, id: &str) -> Result<Vec<Value>> {
    rows(conn,&format!("SELECT a.* FROM {table} j JOIN attachments a ON a.id=j.attachment_id WHERE j.{column}=?"),[id])
}

pub(super) fn timestamp_after(left: &Value, right: &Value) -> bool {
    fn timestamp(v: &Value) -> Option<String> {
        let raw = py_str(v);
        if raw == "0001-01-01T00:00:00Z" {
            return None;
        }
        kassiber_wallets::time::timestamp(&raw).ok()
    }
    match (timestamp(left), timestamp(right)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn recipient(conn: &Connection, profile: &str, reference: &Value) -> Result<Option<Value>> {
    if !truthy(reference) {
        return Ok(None);
    }
    let raw = py_str(reference);
    let reference = strip(&raw);
    if reference.is_empty() {
        return Err(AppError::new("validation", "Recipient ref is required"));
    }
    let by_id = rows(
        conn,
        "SELECT * FROM source_funds_recipients WHERE profile_id=? AND id=?",
        [profile, reference],
    )?;
    let by_label = if by_id.is_empty() {
        rows(
            conn,
            "SELECT * FROM source_funds_recipients WHERE profile_id=? AND label=? AND active=1",
            [profile, reference],
        )?
    } else {
        by_id
    };
    by_label.into_iter().next().map(Some).ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Recipient '{reference}' not found in this profile."),
        )
    })
}

pub fn build(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
    inputs: Option<&Inputs<'_>>,
) -> Result<Value> {
    let profile_id = text(profile, "id");
    let target = resolve_transaction(conn, &profile_id, &text(args, "target_transaction"))?;
    let loaded;
    let inputs = match inputs {
        Some(v) => v,
        None => {
            loaded = load_inputs(conn, profile)?;
            &loaded
        }
    };
    if !std::ptr::eq(inputs.connection, conn) || inputs.profile_id != profile_id {
        return Err(AppError::new(
            "validation",
            "Source-funds report inputs belong to a different book",
        ));
    }
    let active: HashMap<_, _> = inputs.active.iter().map(|r| (text(r, "id"), r)).collect();
    let mut structural = None;
    let recipient = recipient(conn, &profile_id, value(args, "recipient"))?;
    let explicit = value(args, "reveal_mode");
    let mode = disclosure::reveal_mode(if truthy(explicit) {
        explicit
    } else {
        recipient
            .as_ref()
            .map(|r| value(r, "default_reveal_mode"))
            .unwrap_or(&Value::Null)
    })?;
    let options = disclosure::report_options(value(args, "report_options"))?;
    let overrides = value(&options, "reveal_overrides");
    let purpose = disclosure::report_purpose(value(args, "report_purpose"))?;
    let target_amount = if matches!(value(args, "target_amount"), Value::Null)
        || value(args, "target_amount") == &s("")
    {
        integer(value(&target, "amount"))?
    } else {
        let raw = py_str(value(args, "target_amount"));
        if raw.parse::<Decimal>() == Err(kassiber_pydecimal::DecimalError::NonFinite) {
            let word = kassiber_pytext::lower(strip(&raw)).replace('_', "");
            let word = word.trim_start_matches(['+', '-']);
            return Err(AppError::new(
                "internal_error",
                if word.starts_with("snan") {
                    "[<class 'decimal.InvalidOperation'>]"
                } else if word.starts_with("nan") {
                    "cannot convert NaN to integer"
                } else {
                    "cannot convert Infinity to integer"
                },
            ));
        }
        let quantity = decimal(value(args, "target_amount"))?
            .mul(&Decimal::from(100_000_000_000i64), &DECIMAL)
            .map_err(arithmetic)?;
        let quantity = quantity
            .to_fixed_places(0, Rounding::HalfUp)
            .map_err(arithmetic)?;
        let quantity = kassiber_pytext::parse_int(&quantity)
            .map_err(|_| AppError::new("internal_error", "invalid target quantity"))?;
        if quantity < 0.into() {
            return Err(AppError::new(
                "validation",
                "--target-amount must not be negative",
            ));
        }
        quantity
    };
    if target_amount <= 0.into() {
        return Err(AppError::new(
            "validation",
            "target amount must be positive",
        ));
    }
    let max_depth = if matches!(value(args, "max_depth"), Value::Null) {
        8
    } else {
        n(value(args, "max_depth"))?
            .clamp(1.into(), 64.into())
            .try_into()
            .unwrap_or(64usize)
    };
    let target_id = text(&target, "id");
    let target_asset = asset(value(&target, "asset"));
    let traversal =
        super::source_funds_traversal::load(conn, &profile_id, &target_id, max_depth, 5000, 5000)?;
    let mut nodes = BTreeMap::<String, Value>::new();
    let mut edges = Vec::new();
    let mut findings = Vec::new();
    let mut source_mix = BTreeMap::<(String, String), (BigInt, usize)>::new();
    let mut unknown = BTreeMap::<String, (BigInt, String)>::new();
    let mut consumed = HashMap::<String, BigInt>::new();
    let mut txids = BTreeSet::new();
    let mut explorers = BTreeMap::new();
    let mut evidence = BTreeMap::new();
    let mut visited = HashSet::new();
    let mut queued = HashSet::from([target_id.clone()]);
    let mut required = HashMap::from([(target_id.clone(), target_amount.clone())]);
    let mut assets = HashMap::from([(target_id.clone(), target_asset.clone())]);
    let mut depths = HashMap::from([(target_id.clone(), 0usize)]);
    let mut paths = HashMap::from([(target_id.clone(), vec![target_id.clone()])]);
    let mut queue = BTreeSet::from([(traversal.rank[&target_id], target_id.clone())]);
    let mut truncated = false;
    if let Some(cycle) = &traversal.cycle_link_id {
        finding(
            &mut findings,
            "path_cycle",
            "blocker",
            "A reviewed source-funds path forms a cycle.",
            cycle,
            None,
        )?;
    }
    while let Some((_, id)) = queue.pop_first() {
        if truncated || nodes.len() >= 5000 || edges.len() >= 5000 {
            truncated = true;
            size_truncation(&mut findings, &target_id)?;
            break;
        }
        queued.remove(&id);
        let demand = required[&id].clone();
        let demand_asset = assets[&id].clone();
        let depth = depths[&id];
        let path = paths[&id].clone();
        let Some(tx) = transaction(conn, &profile_id, &id)? else {
            finding(
                &mut findings,
                "missing_history",
                "blocker",
                "Referenced transaction is no longer present.",
                &id,
                None,
            )?;
            continue;
        };
        let node_id = format!("tx:{id}");
        nodes.insert(
            node_id.clone(),
            disclosure::transaction_node(
                &tx,
                &mode,
                &q(demand.clone()),
                id == target_id,
                overrides,
            )?,
        );
        if let Some(hop) = kassiber_custody::source_funds::privacy_hop(&json(&tx)) {
            finding(&mut findings,"privacy_hop_unresolved","warning",&format!("{} evidence marks this transaction as an opaque privacy boundary; explicit supporting evidence is needed before treating upstream ownership as proven.",hop.split('_').map(|word| format!("{}{}",word[..1].to_ascii_uppercase(),&word[1..])).collect::<Vec<_>>().join(" ")),&id,None)?;
        }
        let disclosed = disclosure::public_transaction_id(&tx, &mode, id == target_id, overrides);
        if !disclosed.is_empty() {
            txids.insert(disclosed.clone());
            if let Some(explorer) = super::source_funds_explorers::link(conn, &disclosed, &tx)? {
                explorers.insert(disclosed, explorer);
            }
        }
        if value(&tx, "fiat_value") == &Value::Null && value(&tx, "fiat_rate") == &Value::Null {
            finding(
                &mut findings,
                "missing_pricing",
                "blocker",
                "A transaction on the disclosed path has no fiat pricing.",
                &id,
                None,
            )?;
        }
        if asset(&s(demand_asset.as_str())) != asset(value(&tx, "asset")) {
            finding(
                &mut findings,
                "asset_mismatch",
                "blocker",
                "A reviewed path declares a different asset than the transaction being consumed.",
                &id,
                None,
            )?;
        }
        if demand > integer(value(&tx, "amount"))? {
            finding(
                &mut findings,
                "transaction_overallocation",
                "blocker",
                "A reviewed path requires more value than this transaction holds.",
                &id,
                None,
            )?;
        }
        if depth >= max_depth {
            finding(
                &mut findings,
                "path_truncated",
                "blocker",
                "Maximum source-funds path depth reached.",
                &id,
                None,
            )?;
            continue;
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        let links = traversal
            .links
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for link in links.iter().filter(|r| text(r, "state") == "suggested") {
            finding(
                &mut findings,
                "unreviewed_link",
                "blocker",
                "A suggested source-funds link still needs review before export.",
                &text(link, "id"),
                None,
            )?;
        }
        let reviewed: Vec<_> = links
            .iter()
            .filter(|r| text(r, "state") == "reviewed")
            .collect();
        let mut reviewed_total = BigInt::from(0);
        if reviewed.is_empty() {
            finding(&mut findings,"missing_history","blocker","The path stops at a transaction without a reviewed root source or missing-history attestation.",&id,Some((&demand,&demand_asset)))?;
            continue;
        }
        for link in &reviewed {
            if nodes.len() >= 5000 || edges.len() >= 5000 {
                truncated = true;
                size_truncation(&mut findings, &target_id)?;
                break;
            }
            let link_id = text(link, "id");
            let parent_id = text(link, "from_transaction_id");
            if ["custody_component"].contains(&text(link, "method").as_str()) {
                let valid = if let (Some(parent), Some((a, b))) = (
                    active.get(&parent_id),
                    inputs.allocations.get(&(
                        parent_id.clone(),
                        id.clone(),
                        text(link, "link_type"),
                    )),
                ) {
                    *a == n(value(link, "allocation_amount"))?
                        && *b == n(value(link, "from_allocation_amount"))?
                        && asset(value(link, "asset")) == asset(value(&tx, "asset"))
                        && asset(value(link, "from_asset")) == asset(value(parent, "asset"))
                } else {
                    false
                };
                if !valid {
                    finding(&mut findings,"stale_custody_component_lineage","blocker","A reviewed custody link is absent or stale in the current stored projection.",&link_id,None)?;
                    continue;
                }
            }
            if text(link, "method") == "utxo_spend" {
                if structural.is_none() {
                    let owned = rows(
                        conn,
                        "SELECT * FROM wallet_utxos WHERE profile_id=?",
                        [&profile_id],
                    )?;
                    structural = Some(kassiber_custody::source_funds::structural_pairs(
                        &inputs.active.iter().map(json).collect::<Vec<_>>(),
                        &owned.iter().map(json).collect::<Vec<_>>(),
                    ));
                }
                let valid = if let Some((a, b)) = structural
                    .as_ref()
                    .unwrap()
                    .get(&(parent_id.clone(), id.clone()))
                {
                    active.contains_key(&parent_id)
                        && BigInt::from(*a) == n(value(link, "allocation_amount"))?
                        && BigInt::from(*b) == n(value(link, "from_allocation_amount"))?
                        && text(link, "confidence") == "exact"
                } else {
                    false
                };
                if !valid {
                    finding(&mut findings,"stale_structural_lineage","blocker","An earlier funding connection can no longer be verified against this wallet's transactions.",&link_id,None)?;
                    continue;
                }
            }
            if value(link, "allocation_amount") == &Value::Null {
                finding(
                    &mut findings,
                    "ambiguous_allocation",
                    "blocker",
                    "A reviewed link is missing an explicit allocation amount.",
                    &link_id,
                    None,
                )?;
                continue;
            }
            let from_source = truthy(value(link, "from_source_id"));
            let endpoint = if from_source {
                rows(
                    conn,
                    "SELECT * FROM source_funds_sources WHERE id=? AND profile_id=?",
                    [text(link, "from_source_id"), profile_id.clone()],
                )?
                .into_iter()
                .next()
            } else {
                transaction(conn, &profile_id, &parent_id)?
            };
            let mut allocation = value(link, "allocation_amount").clone();
            let mut from_allocation = value(link, "from_allocation_amount").clone();
            let from_or_allocation = if truthy(&from_allocation) {
                &from_allocation
            } else {
                &allocation
            };
            if reviewed.len() == 1
                && text(link, "allocation_policy") == "explicit"
                && demand > 0.into()
                && demand < integer(&allocation)?
                && integer(&allocation)? == integer(value(&tx, "amount"))?
                && endpoint
                    .as_ref()
                    .is_some_and(|e| value(e, "amount") != &Value::Null)
            {
                let endpoint = endpoint.as_ref().unwrap();
                let from = n(from_or_allocation)?;
                if from > 0.into()
                    && from <= integer(value(endpoint, "amount"))?
                    && asset(value(link, "asset")) == demand_asset
                    && asset(if truthy(value(link, "from_asset")) {
                        value(link, "from_asset")
                    } else {
                        value(link, "asset")
                    }) == demand_asset
                    && asset(value(endpoint, "asset")) == demand_asset
                {
                    let numerator = &demand * from;
                    let denominator = integer(&allocation)?;
                    if &numerator % &denominator != 0.into() {
                        finding(&mut findings,"ambiguous_allocation","blocker","The selected amount cannot follow the reviewed allocation ratio in exact millisatoshis.",&link_id,None)?;
                        continue;
                    }
                    from_allocation = q(numerator / denominator);
                    allocation = q(demand.clone());
                }
            }
            reviewed_total += integer(&allocation)?;
            if text(link, "allocation_policy") != "explicit" {
                finding(
                    &mut findings,
                    "ambiguous_allocation",
                    "blocker",
                    "A reviewed link still uses a heuristic or unknown allocation policy.",
                    &link_id,
                    None,
                )?;
            }
            if truthy(value(link, "uses_chain_observation"))
                && !truthy(value(link, "chain_data_confirmed"))
            {
                finding(
                    &mut findings,
                    "unconfirmed_chain_data",
                    "blocker",
                    "Unconfirmed chain observations cannot be used as proof in a PDF export.",
                    &link_id,
                    None,
                )?;
            }
            if text(link, "method") == "chain_observation"
                || truthy(value(link, "uses_chain_observation"))
            {
                finding(
                    &mut findings,
                    "chain_observation_privacy",
                    "warning",
                    "Chain-backend observations are context only and do not prove ownership.",
                    &link_id,
                    None,
                )?;
            }
            if ["coinjoin", "payjoin"].contains(&text(link, "link_type").as_str()) {
                finding(&mut findings,"privacy_hop_unresolved","warning","Privacy-hop links require explicit supporting evidence; unrelated participant inputs are not shown.",&link_id,None)?;
            }
            let attached = attachments(conn, "source_funds_link_attachments", "link_id", &link_id)?;
            for attachment in &attached {
                evidence.insert(
                    text(attachment, "id"),
                    disclosure::attachment(attachment, &mode)?,
                );
            }
            let Some(endpoint) = endpoint else {
                finding(
                    &mut findings,
                    "missing_history",
                    "blocker",
                    if from_source {
                        "Reviewed source record is missing."
                    } else {
                        "Reviewed parent transaction is missing."
                    },
                    &link_id,
                    None,
                )?;
                continue;
            };
            let upstream = integer(if truthy(&from_allocation) {
                &from_allocation
            } else {
                &allocation
            })?;
            let from_id;
            if from_source {
                let source_id = text(&endpoint, "id");
                let source_type = text(&endpoint, "source_type");
                let source_asset = text(&endpoint, "asset");
                if asset(value(&endpoint, "asset"))
                    != asset(if truthy(value(link, "from_asset")) {
                        value(link, "from_asset")
                    } else {
                        value(link, "asset")
                    })
                {
                    finding(
                        &mut findings,
                        "source_asset_mismatch",
                        "blocker",
                        "A reviewed link allocates a different asset than its source record.",
                        &link_id,
                        None,
                    )?;
                }
                if timestamp_after(value(&endpoint, "acquired_at"), value(&tx, "occurred_at")) {
                    finding(
                        &mut findings,
                        "chronology_violation",
                        "blocker",
                        "A reviewed source is dated after the transaction it funds.",
                        &link_id,
                        None,
                    )?;
                }
                let consumption = consumed.entry(source_id.clone()).or_insert(0.into());
                *consumption += &upstream;
                if value(&endpoint, "amount") == &Value::Null {
                    if !["missing_history", "opening_balance_attestation"]
                        .contains(&source_type.as_str())
                    {
                        finding(&mut findings,"source_amount_missing","blocker","A concrete source record needs an amount before it can support export.",&source_id,None)?;
                    }
                } else if *consumption > integer(value(&endpoint, "amount"))? {
                    finding(
                        &mut findings,
                        "source_overallocation",
                        "blocker",
                        "Reviewed links allocate more funds than the source record contains.",
                        &source_id,
                        None,
                    )?;
                }
                from_id = format!("source:{source_id}");
                nodes.insert(
                    from_id.clone(),
                    disclosure::source_node(&endpoint, &mode, &q(upstream.clone()))?,
                );
                match source_type.as_str() {
                    "missing_history"=>finding(&mut findings,"missing_history","warning","Reviewed missing-history gap included; it is not a real root source.",&source_id,Some((&upstream,&source_asset)))?,
                    "opening_balance_attestation"=>finding(&mut findings,"opening_balance_attestation","warning","Opening balance is an attested prior-history stop, not a fully traced root source.",&source_id,None)?,
                    "unknown"=>{unknown.entry(source_id.clone()).or_insert((0.into(),source_asset.clone())).0+=&upstream;},_=>{}
                }
                let mix = source_mix
                    .entry((asset(value(&endpoint, "asset")), source_type))
                    .or_insert((0.into(), 0));
                mix.0 += upstream;
                mix.1 += 1;
                for attachment in attachments(
                    conn,
                    "source_funds_source_attachments",
                    "source_id",
                    &source_id,
                )? {
                    evidence.insert(
                        text(&attachment, "id"),
                        disclosure::attachment(&attachment, &mode)?,
                    );
                }
            } else {
                let parent_id = text(&endpoint, "id");
                let from_asset = asset(if truthy(value(link, "from_asset")) {
                    value(link, "from_asset")
                } else {
                    value(&endpoint, "asset")
                });
                if timestamp_after(value(&endpoint, "occurred_at"), value(&tx, "occurred_at")) {
                    finding(&mut findings,"chronology_violation","blocker","A reviewed parent transaction occurs after the child transaction it funds.",&link_id,None)?;
                }
                if text(link, "link_type") == "self_transfer"
                    && (asset(value(&endpoint, "asset")) != asset(value(&tx, "asset"))
                        || asset(value(link, "asset")) != asset(value(&tx, "asset"))
                        || from_asset != asset(value(&endpoint, "asset")))
                {
                    finding(&mut findings,"asset_mismatch","blocker","A self-transfer link declares a different asset than its parent or target transaction.",&link_id,None)?;
                }
                from_id = format!("tx:{parent_id}");
                nodes.insert(
                    from_id.clone(),
                    disclosure::transaction_node(
                        &endpoint,
                        &mode,
                        &q(upstream.clone()),
                        false,
                        overrides,
                    )?,
                );
                if path.contains(&parent_id) {
                    finding(
                        &mut findings,
                        "path_cycle",
                        "blocker",
                        "A reviewed source-funds path forms a cycle.",
                        &link_id,
                        None,
                    )?;
                } else {
                    if assets
                        .get(&parent_id)
                        .is_some_and(|a| *a != from_asset && !a.is_empty())
                    {
                        finding(
                            &mut findings,
                            "asset_mismatch",
                            "blocker",
                            "A repeated upstream transaction is required with conflicting assets.",
                            &parent_id,
                            None,
                        )?;
                    }
                    assets.entry(parent_id.clone()).or_insert(from_asset);
                    let total = required.entry(parent_id.clone()).or_insert(0.into());
                    *total += upstream;
                    nodes.insert(
                        from_id.clone(),
                        disclosure::transaction_node(
                            &endpoint,
                            &mode,
                            &q(total.clone()),
                            false,
                            overrides,
                        )?,
                    );
                    if *total > integer(value(&endpoint, "amount"))? {
                        finding(
                            &mut findings,
                            "transaction_overallocation",
                            "blocker",
                            "A reviewed path requires more value than this transaction holds.",
                            &parent_id,
                            None,
                        )?;
                    }
                    if visited.contains(&parent_id) {
                        finding(&mut findings,"ambiguous_allocation","blocker","A repeated upstream transaction received additional required amount after it was reviewed.",&parent_id,None)?;
                    } else if queued.insert(parent_id.clone()) {
                        depths.insert(parent_id.clone(), depth + 1);
                        let mut next = path.clone();
                        next.push(parent_id.clone());
                        paths.insert(parent_id.clone(), next);
                        queue.insert((traversal.rank[&parent_id], parent_id));
                    } else {
                        let d = depths.get_mut(&parent_id).unwrap();
                        *d = (*d).max(depth + 1);
                    }
                }
            }
            let mut endpoints = vec![id.clone()];
            if let Some(parent) = from_id.strip_prefix("tx:") {
                endpoints.push(parent.into());
            }
            edges.push(dict([
                ("id", value(link, "id").clone()),
                ("from", s(from_id)),
                ("to", s(node_id.as_str())),
                ("link_type", value(link, "link_type").clone()),
                ("state", value(link, "state").clone()),
                ("confidence", value(link, "confidence").clone()),
                ("method", value(link, "method").clone()),
                ("asset", value(link, "asset").clone()),
                (
                    "allocation_amount",
                    super::source_funds_reads::amount(&allocation)?,
                ),
                ("allocation_amount_msat", q(integer(&allocation)?)),
                (
                    "from_asset",
                    if truthy(value(link, "from_asset")) {
                        value(link, "from_asset")
                    } else {
                        value(link, "asset")
                    }
                    .clone(),
                ),
                (
                    "from_allocation_amount",
                    super::source_funds_reads::amount(&from_allocation)?,
                ),
                ("from_allocation_amount_msat", from_allocation),
                (
                    "allocation_policy",
                    value(link, "allocation_policy").clone(),
                ),
                (
                    "explanation",
                    disclosure::edge_explanation(
                        value(link, "explanation"),
                        &mode,
                        &endpoints,
                        overrides,
                    ),
                ),
                (
                    "attachments",
                    Value::List(
                        attached
                            .iter()
                            .map(|a| disclosure::attachment(a, &mode))
                            .collect::<Result<_>>()?,
                    ),
                ),
            ]));
        }
        if reviewed_total != demand {
            finding(
                &mut findings,
                "ambiguous_allocation",
                "blocker",
                "Reviewed allocations do not exactly cover the required amount.",
                &id,
                None,
            )?;
        }
    }
    if truncated || traversal.truncated {
        size_truncation(&mut findings, &target_id)?;
    }
    for (id, (quantity, asset)) in unknown {
        finding(&mut findings,"unknown_origin","warning","A reviewed root source ends the trace without stating the economic origin of the funds.",&id,Some((&quantity,&asset)))?;
    }
    let hints =
        kassiber_pytext::loads(include_str!("source_funds_hints.json")).expect("finding hints");
    for item in &mut findings {
        let hint = value(&hints, &text(item, "code")).clone();
        replace(item, "next_step", hint);
    }
    let blockers: Vec<_> = findings
        .iter()
        .filter(|r| text(r, "severity") == "blocker")
        .cloned()
        .collect();
    let warnings: Vec<_> = findings
        .iter()
        .filter(|r| text(r, "severity") == "warning")
        .cloned()
        .collect();
    let mut gross = BTreeMap::<String, BigInt>::new();
    for ((asset, _), (quantity, _)) in &source_mix {
        *gross.entry(asset.clone()).or_insert(0.into()) += quantity;
    }
    let matches = gross.len() == 1 && gross.get(&target_asset) == Some(&target_amount);
    let difference = if gross.len() == 1 {
        gross.get(&target_asset).map(|g| g - &target_amount)
    } else {
        None
    };
    let mix = source_mix
        .into_iter()
        .map(|((asset, kind), (quantity, count))| {
            Ok(dict([
                ("source_type", s(kind)),
                ("asset", s(asset)),
                ("amount", amount(&quantity)?),
                ("amount_msat", q(quantity.clone())),
                (
                    "percent_of_target",
                    if matches && quantity <= target_amount {
                        Value::Float(
                            format!("{:.4}", ratio(&quantity, &target_amount)? * 100.0)
                                .parse()
                                .unwrap(),
                        )
                    } else {
                        Value::Null
                    },
                ),
                ("count", q(count)),
            ]))
        })
        .collect::<Result<_>>()?;
    let wallets: BTreeSet<_> = nodes
        .values()
        .filter(|r| text(r, "node_type") == "transaction")
        .map(|r| fallback(r, "wallet"))
        .filter(|s| !s.is_empty())
        .collect();
    let mut nodes: Vec<_> = nodes.into_values().collect();
    nodes.sort_by_key(|r| (text(r, "node_type"), text(r, "label"), text(r, "id")));
    let planned = purpose == "planned_exchange_sale";
    let mut report=dict([
        ("workspace",value(workspace,"label").clone()),("profile",value(profile,"label").clone()),("report_context",disclosure::report_context(profile)),("report_options",options.clone()),
        ("purpose",dict([("type",s(purpose)),("label",s(if planned{"Planned exchange sale"}else{"Already completed transaction"})),("anchor_role",s(if planned{"funds_history_anchor"}else{"completed_transaction"})),("planned_destination",planned_text(args,"planned_destination")),("planned_note",planned_text(args,"planned_note")),("fiat_purchase_note",s(if planned{"If the bitcoin was bought on an exchange, fiat-source evidence for that original purchase is a separate supporting document and should be attached to the root source."}else{""}))])),
        ("target",disclosure::transaction_node(&target,&mode,&q(target_amount.clone()),true,overrides)?),("reveal_mode",s(mode)),("graph",dict([("nodes",Value::List(nodes)),("edges",Value::List(edges.clone()))])),
        ("allocations",dict([("target_amount",amount(&target_amount)?),("target_amount_msat",q(target_amount)),("asset",value(&target,"asset").clone()),("reviewed_edge_count",q(edges.len())),("gross_source_requirement",Value::List(gross.into_iter().map(|(asset,quantity)|Ok(dict([("asset",s(asset)),("amount",amount(&quantity)?),("amount_msat",q(quantity))]))).collect::<Result<_>>()?)),("gross_matches_target",Value::Bool(matches)),("route_difference_msat",difference.as_ref().map(|n|q(n.clone())).unwrap_or(Value::Null)),("route_difference",difference.as_ref().map(amount).transpose()?.unwrap_or(Value::Null))])),
        ("source_mix",Value::List(mix)),("gaps",Value::List(findings.iter().filter(|r|["missing_history","unknown_origin","ambiguous_allocation","privacy_hop_unresolved","path_truncated"].contains(&text(r,"code").as_str())).cloned().collect())),("findings",Value::List(findings)),("explain_gates",dict([("exportable",Value::Bool(blockers.is_empty())),("blockers",Value::List(blockers)),("warnings",Value::List(warnings))])),
        ("disclosure_preview",dict([("txids",Value::List(txids.into_iter().map(s).collect())),("explorer_links",Value::List(explorers.into_values().collect())),("attachments",Value::List(evidence.into_values().collect())),("wallets_named",Value::List(wallets.iter().cloned().map(s).collect())),("privacy_note",s("Txids disclose on-chain neighbors to the recipient. Chain observations are context, not proof of ownership.")),("ownership_note",s(if wallets.len()>1{"Sharing this report demonstrates to the recipient that the wallets named above are under common ownership or control, and links their disclosed transactions to your identity."}else{""})),("excluded",Value::List(["descriptors","xpubs","wallet files","seeds","backend tokens","unrelated wallet history"].into_iter().map(s).collect()))]))
    ]);
    if let Some(recipient) = recipient {
        replace(
            &mut report,
            "recipient",
            dict(
                ["id", "label", "kind", "default_reveal_mode"]
                    .map(|k| (k, value(&recipient, k).clone())),
            ),
        );
    }
    super::source_funds_projection::add_shape(&mut report)?;
    if truthy(value(args, "include_diagrams")) {
        super::source_funds_svg::attach(&mut report)?;
    }
    Ok(report)
}

pub fn execute(conn: &Connection, command: &str, args: &Value) -> Result<Value> {
    if command == "reports.source-funds" && truthy(value(args, "save_case")) {
        return Err(AppError::new(
            "not_implemented",
            "Source-funds case saving requires writer admission",
        ));
    }
    let (workspace, profile, recipe) = prepare(conn, command, args)?;
    let report = build(conn, &workspace, &profile, &recipe, None)?;
    render(command, args, report)
}

pub(super) fn prepare(
    conn: &Connection,
    command: &str,
    args: &Value,
) -> Result<(Value, Value, Value)> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let scope = if desktop { &empty } else { args };
    if desktop && !matches!(args, Value::Null | Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])));
    }
    let mut recipe = if desktop {
        args.clone()
    } else {
        dict(
            [
                "target_transaction",
                "target_amount",
                "planned_destination",
                "planned_note",
                "reveal_mode",
                "max_depth",
                "recipient",
            ]
            .map(|k| (k, value(args, k).clone())),
        )
    };
    if desktop {
        if !matches!(value(args,"target_transaction"),Value::Str(v) if v.as_str().is_some_and(|v|!strip(v).is_empty()))
        {
            return Err(AppError::new(
                "validation",
                format!("{command} requires args.target_transaction"),
            ));
        }
        replace(
            &mut recipe,
            "target_transaction",
            s(strip(&text(args, "target_transaction"))),
        );
        for key in [
            "recipient",
            "planned_destination",
            "planned_note",
            "reveal_mode",
        ] {
            if let Value::Str(v) = value(args, key) {
                replace(
                    &mut recipe,
                    key,
                    s(if key == "recipient" {
                        strip(v.as_str().unwrap_or_default())
                    } else {
                        v.as_str().unwrap_or_default()
                    }),
                );
            } else {
                replace(&mut recipe, key, Value::Null);
            }
        }
        let depth = match value(args, "max_depth") {
            Value::Int(v) if *v > 0.into() => v.clone().min(32.into()),
            Value::Bool(true) => 1.into(),
            _ => 8.into(),
        };
        replace(&mut recipe, "max_depth", q(depth));
        if !matches!(value(args, "report_options"), Value::Dict(_)) {
            replace(&mut recipe, "report_options", Value::Null);
        }
    } else {
        replace(
            &mut recipe,
            "report_purpose",
            value(args, "purpose").clone(),
        );
        let mut overrides = dict::<String>([]);
        for item in list(args, "reveal_override") {
            if let Some((id, decision)) = py_str(&item).split_once('=') {
                replace(&mut overrides, id, s(decision));
            }
        }
        replace(
            &mut recipe,
            "report_options",
            dict([
                ("diagram_detail", value(args, "diagram_detail").clone()),
                ("amount_precision", value(args, "amount_precision").clone()),
                ("mask_recipient", value(args, "mask_recipient").clone()),
                ("omit_sections", value(args, "omit_section").clone()),
                ("reveal_overrides", overrides),
            ]),
        );
    }
    replace(&mut recipe, "include_diagrams", Value::Bool(true));
    let (workspace, profile) = kassiber_setup::references::scope(conn, scope)?;
    Ok((workspace, profile, recipe))
}

pub(super) fn render(command: &str, args: &Value, report: Value) -> Result<Value> {
    if !command.starts_with("ui.") && matches!(text(args, "format").as_str(), "table" | "plain") {
        return super::source_funds_text::render(&report).map(s);
    }
    Ok(report)
}
