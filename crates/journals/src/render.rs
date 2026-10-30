use crate::*;
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_tax_core::{
    ledger::response::*,
    manifest::{DetailKind, EntryType},
};
use serde_json::{json, Value};
use std::{borrow::Cow, collections::HashMap};
const CTX: Context = Context::RP2;
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
fn fixed(d: &Decimal) -> Result<String, Refusal> {
    Ok(d.to_fixed_string()?)
}
fn float(d: &Decimal) -> f64 {
    d.to_string().parse().expect("finite Decimal text")
}
pub(crate) fn msat(d: &Decimal) -> Result<Value, Refusal> {
    if d.is_zero() {
        return Ok(json!(0));
    }
    let shift = d.exponent() + 11;
    // Common BTC amounts fit machine integers. The <=20-digit coefficient
    // cannot round in Python's 32-digit power-of-ten multiply. Keep exotic
    // exponents and wide results on the arbitrary-precision path below.
    if d.coefficient().bits() <= 64 && (-19..=19).contains(&shift) {
        let n = u128::from(d.coefficient().iter_u64_digits().next().unwrap_or(0));
        let magnitude = if shift >= 0 {
            n * 10u128.pow(shift as u32)
        } else {
            let divisor = 10u128.pow((-shift) as u32);
            n / divisor + u128::from(n % divisor >= divisor / 2)
        };
        if let Ok(n) = i64::try_from(magnitude) {
            return Ok(json!(if d.is_sign_negative() { -n } else { n }));
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
        return number(&magnitude);
    }
    let n = d.mul(&Decimal::from_canonical("100000000000E0")?, &CTX)?;
    number(&n.to_fixed_places(0, Rounding::HalfUp)?)
}
fn required<'a>(row: &'a Value, key: &str) -> Result<&'a Value, Refusal> {
    row.get(key)
        .ok_or_else(|| Refusal::new("KeyError", format!("'{key}'")))
}
fn row_key(value: &Value) -> Cow<'_, str> {
    value
        .as_str()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(string(value)))
}
fn lookup<'a>(map: &'a HashMap<Cow<'_, str>, &Value>, id: &str) -> Result<&'a Value, Refusal> {
    map.get(id)
        .copied()
        .ok_or_else(|| Refusal::new("KeyError", format!("'{id}'")))
}
fn decimal_text(s: &str) -> Result<Decimal, Refusal> {
    s.parse::<Decimal>().map_err(Refusal::from)
}

