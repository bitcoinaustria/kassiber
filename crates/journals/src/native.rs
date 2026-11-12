use crate::scalar::{self, dict, kind, repr, text, truth, BigInt, Key, PyStr, Value};
use crate::{render, Refusal};
use kassiber_pydecimal::{Context, Decimal};
use std::collections::HashMap;

mod builder;
mod graph;
mod ownership;
pub use builder::{build, finish, State};

#[derive(Clone, Debug)]
pub struct Error(pub Value);

impl From<Refusal> for Error {
    fn from(value: Refusal) -> Self {
        Self(kassiber_custody::codec::from_json(&serde_json::to_value(value).unwrap()).unwrap())
    }
}

impl From<kassiber_pydecimal::DecimalError> for Error {
    fn from(value: kassiber_pydecimal::DecimalError) -> Self {
        Refusal::from(value).into()
    }
}

type Result<T> = std::result::Result<T, Error>;

fn error(family: &str, message: impl Into<PyStr>) -> Error {
    let mut value = Error::from(Refusal::new(family, ""));
    set(&mut value.0, "message", Value::Str(message.into()));
    value
}

fn join(parts: &[PyStr]) -> PyStr {
    PyStr::from_code_points(parts.iter().flat_map(PyStr::code_points)).unwrap()
}

fn is(value: &Value, literal: &str) -> bool {
    matches!(value, Value::Str(s) if s.is(literal))
}

pub fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    if let Value::Dict(items) = value {
        if let Some((_, value)) = items
            .iter()
            .find(|(k, _)| matches!(k, Key::Str(s) if s.is(key)))
        {
            return value;
        }
    }
    &Value::Null
}

fn required<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    if let Value::Dict(items) = value {
        items
            .iter()
            .find(|(k, _)| matches!(k, Key::Str(s) if s.is(key)))
            .map(|(_, v)| v)
            .ok_or_else(|| error("KeyError", format!("'{key}'")))
    } else {
        Err(error(
            "TypeError",
            match value {
                Value::Str(_) => "string indices must be integers, not 'str'".into(),
                Value::List(_) => "list indices must be integers or slices, not str".into(),
                _ => format!("'{}' object is not subscriptable", kind(value)),
            },
        ))
    }
}

fn get<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    if matches!(value, Value::Dict(_)) {
        Ok(field(value, key))
    } else {
        Err(error(
            "AttributeError",
            format!("'{}' object has no attribute 'get'", kind(value)),
        ))
    }
}

fn get_default<'a>(value: &'a Value, key: &str, default: &'a Value) -> Result<&'a Value> {
    get(value, key)?;
    let Value::Dict(items) = value else {
        unreachable!()
    };
    Ok(items
        .iter()
        .find(|(k, _)| matches!(k,Key::Str(s) if s.is(key)))
        .map(|(_, v)| v)
        .unwrap_or(default))
}

fn set(value: &mut Value, key: &str, replacement: Value) {
    let Value::Dict(items) = value else {
        unreachable!()
    };
    if let Some((_, value)) = items
        .iter_mut()
        .find(|(k, _)| matches!(k, Key::Str(s) if s.is(key)))
    {
        *value = replacement;
    } else {
        items.push((Key::from(key), replacement));
    }
}

fn iterable(value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::List(items) => Ok(items.clone()),
        Value::Dict(items) => Ok(items.iter().map(|(k, _)| scalar::key_value(k)).collect()),
        Value::Str(s) => Ok(s
            .code_points()
            .map(|p| Value::Str(PyStr::from_code_points([p]).unwrap()))
            .collect()),
        _ => Err(error(
            "TypeError",
            format!("'{}' object is not iterable", kind(value)),
        )),
    }
}

fn hashable(value: &Value) -> Result<()> {
    if matches!(value, Value::List(_) | Value::Dict(_)) {
        Err(error(
            "TypeError",
            format!("unhashable type: '{}'", kind(value)),
        ))
    } else {
        Ok(())
    }
}

fn integer(value: &Value) -> Result<Value> {
    Ok(match value {
        Value::Int(_) => value.clone(),
        Value::Bool(v) => Value::from(i64::from(*v)),
        Value::Float(v) if v.is_nan() => {
            return Err(error("ValueError", "cannot convert float NaN to integer"))
        }
        Value::Float(v) if v.is_infinite() => {
            return Err(error(
                "OverflowError",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Float(v) => Value::Int(format!("{:.0}", v.trunc()).parse().unwrap()),
        Value::Str(s) => {
            let parsed = s.as_str().and_then(|s| kassiber_pytext::parse_int(s).ok());
            if let Some(n) = parsed {
                Value::Int(n)
            } else {
                let message = match s.as_str().map(kassiber_pytext::parse_int) {
                    Some(Err(kassiber_pytext::NumberError::TooManyDigits(n))) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
                    _ => format!("invalid literal for int() with base 10: {}", repr(value)),
                };
                return Err(error("ValueError", message));
            }
        }
        _ => {
            return Err(error(
                "TypeError",
                format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kind(value)
            ),
            ))
        }
    })
}

fn int_zero(value: &Value) -> Result<Value> {
    if truth(value) {
        integer(value)
    } else {
        Ok(Value::from(0))
    }
}

fn fallback<'a>(first: &'a Value, second: &'a Value) -> &'a Value {
    if truth(first) {
        first
    } else {
        second
    }
}

fn decimal(value: &Value) -> Result<Decimal> {
    if let Value::Str(s) = value {
        if let Some(d) = s.as_str().and_then(|s| Decimal::from_canonical(s).ok()) {
            return Ok(d);
        }
    }
    Err(error(
        "ValueError",
        format!("not a canonical engine decimal: {}", repr(value)),
    ))
}

fn dec_field(value: &Value, key: &str) -> Result<Decimal> {
    decimal(required(value, key)?)
}
fn dec(value: Decimal) -> Value {
    Value::Decimal(value)
}
fn fixed(value: &Decimal) -> Result<Value> {
    Ok(Value::from(render::fixed(value)?.into_owned()))
}
fn msat(value: &Decimal) -> Result<Value> {
    Ok(Value::Int(
        render::msat(value)?.to_string().parse().unwrap(),
    ))
}
fn at<'a>(values: &'a [Value], index: &Value) -> Result<&'a Value> {
    let Value::Int(index) = integer(index)? else {
        unreachable!()
    };
    let n = index
        .to_string()
        .parse::<isize>()
        .map_err(|_| error("IndexError", "cannot fit 'int' into an index-sized integer"))?;
    let n = if n < 0 {
        n as i128 + values.len() as i128
    } else {
        n as i128
    };
    usize::try_from(n)
        .ok()
        .and_then(|n| values.get(n))
        .ok_or_else(|| error("IndexError", "list index out of range"))
}

