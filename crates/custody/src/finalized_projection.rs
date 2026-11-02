//! Finalized-projection compiler and comparison primitives.
//! Snapshot stage results remain comparison-only; no result grants persistence authority.
use crate::evidence::canonical::LegRef;
use crate::{codec, evidence, graph, interpretation::python_value_string};
use kassiber_pyjson::{BigInt, Value as Py};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::str::FromStr;
mod composition;
mod conversions;
mod fees;
mod money;
mod payouts;
mod principal;
mod quarantines;
mod row;
pub mod runtime;
pub use fees::FeeAllocation;
pub use principal::slice_row;

/// The finalized projection stage. Its rows name their snapshot source rows,
/// so its Python value and JSON are built only on request.
#[derive(Clone, Debug)]
pub struct FinalizedProjection {
    rows: Vec<row::Detached>,
    cross: Vec<Value>,
    intra: Vec<Value>,
    moves: Vec<Value>,
    quarantines: Vec<Py>,
}
impl FinalizedProjection {
    /// The stage's Python value, given the snapshot rows its rows name.
    pub fn py(&self, rows: &[Value]) -> Result<Py, String> {
        let mut result = Py::Dict(vec![
            (
                kassiber_pyjson::Key::Str("$type".into()),
                Py::Str("FinalizedTaxProjection".into()),
            ),
            (
                kassiber_pyjson::Key::Str("cross_asset_pairs".into()),
                Py::List(
                    self.cross
                        .iter()
                        .map(codec::from_json)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                kassiber_pyjson::Key::Str("intra_pairs".into()),
                Py::List(
                    self.intra
                        .iter()
                        .map(codec::from_json)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                kassiber_pyjson::Key::Str("rows".into()),
                Py::List(
                    self.rows
                        .iter()
                        .map(|row| row.py(rows))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                kassiber_pyjson::Key::Str("selected_move_ids".into()),
                Py::List(
                    self.moves
                        .iter()
                        .map(codec::from_json)
                        .collect::<Result<_, _>>()?,
                ),
            ),
        ]);
        graph::stored::put(
            &mut result,
            "quarantines",
            Py::List(self.quarantines.clone()),
        );
        Ok(result)
    }

    /// `convert(&self.py(rows)?)` for a converter that maps every Python
    /// value to its JSON equivalent; JSON that converts to itself is cloned.
    pub fn json_with<E: From<String>>(
        &self,
        rows: &[Value],
        convert: impl Fn(&Py) -> Result<Value, E>,
    ) -> Result<Value, E> {
        let exact = |v: &Value| -> Result<Value, E> {
            if crate::quantity::runtime::plain(v) {
                Ok(v.clone())
            } else {
                convert(&codec::from_json(v)?)
            }
        };
        let list = |items: &[Value]| -> Result<Value, E> {
            Ok(Value::Array(
                items.iter().map(&exact).collect::<Result<_, E>>()?,
            ))
        };
        let mut result = serde_json::Map::new();
        result.insert("$type".into(), json!("FinalizedTaxProjection"));
        result.insert("cross_asset_pairs".into(), list(&self.cross)?);
        result.insert("intra_pairs".into(), list(&self.intra)?);
        result.insert(
            "rows".into(),
            Value::Array(
                self.rows
                    .iter()
                    .map(|row| row.json_with(rows, &exact))
                    .collect::<Result<_, E>>()?,
            ),
        );
        result.insert("selected_move_ids".into(), list(&self.moves)?);
        result.insert(
            "quarantines".into(),
            Value::Array(
                self.quarantines
                    .iter()
                    .map(&convert)
                    .collect::<Result<_, E>>()?,
            ),
        );
        Ok(Value::Object(result))
    }
}

/// A downstream refusal retains its public code and rendered message.
#[derive(Debug)]
pub struct Error {
    pub code: String,
    pub message: String,
}
impl Error {
    // The private primitive vectors predate rendered message comparisons and
    // retain their exception-family-only contract.
    fn legacy_code(self) -> String {
        if self.code == "validation" {
            "AppError".into()
        } else {
            self.code
        }
    }
}
impl From<String> for Error {
    fn from(code: String) -> Self {
        Self {
            message: code.clone(),
            code,
        }
    }
}
impl From<&str> for Error {
    fn from(code: &str) -> Self {
        code.to_owned().into()
    }
}

fn string(value: &Value) -> String {
    // str() of a JSON string is the string itself.
    if let Value::String(text) = value {
        return text.clone();
    }
    python_value_string(&codec::from_json(value).expect("JSON primitive input"))
}
fn text(value: &Value) -> String {
    if codec::truth(value) {
        string(value)
    } else {
        String::new()
    }
}

/// Python's constructors deliberately validate rows before move-pair references.
pub fn validate(rows: &[Value], pairs: &[Value]) -> Result<(), (&'static str, &'static str)> {
    validate_rows(rows, pairs)
}

fn validate_rows<R: row::Fields>(
    rows: &[R],
    pairs: &[Value],
) -> Result<(), (&'static str, &'static str)> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for row in rows {
        if !codec::truth(row.field("custody_finalized_tax_projection")) {
            return Err((
                "TypeError",
                "tax engines accept only finalized custody projection rows",
            ));
        }
        let id = text(row.field("id"));
        if id.is_empty() || !ids.insert(id) {
            return Err((
                "ValueError",
                "finalized tax projection rows need unique ids",
            ));
        }
    }
    for pair in pairs {
        let (out, input) = (&pair["out"], &pair["in"]);
        if !out.is_object() || !input.is_object() {
            return Err((
                "ValueError",
                "finalized move pairs require projected out and in rows",
            ));
        }
        if !ids.contains(&string(&out["id"])) || !ids.contains(&string(&input["id"])) {
            return Err((
                "ValueError",
                "finalized move pairs cannot reference raw/unprojected rows",
            ));
        }
    }
    Ok(())
}

pub fn slice_row_id(hash: &str, start: &BigInt, end: &BigInt, side: &str) -> String {
    format!("custody-tax:{hash}:{start}:{end}:{side}")
}
pub fn fee_row_id(hash: &str, side: &str, amount: &BigInt) -> String {
    format!("custody-tax:{hash}:{side}:{amount}")
}
pub fn payout_row_id(id: &str, side: &str) -> String {
    format!("direct-payout:{id}:{side}")
}

// Match the raw hash function's int(amount or 0), then int(fee or 0).
// Container failures are TypeError, numeric spelling failures are ValueError.
fn observed_hash(row: &Value) -> Result<(String, String), String> {
    for key in ["amount", "fee"] {
        crate::interpretation::integer_or_zero(&row[key])?;
    }
    evidence::observed_hash(row).map_err(|e| e.code.to_owned())
}

/// Capture closed authority on the original row, before its id/amount changes.
fn chain_order(row: &Value) -> Result<Value, String> {
    // int(None) fails, so rows without closed authority have no chain order.
    if row["observation_authority_version"].is_null()
        || codec::integer(&row["observation_authority_version"]).ok() != Some(json!(1))
    {
        return Ok(Value::Null);
    }
    if text(&row["observation_graph_hash"])
        != evidence::graph_hash(&row["raw_json"])
            .map_err(|e| e.code.to_owned())?
            .0
        || text(&row["observation_quantity_hash"]) != observed_hash(row)?.0
    {
        return Ok(Value::Null);
    }
    let Some((chain, network, txid, asset)) = graph::onchain_scope(row) else {
        return Ok(Value::Null);
    };
    let raw = if let Some(raw) = row["raw_json"].as_str() {
        codec::loads(raw).unwrap_or(Py::Null)
    } else {
        codec::from_json(&row["raw_json"])?
    };
    let mut parents = BTreeSet::new();
    let get = graph::stored::get;
    for graph in [&raw, get(&raw, "tx"), get(&raw, "ownership_graph")] {
        for entry in graph::stored::list(get(graph, "vin")) {
            let Py::Int(vout) = get(entry, "vout") else {
                continue;
            };
            if *vout < BigInt::from(0) {
                continue;
            }
            if let Some(parent) = graph::strict_txid(&codec::to_json(get(entry, "txid"))) {
                parents.insert((chain.clone(), network.clone(), parent, asset.clone()));
            }
        }
    }
    Ok(
        json!({"$type":"ChainOrderEvidence", "scope":[chain,network,txid,asset], "parents":{"$set":parents}}),
    )
}

trait Rows {
    fn get(&self, id: &str) -> Option<&Value>;

    fn projected(&self, id: &str) -> row::FinalizedRow<'_> {
        row::FinalizedRow::borrowed(self.row(id))
    }

    fn row(&self, id: &str) -> &Value {
        self.get(id).unwrap_or(&Value::Null)
    }

    fn nonempty(&self, id: &str) -> bool {
        codec::truth(self.row(id))
    }
}

impl Rows for Value {
    fn get(&self, id: &str) -> Option<&Value> {
        Value::get(self, id)
    }
}

pub fn base_row(observation: &Value, rows: &Value) -> Result<Value, Error> {
    base_row_with_rows(LegRef::Json(observation), rows).map(row::FinalizedRow::into_json)
}

fn base_row_with_rows<'a>(
    observation: LegRef<'_>,
    rows: &'a dyn Rows,
) -> Result<row::FinalizedRow<'a>, Error> {
    let id = text(&observation["transaction_id"]);
    let id = if rows.nonempty(&id) {
        id
    } else {
        text(&observation["anchor_transaction_id"])
    };
    let mut result = if rows.nonempty(&id) {
        rows.projected(&id)
    } else {
        row::FinalizedRow::borrowed(&Value::Null)
    };
    for field in [
        "fiat_value",
        "fiat_value_exact",
        "fiat_rate",
        "fiat_rate_exact",
    ] {
        let value = &result[field];
        if !money::empty(value) && !money::plain_finite(value) {
            let label = format!(
                "transaction {}: {field}",
                string(&observation["anchor_transaction_id"])
            );
            money::Money::finite_field(value, &label)?;
        }
    }
    // The projected source row is the row `rows` holds for this id.
    let order = chain_order(result.source())?;
    result.set("custody_chain_order", order);
    for key in ["wallet_id", "asset", "direction", "occurred_at"] {
        result.set(key, observation[key].clone());
    }
    if !codec::truth(&result["external_id"]) {
        result.set(
            "external_id",
            observation["event_key"]["native_event_id"].clone(),
        );
    }
    if !codec::truth(&result["raw_json"]) {
        result.set("raw_json", json!({}));
    }
    result.set("amount_includes_fee", json!(false));
    Ok(result)
}