pub struct LedgerState {
    pub entries: Vec<JournalEntryRow>,
    pub quarantines: Vec<QuarantineRow>,
    pub intra_audit: Vec<Value>,
    pub tax_summary: Vec<TaxSummaryRow>,
    pub account_holdings: Holdings,
    pub wallet_holdings: Holdings,
    pub direct_swap_payouts: Value,
}
/// Operands shared with the future typed finalized stage. Raw rows use transitional JSON.
pub struct LedgerInputs<'a> {
    pub profile: &'a Value,
    pub projection: &'a Value,
    pub wallets: &'a [Value],
    pub direct_payout_records: &'a [Value],
}
struct Reader<'a> {
    inputs: &'a LedgerInputs<'a>,
    rows: HashMap<Cow<'a, str>, usize>,
    row_values: Vec<&'a Value>,
    labels: HashMap<Cow<'a, str>, &'a Value>,
    iso: HashMap<String, String>,
    utc_dates: HashMap<String, bool>,
    sources: Vec<Option<std::sync::Arc<SourceProvenance>>>,
}
impl Reader<'_> {
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
                let valid = *self.utc_dates.entry(s[..10].into()).or_insert_with(|| {
                    kassiber_custody::ledger_request::timestamp_fields(&json!(format!(
                        "{}T00:00:00Z",
                        &s[..10]
                    )))["iso"]
                        .is_string()
                });
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
        asset: &str,
        id: &str,
        spot: &Decimal,
        fee: &Decimal,
        crypto: &Decimal,
    ) -> Result<Source, Refusal> {
        let slot = self.rows.get(id).copied();
        let provenance = if let Some(p) = slot.and_then(|slot| self.sources[slot].as_ref()) {
            p.clone()
        } else {
            let row = slot
                .map(|slot| self.row_values[slot])
                .unwrap_or(&Value::Null);
            let iso = self.iso(&row["occurred_at"])?;
            let provenance = std::sync::Arc::new(SourceProvenance {
                transaction_id: fallback(&row["journal_transaction_id"], &row["id"]).clone(),
                engine_event_id: id.into(),
                occurred_at: iso,
                pricing: PricingFacts(std::array::from_fn(|i| {
                    row[FRAGMENT_PRICING_FIELDS[i]].clone()
                })),
            });
            if let Some(slot) = slot {
                self.sources[slot] = Some(provenance.clone());
            }
            provenance
        };
        let amount = msat(crypto)?;
        Ok(Source {
            provenance,
            asset: asset.into(),
            spot_price_exact: fixed(spot)?,
            fiat_fee_exact: fixed(fee)?,
            crypto_fee_msat_exact: amount.to_string(),
            crypto_fee_msat: amount,
        })
    }
    fn fragment(&mut self, asset: &str, f: &FragmentOut) -> Result<Fragment, Refusal> {
        let amount = msat(&f.crypto_amount.0)?;
        let acquired = f.lot.as_ref().map(|l| msat(&l.crypto_in.0)).transpose()?;
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
            acquisition_quantity_msat_exact: acquired.as_ref().map(Value::to_string),
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
        &self,
        wallet: &Value,
        row: &Value,
        fallback_id: &str,
        occurred_at: Value,
        id: String,
    ) -> Result<JournalEntryRow, Refusal> {
        Ok(JournalEntryRow {
            id,
            workspace_id: required(self.inputs.profile, "workspace_id")?.clone(),
            profile_id: required(self.inputs.profile, "id")?.clone(),
            transaction_id: json!(row
                .get("journal_transaction_id")
                .map(string)
                .unwrap_or_else(|| fallback_id.into())),
            wallet_id: required(wallet, "id")?.clone(),
            account_id: required(wallet, "wallet_account_id")?.clone(),
            occurred_at,
            entry_type: "acquisition",
            asset: String::new(),
            quantity: zero(),
            fiat_value: zero(),
            unit_cost: zero(),
            cost_basis: None,
            proceeds: None,
            gain_loss: None,
            description: String::new(),
            at_category: None,
            at_kennzahl: None,
            capital_gains_type: None,
            calculation: None,
        })
    }
    fn row(&self, id: &str) -> Option<&Value> {
        self.rows.get(id).map(|slot| self.row_values[*slot])
    }
    fn required_row(&self, id: &str) -> Result<&Value, Refusal> {
        self.row(id)
            .ok_or_else(|| Refusal::new("KeyError", format!("'{id}'")))
    }
    fn wallet(&self, label: &str) -> Result<&Value, Refusal> {
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
        item: &EntryOut,
        audits: &[Value],
        response: &LedgerResponse,
        ids: &mut impl FnMut() -> String,
    ) -> Result<JournalEntryRow, Refusal> {
        match item {
            EntryOut::Acquisition(a) => {
                let row = self.required_row(&a.row)?;
                let mut e = self.head(
                    self.wallet(&a.wallet)?,
                    row,
                    &a.row,
                    required(row, "occurred_at")?.clone(),
                    ids(),
                )?;
                e.asset = a.asset.clone();
                e.quantity = jd(&a.quantity);
                e.fiat_value = jd(&a.fiat_value);
                e.unit_cost = jd(&a.unit_cost);
                e.description = a.description.clone();
                Ok(e)
            }
            EntryOut::Realized(a) => {
                let row = self.required_row(&a.row)?;
                let mut e = self.head(
                    self.wallet(&a.wallet)?,
                    row,
                    &a.row,
                    required(row, "occurred_at")?.clone(),
                    ids(),
                )?;
                e.entry_type = a.entry_type.as_str();
                e.asset = a.asset.clone();
                e.quantity = jd(&a.quantity);
                e.fiat_value = jd(&a.proceeds);
                e.cost_basis = Some(jd(&a.cost_basis));
                e.proceeds = Some(jd(&a.proceeds));
                e.gain_loss = Some(jd(&a.gain_loss));
                e.description = a.description.clone();
                e.at_category = a.at_category.clone();
                e.at_kennzahl = a.at_category.as_ref().map(|c| match c.as_str() {
                    "income_general" | "income_capital_yield" => Some(172),
                    "neu_gain" => Some(174),
                    "neu_loss" => Some(176),
                    "alt_spekulation" => Some(801),
                    _ => None,
                });
                e.capital_gains_type = Some(a.capital_gains_type);
                let method = &self.inputs.profile["gains_algorithm"];
                e.calculation = Some(Calculation {
                    schema_version: 1,
                    engine: "kassiber_tax",
                    method: if method.is_null() {
                        String::new()
                    } else {
                        string(method).trim().into()
                    },
                    fragments: a
                        .fragments
                        .iter()
                        .map(|f| self.fragment(&a.asset, f))
                        .collect::<Result<_, _>>()?,
                });
                Ok(e)
            }
            EntryOut::TransferLeg(a) => {
                let audit = audits
                    .get(a.audit)
                    .ok_or_else(|| Refusal::new("IndexError", "list index out of range"))?;
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
                let row = self.row(&row_id).unwrap_or(&Value::Null);
                let mut e = self.head(wallet, row, &row_id, audit["occurred_at"].clone(), ids())?;
                let qty = decimal_text(&kassiber_pyjson::float_repr(
                    audit[if outbound {
                        "crypto_sent"
                    } else {
                        "crypto_received"
                    }]
                    .as_f64()
                    .expect("audit float"),
                ))?;
                e.entry_type = a.entry_type.as_str();
                e.asset = string(&audit["asset"]);
                e.quantity = JournalDecimal(if outbound { qty.neg(&CTX)? } else { qty });
                e.description = response.intra_audit[a.audit].description.clone();
                Ok(e)
            }
        }
    }
}

