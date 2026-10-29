//! Comparison-only journal request preparation. No engine call or persistence.
//! Admission deliberately follows Python's executed order, not field grouping.
use crate::{codec, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
mod config;
mod time;

/// A refusal retains the Python exception family and its structured attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub stage: &'static str,
    pub family: &'static str,
    pub message: String,
    pub details: Box<Value>,
    pub hint: Option<Box<str>>,
    pub code: Option<&'static str>,
    pub invariant: Option<&'static str>,
}
impl Refusal {
    fn new(stage: &'static str, family: &'static str, message: impl Into<String>) -> Self {
        Self {
            stage,
            family,
            message: message.into(),
            details: Box::new(Value::Null),
            hint: None,
            code: None,
            invariant: None,
        }
    }
    fn contract(invariant: &'static str, detail: String) -> Self {
        let mut error = Self::new(
            "invariants",
            "EngineContractError",
            format!("RequestError: {invariant}: {detail}"),
        );
        error.invariant = Some(invariant);
        error
    }
    pub fn document(&self) -> Value {
        json!({"stage":self.stage,"family":self.family,"message":self.message,"details":self.details,"hint":self.hint,"code":self.code,"invariant":self.invariant,"retryable":(self.family == "AppError").then_some(false),"error_class":self.invariant.map(|_| "RequestError")})
    }
}

/// Empty projections bypass configuration and never call the ledger engine.
/// As for Python build_request, the caller owns the constructor tax-support gate.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Empty,
    Request(Value),
}

