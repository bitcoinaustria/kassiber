//! Comparison-only journal request preparation. No engine call or persistence.
//! Admission deliberately follows Python's executed order, not field grouping.
use crate::{codec, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
mod config;
mod time;

/// The shared Python timestamp rendering used by journal fragment provenance.
pub fn timestamp_fields(value: &Value) -> Value {
    time::timestamp(value)
}

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
    /// Python's insertion order of the top-level `details` keys, when it
    /// differs from the sorted JSON map.
    /// A thin reference keeps the refusal within its error-size budget.
    pub details_order: Option<&'static &'static [&'static str]>,
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
            details_order: None,
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

/// A finalized projection row as the builder reads it: its JSON object, or
/// a view whose fields are that object's.
pub trait RequestRow {
    /// The field `key`; `None` when the row has none.
    fn get(&self, key: &str) -> Option<&Value>;
    /// Whether the row has a field whose name satisfies `test`.
    fn any_key(&self, test: &dyn Fn(&str) -> bool) -> bool;
    /// The row's JSON object.
    fn json(&self) -> Cow<'_, Value>;
    /// `row[key]`: the field, or null.
    fn at(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&Value::Null)
    }
}
impl RequestRow for Value {
    fn get(&self, key: &str) -> Option<&Value> {
        Value::get(self, key)
    }
    fn any_key(&self, test: &dyn Fn(&str) -> bool) -> bool {
        self.as_object().is_some_and(|m| m.keys().any(|k| test(k)))
    }
    fn json(&self) -> Cow<'_, Value> {
        Cow::Borrowed(self)
    }
}
impl RequestRow for crate::finalized_projection::RowView<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        crate::finalized_projection::RowView::get(self, key)
    }
    fn any_key(&self, test: &dyn Fn(&str) -> bool) -> bool {
        crate::finalized_projection::RowView::any_key(self, test)
    }
    fn json(&self) -> Cow<'_, Value> {
        Cow::Owned(crate::finalized_projection::RowView::json(self))
    }
}

/// Ordered wallet references: mapping keys and values are both admission facts.
/// These are compiler operands, never additions to snapshot v1.
pub fn build_request(
    profile: &Value,
    projection: &Value,
    wallets: &[(Value, Value)],
    operation: &str,
) -> Result<Outcome, Refusal> {
    build_request_rows(
        profile,
        list(&projection["rows"]),
        list(&projection["intra_pairs"]),
        list(&projection["cross_asset_pairs"]),
        wallets,
        operation,
    )
}

