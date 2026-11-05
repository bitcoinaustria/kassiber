//! `core/transaction_verify.py` (`transactions verify`): one imported
//! transaction against the raw graph already stored for it. Conservation
//! (inputs minus outputs against the recorded fee), each owning wallet's
//! imported leg, and rows of the txid whose wallet owns no leg.
//!
//! It never contacts the network, never writes, and never returns
//! addresses, scripts, descriptors or derivation paths. An output the local
//! ownership index does not match is `owned: false`, not external.
use super::*;

const SATS_TO_MSAT: i128 = 1000;
const MAX_NODES_PER_SIDE: usize = 100;
const MAX_ROWS: usize = 50;

const DEFINITIONS: [(&str, &str); 5] = [
    ("status", "ok: every check passed; mismatch: at least one check failed; partial: no failure but some check could not run; unverifiable: no stored graph to check against."),
    ("conservation", "Sum of input values minus sum of output values against the fee the importer recorded, in msat; a source that reports no fee had it derived from the same graph, which then only proves the graph is self-consistent."),
    ("legs", "Per wallet of this book owning a graph input or output: owned outputs minus owned inputs (expected_delta_msat) against the imported row's wallet delta (inbound amount, or minus amount and fee). missing_imported_leg means no row of that wallet carries this txid."),
    ("unexplained_rows", "Rows with this txid whose wallet owns no graph leg; expected for an exchange or custodial record of the same transfer, a mismatch for a wallet this book observes on chain."),
    ("keychain", "receive or change from the owning wallet's derivation branch; unknown when the ownership evidence names no branch (address lists, unlabeled inventory)."),
];

/// `_sats`: an `int` that is not a `bool` (sats or msat alike).
fn integer(value: &Value) -> Option<i128> {
    match value {
        Value::Int(n) => i128::try_from(n.clone()).ok(),
        _ => None,
    }
}

fn msat(value: Option<i128>) -> Value {
    value.map_or(Value::Null, |sats| int_value(sats * SATS_TO_MSAT))
}

/// `_graph_source`: where the compared graph came from.
fn graph_source(enriched: &Value) -> &'static str {
    match g(g(enriched, "_graphLookupWarning"), "code") {
        Value::Str(code) if code.as_str() == Some("local_transaction_reference") => {
            "local_reference"
        }
        Value::Str(code) if code.as_str() == Some("cached_reference_graph") => {
            "cached_public_reference"
        }
        _ => "stored_observation",
    }
}

/// The exact role `branch_evidence(label, None)` names, if any.
fn exact_role(label: &str) -> Option<&'static str> {
    let normalized: String = lower(&strip(label))
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let mut tokens: BTreeSet<String> = normalized.split_whitespace().map(str::to_owned).collect();
    tokens.insert(normalized.replace(' ', ""));
    if tokens.contains("change") || tokens.contains("internal") {
        Some("change")
    } else if ["receive", "external", "deposit"]
        .iter()
        .any(|role| tokens.contains(*role))
    {
        Some("receive")
    } else {
        None
    }
}

/// `_keychain`: the one exact branch role all matches agree on.
fn keychain(matches: &[Owned]) -> &'static str {
    let roles: BTreeSet<&str> = matches
        .iter()
        .filter_map(|m| exact_role(&m.branch_label))
        .collect();
    match roles.iter().next() {
        Some(role) if roles.len() == 1 => role,
        _ => "unknown",
    }
}

/// `_node_view`.
fn node_view(node: &Value, matches: &[Owned]) -> Value {
    let ids: BTreeSet<&str> = matches.iter().map(|m| m.wallet_id.as_str()).collect();
    let labels: BTreeSet<&str> = matches.iter().map(|m| m.wallet_label.as_str()).collect();
    // One label, or every label joined; `None` when nothing matched.
    let wallet = if labels.is_empty() {
        Value::Null
    } else {
        s(labels.iter().copied().collect::<Vec<_>>().join(", "))
    };
    dict([
        ("index", val(node, "index")),
        ("amount_msat", msat(integer(g(node, "valueSats")))),
        ("value_state", val(node, "valueState")),
        ("owned", Value::Bool(ids.len() == 1)),
        ("ambiguous", Value::Bool(ids.len() > 1)),
        (
            "wallet_id",
            match ids.iter().next() {
                Some(id) if ids.len() == 1 => s(*id),
                _ => Value::Null,
            },
        ),
        ("wallet", wallet),
        (
            "keychain",
            if ids.is_empty() {
                Value::Null
            } else {
                s(keychain(matches))
            },
        ),
    ])
}