pub fn fee_row(
    observation: &Value,
    amount: &BigInt,
    rows: &Value,
    side: &str,
) -> Result<Value, Error> {
    fee_row_with_rows(LegRef::Json(observation), amount, rows, side)
        .map(row::FinalizedRow::into_json)
}

fn fee_row_with_rows<'a>(
    observation: LegRef<'_>,
    amount: &BigInt,
    rows: &'a dyn Rows,
    side: &str,
) -> Result<row::FinalizedRow<'a>, Error> {
    let mut base = base_row_with_rows(observation, rows)?;
    if codec::truth(&observation["authoritative_chain_observation"])
        && observation["fee_attribution"] == "exact"
    {
        base.set("privacy_boundary", Value::Null);
    }
    base.set(
        "id",
        json!(fee_row_id(
            &string(&observation["quantity_hash"]),
            side,
            amount
        )),
    );
    base.set(
        "journal_transaction_id",
        observation["anchor_transaction_id"].clone(),
    );
    base.set("amount", json!(0));
    base.set("fee", number(amount));
    base.set("fiat_value", Value::Null);
    base.set("fiat_value_exact", Value::Null);
    base.set("amount_includes_fee", json!(false));
    base.set("custody_finalized_tax_projection", json!(true));
    base.set(
        "custody_quantity_hash",
        observation["quantity_hash"].clone(),
    );
    base.set("custody_slice_start_msat", json!(0));
    base.set("custody_slice_end_msat", json!(0));
    Ok(base)
}