fn float(value: &Value) -> Result<f64> {
    match value {
        Value::Float(v) => Ok(*v),
        Value::Int(v) => v
            .to_string()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| error("OverflowError", "int too large to convert to float")),
        Value::Bool(v) => Ok(f64::from(u8::from(*v))),
        Value::Str(s) => s
            .as_str()
            .and_then(|s| kassiber_pytext::parse_float(s).ok())
            .ok_or_else(|| {
                error(
                    "ValueError",
                    format!("could not convert string to float: {}", repr(value)),
                )
            }),
        _ => Err(error(
            "TypeError",
            format!(
                "float() argument must be a string or a real number, not '{}'",
                kind(value)
            ),
        )),
    }
}

struct Reader<'a> {
    profile: &'a Value,
    rows: HashMap<PyStr, Value>,
    labels: HashMap<PyStr, &'a Value>,
    method: PyStr,
}

impl Reader<'_> {
    fn row(&self, id: &Value) -> Result<&Value> {
        hashable(id)?;
        Ok(match id {
            Value::Str(s) => self.rows.get(s),
            _ => None,
        }
        .unwrap_or(&Value::Null))
    }
    fn wallet(&self, label: &Value, entry: bool) -> Result<Option<&Value>> {
        hashable(label)?;
        let wallet = match label {
            Value::Str(s) => self.labels.get(s).copied(),
            _ => None,
        };
        if entry && wallet.is_none() {
            let mut err = error(
                "AppError",
                join(&[
                    "RP2 emitted transaction for unknown wallet '".into(),
                    text(label),
                    "'".into(),
                ]),
            );
            set(&mut err.0, "code", Value::from("internal"));
            return Err(err);
        }
        Ok(wallet)
    }
    fn head(
        &self,
        wallet: &Value,
        row: &Value,
        id: &Value,
        occurred: Value,
        entry: (&Value, &Value),
        ids: &mut impl FnMut() -> String,
    ) -> Result<Value> {
        let (entry_type, asset) = entry;
        hashable(required(wallet, "id")?)?;
        hashable(entry_type)?;
        hashable(asset)?;
        required(self.profile, "id")?;
        let id_value = ids();
        let txid = match row {
            Value::Dict(items)
                if items
                    .iter()
                    .any(|(k, _)| matches!(k, Key::Str(s) if s.is("journal_transaction_id"))) =>
            {
                field(row, "journal_transaction_id")
            }
            _ => id,
        };
        Ok(dict([
            ("id", Value::from(id_value)),
            (
                "workspace_id",
                required(self.profile, "workspace_id")?.clone(),
            ),
            ("profile_id", required(self.profile, "id")?.clone()),
            ("transaction_id", Value::Str(text(txid))),
            ("wallet_id", required(wallet, "id")?.clone()),
            ("account_id", required(wallet, "wallet_account_id")?.clone()),
            ("occurred_at", occurred),
        ]))
    }
    fn source(&self, asset: &Value, facts: &Value) -> Result<Value> {
        if matches!(facts, Value::Null) {
            return Ok(Value::Null);
        }
        let id = required(facts, "row")?;
        let row = self.row(id)?;
        let crypto = dec_field(facts, "crypto_fee")?;
        let occurred = field(row, "occurred_at");
        hashable(occurred)?;
        let json = scalar::to_json(occurred).ok_or_else(|| error("KeyError", "'iso'"))?;
        let iso = kassiber_custody::ledger_request::timestamp_fields(&json);
        let iso = iso["iso"]
            .as_str()
            .ok_or_else(|| error("KeyError", "'iso'"))?;
        let spot = fixed(&dec_field(facts, "spot_price")?)?;
        let fee = fixed(&dec_field(facts, "fiat_fee")?)?;
        let amount = msat(&crypto)?;
        let pricing = Value::Dict(
            crate::FRAGMENT_PRICING_FIELDS
                .iter()
                .map(|k| (Key::from(*k), field(row, k).clone()))
                .collect(),
        );
        Ok(dict([
            (
                "transaction_id",
                fallback(field(row, "journal_transaction_id"), field(row, "id")).clone(),
            ),
            ("engine_event_id", id.clone()),
            ("occurred_at", Value::from(iso)),
            ("asset", asset.clone()),
            ("spot_price_exact", spot),
            ("fiat_fee_exact", fee),
            ("crypto_fee_msat", amount),
            ("crypto_fee_msat_exact", Value::Str(text(&msat(&crypto)?))),
            ("pricing", pricing),
        ]))
    }
    fn fragment(&self, asset: &Value, value: &Value) -> Result<Value> {
        let amount = dec_field(value, "crypto_amount")?;
        let override_value = required(value, "unit_basis_override")?;
        let override_value = (!matches!(override_value, Value::Null))
            .then(|| decimal(override_value))
            .transpose()?;
        let lot = required(value, "lot")?;
        let acquired = truth(lot)
            .then(|| dec_field(lot, "crypto_in"))
            .transpose()?;
        Ok(dict([
            ("quantity_msat", msat(&amount)?),
            ("quantity_msat_exact", Value::Str(text(&msat(&amount)?))),
            ("cost_basis_exact", fixed(&dec_field(value, "cost_basis")?)?),
            ("proceeds_exact", fixed(&dec_field(value, "proceeds")?)?),
            ("gain_loss_exact", fixed(&dec_field(value, "gain_loss")?)?),
            (
                "unit_basis_override_exact",
                override_value
                    .as_ref()
                    .map(fixed)
                    .transpose()?
                    .unwrap_or(Value::Null),
            ),
            (
                "basis_authority",
                Value::from(if override_value.is_some() {
                    "engine_unit_override"
                } else if truth(lot) {
                    "acquired_lot"
                } else {
                    "income"
                }),
            ),
            (
                "acquisition_basis_exact",
                if truth(lot) {
                    fixed(&dec_field(lot, "basis")?)?
                } else {
                    Value::Null
                },
            ),
            (
                "acquisition_quantity_msat",
                acquired
                    .as_ref()
                    .map(msat)
                    .transpose()?
                    .unwrap_or(Value::Null),
            ),
            (
                "acquisition_quantity_msat_exact",
                acquired
                    .as_ref()
                    .map(|v| Ok::<_, Error>(Value::Str(text(&msat(v)?))))
                    .transpose()?
                    .unwrap_or(Value::Null),
            ),
            ("event", self.source(asset, required(value, "event")?)?),
            ("lot", self.source(asset, lot)?),
        ]))
    }
    fn entry(
        &self,
        value: &Value,
        audits: &[Value],
        raw_audits: &[Value],
        ids: &mut impl FnMut() -> String,
    ) -> Result<Value> {
        let entry_type = required(value, "entry_type")?;
        if is(entry_type, "transfer_out") || is(entry_type, "transfer_in") {
            let index = required(value, "audit")?;
            let audit = at(audits, index)?;
            let outbound = is(entry_type, "transfer_out");
            let label = required(
                audit,
                if outbound {
                    "from_wallet_label"
                } else {
                    "to_wallet_label"
                },
            )?;
            let wallet = self
                .wallet(label, false)?
                .ok_or_else(|| error("KeyError", repr(label)))?;
            let id = required(audit, if outbound { "out_id" } else { "in_id" })?;
            let row = self.row(id)?;
            let amount = float(required(
                audit,
                if outbound {
                    "crypto_sent"
                } else {
                    "crypto_received"
                },
            )?)?;
            let quantity = crate::JournalQuantity::from_audit(amount, outbound)?;
            let quantity = match quantity {
                crate::JournalQuantity::Finite(value) => Value::Decimal(value.0),
                crate::JournalQuantity::NonFinite { text } => dict([(
                    "$decimal",
                    Value::from(match text {
                        crate::NonFiniteDecimal::PositiveInfinity => "Infinity",
                        crate::NonFiniteDecimal::NegativeInfinity => "-Infinity",
                        crate::NonFiniteDecimal::NaN => "NaN",
                    }),
                )]),
            };
            let mut entry = self.head(
                wallet,
                row,
                id,
                required(audit, "occurred_at")?.clone(),
                (entry_type, required(audit, "asset")?),
                ids,
            )?;
            for (k, v) in [
                ("entry_type", entry_type.clone()),
                ("asset", required(audit, "asset")?.clone()),
                ("quantity", quantity),
                ("fiat_value", dec(Decimal::zero())),
                ("unit_cost", dec(Decimal::zero())),
                ("cost_basis", Value::Null),
                ("proceeds", Value::Null),
                ("gain_loss", Value::Null),
                (
                    "description",
                    required(at(raw_audits, index)?, "description")?.clone(),
                ),
            ] {
                set(&mut entry, k, v);
            }
            return Ok(entry);
        }
        let acquisition = is(entry_type, "acquisition");
        let id = required(value, "row")?;
        let row = self.row(id)?;
        let wallet = self.wallet(required(value, "wallet")?, true)?.unwrap();
        let fiat = dec_field(
            value,
            if acquisition {
                "fiat_value"
            } else {
                "proceeds"
            },
        )?;
        let occurred = required(row, "occurred_at")?.clone();
        let asset = required(value, "asset")?;
        let mut entry = self.head(wallet, row, id, occurred, (entry_type, asset), ids)?;
        for (k, v) in [
            ("entry_type", entry_type.clone()),
            ("asset", asset.clone()),
            ("quantity", dec(dec_field(value, "quantity")?)),
            ("fiat_value", dec(fiat.clone())),
            (
                "unit_cost",
                dec(if acquisition {
                    dec_field(value, "unit_cost")?
                } else {
                    Decimal::zero()
                }),
            ),
            (
                "cost_basis",
                if acquisition {
                    Value::Null
                } else {
                    dec(dec_field(value, "cost_basis")?)
                },
            ),
            (
                "proceeds",
                if acquisition { Value::Null } else { dec(fiat) },
            ),
            (
                "gain_loss",
                if acquisition {
                    Value::Null
                } else {
                    dec(dec_field(value, "gain_loss")?)
                },
            ),
            ("description", required(value, "description")?.clone()),
        ] {
            set(&mut entry, k, v);
        }
        if !acquisition {
            let category = required(value, "at_category")?;
            if !matches!(category, Value::Null) {
                set(&mut entry, "at_category", category.clone());
                let category = Value::Str(text(category));
                let code =
                    if is(&category, "income_general") || is(&category, "income_capital_yield") {
                        Some(172)
                    } else if is(&category, "neu_gain") {
                        Some(174)
                    } else if is(&category, "neu_loss") {
                        Some(176)
                    } else if is(&category, "alt_spekulation") {
                        Some(801)
                    } else {
                        None
                    };
                set(
                    &mut entry,
                    "at_kennzahl",
                    code.map(Value::from).unwrap_or(Value::Null),
                );
            }
            set(
                &mut entry,
                "capital_gains_type",
                required(value, "capital_gains_type")?.clone(),
            );
            let fragments = iterable(required(value, "fragments")?)?
                .iter()
                .map(|v| self.fragment(asset, v))
                .collect::<Result<Vec<_>>>()?;
            set(
                &mut entry,
                "calculation",
                dict([
                    ("schema_version", Value::from(1)),
                    ("engine", Value::from("kassiber_tax")),
                    ("method", Value::Str(self.method.clone())),
                    ("fragments", Value::List(fragments)),
                ]),
            );
        }
        Ok(entry)
    }
}