/// `row_boundary_amounts(row).wallet_delta_msat`.
fn wallet_delta(row: &Value) -> i128 {
    let direction = lower(&strip(&text_or(g(row, "direction"))));
    let observed = int_or_none(g(row, "amount")).unwrap_or(0);
    let fee = int_or_none(g(row, "fee")).unwrap_or(0);
    let principal = if direction == "outbound" && truthy(g(row, "amount_includes_fee")) {
        observed - fee
    } else {
        observed
    };
    if direction == "outbound" {
        -(principal + fee)
    } else {
        observed
    }
}

fn row_view(row: &Value) -> Vec<(&'static str, Value)> {
    vec![
        ("transaction_id", s(py_str(g(row, "id")))),
        ("direction", val(row, "direction")),
        ("recorded_delta_msat", int_value(wallet_delta(row))),
        ("excluded", Value::Bool(truthy(g(row, "excluded")))),
    ]
}

/// A per-wallet leg while the graph is walked.
struct Leg {
    wallet_id: String,
    wallet: Value,
    inputs_msat: i128,
    outputs_msat: i128,
    inputs: i128,
    outputs: i128,
    complete: bool,
}

/// `verify_transaction(conn, profile_id, transaction_ref)`. Local only.
pub fn verify(conn: &Connection, profile_id: &str, reference: &str) -> Result<Value> {
    let reference = strip(reference);
    if reference.is_empty() {
        return Err(AppError::new(
            "validation",
            "Transaction verification requires a transaction id or txid",
        ));
    }
    let Some(row) = fetch(conn, profile_id, &reference)? else {
        return Err(
            AppError::new("not_found", "Transaction was not found in the active book")
                .with_details(dict([("transaction", s(reference.as_str()))])),
        );
    };
    let row_raw = json_obj(g(&row, "raw_json"));
    // No runtime config and no public lookup: local observation, stored
    // references and cached references only.
    let enriched = enrich(conn, profile_id, &row, row_raw.clone(), false)?;
    let local = local_outpoint_amounts(conn, profile_id, &row, &enriched)?;
    let graph = parse_graph(&row, &enriched, &local)?;
    let (chain, network) = default_row_chain_network(&row);
    let txid = txid_from_row(&row);
    let mut payload = vec![
        ("transaction_id", s(py_str(g(&row, "id")))),
        ("txid", txid.as_deref().map_or(Value::Null, s)),
        ("chain", s(chain.as_str())),
        ("network", s(network.as_str())),
        ("asset", val(&row, "asset")),
        ("network_contacted", Value::Bool(false)),
        (
            "definitions",
            dict(DEFINITIONS.map(|(key, text)| (key, s(text)))),
        ),
    ];
    let Some(txid) = txid.filter(|_| graph.support != "graphless") else {
        payload.extend([
            ("status", s("unverifiable")),
            (
                "reasons",
                Value::List(vec![s(graph.reason.unwrap_or("no_txid"))]),
            ),
            (
                "message",
                s("No raw transaction graph is stored for this row, so it can only be compared with other imported rows. Refreshing the wallet from its chain source or opening the transaction graph with a reference lookup stores one."),
            ),
            ("conservation", Value::Null),
            ("legs", Value::Null),
            ("unexplained_rows", Value::Null),
            ("inputs", Value::List(vec![])),
            ("outputs", Value::List(vec![])),
        ]);
        return Ok(dict(payload));
    };
    let mut reasons: BTreeSet<String> = BTreeSet::new();
    let mut statuses: Vec<&str> = Vec::new();
    payload.push(("graph_source", s(graph_source(&enriched))));

    // Conservation: inputs - outputs = fee.
    let input_values: Vec<_> = graph
        .inputs
        .iter()
        .map(|n| integer(g(n, "valueSats")))
        .collect();
    let output_values: Vec<_> = graph
        .outputs
        .iter()
        .map(|n| integer(g(n, "valueSats")))
        .collect();
    let mut recorded = None;
    let mut fee_source = None;
    if get(&row_raw, "vin").is_some() {
        recorded = integer(g(&row_raw, "fee"));
        fee_source = recorded.map(|_| "observer_record");
    }
    if recorded.is_none() {
        recorded = integer(g(&enriched, "fee"));
        fee_source = recorded.map(|_| "reference_graph");
    }
    if recorded.is_none() && is_dict(&graph.fee) && chain == "liquid" {
        recorded = integer(g(&graph.fee, "valueSats"));
        fee_source = recorded.map(|_| "fee_output");
    }
    let mut totals = (None, None, None);
    let (status, reason): (&str, Option<String>) = if graph
        .inputs
        .iter()
        .any(|n| truthy(g(n, "_coinbase")) || truthy(g(n, "_pegin")))
    {
        ("not_applicable", Some("newly_issued_or_pegged_in".into()))
    } else if graph.inputs.is_empty()
        || input_values
            .iter()
            .chain(&output_values)
            .any(Option::is_none)
    {
        (
            "unverifiable",
            Some(graph.reason.unwrap_or("graph_values_missing").into()),
        )
    } else {
        let total_in: i128 = input_values.iter().flatten().sum();
        let total_out: i128 = output_values.iter().flatten().sum();
        let expected = total_in - total_out;
        totals = (Some(total_in), Some(total_out), Some(expected));
        if expected < 0 {
            ("mismatch", Some("outputs_exceed_inputs".into()))
        } else if recorded.is_none() {
            ("unverifiable", Some("no_recorded_fee".into()))
        } else if Some(expected) != recorded {
            ("mismatch", Some("fee_differs".into()))
        } else {
            ("ok", None)
        }
    };
    statuses.push(status);
    if let Some(reason) = &reason {
        reasons.insert(format!("conservation:{reason}"));
    }
    payload.push((
        "conservation",
        dict([
            ("inputs_msat", msat(totals.0)),
            ("outputs_msat", msat(totals.1)),
            ("fee_expected_msat", msat(totals.2)),
            ("fee_recorded_msat", msat(recorded)),
            ("fee_source", fee_source.map_or(Value::Null, s)),
            ("status", s(status)),
            ("reason", reason.map_or(Value::Null, s)),
        ]),
    ));

    let mut references = BTreeSet::new();
    for node in graph.inputs.iter().chain(&graph.outputs) {
        if let Some(point) = string_or_none(g(node, "outpoint")) {
            references.insert(lower(&point));
        }
        if truthy(g(node, "_script")) {
            references.insert(lower(&py_str(g(node, "_script"))));
        }
    }
    let (index, _, observed_wallets) =
        crate::loader::owned_index_and_wallets(conn, profile_id, &references)
            .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let wanted = norm_chain_network(&s(chain.as_str()), &s(network.as_str()));
    let mut legs: Vec<Leg> = Vec::new();
    let mut ambiguous = 0;
    let mut account = |node: &Value, matches: &[Owned], side: &str| -> Value {
        let view = node_view(node, matches);
        if view_flag(&view, "ambiguous") {
            ambiguous += 1;
        } else if view_flag(&view, "owned") {
            let wallet_id = py_str(g(&view, "wallet_id"));
            let position = match legs.iter().position(|leg| leg.wallet_id == wallet_id) {
                Some(position) => position,
                None => {
                    legs.push(Leg {
                        wallet_id,
                        wallet: val(&view, "wallet"),
                        inputs_msat: 0,
                        outputs_msat: 0,
                        inputs: 0,
                        outputs: 0,
                        complete: true,
                    });
                    legs.len() - 1
                }
            };
            let leg = &mut legs[position];
            let amount = integer(g(&view, "amount_msat"));
            if side == "input" {
                leg.inputs += 1;
            } else {
                leg.outputs += 1;
            }
            match amount {
                None => leg.complete = false,
                Some(amount) if side == "input" => leg.inputs_msat += amount,
                Some(amount) => leg.outputs_msat += amount,
            }
        }
        view
    };
    let mut inputs = Vec::new();
    for node in &graph.inputs {
        inputs.push(account(
            node,
            &owned_matches(&index, node, &wanted),
            "input",
        ));
    }
    let mut outputs = Vec::new();
    for node in &graph.outputs {
        let script = string_or_none(g(node, "_script"));
        if is_unspendable(script.as_deref()) || truthy(g(node, "_pegoutAddress")) {
            let mut view = node_view(node, &[]);
            set(&mut view, "unspendable", Value::Bool(true));
            outputs.push(view);
            continue;
        }
        outputs.push(account(
            node,
            &owned_matches(&index, node, &wanted),
            "output",
        ));
    }

    let mut rows_by_wallet: Vec<(String, Vec<Value>)> = Vec::new();
    for candidate in rows(
        conn,
        "SELECT t.*, w.label AS wallet_label, w.kind AS wallet_kind, w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND lower(t.external_id) = ? ORDER BY t.occurred_at, t.created_at, t.id",
        [profile_id, lower(&txid).as_str()],
    )? {
        let (row_chain, row_network) = default_row_chain_network(&candidate);
        if norm_chain_network(&s(row_chain), &s(row_network)) != wanted {
            continue;
        }
        let wallet_id = py_str(g(&candidate, "wallet_id"));
        match rows_by_wallet.iter_mut().find(|(id, _)| *id == wallet_id) {
            Some((_, found)) => found.push(candidate),
            None => rows_by_wallet.push((wallet_id, vec![candidate])),
        }
    }
    let wallet_rows = |wallet_id: &str| -> &[Value] {
        rows_by_wallet
            .iter()
            .find(|(id, _)| id == wallet_id)
            .map_or(&[], |(_, found)| found.as_slice())
    };

    legs.sort_by(|a, b| {
        (text_or(&a.wallet), &a.wallet_id).cmp(&(text_or(&b.wallet), &b.wallet_id))
    });
    let mut leg_views = Vec::new();
    for leg in &legs {
        let expected = leg.complete.then(|| leg.outputs_msat - leg.inputs_msat);
        let found: Vec<Value> = wallet_rows(&leg.wallet_id)
            .iter()
            .map(|row| dict(row_view(row)))
            .collect();
        let active: Vec<&Value> = found
            .iter()
            .filter(|item| !view_flag(item, "excluded"))
            .collect();
        let (status, reason) = if found.is_empty() {
            ("mismatch", Some("missing_imported_leg"))
        } else if active.is_empty() {
            ("excluded", Some("imported_leg_excluded"))
        } else if active.len() > 1 {
            ("mismatch", Some("several_imported_legs"))
        } else if expected.is_none() {
            ("unverifiable", Some("owned_values_hidden"))
        } else if integer(g(active[0], "recorded_delta_msat")) != expected {
            ("mismatch", Some("amount_differs"))
        } else {
            ("ok", None)
        };
        statuses.push(if status == "excluded" { "ok" } else { status });
        if let Some(reason) = reason.filter(|_| status != "excluded") {
            reasons.insert(format!("leg:{reason}"));
        }
        leg_views.push(dict([
            ("wallet_id", s(leg.wallet_id.as_str())),
            ("wallet", leg.wallet.clone()),
            ("owned_inputs_msat", int_value(leg.inputs_msat)),
            ("owned_outputs_msat", int_value(leg.outputs_msat)),
            ("owned_input_count", int_value(leg.inputs)),
            ("owned_output_count", int_value(leg.outputs)),
            (
                "expected_delta_msat",
                expected.map_or(Value::Null, int_value),
            ),
            ("rows", Value::List(found)),
            ("status", s(status)),
            ("reason", reason.map_or(Value::Null, s)),
        ]));
    }
    let mut unexplained = Vec::new();
    for (wallet_id, found) in &rows_by_wallet {
        if legs.iter().any(|leg| leg.wallet_id == *wallet_id) {
            continue;
        }
        let observed = observed_wallets.contains(wallet_id);
        for candidate in found {
            let excluded = truthy(g(candidate, "excluded"));
            let status = if excluded {
                "excluded"
            } else if observed {
                "mismatch"
            } else {
                "external_record"
            };
            let mut item = row_view(candidate);
            item.extend([
                ("wallet_id", s(wallet_id.as_str())),
                ("wallet", val(candidate, "wallet_label")),
                ("wallet_kind", val(candidate, "wallet_kind")),
                ("observed_wallet", Value::Bool(observed)),
                ("status", s(status)),
            ]);
            unexplained.push(dict(item));
            if status == "mismatch" {
                statuses.push("mismatch");
                reasons.insert("leg:row_without_owned_graph_leg".into());
            }
        }
    }
    if ambiguous > 0 {
        statuses.push("unverifiable");
        reasons.insert("legs:ambiguous_ownership".into());
    }

    let status = if statuses.contains(&"mismatch") {
        "mismatch"
    } else if statuses
        .iter()
        .all(|s| ["ok", "not_applicable"].contains(s))
    {
        "ok"
    } else {
        "partial"
    };
    let (input_count, output_count) = (inputs.len(), outputs.len());
    inputs.truncate(MAX_NODES_PER_SIDE);
    outputs.truncate(MAX_NODES_PER_SIDE);
    unexplained.truncate(MAX_ROWS);
    payload.extend([
        ("status", s(status)),
        ("reasons", Value::List(reasons.into_iter().map(s).collect())),
        ("legs", Value::List(leg_views)),
        ("unexplained_rows", Value::List(unexplained)),
        ("input_count", Value::Int(input_count.into())),
        ("output_count", Value::Int(output_count.into())),
        (
            "nodes_truncated",
            Value::Bool(input_count > MAX_NODES_PER_SIDE || output_count > MAX_NODES_PER_SIDE),
        ),
        ("inputs", Value::List(inputs)),
        ("outputs", Value::List(outputs)),
    ]);
    Ok(dict(payload))
}