fn integer_big(v: &Value) -> Result<BigInt, String> {
    // int(n or 0) of a JSON integer within i64 is that integer.
    if let Some(n) = v.as_i64() {
        return Ok(BigInt::from(n));
    }
    let value = crate::interpretation::integer_or_zero(v)?;
    BigInt::from_str(&value.to_string()).map_err(|_| "ValueError".into())
}
fn number(n: &BigInt) -> Value {
    if let Ok(n) = i64::try_from(n) {
        return Value::from(n);
    }
    if let Ok(n) = u64::try_from(n) {
        return Value::from(n);
    }
    serde_json::from_str(&n.to_string()).expect("exact integer")
}

/// Private compiler seam for lossless interpreter diagnostics. This is not raw
/// snapshot admission: callers still supply the independently compared operands.
/// The copied diagnostic population never passes through serde JSON values.
pub fn compose_lossless(value: &Value, diagnostics: &[Py]) -> Result<Py, String> {
    compose_lossless_detailed(value, diagnostics).map_err(Error::legacy_code)
}

pub(crate) fn compose_lossless_detailed(value: &Value, diagnostics: &[Py]) -> Result<Py, Error> {
    composition::evaluate_lossless(value, diagnostics)
}

/// Private fixture operation, not snapshot admission or a product interface.
pub fn primitive(value: &Value) -> Value {
    let integer =
        |key: &str| BigInt::from_str(&value[key].to_string()).expect("integer fixture operand");
    let result = match value["op"].as_str().unwrap() {
        "validate" => {
            return match validate(
                value["rows"].as_array().unwrap(),
                value["pairs"].as_array().unwrap(),
            ) {
                Ok(()) => json!({"expected":true}),
                Err((error, message)) => json!({"error":error,"message":message}),
            }
        }
        "ids" => Ok(json!([
            slice_row_id(
                &string(&value["hash"]),
                &integer("start"),
                &integer("end"),
                &string(&value["side"])
            ),
            fee_row_id(
                &string(&value["hash"]),
                &string(&value["side"]),
                &integer("amount")
            ),
            payout_row_id(&string(&value["payout_id"]), &string(&value["side"]))
        ])),
        "slice" => slice_row(
            &value["observation"],
            &value["quantity"],
            &value["rows"],
            &string(&value["side"]),
            &integer("fee"),
        )
        .map_err(Error::legacy_code),
        "payout_valid" => payouts::valid(&value["records"], &value["rows"]),
        "payout_time" => payouts::time(&value["payout"]),
        "payout_settlement" => payouts::settlement(
            LegRef::Json(&value["observation"]),
            &value["payout"],
            &value["rows"],
        )
        .map_err(Error::legacy_code),
        "payout_price" => {
            payouts::price(value["row"].clone(), &value["payout"], &integer("amount"))
        }
        "fees" => fees::evaluate(value),
        "conversions" => conversions::evaluate(value),
        "finalize" | "compose" | "connected" | "basis" => composition::evaluate(value),
        "base" => base_row(&value["observation"], &value["rows"]).map_err(Error::legacy_code),
        "fee" => fee_row(
            &value["observation"],
            &integer("amount"),
            &value["rows"],
            &string(&value["side"]),
        )
        .map_err(Error::legacy_code),
        _ => panic!("unknown private projection operation"),
    };
    match result {
        Ok(expected) => json!({"expected":expected}),
        Err(error) => json!({"error":error}),
    }
}

// Exact request decimals reuse the existing CPython parser, without rounding.
pub(crate) fn request_decimal(value: &Value) -> Result<String, String> {
    match money::Money::parse(value)? {
        money::Money::Finite(value) => Ok(value.to_canonical()),
        money::Money::Special(value) => Ok(value),
    }
}