pub struct Ledger {
    pub entries: Vec<Value>,
    pub quarantines: Vec<Value>,
    pub intra_audit: Vec<Value>,
    pub account_holdings: Value,
    pub wallet_holdings: Value,
    pub direct_swap_payouts: Value,
    pub tax_summary: Vec<Value>,
}

impl Ledger {
    pub fn snapshot(&self, projection: &Value) -> Value {
        scalar::snapshot(&dict([
            ("entries", Value::List(self.entries.clone())),
            ("quarantines", Value::List(self.quarantines.clone())),
            ("intra_audit", Value::List(self.intra_audit.clone())),
            (
                "cross_asset_pairs",
                field(projection, "cross_asset_pairs").clone(),
            ),
            ("direct_swap_payouts", self.direct_swap_payouts.clone()),
            ("tax_summary", Value::List(self.tax_summary.clone())),
            ("account_holdings", self.account_holdings.clone()),
            ("wallet_holdings", self.wallet_holdings.clone()),
        ]))
    }
}

pub fn read_ledger(
    profile: &Value,
    projection: &Value,
    wallet_refs: &Value,
    payouts: &[Value],
    response: &Value,
    mut ids: impl FnMut() -> String,
) -> Result<Ledger> {
    if matches!(response, Value::Null) {
        return Ok(Ledger {
            entries: vec![],
            quarantines: iterable(required(projection, "quarantines")?)?,
            intra_audit: vec![],
            account_holdings: dict([]),
            wallet_holdings: dict([]),
            direct_swap_payouts: direct_payouts(projection, payouts)?,
            tax_summary: vec![],
        });
    }
    if !matches!(get(response, "ok")?, Value::Bool(true)) {
        return Err(ledger_error(fallback(get(response, "error")?, &dict([])))?);
    }
    let mut rows = HashMap::new();
    for row in iterable(required(projection, "rows")?)? {
        rows.insert(text(required(&row, "id")?), row);
    }
    for carry in iterable(required(response, "carried_rows")?)? {
        let id = text(required(&carry, "row")?);
        let row = rows
            .get_mut(&id)
            .ok_or_else(|| error("KeyError", kassiber_pytext::repr_py(&id)))?;
        let value = required(&carry, "value")?;
        let rate = get(&carry, "rate")?;
        let rate_float = if matches!(rate, Value::Null) {
            Value::Null
        } else {
            Value::Float(float(rate)?)
        };
        for (k, v) in [
            ("fiat_rate", rate_float),
            ("fiat_value", value.clone()),
            ("fiat_rate_exact", rate.clone()),
            ("fiat_value_exact", value.clone()),
            ("fiat_price_source", Value::from("manual")),
            ("pricing_source_kind", Value::from("manual_override")),
            ("pricing_quality", Value::from("exact")),
            ("pricing_provider", Value::Null),
            ("pricing_pair", Value::Null),
            ("pricing_granularity", Value::Null),
            ("pricing_method", Value::from("carrying_value")),
        ] {
            set(row, k, v);
        }
    }
    let Value::Dict(wallet_refs) = wallet_refs else {
        return Err(error(
            "AttributeError",
            format!("'{}' object has no attribute 'values'", kind(wallet_refs)),
        ));
    };
    let mut labels = HashMap::new();
    for (_, wallet) in wallet_refs {
        let label = get(wallet, "label")?;
        if !matches!(label, Value::Null) && !is(label, "") {
            labels.insert(text(label), wallet);
        }
    }
    let raw_audits = iterable(required(response, "intra_audit")?)?;
    let audits = raw_audits
        .iter()
        .map(|a| audit(projection, wallet_refs, a))
        .collect::<Result<Vec<_>>>()?;
    let method = field(profile, "gains_algorithm");
    let method = if matches!(method, Value::Null) {
        PyStr::new()
    } else {
        kassiber_pytext::strip_py(&text(method))
    };
    let reader = Reader {
        profile,
        rows,
        labels,
        method,
    };
    let entries = iterable(required(response, "entries")?)?
        .iter()
        .map(|v| reader.entry(v, &audits, &raw_audits, &mut ids))
        .collect::<Result<Vec<_>>>()?;
    let (account_holdings, wallet_holdings) = holdings(required(response, "holdings")?, &reader)?;
    let mut quarantines = iterable(required(projection, "quarantines")?)?;
    for q in iterable(required(response, "quarantines")?)? {
        quarantines.push(quarantine(profile, &reader.rows, &q)?);
    }
    let direct_swap_payouts = direct_payouts(projection, payouts)?;
    let tax_summary = iterable(required(response, "tax_summary")?)?
        .iter()
        .map(summary)
        .collect::<Result<Vec<_>>>()?;
    Ok(Ledger {
        entries,
        quarantines,
        intra_audit: audits,
        account_holdings,
        wallet_holdings,
        direct_swap_payouts,
        tax_summary,
    })
}