fn view_flag(view: &Value, key: &str) -> bool {
    g(view, key) == &Value::Bool(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_roles_follow_branch_evidence() {
        assert_eq!(exact_role("m/1 Change"), Some("change"));
        assert_eq!(exact_role("internal"), Some("change"));
        assert_eq!(exact_role("p2wpkh-receive"), Some("receive"));
        assert_eq!(exact_role("Deposit"), Some("receive"));
        assert_eq!(exact_role("address"), None);
        assert_eq!(exact_role(""), None);
        let owned = |label: &str| Owned {
            wallet_id: "w".into(),
            wallet_label: "W".into(),
            branch_label: label.into(),
        };
        assert_eq!(keychain(&[owned("change"), owned("address")]), "change");
        assert_eq!(keychain(&[owned("change"), owned("receive")]), "unknown");
        assert_eq!(keychain(&[owned("inventory")]), "unknown");
    }

    #[test]
    fn wallet_delta_follows_boundary_amounts() {
        let row = |direction: &str, included: i64| {
            dict([
                ("direction", s(direction)),
                ("amount", Value::Int(1000.into())),
                ("fee", Value::Int(10.into())),
                ("amount_includes_fee", Value::Int(included.into())),
            ])
        };
        assert_eq!(wallet_delta(&row(" Outbound ", 0)), -1010);
        assert_eq!(wallet_delta(&row("outbound", 1)), -1000);
        assert_eq!(wallet_delta(&row("inbound", 1)), 1000);
        assert_eq!(wallet_delta(&row("swap", 0)), 1000);
    }
}