/// Ordered wallet references: mapping keys and values are both admission facts.
/// These are compiler operands, never additions to snapshot v1.
pub fn build_request(
    profile: &Value,
    projection: &Value,
    wallets: &[(Value, Value)],
    operation: &str,
) -> Result<Outcome, Refusal> {
    let rows = list(&projection["rows"]);
    if rows.is_empty() {
        return Ok(Outcome::Empty);
    }
    let (policy, names) = config::check(profile, wallets, rows)?;
    for pair in list(&projection["intra_pairs"]) {
        if string(&pair["out"]["asset"]) != string(&pair["in"]["asset"]) {
            return Err(Refusal::new(
                "crossed_assets",
                "AssertionError",
                "finalized same-asset move pair crossed assets",
            ));
        }
    }
    config::pool_scope(profile)?;
    invariants(rows, list(&projection["intra_pairs"]), wallets)?;
    let holds = holds(rows)?;
    // Keep expression evaluation sequential. A deferred timestamp object does
    // not suppress the immediate cutoff conversion error of the same row.
    let holder = text(&profile["label"]);
    let mut refs = Vec::new();
    for (_, item) in wallets {
        refs.push(json!({"id":string(required(item,"id")?),"label":text(&item["label"])}));
    }
    let rows = rows
        .iter()
        .map(|row| request_row(row, &holds))
        .collect::<Result<Vec<_>, _>>()?;
    let mut intra = Vec::new();
    for pair in list(&projection["intra_pairs"]) {
        intra.push(json!({"out":string(required(&pair["out"],"id")?),"in":string(required(&pair["in"],"id")?),"source":text(&pair["source"]),"group_id":if empty(&pair["group_id"]) {Value::Null} else {json!(string(&pair["group_id"]))}}));
    }
    let mut cross = Vec::new();
    for pair in list(&projection["cross_asset_pairs"]) {
        let pair_id = text(&pair["pair_id"]);
        let out_id = string(required(pair, "out_id")?);
        let in_id = string(required(pair, "in_id")?);
        let assets: BTreeSet<_> = [&pair["out_asset"], &pair["in_asset"]]
            .into_iter()
            .map(|v| or_text(v).py_strip().py_upper())
            .collect();
        cross.push(json!({"pair_id":pair_id,"out_id":out_id,"in_id":in_id,"out_transaction_id":text(&pair["out_transaction_id"]),"in_transaction_id":text(&pair["in_transaction_id"]),"out_asset":text(&pair["out_asset"]),"in_asset":text(&pair["in_asset"]),"policy":text(&pair["policy"]),"bitcoin_rail":assets==BTreeSet::from(["BTC".into(),"LBTC".into()])}));
    }
    Ok(Outcome::Request(
        json!({"schema_version":2,"operation":operation,"policy":policy,"config":names,"holder":holder,"wallets":refs,"rows":rows,"intra_pairs":intra,"cross_asset_pairs":cross}),
    ))
}
fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value, Refusal> {
    value
        .get(key)
        .ok_or_else(|| Refusal::new("construction", "KeyError", repr(&json!(key))))
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn string(v: &Value) -> String {
    crate::interpretation::python_value_string(&codec::from_json(v).expect("JSON operand"))
}
fn repr(v: &Value) -> String {
    crate::interpretation::value_repr(&codec::from_json(v).expect("JSON operand"))
}
fn or_text(v: &Value) -> String {
    if codec::truth(v) {
        string(v)
    } else {
        String::new()
    }
}
fn text(v: &Value) -> Value {
    if v.is_null() {
        Value::Null
    } else {
        json!(string(v))
    }
}
fn empty(v: &Value) -> bool {
    v.is_null() || v == ""
}
fn integer(v: &Value) -> Option<BigInt> {
    let n = v.as_number()?.to_string();
    if n.contains(['.', 'e', 'E']) {
        None
    } else {
        BigInt::from_str(&n).ok()
    }
}
fn privacy(row: &Value) -> bool {
    !row["privacy_boundary"].is_null()
        && matches!(
            string(&row["privacy_boundary"])
                .py_strip()
                .py_lower()
                .replace('-', "_")
                .as_str(),
            "coinjoin" | "collaborative" | "payjoin" | "payment_in_coinjoin" | "sweep"
        )
}
fn decimal(values: &[&Value]) -> Result<Value, ()> {
    for v in values {
        if empty(v) {
            continue;
        }
        return crate::finalized_projection::request_decimal(v)
            .map(|v| json!(v))
            .map_err(|_| ());
    }
    Ok(Value::Null)
}
fn invariants(rows: &[Value], pairs: &[Value], wallets: &[(Value, Value)]) -> Result<(), Refusal> {
    for (key, item) in wallets {
        if string(key) != string(&item["id"]) {
            return Err(Refusal::contract(
                "wallet_ref_id_mismatch",
                format!(
                    "wallet reference {} has id {}",
                    repr(&json!(string(key))),
                    repr(&item["id"])
                ),
            ));
        }
    }
    let ids: BTreeSet<_> = wallets.iter().map(|(key, _)| string(key)).collect();
    for row in rows {
        let id = or_text(&row["id"]);
        let quoted = repr(&json!(id));
        let fail = |name, detail| Err(Refusal::contract(name, detail));
        if ![
            "custody-tax:",
            "cross-split:",
            "direct-payout:",
            "multi-consol:",
            "owned-derive:",
            "recorded-fanout:",
        ]
        .iter()
        .any(|p| id.starts_with(p))
        {
            return fail(
                "non_synthetic_row_id",
                format!("row {quoted} is not a projection row"),
            );
        }
        if row["journal_transaction_id"]
            .as_str()
            .is_none_or(str::is_empty)
        {
            return fail(
                "missing_anchor",
                format!("row {quoted} has no journal transaction"),
            );
        }
        if !ids.contains(&string(&row["wallet_id"])) {
            return fail(
                "unknown_wallet",
                format!("row {quoted} names a wallet the wallet references do not list"),
            );
        }
        let direction = row["direction"].as_str().unwrap_or("");
        if !["inbound", "outbound"].contains(&direction) {
            return fail(
                "unknown_direction",
                format!("row {quoted} has direction {}", repr(&row["direction"])),
            );
        }
        let (Some(amount), Some(fee)) = (integer(&row["amount"]), integer(&row["fee"])) else {
            return fail(
                "non_integer_msat",
                format!(
                    "row {quoted} has amount {} and fee {}",
                    repr(&row["amount"]),
                    repr(&row["fee"])
                ),
            );
        };
        let zero = BigInt::from(0);
        if fee < zero {
            return fail("negative_fee", format!("row {quoted} has fee {fee}"));
        }
        if amount < zero {
            return fail(
                "negative_amount",
                format!("row {quoted} has amount {amount}"),
            );
        }
        if (direction == "inbound" && amount <= zero)
            || (direction == "outbound" && &amount + &fee <= zero)
        {
            return fail(
                "non_positive_amount",
                format!("{direction} row {quoted} has amount {amount} and fee {fee}"),
            );
        }
        if codec::truth(&row["amount_includes_fee"]) {
            return fail(
                "amount_includes_fee",
                format!("row {quoted} includes its fee in its amount"),
            );
        }
        if row
            .as_object()
            .is_some_and(|m| m.keys().any(|k| k.starts_with("custody_component_")))
        {
            return fail(
                "custody_component_fields",
                format!("row {quoted} carries custody component fields"),
            );
        }
        for field in ["fiat_rate", "fiat_value"] {
            let values = [&row[format!("{field}_exact")], &row[field]];
            if decimal(&values).is_err() {
                let corrupt = values.into_iter().find(|v| !empty(v)).unwrap();
                return fail(
                    "corrupt_decimal_text",
                    format!("row {quoted} has {field} {}", repr(corrupt)),
                );
            }
        }
    }
    let mut legs = BTreeSet::new();
    for pair in pairs {
        let out = &pair["out"];
        let input = &pair["in"];
        let name = format!("{} -> {}", repr(&out["id"]), repr(&input["id"]));
        if codec::truth(&pair["group_block_rows"]) {
            return Err(Refusal::contract(
                "group_block_rows",
                format!("move {name} blocks rows outside its pair"),
            ));
        }
        if (privacy(out) || privacy(input)) && pair["kind"] != "custody-reviewed-coinjoin" {
            return Err(Refusal::contract(
                "unreviewed_privacy_pair",
                format!("move {name} crosses a privacy boundary without review"),
            ));
        }
        for leg in [out, input] {
            let id = string(&leg["id"]);
            if !legs.insert(id.clone()) {
                return Err(Refusal::contract(
                    "shared_pair_leg",
                    format!("row {} is a leg of two moves", repr(&json!(id))),
                ));
            }
        }
        if !crate::graph::stored::equal(
            &codec::from_json(&out["amount"]).unwrap(),
            &codec::from_json(&input["amount"]).unwrap(),
            false,
        ) {
            return Err(Refusal::contract(
                "unequal_move_amounts",
                format!(
                    "move {name} sends {} and receives {}",
                    repr(&out["amount"]),
                    repr(&input["amount"])
                ),
            ));
        }
    }
    Ok(())
}
fn holds(rows: &[Value]) -> Result<BTreeMap<String, &'static str>, Refusal> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for row in rows {
        let asset = string(&row["asset"]);
        if let Some((_, group)) = groups.iter_mut().find(|(a, _)| *a == asset) {
            group.push(row.clone());
        } else {
            groups.push((asset, vec![row.clone()]));
        }
    }
    let mut holds = BTreeMap::new();
    for (_, rows) in groups {
        let fail = |family: String| {
            Refusal::new(
                "holds",
                match family.as_str() {
                    "OverflowError" => "OverflowError",
                    "ValueError" => "ValueError",
                    "TypeError" => "TypeError",
                    _ => "RuntimeError",
                },
                family,
            )
        };
        let conflicts = crate::interpretation::ledger_conflicts(&rows).map_err(fail)?;
        let pending = crate::interpretation::ledger_pending(&rows).map_err(fail)?;
        for row in &rows {
            let id = string(&row["id"]);
            if conflicts.contains(&id) {
                holds.insert(id, "conflicting_spend");
            } else if pending.contains(&id) {
                holds.insert(id, "pending_onchain_confirmation");
            }
        }
    }
    Ok(holds)
}
fn request_row(row: &Value, holds: &BTreeMap<String, &str>) -> Result<Value, Refusal> {
    let id = string(required(row, "id")?);
    let anchor = required(row, "journal_transaction_id")?;
    let asset = string(&row["asset"]);
    let wallet = string(required(row, "wallet_id")?);
    let direction = required(row, "direction")?;
    let ts = time::timestamp(&row["occurred_at"]);
    let cutoff = time::cutoff(&row["occurred_at"])?;
    let amount = required(row, "amount")?;
    let fee = required(row, "fee")?;
    let rate = decimal(&[&row["fiat_rate_exact"], &row["fiat_rate"]]).map_err(|_| {
        Refusal::new(
            "construction",
            "InvalidOperation",
            "[<class 'decimal.ConversionSyntax'>]",
        )
    })?;
    let value = decimal(&[&row["fiat_value_exact"], &row["fiat_value"]]).map_err(|_| {
        Refusal::new(
            "construction",
            "InvalidOperation",
            "[<class 'decimal.ConversionSyntax'>]",
        )
    })?;
    let kind = if empty(&row["kind_override"]) {
        &row["kind"]
    } else {
        &row["kind_override"]
    };
    let kind = if kind.is_null() {
        String::new()
    } else {
        string(kind).py_strip().py_lower().replace(['-', ' '], "_")
    };
    let row_text = ["note", "description", "kind"]
        .into_iter()
        .find_map(|key| codec::truth(&row[key]).then(|| text(&row[key])))
        .unwrap_or(Value::Null);
    let regime = if row["at_regime_override"] == "alt" || row["at_regime_override"] == "neu" {
        row["at_regime_override"].clone()
    } else {
        Value::Null
    };
    let category = if codec::truth(&row["at_category_override"]) {
        text(&row["at_category_override"])
    } else {
        Value::Null
    };
    let taxability = crate::graph::stored::equal(
        &codec::from_json(&row["taxability_override"]).unwrap(),
        &kassiber_pyjson::Value::Int(BigInt::from(0)),
        false,
    );
    let order = &row["custody_chain_order"];
    let order = if order.is_null() {
        Value::Null
    } else {
        let mut parents = list(
            order
                .get("parents")
                .and_then(|v| v.get("$set"))
                .unwrap_or(&order["parents"]),
        )
        .to_vec();
        parents.sort_by_key(|p| list(p).iter().map(string).collect::<Vec<_>>());
        json!({"scope":list(&order["scope"]).iter().map(string).collect::<Vec<_>>(),"parents":parents.iter().map(|p|list(p).iter().map(string).collect::<Vec<_>>()).collect::<Vec<_>>()})
    };
    Ok(
        json!({"id":id,"anchor":anchor,"asset":asset,"wallet_id":wallet,"direction":direction,"ts":ts,"cutoff_instant":cutoff,"amount_msat":amount,"fee_msat":fee,"rate":rate,"value":value,"coarse":row["pricing_quality"]=="coarse_fallback","kind":kind,"text":row_text,"regime_override":regime,"category_override":category,"taxability_off":taxability,"privacy_hop":privacy(row),"hold":holds.get(&id),"chain_order":order}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors(prefix: &str) {
        let cases: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_fusion_builder.json"
        ))
        .unwrap();
        let mut count = 0;
        for case in cases.as_array().unwrap() {
            if !case["case"].as_str().unwrap().starts_with(prefix) {
                continue;
            }
            count += 1;
            let input = &case["input"];
            let wallets = input["wallets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect::<Vec<_>>();
            let actual =
                match build_request(&input["profile"], &input["projection"], &wallets, "ledger") {
                    Ok(Outcome::Empty) => json!({"outcome":"empty"}),
                    Ok(Outcome::Request(request)) => json!({"outcome":"request","request":request}),
                    Err(error) => {
                        let mut error = error.document();
                        error.as_object_mut().unwrap().remove("stage");
                        json!({"outcome":"refusal","error":error})
                    }
                };
            assert_eq!(actual, case["expected"], "{}", case["case"]);
        }
        assert!(count > 0, "vacuous fixture filter {prefix}");
    }

    #[test]
    fn fusion_builder_admission_order() {
        vectors("ordering-");
    }
    #[test]
    fn fusion_builder_field_choice() {
        vectors("row-");
    }
    #[test]
    fn fusion_builder_configuration() {
        vectors("profile-");
    }
    #[test]
    fn fusion_builder_pair_invariants() {
        vectors("pair-");
    }
    #[test]
    fn fusion_builder_empty() {
        vectors("empty-");
    }
}