fn audit(projection: &Value, wallets: &[(Key, Value)], value: &Value) -> Result<Value> {
    let pairs = iterable(required(projection, "intra_pairs")?)?;
    let pair = at(&pairs, required(value, "pair")?).map_err(|mut err| {
        if is(field(&err.0, "family"), "IndexError")
            && is(field(&err.0, "message"), "list index out of range")
        {
            set(
                &mut err.0,
                "message",
                Value::from("tuple index out of range"),
            );
        }
        err
    })?;
    let out = required(pair, "out")?;
    let inbound = required(pair, "in")?;
    let wallet = |leg: &Value| -> Result<&Value> {
        let id = required(leg, "wallet_id")?;
        hashable(id)?;
        wallets
            .iter()
            .find(|(key, _)| key_equal(&scalar::key_value(key), id))
            .map(|(_, wallet)| wallet)
            .ok_or_else(|| error("KeyError", repr(id)))
    };
    let from = wallet(out)?;
    let to = wallet(inbound)?;
    let spot = required(value, "spot_price")?;
    let mut audit = dict([
        ("out_id", required(out, "id")?.clone()),
        ("in_id", required(inbound, "id")?.clone()),
        (
            "out_anchor_transaction_id",
            Value::Str(text(fallback(
                field(out, "journal_transaction_id"),
                required(out, "id")?,
            ))),
        ),
        (
            "in_anchor_transaction_id",
            Value::Str(text(fallback(
                field(inbound, "journal_transaction_id"),
                required(inbound, "id")?,
            ))),
        ),
        ("from_wallet_id", required(from, "id")?.clone()),
        ("from_wallet_label", required(from, "label")?.clone()),
        ("to_wallet_id", required(to, "id")?.clone()),
        ("to_wallet_label", required(to, "label")?.clone()),
        ("asset", Value::Str(text(field(out, "asset")))),
        ("occurred_at", required(out, "occurred_at")?.clone()),
        ("external_id", required(out, "external_id")?.clone()),
        (
            "crypto_sent",
            Value::Float(render::float(&dec_field(value, "sent")?)),
        ),
        (
            "crypto_received",
            Value::Float(render::float(&dec_field(value, "received")?)),
        ),
        (
            "crypto_fee",
            Value::Float(render::float(&dec_field(value, "fee")?)),
        ),
        ("crypto_sent_msat", integer(required(value, "sent_msat")?)?),
        (
            "crypto_received_msat",
            integer(required(value, "received_msat")?)?,
        ),
        ("crypto_fee_msat", integer(required(value, "fee_msat")?)?),
        (
            "spot_price",
            Value::Float(if matches!(spot, Value::Null) {
                0.0
            } else {
                render::float(&decimal(spot)?)
            }),
        ),
        ("pairing_source", get(pair, "source")?.clone()),
    ]);
    let regime = required(value, "at_regime")?;
    if truth(regime) {
        set(&mut audit, "at_regime", regime.clone());
    }
    let flows = required(value, "regime_flows")?;
    if truth(flows) {
        let mut rendered = dict([]);
        for k in ["out", "in"] {
            set(&mut rendered, k, mapping(required(flows, k)?)?);
        }
        set(&mut audit, "regime_flows", rendered);
    }
    let pool = required(value, "at_pool")?;
    if truth(pool) {
        set(&mut audit, "at_pool", pool.clone());
    }
    let group = field(pair, "group_id");
    if truth(group) {
        set(&mut audit, "transfer_group_id", Value::Str(text(group)));
    }
    Ok(audit)
}

fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Bool(a), Value::Int(b)) | (Value::Int(b), Value::Bool(a)) => {
            BigInt::from(u8::from(*a)) == *b
        }
        (Value::Float(a), Value::Int(b)) | (Value::Int(b), Value::Float(a)) => {
            a.is_finite()
                && a.fract() == 0.0
                && format!("{a:.0}").parse::<BigInt>().is_ok_and(|a| a == *b)
        }
        (Value::Bool(a), Value::Float(b)) | (Value::Float(b), Value::Bool(a)) => {
            f64::from(u8::from(*a)) == *b
        }
        _ => a == b,
    }
}

fn key_equal(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Float(a), Value::Float(b)) if a.is_nan() && b.is_nan()) || equal(a, b)
}

fn holdings(values: &Value, reader: &Reader<'_>) -> Result<(Value, Value)> {
    type Pairs = Vec<(Vec<Value>, (Decimal, Decimal))>;
    fn add(pairs: &mut Pairs, key: Vec<Value>, quantity: &Decimal, basis: &Decimal) -> Result<()> {
        for v in &key {
            hashable(v)?;
        }
        let index = pairs
            .iter()
            .position(|(k, _)| k.iter().zip(&key).all(|(a, b)| key_equal(a, b)))
            .unwrap_or(pairs.len());
        if index == pairs.len() {
            pairs.push((key, (Decimal::zero(), Decimal::zero())));
        }
        let (_, (q, b)) = &mut pairs[index];
        *q = q.add(quantity, &Context::RP2)?;
        *b = b.add(basis, &Context::RP2)?;
        Ok(())
    }
    fn encode(pairs: Pairs) -> Value {
        if pairs.is_empty() {
            return dict([]);
        }
        dict([(
            "$pairs",
            Value::List(
                pairs
                    .into_iter()
                    .map(|(key, (q, b))| {
                        Value::List(vec![
                            Value::List(key),
                            dict([("quantity", dec(q)), ("cost_basis", dec(b))]),
                        ])
                    })
                    .collect(),
            ),
        )])
    }
    let mut assets = Vec::<(PyStr, Vec<Value>)>::new();
    for value in iterable(values)? {
        let asset = text(required(&value, "asset")?);
        if let Some((_, items)) = assets.iter_mut().find(|(k, _)| *k == asset) {
            items.push(value);
        } else {
            assets.push((asset, vec![value]));
        }
    }
    let mut accounts = vec![];
    let mut wallets = vec![];
    for (asset, values) in assets {
        let mut asset_accounts = vec![];
        let mut asset_wallets = vec![];
        for value in values {
            let Some(wallet) = reader.wallet(required(&value, "wallet")?, false)? else {
                continue;
            };
            let q = dec_field(&value, "quantity")?;
            let b = dec_field(&value, "cost_basis")?;
            let account = vec![
                required(wallet, "wallet_account_id")?.clone(),
                required(wallet, "account_code")?.clone(),
                required(wallet, "account_label")?.clone(),
                Value::Str(asset.clone()),
            ];
            let wallet = vec![
                required(wallet, "id")?.clone(),
                required(wallet, "label")?.clone(),
                required(wallet, "account_code")?.clone(),
                Value::Str(asset.clone()),
            ];
            add(&mut asset_accounts, account, &q, &b)?;
            add(&mut asset_wallets, wallet, &q, &b)?;
        }
        for (key, (q, b)) in asset_accounts {
            add(&mut accounts, key, &q, &b)?;
        }
        for (key, (q, b)) in asset_wallets {
            add(&mut wallets, key, &q, &b)?;
        }
    }
    Ok((encode(accounts), encode(wallets)))
}

