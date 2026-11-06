use crate::*;
use kassiber_custody::finalized_projection::RowView;
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{Key, Value as Py};
use kassiber_tax_core::{
    ledger::response::*,
    manifest::{DetailKind, EntryType},
};
use serde_json::{json, Value};
use std::{borrow::Cow, collections::HashMap};
const CTX: Context = Context::RP2;
mod response;
pub(crate) use response::response_refusal;
const REVIEW: [&str; 7] = [
    "pricing_quality",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_granularity",
    "pricing_method",
];
fn jd(d: &kassiber_tax_core::model::Dec) -> JournalDecimal {
    JournalDecimal(d.0.clone())
}
fn zero() -> JournalDecimal {
    JournalDecimal(Decimal::zero())
}
pub(crate) fn fixed(d: &Decimal) -> Result<Cow<'static, str>, Refusal> {
    if d.is_zero() && d.exponent() >= 0 {
        return Ok(Cow::Borrowed(if d.is_sign_negative() { "-0" } else { "0" }));
    }
    if let Some(coefficient) = d
        .coefficient_u128()
        .filter(|_| (-64..=64).contains(&d.exponent()))
    {
        use std::fmt::Write;
        let exponent = d.exponent();
        let digits = d.adjusted() - exponent + 1;
        let integer_digits = digits + exponent;
        let padding = if exponent >= 0 {
            exponent
        } else {
            (-integer_digits).max(0)
        };
        let mut text = String::with_capacity((digits + padding + 3) as usize);
        if d.is_sign_negative() {
            text.push('-');
        }
        if exponent < 0 && integer_digits <= 0 {
            text.push_str("0.");
            text.extend(std::iter::repeat_n('0', padding as usize));
        }
        write!(&mut text, "{coefficient}").expect("String formatting");
        if exponent >= 0 {
            text.extend(std::iter::repeat_n('0', padding as usize));
        } else if integer_digits > 0 {
            text.insert(
                usize::from(d.is_sign_negative()) + integer_digits as usize,
                '.',
            );
        }
        return Ok(Cow::Owned(text));
    }
    Ok(Cow::Owned(d.to_fixed_string()?))
}
pub(crate) fn float(d: &Decimal) -> f64 {
    d.to_string().parse().expect("finite Decimal text")
}
pub(crate) fn msat(d: &Decimal) -> Result<Value, Refusal> {
    Ok(fragment_msat(d)?.into_value())
}
fn fragment_msat(d: &Decimal) -> Result<JournalMsat, Refusal> {
    if d.is_zero() {
        return Ok(JournalMsat::Small(0));
    }
    let shift = d.exponent() + 11;
    // Common BTC amounts fit machine integers. The <=20-digit coefficient
    // cannot round in Python's 32-digit power-of-ten multiply. Keep exotic
    // exponents and wide results on the arbitrary-precision path below.
    if let Some(n) = d.coefficient_u64().filter(|_| (-19..=19).contains(&shift)) {
        let n = u128::from(n);
        let magnitude = if shift >= 0 {
            n * 10u128.pow(shift as u32)
        } else {
            let divisor = 10u128.pow((-shift) as u32);
            n / divisor + u128::from(n % divisor >= divisor / 2)
        };
        if let Ok(n) = i64::try_from(magnitude) {
            return Ok(JournalMsat::Small(if d.is_sign_negative() {
                -n
            } else {
                n
            }));
        }
    }
    let digits = d.coefficient().to_str_radix(10);
    // A power-of-ten multiply cannot change a <=32-digit coefficient's value.
    // Avoid Decimal allocation for ordinary exact amounts; wide/exotic values
    // still use the context-rounded Python operation below.
    if digits.len() <= 32 && (-100..=100).contains(&shift) {
        let mut magnitude = if shift >= 0 {
            let mut text = digits;
            text.extend(std::iter::repeat_n('0', shift as usize));
            text
        } else {
            let keep = digits.len() as i64 + shift;
            if keep < 0 {
                "0".into()
            } else {
                let mut text = if keep == 0 {
                    "0".into()
                } else {
                    digits[..keep as usize].to_owned()
                };
                if digits.as_bytes()[keep as usize] >= b'5' {
                    let mut n: kassiber_pyjson::BigInt = text.parse().expect("digits");
                    n += 1;
                    text = n.to_string();
                }
                text
            }
        };
        if d.is_sign_negative() && magnitude != "0" {
            magnitude.insert(0, '-');
        }
        return Ok(JournalMsat::Wide(Box::new(number(&magnitude)?)));
    }
    let n = d.mul(&Decimal::from_canonical("100000000000E0")?, &CTX)?;
    Ok(JournalMsat::Wide(Box::new(number(
        &n.to_fixed_places(0, Rounding::HalfUp)?,
    )?)))
}
fn required<'a>(row: &'a Value, key: &str) -> Result<&'a Value, Refusal> {
    if !row.is_object() {
        let message = match row {
            Value::String(_) => "string indices must be integers, not 'str'".into(),
            Value::Array(_) => "list indices must be integers or slices, not str".into(),
            _ => format!(
                "'{}' object is not subscriptable",
                match row {
                    Value::Null => "NoneType",
                    Value::Bool(_) => "bool",
                    Value::Number(n) if n.to_string().contains(['.', 'e', 'E']) => "float",
                    _ => "int",
                }
            ),
        };
        return Err(Refusal::new("TypeError", message));
    }
    row.as_object()
        .and_then(|fields| fields.get(key))
        .ok_or_else(|| Refusal::new("KeyError", format!("'{key}'")))
}
fn row_key(value: &Value) -> Cow<'_, str> {
    value
        .as_str()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(string(value)))
}
fn lookup<'a>(map: &HashMap<Cow<'_, str>, &'a Value>, id: &str) -> Result<&'a Value, Refusal> {
    map.get(id)
        .copied()
        .ok_or_else(|| Refusal::new("KeyError", format!("'{id}'")))
}
fn pricing_fields(row: Row<'_>) -> [&Value; 12] {
    std::array::from_fn(|i| row.at(FRAGMENT_PRICING_FIELDS[i]))
}
/// A projection row: its JSON object, or a view whose fields are that object's.
#[derive(Clone, Copy)]
enum Row<'r> {
    Json(&'r Value),
    View(&'r RowView<'r>),
}
const NULL_ROW: Row<'static> = Row::Json(&Value::Null);
impl<'r> Row<'r> {
    /// The field `key`; none when the row is not an object or lacks it.
    fn get(self, key: &str) -> Option<&'r Value> {
        match self {
            Self::Json(row) => row.as_object().and_then(|fields| fields.get(key)),
            Self::View(row) => row.get(key),
        }
    }
    /// The field `key`, or null.
    fn at(self, key: &str) -> &'r Value {
        self.get(key).unwrap_or(&Value::Null)
    }
    /// `row[key]` as Python reads it, refusing as `required` does.
    fn required(self, key: &str) -> Result<&'r Value, Refusal> {
        match self {
            Self::Json(row) => required(row, key),
            Self::View(row) => row
                .get(key)
                .ok_or_else(|| Refusal::new("KeyError", format!("'{key}'"))),
        }
    }
    /// The row's JSON object.
    fn json(self) -> Value {
        match self {
            Self::Json(row) => row.clone(),
            Self::View(row) => row.json(),
        }
    }
    /// Whether both name the same row.
    fn same(self, other: Row<'_>) -> bool {
        match (self, other) {
            (Self::Json(a), Row::Json(b)) => std::ptr::eq(a, b),
            (Self::View(a), Row::View(b)) => std::ptr::eq(a, b),
            _ => false,
        }
    }
}
impl std::ops::Index<&str> for Row<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        self.at(key)
    }
}
pub struct LedgerState<'i> {
    pub entries: Vec<JournalEntryRow<'i>>,
    pub quarantines: Vec<QuarantineRow>,
    pub intra_audit: Vec<TransferAudit>,
    pub tax_summary: Vec<TaxSummaryRow>,
    pub account_holdings: Holdings,
    pub wallet_holdings: Holdings,
    pub direct_swap_payouts: Value,
}
/// Operands shared with the future typed finalized stage. Raw rows use transitional JSON.
pub struct LedgerInputs<'a> {
    pub profile: &'a Value,
    pub projection: &'a Value,
    /// The projection's rows as views, read in place of `projection["rows"]`.
    pub rows: Option<&'a [RowView<'a>]>,
    pub wallets: &'a [Value],
    pub direct_payout_records: &'a [Value],
}
impl<'a> LedgerInputs<'a> {
    fn rows(&self) -> Vec<Row<'a>> {
        match self.rows {
            Some(views) => views.iter().map(Row::View).collect(),
            None => array(&self.projection["rows"])
                .iter()
                .map(Row::Json)
                .collect(),
        }
    }
}
struct Reader<'a, 'i> {
    inputs: &'a LedgerInputs<'i>,
    original_rows: Vec<Row<'i>>,
    rows: HashMap<Cow<'a, str>, usize>,
    row_values: Vec<Row<'a>>,
    labels: HashMap<Cow<'i, str>, &'i Value>,
    iso: HashMap<String, String>,
    utc_dates: HashMap<String, bool>,
    sources: Vec<Option<std::sync::Arc<SourceProvenance<'i>>>>,
    source_values: Vec<Option<SourceValue<'i>>>,
    heads: HashMap<usize, HeadValues<'i>>,
    method: Option<Cow<'i, str>>,
}
struct HeadValues<'a> {
    wallet_id: &'a Value,
    profile_id: &'a Value,
    workspace_id: &'a Value,
    account_id: &'a Value,
}
struct SourceValue<'i> {
    spot: &'i Decimal,
    fee: &'i Decimal,
    crypto: &'i Decimal,
    source: std::sync::Arc<Source<'i>>,
}
impl<'a, 'i> Reader<'a, 'i> {
    fn iso(&mut self, raw: &Value) -> Result<String, Refusal> {
        if raw.is_array() || raw.is_object() {
            let kind = if raw.is_array() { "list" } else { "dict" };
            return Err(Refusal::new(
                "TypeError",
                format!("unhashable type: '{kind}'"),
            ));
        }
        // The usual UTC seconds spelling needs no timezone/calendar projections.
        // Validate each date with the shared parser once, then only its clock.
        // Every other spelling and every invalid value uses that same parser.
        if let Some(s) = raw.as_str() {
            let b = s.as_bytes();
            if b.len() == 20
                && b[4] == b'-'
                && b[7] == b'-'
                && b[10] == b'T'
                && b[13] == b':'
                && b[16] == b':'
                && b[19] == b'Z'
                && b.iter()
                    .enumerate()
                    .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19) || c.is_ascii_digit())
                && &s[11..13] < "24"
                && &s[14..16] < "60"
                && &s[17..19] < "60"
            {
                let valid = if let Some(valid) = self.utc_dates.get(&s[..10]) {
                    *valid
                } else {
                    let valid = kassiber_custody::ledger_request::timestamp_fields(&json!(
                        format!("{}T00:00:00Z", &s[..10])
                    ))["iso"]
                        .is_string();
                    self.utc_dates.insert(s[..10].into(), valid);
                    valid
                };
                if valid {
                    return Ok(format!("{}+00:00", &s[..19]));
                }
            }
        }
        let key = string(raw);
        if let Some(s) = self.iso.get(&key) {
            return Ok(s.clone());
        }
        let ts = kassiber_custody::ledger_request::timestamp_fields(raw);
        let iso = ts["iso"]
            .as_str()
            .ok_or_else(|| Refusal::new("KeyError", "'iso'"))?
            .to_owned();
        self.iso.insert(key, iso.clone());
        Ok(iso)
    }
    fn source(
        &mut self,
        asset: &'i str,
        id: &'i str,
        spot: &'i Decimal,
        fee: &'i Decimal,
        crypto: &'i Decimal,
    ) -> Result<std::sync::Arc<Source<'i>>, Refusal> {
        let slot = self.rows.get(id).copied();
        if let Some(cached) = slot.and_then(|slot| self.source_values[slot].as_ref()) {
            if cached.source.asset == asset
                && cached.spot.same_representation(spot)
                && cached.fee.same_representation(fee)
                && cached.crypto.same_representation(crypto)
            {
                return Ok(cached.source.clone());
            }
        }
        let provenance = if let Some(p) = slot.and_then(|slot| self.sources[slot].as_ref()) {
            p.clone()
        } else {
            let row = slot.map(|slot| self.row_values[slot]).unwrap_or(NULL_ROW);
            let iso = self.iso(row.at("occurred_at"))?;
            let pricing = if let Some(original) = slot
                .and_then(|slot| self.original_rows.get(slot))
                .filter(|original| original.same(row))
            {
                PricingFacts::Borrowed(pricing_fields(*original))
            } else {
                PricingFacts::Owned(Box::new(pricing_fields(row).map(Clone::clone)))
            };
            let provenance = std::sync::Arc::new(SourceProvenance {
                transaction_id: fallback(row.at("journal_transaction_id"), row.at("id")).clone(),
                engine_event_id: Cow::Borrowed(id),
                occurred_at: iso,
                pricing,
            });
            if let Some(slot) = slot {
                self.sources[slot] = Some(provenance.clone());
            }
            provenance
        };
        let amount = fragment_msat(crypto)?;
        let source = std::sync::Arc::new(Source {
            provenance,
            asset: Cow::Borrowed(asset),
            spot_price_exact: fixed(spot)?,
            fiat_fee_exact: fixed(fee)?,
            crypto_fee_msat_exact: if amount.as_i64() == Some(0) {
                Cow::Borrowed("0")
            } else {
                Cow::Owned(amount.to_string())
            },
            crypto_fee_msat: amount,
        });
        if let Some(slot) = slot {
            self.source_values[slot] = Some(SourceValue {
                spot,
                fee,
                crypto,
                source: source.clone(),
            });
        }
        Ok(source)
    }
    fn fragment(&mut self, asset: &'i str, f: &'i FragmentOut) -> Result<Fragment<'i>, Refusal> {
        let amount = fragment_msat(&f.crypto_amount.0)?;
        let acquired = f
            .lot
            .as_ref()
            .map(|l| fragment_msat(&l.crypto_in.0))
            .transpose()?;
        Ok(Fragment {
            quantity_msat_exact: amount.to_string(),
            quantity_msat: amount,
            cost_basis_exact: fixed(&f.cost_basis.0)?,
            proceeds_exact: fixed(&f.proceeds.0)?,
            gain_loss_exact: fixed(&f.gain_loss.0)?,
            unit_basis_override_exact: f
                .unit_basis_override
                .as_ref()
                .map(|d| fixed(&d.0))
                .transpose()?,
            basis_authority: if f.unit_basis_override.is_some() {
                "engine_unit_override"
            } else if f.lot.is_some() {
                "acquired_lot"
            } else {
                "income"
            },
            acquisition_basis_exact: f.lot.as_ref().map(|l| fixed(&l.basis.0)).transpose()?,
            acquisition_quantity_msat_exact: acquired.as_ref().map(ToString::to_string),
            acquisition_quantity_msat: acquired,
            event: self.source(
                asset,
                &f.event.row,
                &f.event.spot_price.0,
                &f.event.fiat_fee.0,
                &f.event.crypto_fee.0,
            )?,
            lot: f
                .lot
                .as_ref()
                .map(|l| {
                    self.source(
                        asset,
                        &l.row,
                        &l.spot_price.0,
                        &l.fiat_fee.0,
                        &l.crypto_fee.0,
                    )
                })
                .transpose()?,
        })
    }
    fn head(
        &mut self,
        wallet: &'i Value,
        row: Row<'_>,
        fallback_id: &str,
        occurred_at: Value,
        ids: &mut impl FnMut() -> String,
    ) -> Result<JournalEntryRow<'i>, Refusal> {
        let key = wallet as *const Value as usize;
        let (head, id) = match self.heads.entry(key) {
            std::collections::hash_map::Entry::Occupied(head) => (head.into_mut(), ids()),
            std::collections::hash_map::Entry::Vacant(head) => {
                let wallet_id = required(wallet, "id")?;
                ensure_hashable(wallet_id)?;
                let profile_id = required(self.inputs.profile, "id")?;
                let id = ids();
                let workspace_id = required(self.inputs.profile, "workspace_id")?;
                let account_id = required(wallet, "wallet_account_id")?;
                (
                    head.insert(HeadValues {
                        wallet_id,
                        profile_id,
                        workspace_id,
                        account_id,
                    }),
                    id,
                )
            }
        };
        Ok(Self::entry_head(head, row, fallback_id, occurred_at, id))
    }
    fn entry_head(
        head: &HeadValues<'i>,
        row: Row<'_>,
        fallback_id: &str,
        occurred_at: Value,
        id: String,
    ) -> JournalEntryRow<'i> {
        JournalEntryRow {
            id,
            workspace_id: head.workspace_id,
            profile_id: head.profile_id,
            transaction_id: Value::String(
                row.get("journal_transaction_id")
                    .map(string)
                    .unwrap_or_else(|| fallback_id.into()),
            ),
            wallet_id: head.wallet_id,
            account_id: head.account_id,
            occurred_at,
            entry_type: "acquisition",
            asset: Cow::Borrowed(""),
            quantity: zero().into(),
            fiat_value: zero(),
            unit_cost: zero(),
            cost_basis: None,
            proceeds: None,
            gain_loss: None,
            description: Cow::Borrowed(""),
            at_category: None,
            at_kennzahl: None,
            capital_gains_type: None,
            calculation: None,
        }
    }
    fn row(&self, id: &str) -> Option<Row<'a>> {
        self.rows.get(id).map(|slot| self.row_values[*slot])
    }
    fn required_row(&self, id: &str) -> Result<Row<'a>, Refusal> {
        self.row(id)
            .ok_or_else(|| Refusal::new("KeyError", format!("'{id}'")))
    }
    fn wallet(&self, label: &str) -> Result<&'i Value, Refusal> {
        self.labels.get(label).copied().ok_or_else(|| {
            let mut e = Refusal::new(
                "AppError",
                format!("RP2 emitted transaction for unknown wallet '{label}'"),
            );
            e.code = Some("internal".into());
            e
        })
    }
    fn entry(
        &mut self,
        item: &'i EntryOut,
        audits: &[TransferAudit],
        response: &'i LedgerResponse,
        ids: &mut impl FnMut() -> String,
    ) -> Result<JournalEntryRow<'i>, Refusal> {
        match item {
            EntryOut::Acquisition(a) => {
                let row = self.row(&a.row).unwrap_or(NULL_ROW);
                let mut e = self.head(
                    self.wallet(&a.wallet)?,
                    row,
                    &a.row,
                    row.required("occurred_at")?.clone(),
                    ids,
                )?;
                e.asset = Cow::Borrowed(&a.asset);
                e.quantity = jd(&a.quantity).into();
                e.fiat_value = jd(&a.fiat_value);
                e.unit_cost = jd(&a.unit_cost);
                e.description = Cow::Borrowed(&a.description);
                Ok(e)
            }
            EntryOut::Realized(a) => {
                let row = self.row(&a.row).unwrap_or(NULL_ROW);
                let mut e = self.head(
                    self.wallet(&a.wallet)?,
                    row,
                    &a.row,
                    row.required("occurred_at")?.clone(),
                    ids,
                )?;
                e.entry_type = a.entry_type.as_str();
                e.asset = Cow::Borrowed(&a.asset);
                e.quantity = jd(&a.quantity).into();
                e.fiat_value = jd(&a.proceeds);
                e.cost_basis = Some(jd(&a.cost_basis));
                e.proceeds = Some(jd(&a.proceeds));
                e.gain_loss = Some(jd(&a.gain_loss));
                e.description = Cow::Borrowed(&a.description);
                e.at_category = a.at_category.clone();
                e.at_kennzahl = a.at_category.as_ref().map(|c| match c.as_str() {
                    "income_general" | "income_capital_yield" => Some(172),
                    "neu_gain" => Some(174),
                    "neu_loss" => Some(176),
                    "alt_spekulation" => Some(801),
                    _ => None,
                });
                e.capital_gains_type = Some(a.capital_gains_type);
                let method = self
                    .method
                    .get_or_insert_with(|| {
                        let method = &self.inputs.profile["gains_algorithm"];
                        if method.is_null() {
                            Cow::Borrowed("")
                        } else if let Some(method) = method.as_str() {
                            Cow::Borrowed(kassiber_pytext::strip(method))
                        } else {
                            Cow::Owned(kassiber_pytext::strip(&string(method)).into())
                        }
                    })
                    .clone();
                e.calculation = Some(Calculation {
                    schema_version: 1,
                    engine: "kassiber_tax",
                    method,
                    fragments: {
                        let mut fragments = Vec::with_capacity(a.fragments.len());
                        for fragment in &a.fragments {
                            fragments.push(self.fragment(&a.asset, fragment)?);
                        }
                        fragments
                    },
                });
                Ok(e)
            }
            EntryOut::TransferLeg(a) => {
                let rendered = audits
                    .get(a.audit)
                    .ok_or_else(|| Refusal::new("IndexError", "list index out of range"))?;
                let audit = &rendered.facts;
                let outbound = a.entry_type == EntryType::TransferOut;
                let label = string(
                    &audit[if outbound {
                        "from_wallet_label"
                    } else {
                        "to_wallet_label"
                    }],
                );
                let wallet = lookup(&self.labels, &label)?;
                let row_id = string(&audit[if outbound { "out_id" } else { "in_id" }]);
                let row = self.row(&row_id).unwrap_or(NULL_ROW);
                let qty = JournalQuantity::from_audit(
                    if outbound {
                        rendered.crypto_sent
                    } else {
                        rendered.crypto_received
                    },
                    outbound,
                )?;
                let mut e = self.head(wallet, row, &row_id, audit["occurred_at"].clone(), ids)?;
                e.entry_type = a.entry_type.as_str();
                e.asset = Cow::Owned(string(&audit["asset"]));
                e.quantity = qty;
                e.description = Cow::Borrowed(&response.intra_audit[a.audit].description);
                Ok(e)
            }
        }
    }
}