/// Render the typed engine result; no JSON serialization on the per-entry path.
pub fn render_ledger(
    inputs: &LedgerInputs<'_>,
    response: &LedgerResponse,
    mut ids: impl FnMut() -> String,
) -> Result<LedgerState, Refusal> {
    let span = kassiber_custody::timing::span("render.inputs");
    let mut carried = HashMap::new();
    let mut row_values: Vec<_> = array(&inputs.projection["rows"]).iter().collect();
    let rows: HashMap<_, _> = row_values
        .iter()
        .enumerate()
        .map(|(slot, row)| (row_key(&row["id"]), slot))
        .collect();
    for carry in &response.carried_rows {
        let row = carried.entry(carry.row.clone()).or_insert_with(|| {
            rows.get(carry.row.as_str())
                .map(|slot| row_values[*slot])
                .unwrap_or(&Value::Null)
                .clone()
        });
        if row.is_null() {
            return Err(Refusal::new("KeyError", format!("'{}'", carry.row)));
        }
        row["fiat_rate"] = carry
            .rate
            .as_ref()
            .map(|s| s.parse::<f64>().map(|n| json!(n)))
            .transpose()
            .map_err(|e| Refusal::new("ValueError", e.to_string()))?
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
        row_values[rows[id.as_str()]] = row;
    }
    let sources = vec![None; row_values.len()];
    let labels = inputs
        .wallets
        .iter()
        .filter(|w| !w["label"].is_null() && w["label"] != "")
        .map(|w| (row_key(&w["label"]), w))
        .collect();
    let mut reader = Reader {
        inputs,
        rows,
        row_values,
        labels,
        iso: HashMap::new(),
        utc_dates: HashMap::new(),
        sources,
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
    let entries = response
        .entries
        .iter()
        .map(|e| reader.entry(e, &audits, response, &mut ids))
        .collect::<Result<_, _>>()?;
    drop(span);
    let span = kassiber_custody::timing::span("render.quarantine_holdings");
    let mut quarantines = array(&inputs.projection["quarantines"])
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone()).map_err(|e| Refusal::new("TypeError", e.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for q in &response.quarantines {
        quarantines.push(quarantine(inputs.profile, reader.required_row(&q.row)?, q)?);
    }
    let (account_holdings, wallet_holdings) = holdings(&response.holdings, &reader.labels)?;
    drop(span);
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
    let direct_swap_payouts = direct_payouts(inputs)?;
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
fn audit(inputs: &LedgerInputs<'_>, a: &AuditOut) -> Result<Value, Refusal> {
    let pair = array(&inputs.projection["intra_pairs"])
        .get(a.pair)
        .ok_or_else(|| Refusal::new("IndexError", "list index out of range"))?;
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
    let mut v = json!({"out_id":out["id"],"in_id":inbound["id"],"out_anchor_transaction_id":string(fallback(&out["journal_transaction_id"],&out["id"])),"in_anchor_transaction_id":string(fallback(&inbound["journal_transaction_id"],&inbound["id"])),"from_wallet_id":from["id"],"from_wallet_label":from["label"],"to_wallet_id":to["id"],"to_wallet_label":to["label"],"asset":string(&out["asset"]),"occurred_at":out["occurred_at"],"external_id":out["external_id"],"crypto_sent":float(&a.sent.0),"crypto_received":float(&a.received.0),"crypto_fee":float(&a.fee.0),"crypto_sent_msat":a.sent_msat,"crypto_received_msat":a.received_msat,"crypto_fee_msat":a.fee_msat,"spot_price":a.spot_price.as_ref().map(|d|float(&d.0)).unwrap_or(0.0),"pairing_source":pair["source"]});
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
    Ok(v)
}
fn detail_value(v: &Value) -> Value {
    match v {
        Value::Object(m) if m.len() == 1 && m.contains_key("float") => json!(m["float"]
            .as_str()
            .expect("float text")
            .parse::<f64>()
            .expect("canonical float")),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), detail_value(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(detail_value).collect()),
        _ => v.clone(),
    }
}
fn quarantine(profile: &Value, row: &Value, q: &QuarantineOut) -> Result<QuarantineRow, Refusal> {
    let mut detail = detail_value(&Value::Object(q.detail.clone()));
    match q.detail_kind {
        DetailKind::PricingReview => {
            for k in REVIEW {
                detail[k] = row[k].clone();
            }
        }
        DetailKind::Hold => {
            detail["external_id"] = json!(if truth(&row["external_id"]) {
                string(&row["external_id"])
            } else {
                String::new()
            })
        }
        DetailKind::PrivacyHop => {
            let boundary = string(&row["privacy_boundary"])
                .trim()
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
                    detail[k] = v;
                }
            }
        }
        DetailKind::Plain | DetailKind::PartnerLeg => {}
    }
    Ok(QuarantineRow {
        transaction_id: fallback(&row["journal_transaction_id"], required(row, "id")?).clone(),
        workspace_id: required(profile, "workspace_id")?.clone(),
        profile_id: required(profile, "id")?.clone(),
        reason: q.reason.as_str().into(),
        detail_json: pyjson(&detail, false)?,
    })
}
fn boolish(v: &Value) -> bool {
    v == &Value::Bool(true)
        || matches!(
            string(v).trim().to_lowercase().as_str(),
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
        slots: &mut HashMap<String, usize>,
        key: Vec<Value>,
        item: &HoldingOut,
    ) -> Result<(), Refusal> {
        let index = slots.len();
        let slot = *slots
            .entry(serde_json::to_string(&key).expect("key"))
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
                    w["wallet_account_id"].clone(),
                    w["account_code"].clone(),
                    w["account_label"].clone(),
                    json!(item.asset),
                ],
                item,
            )?;
            add(
                &mut wallets,
                &mut wallet_slots,
                vec![
                    w["id"].clone(),
                    w["label"].clone(),
                    w["account_code"].clone(),
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
fn direct_payouts(inputs: &LedgerInputs<'_>) -> Result<Value, Refusal> {
    if inputs.direct_payout_records.is_empty() {
        return Ok(json!([]));
    }
    let mut rows: HashMap<String, Vec<&Value>> = HashMap::new();
    for r in array(&inputs.projection["rows"]) {
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
        let candidates: Vec<_> = rows
            .get(&string(out))
            .into_iter()
            .flatten()
            .filter(|row| {
                (!truth(&r["out_asset"]) || string(&row["asset"]) == string(&r["out_asset"]))
                    && (amount.is_null()
                        || amount == ""
                        || integer(&row["amount"]) == integer(amount))
            })
            .collect();
        let source = if candidates.len() == 1 {
            *candidates[0]
        } else {
            &Value::Null
        };
        let mut v = json!({"payout_id":string(id),"out_id":string(out),"out_asset":fallback(&r["out_asset"],&source["asset"]),"out_amount_msat":integer(fallback(amount,&source["amount"])),"payout_amount_msat":integer(&r["payout_amount"]),"swap_fee_msat":integer(&r["swap_fee_msat"])});
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
fn integer(v: &Value) -> Value {
    if truth(v) {
        number(&string(v)).unwrap_or(Value::Null)
    } else {
        json!(0)
    }
}
/// Persistence calls this after deduplication; build() itself keeps raw quarantines.
pub fn enrich_quarantine_detail(
    detail_json: &Value,
    candidates: &[Value],
) -> Result<String, Refusal> {
    let mut detail =
        serde_json::from_str::<Value>(detail_json.as_str().unwrap_or("")).unwrap_or(json!({}));
    if !detail.is_object() {
        detail = json!({});
    }
    if !candidates.is_empty() {
        detail["ownership_review_candidates"] = json!(candidates);
    }
    pyjson(&detail, true)
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