fn detail(value: &Value) -> Result<Value> {
    Ok(match value {
        Value::Dict(items)
            if items.len() == 1 && matches!(&items[0].0, Key::Str(s) if s.is("float")) =>
        {
            Value::Float(render::float(&decimal(&items[0].1)?))
        }
        Value::Dict(items) => Value::Dict(
            items
                .iter()
                .map(|(k, v)| Ok((k.clone(), detail(v)?)))
                .collect::<Result<_>>()?,
        ),
        Value::List(items) => Value::List(items.iter().map(detail).collect::<Result<_>>()?),
        _ => value.clone(),
    })
}

/// `build_tax_policy(profile).fiat_currency`: the country's currency, else
/// the profile's, stripped and upper-cased.
fn book_currency(profile: &Value) -> String {
    let raw = field(profile, "tax_country");
    let country = if truth(raw) {
        kassiber_pytext::lower(kassiber_pytext::strip(
            text(raw).as_str().unwrap_or_default(),
        ))
    } else {
        String::new()
    };
    let country = if country.is_empty() {
        kassiber_tax_countries::DEFAULT_CODE
    } else {
        &country
    };
    match kassiber_tax_countries::lookup(country).map(|entry| entry.fiat) {
        Some(kassiber_tax_countries::Fiat::Fixed(code)) => code.to_owned(),
        _ => kassiber_pytext::upper(kassiber_pytext::strip(
            text(field(profile, "fiat_currency"))
                .as_str()
                .unwrap_or_default(),
        )),
    }
}