/// Render the typed engine result; no JSON serialization on the per-entry path.
pub fn render_ledger<'i>(
    inputs: &LedgerInputs<'i>,
    response: &'i LedgerResponse,
    mut ids: impl FnMut() -> String,
) -> Result<LedgerState<'i>, Refusal> {
    let span = kassiber_custody::timing::span("render.inputs");
    let mut carried = HashMap::new();
    let original_rows = inputs.rows();
    let mut row_values: Vec<Row<'_>> = original_rows.clone();
    let mut rows = HashMap::with_capacity(row_values.len());
    for (slot, row) in row_values.iter().enumerate() {
        rows.insert(row_key(row.required("id")?), slot);
    }
    for carry in &response.carried_rows {
        let row = carried.entry(carry.row.clone()).or_insert_with(|| {
            rows.get(carry.row.as_str())
                .map(|slot| row_values[*slot])
                .unwrap_or(NULL_ROW)
                .json()
        });
        if row.is_null() {
            return Err(Refusal::new("KeyError", format!("'{}'", carry.row)));
        }
        row["fiat_rate"] = carry
            .rate
            .as_ref()
            .map(|s| {
                kassiber_pytext::parse_float(s)
                    .map(|n| {
                        if n.is_finite() {
                            json!(n)
                        } else {
                            json!({"$float":kassiber_pyjson::float_repr(n)})
                        }
                    })
                    .map_err(|_| {
                        Refusal::new(
                            "ValueError",
                            format!(
                                "could not convert string to float: {}",
                                kassiber_pytext::repr_str(s)
                            ),
                        )
                    })
            })
            .transpose()?
            .unwrap_or(Value::Null);
        row["fiat_value"] = json!(carry.value);
        row["fiat_rate_exact"] = json!(carry.rate);
        row["fiat_value_exact"] = json!(carry.value);
        for (k, v) in [
            ("fiat_price_source", json!("manual")),
            ("pricing_source_kind", json!("manual_override")),
            ("pricing_quality", json!("exact")),
            ("pricing_provider", Value::Null),
            ("pricing_pair", Value::Null),
            ("pricing_granularity", Value::Null),
            ("pricing_method", json!("carrying_value")),
        ] {
            row[k] = v;
        }
    }
    for (id, row) in &carried {
        row_values[rows[id.as_str()]] = Row::Json(row);
    }
    let sources = vec![None; row_values.len()];
    let source_values = std::iter::repeat_with(|| None)
        .take(row_values.len())
        .collect();
    let mut labels = HashMap::new();
    for wallet in inputs.wallets {
        let label = response::get(wallet, "label")?;
        if !label.is_null() && label != "" {
            labels.insert(row_key(label), wallet);
        }
    }
    let mut reader = Reader {
        inputs,
        original_rows,
        rows,
        row_values,
        labels,
        iso: HashMap::new(),
        utc_dates: HashMap::new(),
        sources,
        source_values,
        heads: HashMap::new(),
        method: None,
    };
    drop(span);
    let span = kassiber_custody::timing::span("render.audit");
    let audits = response
        .intra_audit
        .iter()
        .map(|a| audit(inputs, a))
        .collect::<Result<Vec<_>, _>>()?;
    drop(span);
    let span = kassiber_custody::timing::span("render.entries");
    let mut entries = Vec::with_capacity(response.entries.len());
    for entry in &response.entries {
        entries.push(reader.entry(entry, &audits, response, &mut ids)?);
    }
    drop(span);
    let span = kassiber_custody::timing::span("render.quarantine_holdings");
    let (account_holdings, wallet_holdings) = holdings(&response.holdings, &reader.labels)?;
    let mut quarantines = array(&inputs.projection["quarantines"])
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone()).map_err(|e| Refusal::new("TypeError", e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for q in &response.quarantines {
        quarantines.push(quarantine(inputs.profile, reader.required_row(&q.row)?, q)?);
    }
    drop(span);
    let direct_swap_payouts = direct_payouts(inputs)?;
    let span = kassiber_custody::timing::span("render.summary");
    let tax_summary = response
        .tax_summary
        .iter()
        .map(|s| {
            Ok(TaxSummaryRow {
                year: s.year,
                asset: s.asset.clone(),
                transaction_type: s.transaction_type,
                capital_gains_type: if s.long_term { "long" } else { "short" },
                quantity: float(&s.quantity.0),
                quantity_msat: msat(&s.quantity.0)?,
                proceeds: float(&s.proceeds.0),
                cost_basis: float(&s.cost_basis.0),
                gain_loss: float(&s.gain_loss.0),
            })
        })
        .collect::<Result<_, Refusal>>()?;
    drop(span);
    let span = kassiber_custody::timing::span("render.finish");
    drop(reader);
    drop(span);
    Ok(LedgerState {
        entries,
        quarantines,
        intra_audit: audits,
        tax_summary,
        account_holdings,
        wallet_holdings,
        direct_swap_payouts,
    })
}
fn audit(inputs: &LedgerInputs<'_>, a: &AuditOut) -> Result<TransferAudit, Refusal> {
    let pair = array(&inputs.projection["intra_pairs"])
        .get(a.pair)
        .ok_or_else(|| Refusal::new("IndexError", "tuple index out of range"))?;
    let out = &pair["out"];
    let inbound = &pair["in"];
    let from = inputs
        .wallets
        .iter()
        .find(|w| w["id"] == out["wallet_id"])
        .ok_or_else(|| Refusal::new("KeyError", format!("'{}'", string(&out["wallet_id"]))))?;
    let to = inputs
        .wallets
        .iter()
        .find(|w| w["id"] == inbound["wallet_id"])
        .ok_or_else(|| Refusal::new("KeyError", format!("'{}'", string(&inbound["wallet_id"]))))?;
    let mut v = json!({"out_id":out["id"],"in_id":inbound["id"],"out_anchor_transaction_id":string(fallback(&out["journal_transaction_id"],&out["id"])),"in_anchor_transaction_id":string(fallback(&inbound["journal_transaction_id"],&inbound["id"])),"from_wallet_id":from["id"],"from_wallet_label":from["label"],"to_wallet_id":to["id"],"to_wallet_label":to["label"],"asset":string(&out["asset"]),"occurred_at":out["occurred_at"],"external_id":out["external_id"],"crypto_sent_msat":a.sent_msat,"crypto_received_msat":a.received_msat,"crypto_fee_msat":a.fee_msat,"pairing_source":pair["source"]});
    if let Some(r) = a.at_regime {
        v["at_regime"] = json!(r);
    }
    if let Some(f) = &a.regime_flows {
        v["regime_flows"] = json!(f);
    }
    if let Some(p) = &a.at_pool {
        if !p.is_empty() {
            v["at_pool"] = json!(p);
        }
    }
    if truth(&pair["group_id"]) {
        v["transfer_group_id"] = json!(string(&pair["group_id"]));
    }
    Ok(TransferAudit {
        facts: v,
        crypto_sent: float(&a.sent.0),
        crypto_received: float(&a.received.0),
        crypto_fee: float(&a.fee.0),
        spot_price: a.spot_price.as_ref().map(|d| float(&d.0)).unwrap_or(0.0),
    })
}
fn detail_value(v: &Value) -> Result<Py, Refusal> {
    Ok(match v {
        Value::Object(m) if m.len() == 1 && m.contains_key("float") => {
            let operand = &m["float"];
            let decimal = operand
                .as_str()
                .and_then(|text| Decimal::from_canonical(text).ok())
                .ok_or_else(|| {
                    let repr = match operand {
                        Value::Null => "None".into(),
                        Value::Bool(true) => "True".into(),
                        Value::Bool(false) => "False".into(),
                        _ => operand.to_string(),
                    };
                    let repr = operand
                        .as_str()
                        .map(kassiber_pytext::repr_str)
                        .unwrap_or(repr);
                    Refusal::new(
                        "ValueError",
                        format!("not a canonical engine decimal: {repr}"),
                    )
                })?;
            Py::Float(float(&decimal))
        }
        Value::Object(m) => Py::Dict(
            m.iter()
                .map(|(k, v)| Ok((Key::Str(k.as_str().into()), detail_value(v)?)))
                .collect::<Result<_, Refusal>>()?,
        ),
        Value::Array(a) => Py::List(a.iter().map(detail_value).collect::<Result<_, _>>()?),
        _ => kassiber_custody::codec::from_json(v)?,
    })
}
fn py_detail(value: &Py, compact: bool) -> Result<String, Refusal> {
    let mut options = kassiber_pyjson::Options::new().sort_keys(true);
    if compact {
        options = options.compact();
    }
    kassiber_pyjson::dumps(value, &options)
        .map_err(|error| Refusal::new("ValueError", error.to_string()))
}
fn quarantine(profile: &Value, row: Row<'_>, q: &QuarantineOut) -> Result<QuarantineRow, Refusal> {
    let mut detail = detail_value(&Value::Object(q.detail.clone()))?;
    let mut enriched = json!({});
    match q.detail_kind {
        DetailKind::PricingReview => {
            for k in REVIEW {
                enriched[k] = row[k].clone();
            }
        }
        DetailKind::Hold => {
            enriched["external_id"] = json!(if truth(&row["external_id"]) {
                string(&row["external_id"])
            } else {
                String::new()
            })
        }
        DetailKind::PrivacyHop => {
            let boundary = kassiber_pytext::strip(&string(&row["privacy_boundary"]))
                .to_lowercase()
                .replace('-', "_");
            if matches!(
                boundary.as_str(),
                "coinjoin" | "collaborative" | "payjoin" | "payment_in_coinjoin" | "sweep"
            ) {
                let raw = if row["raw_json"].is_object() {
                    row["raw_json"].clone()
                } else {
                    serde_json::from_str(row["raw_json"].as_str().unwrap_or(""))
                        .unwrap_or(Value::Null)
                };
                let likely = [
                    "islikelycoinjoin",
                    "is_likely_coinjoin",
                    "isLikelyCoinJoin",
                    "likely_coinjoin",
                    "likelyCoinJoin",
                ]
                .iter()
                .any(|k| boolish(&raw[*k]));
                for (k, v) in [
                    ("privacy_boundary", json!(boundary)),
                    ("privacy_hop", json!(boundary)),
                    ("source", fallback(&raw["source"], &json!("")).clone()),
                    ("islikelycoinjoin", json!(likely)),
                    ("required_for", json!("explicit_user_owned_provenance")),
                ] {
                    enriched[k] = v;
                }
            }
        }
        DetailKind::Plain | DetailKind::PartnerLeg => {}
    }
    if let Py::Dict(items) = &mut detail {
        for (key, value) in enriched.as_object().expect("enrichment object") {
            let key = Key::Str(key.as_str().into());
            let value = kassiber_custody::codec::from_json(value)?;
            if let Some((_, old)) = items.iter_mut().find(|(k, _)| *k == key) {
                *old = value;
            } else {
                items.push((key, value));
            }
        }
    }
    Ok(QuarantineRow {
        transaction_id: fallback(&row["journal_transaction_id"], row.required("id")?).clone(),
        workspace_id: required(profile, "workspace_id")?.clone(),
        profile_id: required(profile, "id")?.clone(),
        reason: q.reason.as_str().into(),
        detail_json: py_detail(&detail, false)?,
    })
}
fn boolish(v: &Value) -> bool {
    v == &Value::Bool(true)
        || matches!(
            kassiber_pytext::strip(&string(v)).to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
}
fn holdings(
    items: &[HoldingOut],
    labels: &HashMap<Cow<'_, str>, &Value>,
) -> Result<(Holdings, Holdings), Refusal> {
    // The asset is part of each key, so distinct assets cannot round one another.
    let mut accounts = Holdings { pairs: vec![] };
    let mut wallets = Holdings { pairs: vec![] };
    let mut account_slots = HashMap::new();
    let mut wallet_slots = HashMap::new();
    fn add(
        h: &mut Holdings,
        slots: &mut HashMap<Vec<HoldingKey>, usize>,
        key: Vec<Value>,
        item: &HoldingOut,
    ) -> Result<(), Refusal> {
        let index = slots.len();
        let slot = *slots
            .entry(key.iter().map(holding_key).collect::<Result<_, _>>()?)
            .or_insert(index);
        if slot == h.pairs.len() {
            h.pairs.push((
                key,
                Holding {
                    quantity: zero(),
                    cost_basis: zero(),
                },
            ));
        }
        let value = &mut h.pairs[slot].1;
        value.quantity.0 = value.quantity.0.add(&item.quantity.0, &CTX)?;
        value.cost_basis.0 = value.cost_basis.0.add(&item.cost_basis.0, &CTX)?;
        Ok(())
    }
    let mut assets = Vec::<Vec<&HoldingOut>>::new();
    let mut asset_slots = HashMap::new();
    for item in items {
        let next = asset_slots.len();
        let slot = *asset_slots.entry(&item.asset).or_insert(next);
        if slot == assets.len() {
            assets.push(Vec::new());
        }
        assets[slot].push(item);
    }
    for item in assets.into_iter().flatten() {
        if let Some(w) = labels.get(item.wallet.as_str()) {
            add(
                &mut accounts,
                &mut account_slots,
                vec![
                    required(w, "wallet_account_id")?.clone(),
                    required(w, "account_code")?.clone(),
                    required(w, "account_label")?.clone(),
                    json!(item.asset),
                ],
                item,
            )?;
            add(
                &mut wallets,
                &mut wallet_slots,
                vec![
                    required(w, "id")?.clone(),
                    required(w, "label")?.clone(),
                    required(w, "account_code")?.clone(),
                    json!(item.asset),
                ],
                item,
            )?;
        }
    }
    // Python adds each asset subtotal to another Decimal(0), including its rounding.
    for h in [&mut accounts, &mut wallets] {
        for (_, v) in &mut h.pairs {
            v.quantity.0 = Decimal::zero().add(&v.quantity.0, &CTX)?;
            v.cost_basis.0 = Decimal::zero().add(&v.cost_basis.0, &CTX)?;
        }
    }
    Ok((accounts, wallets))
}
#[derive(PartialEq, Eq, Hash)]
enum HoldingKey {
    Null,
    Integer(kassiber_pyjson::BigInt),
    Float(u64),
    String(String),
    Bytes(Vec<u8>),
}
fn unhashable(value: &Value) -> Refusal {
    Refusal::new(
        "TypeError",
        format!(
            "unhashable type: '{}'",
            if value.is_array() { "list" } else { "dict" }
        ),
    )
}
fn ensure_hashable(value: &Value) -> Result<(), Refusal> {
    // Loaded SQLite BLOBs stay Python bytes, which are hashable.
    if (value.is_array() || value.is_object())
        && kassiber_custody::codec::raw_bytes(value).is_none()
    {
        return Err(unhashable(value));
    }
    Ok(())
}
fn holding_key(value: &Value) -> Result<HoldingKey, Refusal> {
    Ok(match value {
        Value::Null => HoldingKey::Null,
        Value::Bool(v) => HoldingKey::Integer(i64::from(*v).into()),
        Value::Number(n) => {
            let text = n.to_string();
            if text.contains(['.', 'e', 'E']) {
                let value = text.parse::<f64>().expect("JSON float");
                if value.is_finite() && value.fract() == 0.0 {
                    HoldingKey::Integer(format!("{value:.0}").parse().expect("integer float"))
                } else {
                    HoldingKey::Float(value.to_bits())
                }
            } else {
                HoldingKey::Integer(text.parse().expect("JSON integer"))
            }
        }
        Value::String(text) => HoldingKey::String(text.clone()),
        _ => match kassiber_custody::codec::raw_bytes(value) {
            Some(bytes) => HoldingKey::Bytes(bytes),
            None => return Err(unhashable(value)),
        },
    })
}
fn direct_payouts(inputs: &LedgerInputs<'_>) -> Result<Value, Refusal> {
    if inputs.direct_payout_records.is_empty() {
        return Ok(json!([]));
    }
    let mut rows: HashMap<String, Vec<Row<'_>>> = HashMap::new();
    for r in inputs.rows() {
        if truth(&r["journal_transaction_id"]) && r["direction"] == "outbound" {
            rows.entry(string(&r["journal_transaction_id"]))
                .or_default()
                .push(r);
        }
    }
    let mut result = vec![];
    for r in inputs.direct_payout_records {
        let id = &r["id"];
        let out = &r["out_transaction_id"];
        if !truth(id) || !truth(out) {
            continue;
        }
        let amount = fallback(&r["out_amount"], &r["out_amount_msat"]);
        let mut candidates = vec![];
        for row in rows.get(&string(out)).into_iter().flatten() {
            if truth(&r["out_asset"])
                && string(fallback(&row["asset"], &json!(""))) != string(&r["out_asset"])
            {
                continue;
            }
            if !amount.is_null()
                && amount != ""
                && crate::integer::integer_or_zero(&row["amount"])?
                    != crate::integer::integer(amount)?
            {
                continue;
            }
            candidates.push(row);
        }
        let source = if candidates.len() == 1 {
            *candidates[0]
        } else {
            NULL_ROW
        };
        let mut v = json!({"payout_id":string(id),"out_id":string(out),"out_asset":fallback(&r["out_asset"],&source["asset"]),"out_amount_msat":crate::integer::integer_or_zero(fallback(amount,&source["amount"]))?,"payout_amount_msat":crate::integer::integer_or_zero(&r["payout_amount"])?,"swap_fee_msat":crate::integer::integer_or_zero(&r["swap_fee_msat"])?});
        for k in [
            "kind",
            "policy",
            "payout_asset",
            "payout_occurred_at",
            "payout_external_id",
            "counterparty",
            "swap_fee_kind",
        ] {
            v[k] = r[k].clone();
        }
        result.push(v);
    }
    Ok(json!(result))
}
/// Persistence calls this after deduplication; build() itself keeps raw quarantines.
pub fn enrich_quarantine_detail(
    detail_json: &Value,
    candidates: &[Value],
) -> Result<String, Refusal> {
    crate::persistence::enrich_quarantine_detail(detail_json.as_str().unwrap_or(""), candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn msat_conversion_matches_python_at_rounding_and_width_boundaries() {
        let mut vectors: Vec<(String, Value)> =
            serde_json::from_str(include_str!("../tests/data/msat.json")).unwrap();
        vectors.extend(
            serde_json::from_str::<Vec<(String, Value)>>(include_str!(
                "../tests/data/msat-machine.json"
            ))
            .unwrap(),
        );
        for (input, expected) in vectors {
            assert_eq!(
                msat(&input.parse::<Decimal>().unwrap()).unwrap(),
                expected,
                "{input}"
            );
        }
    }
}

#[cfg(test)]
mod fixed_tests {
    use super::*;

    #[test]
    fn fixed_point_text_matches_decimal_formatting() {
        let mut coefficient = 0u128;
        for index in 0..200 {
            for exponent in [-65, -64, -39, -20, -2, -1, 0, 1, 20, 64, 65] {
                for sign in ["", "-"] {
                    let value = Decimal::from_canonical(&format!("{sign}{coefficient}E{exponent}"))
                        .unwrap();
                    assert_eq!(
                        fixed(&value).unwrap(),
                        value.to_fixed_string().unwrap(),
                        "{value}"
                    );
                }
            }
            coefficient = if index == 0 {
                u128::MAX
            } else {
                coefficient
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407)
            };
        }
        for operand in [
            "10000000000000000000000000000000000000000E-39",
            "-0E-100",
            "0E100",
            "1E-100000002",
            "1E100000001",
        ] {
            let value = Decimal::from_canonical(operand).unwrap();
            match value.to_fixed_string() {
                Ok(expected) => assert_eq!(fixed(&value).unwrap(), expected),
                Err(expected) => assert_eq!(fixed(&value).unwrap_err().family, expected.name()),
            }
        }
    }
}