/// `build_request` of a projection whose rows, `intra_pairs` and
/// `cross_asset_pairs` are given apart.
pub fn build_request_rows<R: RequestRow>(
    profile: &Value,
    rows: &[R],
    intra_pairs: &[Value],
    cross_asset_pairs: &[Value],
    wallets: &[(Value, Value)],
    operation: &str,
) -> Result<Outcome, Refusal> {
    if rows.is_empty() {
        return Ok(Outcome::Empty);
    }
    let (policy, names) = config::check(profile, wallets, rows)?;
    for pair in intra_pairs {
        if string(&pair["out"]["asset"]) != string(&pair["in"]["asset"]) {
            return Err(Refusal::new(
                "crossed_assets",
                "AssertionError",
                "finalized same-asset move pair crossed assets",
            ));
        }
    }
    config::pool_scope(profile)?;
    let decimals = invariants(rows, intra_pairs, wallets)?;
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
        .zip(decimals)
        .map(|(row, [rate, value])| request_row(row, &holds, rate, value))
        .collect::<Result<Vec<_>, _>>()?;
    let mut intra = Vec::new();
    for pair in intra_pairs {
        intra.push(json!({"out":string(required(&pair["out"],"id")?),"in":string(required(&pair["in"],"id")?),"source":text(&pair["source"]),"group_id":if empty(&pair["group_id"]) {Value::Null} else {json!(string(&pair["group_id"]))}}));
    }
    let mut cross = Vec::new();
    for pair in cross_asset_pairs {
        let pair_id = text(&pair["pair_id"]);
        let out_id = string(required(pair, "out_id")?);
        let in_id = string(required(pair, "in_id")?);
        let assets: BTreeSet<_> = [&pair["out_asset"], &pair["in_asset"]]
            .into_iter()
            .map(|v| or_text(v).py_strip().py_upper())
            .collect();
        cross.push(json!({"pair_id":pair_id,"out_id":out_id,"in_id":in_id,"out_transaction_id":text(&pair["out_transaction_id"]),"in_transaction_id":text(&pair["in_transaction_id"]),"out_asset":text(&pair["out_asset"]),"in_asset":text(&pair["in_asset"]),"policy":text(&pair["policy"]),"bitcoin_rail":assets==BTreeSet::from(["BTC".into(),"LBTC".into()])}));
    }
    Ok(Outcome::Request(Value::Object(serde_json::Map::from_iter(
        [
            ("schema_version".into(), json!(2)),
            ("operation".into(), json!(operation)),
            ("policy".into(), policy),
            ("config".into(), names),
            ("holder".into(), holder),
            ("wallets".into(), Value::Array(refs)),
            ("rows".into(), Value::Array(rows)),
            ("intra_pairs".into(), Value::Array(intra)),
            ("cross_asset_pairs".into(), Value::Array(cross)),
        ],
    ))))
}
/// Python's compact request wire order is observable in engine parse errors.
/// Restrict ordering to the builder's schema; never serialize caller metadata.
pub fn request_json(request: &Value) -> String {
    use kassiber_pyjson::Options;
    fn ordered(value: &Value, options: &Options, out: &mut String) {
        match value {
            Value::Object(items) => {
                let keys: &[&str] = if items.contains_key("schema_version") {
                    &[
                        "schema_version",
                        "operation",
                        "policy",
                        "config",
                        "holder",
                        "wallets",
                        "rows",
                        "intra_pairs",
                        "cross_asset_pairs",
                    ]
                } else if items.contains_key("country") {
                    &["country", "method", "method_label", "require_coarse_review"]
                } else if items.contains_key("long_term_days")
                    || items.get("kind").is_some_and(|v| v == "at")
                {
                    &["kind", "long_term_days"]
                } else if items.contains_key("assets") {
                    &["assets", "exchanges", "holders"]
                } else if items.contains_key("amount_msat") {
                    &[
                        "id",
                        "anchor",
                        "asset",
                        "wallet_id",
                        "direction",
                        "ts",
                        "cutoff_instant",
                        "amount_msat",
                        "fee_msat",
                        "rate",
                        "value",
                        "coarse",
                        "kind",
                        "text",
                        "regime_override",
                        "category_override",
                        "taxability_off",
                        "privacy_hop",
                        "hold",
                        "chain_order",
                    ]
                } else if items.contains_key("raw") {
                    &[
                        "raw",
                        "error",
                        "error_class",
                        "us",
                        "offset_s",
                        "date",
                        "year",
                        "vienna_date",
                        "display",
                        "iso",
                    ]
                } else if items.contains_key("out_id") {
                    &[
                        "pair_id",
                        "out_id",
                        "in_id",
                        "out_transaction_id",
                        "in_transaction_id",
                        "out_asset",
                        "in_asset",
                        "policy",
                        "bitcoin_rail",
                    ]
                } else if items.contains_key("out") {
                    &["out", "in", "source", "group_id"]
                } else if items.contains_key("scope") {
                    &["scope", "parents"]
                } else if items.contains_key("label") {
                    &["id", "label"]
                } else {
                    &["error", "us"]
                };
                assert!(
                    items.keys().all(|key| keys.contains(&key.as_str())),
                    "unknown ledger request field"
                );
                out.push('{');
                let mut first = true;
                for key in keys {
                    if let Some(value) = items.get(*key) {
                        if !first {
                            out.push(',');
                        }
                        first = false;
                        kassiber_pyjson::dumps_str_into(key, options, out);
                        out.push(':');
                        ordered(value, options, out);
                    }
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    ordered(item, options, out);
                }
                out.push(']');
            }
            Value::String(value) => kassiber_pyjson::dumps_str_into(value, options, out),
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Number(value) if value.as_i64().is_some() => {
                out.push_str(&value.as_i64().unwrap().to_string());
            }
            // Keep Python's exact integer and float spelling, including amounts
            // the engine will refuse. Their wire position appears in its error.
            value => out.push_str(
                &kassiber_pyjson::dumps(&codec::from_json(value).expect("builder scalar"), options)
                    .expect("builder request serialization"),
            ),
        }
    }
    let options = Options::new().compact().ensure_ascii(false);
    let mut out = String::new();
    ordered(request, &options, &mut out);
    out
}