fn quarantine(profile: &Value, rows: &HashMap<PyStr, Value>, value: &Value) -> Result<Value> {
    let id = text(required(value, "row")?);
    let row = rows
        .get(&id)
        .ok_or_else(|| error("KeyError", kassiber_pytext::repr_py(&id)))?;
    let template = required(value, "detail_kind")?;
    if ![
        "plain",
        "partner_leg",
        "pricing_review",
        "privacy_hop",
        "hold",
        "price_currency",
    ]
    .iter()
    .any(|s| text(template).is(s))
    {
        let mut e = error(
            "EngineContractError",
            format!(
                "RequestError: unknown quarantine detail kind {}",
                repr(template)
            ),
        );
        set(&mut e.0, "error_class", Value::from("RequestError"));
        return Err(e);
    }
    let raw = required(value, "detail")?;
    if !matches!(raw, Value::Dict(_)) {
        return Err(error(
            "AttributeError",
            format!("'{}' object has no attribute 'items'", kind(raw)),
        ));
    }
    let mut detail = detail(raw)?;
    if text(template).is("pricing_review") {
        for key in [
            "pricing_quality",
            "pricing_source_kind",
            "pricing_provider",
            "pricing_pair",
            "pricing_timestamp",
            "pricing_granularity",
            "pricing_method",
        ] {
            set(&mut detail, key, field(row, key).clone());
        }
    } else if text(template).is("hold") {
        set(
            &mut detail,
            "external_id",
            Value::Str(text(fallback(field(row, "external_id"), &Value::from("")))),
        );
    } else if text(template).is("price_currency") {
        set(
            &mut detail,
            "external_id",
            Value::Str(text(fallback(field(row, "external_id"), &Value::from("")))),
        );
        let currency = field(row, "fiat_currency");
        let currency = if truth(currency) {
            text(currency).as_str().unwrap_or_default().to_owned()
        } else {
            String::new()
        };
        set(
            &mut detail,
            "price_currency",
            Value::from(currency.trim_matches(' ').to_ascii_uppercase()),
        );
        set(
            &mut detail,
            "book_currency",
            Value::from(book_currency(profile)),
        );
    } else if text(template).is("privacy_hop") {
        let boundary = kassiber_pytext::strip_py(&text(field(row, "privacy_boundary")));
        let boundary = kassiber_pytext::lower(boundary.as_str().unwrap_or("")).replace('-', "_");
        if [
            "coinjoin",
            "collaborative",
            "payjoin",
            "payment_in_coinjoin",
            "sweep",
        ]
        .contains(&boundary.as_str())
        {
            let raw = field(row, "raw_json");
            let raw = match raw {
                Value::Dict(_) => raw.clone(),
                Value::Str(s) => s
                    .as_str()
                    .and_then(|s| scalar::decode(s).ok())
                    .unwrap_or(Value::Null),
                _ => Value::Null,
            };
            let likely = [
                "islikelycoinjoin",
                "is_likely_coinjoin",
                "isLikelyCoinJoin",
                "likely_coinjoin",
                "likelyCoinJoin",
            ]
            .iter()
            .any(|k| {
                let v = field(&raw, k);
                matches!(v, Value::Bool(true))
                    || kassiber_pytext::strip_py(&text(v))
                        .as_str()
                        .is_some_and(|s| {
                            ["1", "true", "yes", "on"].contains(&kassiber_pytext::lower(s).as_str())
                        })
            });
            for (k, v) in [
                ("privacy_boundary", Value::from(boundary.clone())),
                ("privacy_hop", Value::from(boundary)),
                (
                    "source",
                    fallback(field(&raw, "source"), &Value::from("")).clone(),
                ),
                ("islikelycoinjoin", Value::Bool(likely)),
                (
                    "required_for",
                    Value::from("explicit_user_owned_provenance"),
                ),
            ] {
                set(&mut detail, k, v);
            }
        }
    }
    let reason = text(required(value, "reason")?);
    let workspace = required(profile, "workspace_id")?.clone();
    let profile = required(profile, "id")?.clone();
    let id = required(row, "id")?;
    let detail_json =
        kassiber_pyjson::dumps(&detail, &kassiber_pyjson::Options::new().sort_keys(true))
            .map_err(|e| error("ValueError", e.to_string()))?;
    Ok(dict([
        ("workspace_id", workspace),
        ("profile_id", profile),
        (
            "transaction_id",
            fallback(field(row, "journal_transaction_id"), id).clone(),
        ),
        ("reason", Value::Str(reason)),
        ("detail_json", Value::from(detail_json)),
    ]))
}

fn direct_payouts(projection: &Value, values: &[Value]) -> Result<Value> {
    let mut result = vec![];
    for value in values {
        let id = field(value, "id");
        let out = field(value, "out_transaction_id");
        if !truth(id) || !truth(out) {
            continue;
        }
        let amount = fallback(field(value, "out_amount"), field(value, "out_amount_msat"));
        let mut candidates = vec![];
        for row in iterable(required(projection, "rows")?)? {
            if !truth(field(&row, "journal_transaction_id"))
                || text(field(&row, "journal_transaction_id")) != text(out)
                || !is(field(&row, "direction"), "outbound")
            {
                continue;
            }
            let asset = field(value, "out_asset");
            if truth(asset) && text(fallback(field(&row, "asset"), &Value::from(""))) != text(asset)
            {
                continue;
            }
            if !matches!(amount, Value::Null)
                && !is(amount, "")
                && int_zero(field(&row, "amount"))? != integer(amount)?
            {
                continue;
            }
            candidates.push(row);
        }
        let source = if candidates.len() == 1 {
            &candidates[0]
        } else {
            &Value::Null
        };
        let mut payout = dict([
            ("payout_id", Value::Str(text(id))),
            ("kind", field(value, "kind").clone()),
            ("policy", field(value, "policy").clone()),
            ("out_id", Value::Str(text(out))),
            (
                "out_asset",
                fallback(field(value, "out_asset"), field(source, "asset")).clone(),
            ),
            (
                "out_amount_msat",
                int_zero(fallback(amount, field(source, "amount")))?,
            ),
            ("payout_asset", field(value, "payout_asset").clone()),
            (
                "payout_amount_msat",
                int_zero(field(value, "payout_amount"))?,
            ),
        ]);
        for k in ["payout_occurred_at", "payout_external_id", "counterparty"] {
            set(&mut payout, k, field(value, k).clone());
        }
        set(
            &mut payout,
            "swap_fee_msat",
            int_zero(field(value, "swap_fee_msat"))?,
        );
        set(
            &mut payout,
            "swap_fee_kind",
            field(value, "swap_fee_kind").clone(),
        );
        result.push(payout);
    }
    Ok(Value::List(result))
}

fn summary(value: &Value) -> Result<Value> {
    let quantity = dec_field(value, "quantity")?;
    Ok(dict([
        ("year", integer(required(value, "year")?)?),
        ("asset", required(value, "asset")?.clone()),
        (
            "transaction_type",
            required(value, "transaction_type")?.clone(),
        ),
        (
            "capital_gains_type",
            Value::from(if truth(required(value, "long_term")?) {
                "long"
            } else {
                "short"
            }),
        ),
        ("quantity", Value::Float(render::float(&quantity))),
        ("quantity_msat", msat(&quantity)?),
        (
            "proceeds",
            Value::Float(render::float(&dec_field(value, "proceeds")?)),
        ),
        (
            "cost_basis",
            Value::Float(render::float(&dec_field(value, "cost_basis")?)),
        ),
        (
            "gain_loss",
            Value::Float(render::float(&dec_field(value, "gain_loss")?)),
        ),
    ]))
}

fn ledger_error(value: &Value) -> Result<Error> {
    let stage = text(get(value, "stage")?);
    let inner = get(value, "inner")?;
    let empty = dict([]);
    let inner = fallback(inner, &empty);
    let stage_str = stage.as_str().unwrap_or("");
    if ![
        "request",
        "unsupported",
        "prepare",
        "classify",
        "validate",
        "compute_multi",
        "compute",
    ]
    .contains(&stage_str)
    {
        let mut e = error(
            "EngineContractError",
            format!(
                "RequestError: unknown ledger failure stage {}",
                kassiber_pytext::repr_py(&stage)
            ),
        );
        set(&mut e.0, "error_class", Value::from("RequestError"));
        return Ok(e);
    }
    let class = text(get(inner, "class")?);
    let message = text(get(inner, "message")?);
    let family = if ["request", "unsupported"].contains(&stage_str) {
        "EngineContractError"
    } else {
        match class.as_str().unwrap_or("") {
            "ValueError" => "EngineValueError",
            "TypeError" => "EngineTypeError",
            "RuntimeError" => "EngineRuntimeError",
            "BuiltinValueError" => "ValueError",
            "AppError" => "AppError",
            "InvalidOperation" => "InvalidOperation",
            "DivisionByZero" => "DivisionByZero",
            "Overflow" => "Overflow",
            _ => "EngineContractError",
        }
    };
    let mut cause = error(
        family,
        if family == "EngineContractError" {
            join(&[class.clone(), ": ".into(), message])
        } else {
            message
        },
    );
    if family == "EngineContractError" {
        set(&mut cause.0, "error_class", Value::Str(class));
    }
    if family == "AppError" {
        set(&mut cause.0, "code", Value::from("app_error"));
    }
    if ["validate", "compute_multi", "compute"].contains(&stage_str) {
        let prefix = match stage_str {
            "validate" => PyStr::from("RP2 cross-asset input validation failed: "),
            "compute_multi" => PyStr::from("RP2 multi-asset tax calculation failed: "),
            _ => join(&[
                "RP2 tax calculation failed for asset '".into(),
                text(get(value, "asset")?),
                "': ".into(),
            ]),
        };
        let mut wrapped = error(
            "AppError",
            join(&[prefix, text(field(&cause.0, "message"))]),
        );
        set(
            &mut wrapped.0,
            "code",
            Value::from(if stage_str == "validate" {
                "rp2_input_validation"
            } else {
                "app_error"
            }),
        );
        set(&mut wrapped.0, "cause", cause.0);
        return Ok(wrapped);
    }
    Ok(cause)
}

fn mapping(value: &Value) -> Result<Value> {
    if matches!(value, Value::Dict(_)) {
        return Ok(value.clone());
    }
    let mut result: Vec<(Key, Value)> = vec![];
    for (index, item) in iterable(value)?.iter().enumerate() {
        let pair = iterable(item).map_err(|_| {
            error(
                "TypeError",
                format!("cannot convert dictionary update sequence element #{index} to a sequence"),
            )
        })?;
        if pair.len() != 2 {
            return Err(error(
                "ValueError",
                format!(
                    "dictionary update sequence element #{index} has length {}; 2 is required",
                    pair.len()
                ),
            ));
        }
        hashable(&pair[0])?;
        let key = match &pair[0] {
            Value::Null => Key::Null,
            Value::Bool(v) => Key::Bool(*v),
            Value::Int(v) => Key::Int(v.clone()),
            Value::Float(v) => Key::Float(*v),
            Value::Str(v) => Key::Str(v.clone()),
            _ => unreachable!(),
        };
        if let Some((_, value)) = result
            .iter_mut()
            .find(|(k, _)| key_equal(&scalar::key_value(k), &pair[0]))
        {
            *value = pair[1].clone();
        } else {
            result.push((key, pair[1].clone()));
        }
    }
    Ok(Value::Dict(result))
}

#[cfg(test)]
mod price_currency_tests {
    use super::*;

    #[test]
    fn a_price_currency_hold_reads_the_rows_currency_and_the_books() {
        let row = dict([
            ("id", Value::from("custody-tax:a")),
            ("journal_transaction_id", Value::from("a")),
            ("external_id", Value::from("buy-1")),
            ("fiat_currency", Value::from(" eur ")),
        ]);
        let rows = HashMap::from([(text(&Value::from("custody-tax:a")), row)]);
        let held = dict([
            ("row", Value::from("custody-tax:a")),
            ("reason", Value::from("price_currency_mismatch")),
            ("detail_kind", Value::from("price_currency")),
            (
                "detail",
                dict([("required_for", Value::from("book_currency_price"))]),
            ),
        ]);
        for (country, fiat, book) in [
            ("generic", "usd", "USD"),
            ("at", "USD", "EUR"),
            ("gb", "EUR", "GBP"),
        ] {
            let profile = dict([
                ("id", Value::from("p")),
                ("workspace_id", Value::from("w")),
                ("tax_country", Value::from(country)),
                ("fiat_currency", Value::from(fiat)),
            ]);
            let rendered = quarantine(&profile, &rows, &held).unwrap();
            assert_eq!(
                text(field(&rendered, "detail_json")).as_str(),
                Some(
                    format!(
                        r#"{{"book_currency": "{book}", "external_id": "buy-1", "price_currency": "EUR", "required_for": "book_currency_price"}}"#
                    )
                    .as_str()
                ),
                "{country}"
            );
        }
    }
}