fn required<'a>(value: &'a (impl RequestRow + ?Sized), key: &str) -> Result<&'a Value, Refusal> {
    value
        .get(key)
        .ok_or_else(|| Refusal::new("construction", "KeyError", repr(&json!(key))))
}
fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn string(v: &Value) -> String {
    if let Value::String(value) = v {
        return value.clone();
    }
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
fn privacy(row: &impl RequestRow) -> bool {
    !row.at("privacy_boundary").is_null()
        && matches!(
            string(row.at("privacy_boundary"))
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
fn invariants(
    rows: &[impl RequestRow],
    pairs: &[Value],
    wallets: &[(Value, Value)],
) -> Result<Vec<[Value; 2]>, Refusal> {
    struct Quoted<'a>(&'a str);
    impl std::fmt::Display for Quoted<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&repr(&json!(self.0)))
        }
    }
    let mut decimals = Vec::with_capacity(rows.len());
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
        let id = or_text(row.at("id"));
        let quoted = Quoted(&id);
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
        if row
            .at("journal_transaction_id")
            .as_str()
            .is_none_or(str::is_empty)
        {
            return fail(
                "missing_anchor",
                format!("row {quoted} has no journal transaction"),
            );
        }
        if !ids.contains(&string(row.at("wallet_id"))) {
            return fail(
                "unknown_wallet",
                format!("row {quoted} names a wallet the wallet references do not list"),
            );
        }
        let direction = row.at("direction").as_str().unwrap_or("");
        if !["inbound", "outbound"].contains(&direction) {
            return fail(
                "unknown_direction",
                format!("row {quoted} has direction {}", repr(row.at("direction"))),
            );
        }
        let (Some(amount), Some(fee)) = (integer(row.at("amount")), integer(row.at("fee"))) else {
            return fail(
                "non_integer_msat",
                format!(
                    "row {quoted} has amount {} and fee {}",
                    repr(row.at("amount")),
                    repr(row.at("fee"))
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
        if codec::truth(row.at("amount_includes_fee")) {
            return fail(
                "amount_includes_fee",
                format!("row {quoted} includes its fee in its amount"),
            );
        }
        if row.any_key(&|k| k.starts_with("custody_component_")) {
            return fail(
                "custody_component_fields",
                format!("row {quoted} carries custody component fields"),
            );
        }
        let mut row_decimals = [Value::Null, Value::Null];
        for (index, (field, exact)) in [
            ("fiat_rate", "fiat_rate_exact"),
            ("fiat_value", "fiat_value_exact"),
        ]
        .into_iter()
        .enumerate()
        {
            let values = [row.at(exact), row.at(field)];
            match decimal(&values) {
                Ok(value) => row_decimals[index] = value,
                Err(()) => {
                    let corrupt = values.into_iter().find(|v| !empty(v)).unwrap();
                    return fail(
                        "corrupt_decimal_text",
                        format!("row {quoted} has {field} {}", repr(corrupt)),
                    );
                }
            }
        }
        decimals.push(row_decimals);
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
    Ok(decimals)
}
fn holds(rows: &[impl RequestRow]) -> Result<BTreeMap<String, &'static str>, Refusal> {
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    for row in rows {
        let asset = string(row.at("asset"));
        let graphless = graphless_projection(row);
        if let Some((_, group)) = groups.iter_mut().find(|(a, _)| *a == asset) {
            if !graphless {
                group.push(row.json().into_owned());
            }
        } else {
            // Retain the asset's first-seen position even when its first row
            // has no physical scope: later groups can refuse in that order.
            groups.push((
                asset,
                if graphless {
                    vec![]
                } else {
                    vec![row.json().into_owned()]
                },
            ));
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

// Custody slice ids are excluded by onchain_scope and are deliberately absent
// from the confirmation detector's renamed synthetic-id list. Its remaining
// fallback requires a parsed graph. Prove that graph absent before copying and
// parsing ordinary commercial rows twice. Anything uncertain takes the full
// detector, including malformed JSON, graph arrays and Liquid raw hex.
fn graphless_projection(row: &impl RequestRow) -> bool {
    if !row
        .at("id")
        .as_str()
        .is_some_and(|id| id.starts_with("custody-tax:"))
    {
        return false;
    }
    let parsed;
    let raw = if let Some(text) = row.at("raw_json").as_str() {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return false;
        };
        parsed = value;
        &parsed
    } else {
        row.at("raw_json")
    };
    if !raw.is_object() {
        return true;
    }
    !codec::truth(&raw["raw_hex"])
        && [raw, &raw["tx"], &raw["ownership_graph"]]
            .into_iter()
            .all(|payload| !payload["vin"].is_array() || !payload["vout"].is_array())
}
/// `_coarse_needs_review`: only an unaccepted or changed coarse price is held.
fn coarse_needs_review(row: &impl RequestRow) -> Result<bool, Refusal> {
    crate::coarse_review::needs_review(|key| row.get(key))
        .map_err(|(family, message)| Refusal::new("construction", family, message))
}
fn request_row(
    row: &impl RequestRow,
    holds: &BTreeMap<String, &str>,
    rate: Value,
    value: Value,
) -> Result<Value, Refusal> {
    let id = string(required(row, "id")?);
    let anchor = required(row, "journal_transaction_id")?;
    let asset = string(row.at("asset"));
    let wallet = string(required(row, "wallet_id")?);
    let direction = required(row, "direction")?;
    let ts = time::timestamp(row.at("occurred_at"));
    let cutoff = time::cutoff(row.at("occurred_at"))?;
    let amount = required(row, "amount")?;
    let fee = required(row, "fee")?;
    let kind = if empty(row.at("kind_override")) {
        row.at("kind")
    } else {
        row.at("kind_override")
    };
    let kind = if kind.is_null() {
        String::new()
    } else {
        string(kind).py_strip().py_lower().replace(['-', ' '], "_")
    };
    let row_text = ["note", "description", "kind"]
        .into_iter()
        .find_map(|key| codec::truth(row.at(key)).then(|| text(row.at(key))))
        .unwrap_or(Value::Null);
    let regime = if row.at("at_regime_override") == "alt" || row.at("at_regime_override") == "neu" {
        row.at("at_regime_override").clone()
    } else {
        Value::Null
    };
    let category = if codec::truth(row.at("at_category_override")) {
        text(row.at("at_category_override"))
    } else {
        Value::Null
    };
    let taxability = crate::graph::stored::equal(
        &codec::from_json(row.at("taxability_override")).unwrap(),
        &kassiber_pyjson::Value::Int(BigInt::from(0)),
        false,
    );
    let order = row.at("custody_chain_order");
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
    let hold = holds
        .get(&id)
        .map(|hold| json!(hold))
        .unwrap_or(Value::Null);
    Ok(Value::Object(serde_json::Map::from_iter([
        ("id".into(), Value::String(id)),
        ("anchor".into(), anchor.clone()),
        ("asset".into(), Value::String(asset)),
        ("wallet_id".into(), Value::String(wallet)),
        ("direction".into(), direction.clone()),
        ("ts".into(), ts),
        ("cutoff_instant".into(), cutoff),
        ("amount_msat".into(), amount.clone()),
        ("fee_msat".into(), fee.clone()),
        ("rate".into(), rate),
        ("value".into(), value),
        ("coarse".into(), Value::Bool(coarse_needs_review(row)?)),
        ("kind".into(), Value::String(kind)),
        ("text".into(), row_text),
        ("regime_override".into(), regime),
        ("category_override".into(), category),
        ("taxability_off".into(), Value::Bool(taxability)),
        ("privacy_hop".into(), Value::Bool(privacy(row))),
        ("hold".into(), hold),
        ("chain_order".into(), order),
    ])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_graphless_rows_keep_physical_confirmation_scope() {
        let txid = "a".repeat(64);
        let row = json!({"id":"cross-split:pending", "external_id":txid,
            "external_id_kind":"txid", "asset":"BTC",
            "raw_json":{"txid":txid,"status":{"confirmed":false}}});
        assert_eq!(
            holds(&[row]).unwrap().get("cross-split:pending"),
            Some(&"pending_onchain_confirmation")
        );
    }

    #[test]
    fn fusion_graphless_rows_match_confirmation_detectors() {
        let txid = "a".repeat(64);
        for raw in [
            json!({}),
            json!({"txid":txid,"status":{"confirmed":false}}),
            json!({"tx":{"txid":txid,"status":{"confirmed":false}}}),
            json!({"txid":txid,"status":{"confirmed":false},"vin":[],"vout":[]}),
            json!({"ownership_graph":{"txid":txid,"vin":[],"vout":[]}}),
            json!({"chain":"liquid","raw_hex":"00"}),
        ] {
            for raw in [raw.clone(), json!(raw.to_string())] {
                let rows = [json!({"id":"custody-tax:pending", "external_id":txid,
                    "external_id_kind":"txid", "asset":"BTC", "raw_json":raw})];
                let conflicts = crate::interpretation::ledger_conflicts(&rows).unwrap();
                let pending = crate::interpretation::ledger_pending(&rows).unwrap();
                let expected = if !conflicts.is_empty() {
                    Some(&"conflicting_spend")
                } else if !pending.is_empty() {
                    Some(&"pending_onchain_confirmation")
                } else {
                    None
                };
                assert_eq!(holds(&rows).unwrap().get("custody-tax:pending"), expected);
            }
        }
    }

    #[test]
    fn fusion_holds_read_whole_rows_through_views() {
        let txid = "a".repeat(64);
        let rows = vec![
            json!({"id":"cross-split:pending", "external_id":txid, "external_id_kind":"txid",
                "asset":"BTC", "raw_json":{"txid":txid,"status":{"confirmed":false}}}),
            json!({"id":"custody-tax:graphless", "asset":"BTC", "raw_json":"{}"}),
        ];
        let projection = crate::finalized_projection::of_rows(rows.clone());
        let (_, views) = projection
            .json_with_views(&[], |py| Ok::<_, String>(codec::to_json(py)))
            .unwrap();
        let from_views = holds(&views).unwrap();
        assert_eq!(from_views, holds(&rows).unwrap());
        assert_eq!(
            from_views.get("cross-split:pending"),
            Some(&"pending_onchain_confirmation")
        );
    }

    #[test]
    fn fusion_request_wire_order() {
        let cases: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/custody_fusion_wire.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let input = &case["input"];
            let wallets = input["wallets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect::<Vec<_>>();
            let Outcome::Request(request) =
                build_request(&input["profile"], &input["projection"], &wallets, "ledger").unwrap()
            else {
                panic!("active wire witness");
            };
            assert_eq!(
                request_json(&request),
                case["request_text"],
                "{}",
                case["case"]
            );
        }
        assert_eq!(cases.as_array().unwrap().len(), 5);
    }

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
