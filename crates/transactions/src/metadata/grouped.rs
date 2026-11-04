//! One reviewed metadata save, including exact pricing and one forward audit event.
use super::*;
use kassiber_envelope::py_str;
use kassiber_pydecimal::{Context, Decimal, DecimalError, Rounding};
use kassiber_pytext::{is_alpha, upper};

/// Presence and JSON null have different meanings for nullable authored fields.
/// Values are validated here, before any write; transports never interpret them.
#[derive(Default)]
pub struct GroupedUpdate {
    pub transaction: String,
    pub note: Option<Value>,
    pub tags: Option<Value>,
    pub excluded: Option<Value>,
    pub pricing: Option<PricingUpdate>,
    pub review_status: Option<Value>,
    pub taxable: Option<Value>,
    pub at_regime: Option<Value>,
    pub at_category: Option<Value>,
    pub kind: Option<Value>,
    pub source: String,
    pub reason: Option<kassiber_pyjson::PyStr>,
}

pub struct PricingUpdate {
    pub fiat_currency: Value,
    pub fiat_rate: Value,
    pub fiat_value: Value,
    pub source_kind: Value,
    pub quality: Value,
    pub external_ref: Value,
    pub method: Value,
}

impl Default for PricingUpdate {
    fn default() -> Self {
        Self {
            fiat_currency: Value::Null,
            fiat_rate: Value::Null,
            fiat_value: Value::Null,
            source_kind: Value::Null,
            quality: Value::Null,
            external_ref: Value::Null,
            method: Value::Null,
        }
    }
}

fn optional(raw: &Value, field: &str, max: usize) -> Result<Option<String>> {
    if matches!(raw, Value::Null) {
        return Ok(None);
    }
    let text = string(raw)
        .ok_or_else(|| AppError::new("validation", format!("{field} must be a string or null")))?;
    let text = strip(text);
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > max {
        return Err(AppError::new(
            "validation",
            format!("{field} cannot exceed {max} characters"),
        ));
    }
    Ok(Some(text.into()))
}

fn choice(raw: &Value, field: &str, choices: &[&str]) -> Result<Value> {
    supported(optional(raw, field, 64)?, field, choices)
}

fn supported(clean: Option<String>, field: &str, choices: &[&str]) -> Result<Value> {
    let Some(clean) = clean else {
        return Ok(Value::Null);
    };
    if !choices.contains(&clean.as_str()) {
        return Err(
            AppError::new("validation", format!("{field} is not supported"))
                .with_details(dict([(field, s(clean))])),
        );
    }
    Ok(s(clean))
}

fn boolean(raw: &Value, field: &str) -> Result<Value> {
    match raw {
        Value::Null | Value::Bool(_) => Ok(raw.clone()),
        _ => Err(AppError::new(
            "validation",
            format!("{field} must be a boolean"),
        )),
    }
}

fn clean_tags(raw: Option<&Value>) -> Result<Option<Vec<String>>> {
    let raw = match raw {
        None | Some(Value::Null) => return Ok(None),
        Some(raw) => raw,
    };
    let error = || AppError::new("validation", "tags must be a list of strings");
    let Value::List(items) = raw else {
        return Err(error());
    };
    let mut labels = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let label = strip(string(item).ok_or_else(error)?);
        if label.is_empty() {
            continue;
        }
        if label.chars().count() > 128 {
            return Err(
                AppError::new("validation", "tag cannot exceed 128 characters")
                    .with_details(dict([("tag", s(label))])),
            );
        }
        if seen.insert(lower(label)) {
            labels.push(label.to_owned());
        }
    }
    Ok(Some(labels))
}

const SOURCES: &[&str] = &[
    "fmv_provider",
    "manual_rate_cache",
    "generic_import",
    "wallet_export",
    "btcpay_wallet_export",
    "exchange_execution",
    "btcpay_invoice",
    "btcpay_payment",
    "btcpay_payout",
    "manual_override",
];
const QUALITIES: &[&str] = &["exact", "provider_sample", "coarse_fallback", "missing"];
const KINDS: &[&str] = &[
    "airdrop",
    "buy",
    "deposit",
    "hardfork",
    "income",
    "interest",
    "lending_interest",
    "mining",
    "staking",
    "wages",
    "donation",
    "gift",
    "lost",
    "sell",
    "spend",
    "stolen",
    "withdrawal",
];
// kassiber/__init__.py sets the application Decimal context to 32 significant
// digits, including for worker threads. Construction preserves authored scale.
const CONTEXT: Context = Context::new(32, Rounding::HalfEven);

fn decimal(raw: &Value, field: &str) -> Result<Option<Decimal>> {
    if matches!(raw, Value::Null) || *raw == s("") {
        return Ok(None);
    }
    kassiber_pydecimal::finite_price(&py_str(raw))
        .map(Some)
        .map_err(|error| AppError::new("validation", error.message(field)))
}

fn arithmetic(error: DecimalError) -> AppError {
    AppError::new("internal_error", error.to_string())
}

fn pricing(
    conn: &Connection,
    scope: &Scope<'_>,
    tx: &Value,
    input: &PricingUpdate,
) -> Result<Vec<(String, Value)>> {
    let source = optional(&input.source_kind, "pricing_source_kind", 64)?;
    let quality = optional(&input.quality, "pricing_quality", 64)?;
    let source = supported(source, "pricing_source_kind", SOURCES)?;
    let mut quality = supported(quality, "pricing_quality", QUALITIES)?;
    let fallback = if kassiber_envelope::truthy(&value(tx, "fiat_currency")) {
        value(tx, "fiat_currency")
    } else {
        let profile = rows(
            conn,
            "SELECT fiat_currency FROM profiles WHERE id=?",
            [scope.profile_id],
        )?
        .remove(0);
        value(&profile, "fiat_currency")
    };
    let currency = if matches!(input.fiat_currency, Value::Null) || input.fiat_currency == s("") {
        &fallback
    } else {
        &input.fiat_currency
    };
    let currency = string(currency)
        .ok_or_else(|| AppError::new("validation", "fiat_currency must be a string"))?;
    let currency = upper(strip(currency));
    if currency.chars().count() != 3 || !currency.chars().all(|c| is_alpha(c as u32)) {
        return Err(AppError::new(
            "validation",
            "fiat_currency must be a 3-letter currency code",
        ));
    }
    let keys = [
        "fiat_rate",
        "fiat_value",
        "fiat_rate_exact",
        "fiat_value_exact",
        "fiat_price_source",
        "pricing_source_kind",
        "pricing_provider",
        "pricing_pair",
        "pricing_timestamp",
        "pricing_fetched_at",
        "pricing_granularity",
        "pricing_method",
        "pricing_external_ref",
        "pricing_quality",
    ];
    let mut payload: BTreeMap<String, Value> =
        keys.into_iter().map(|k| (k.into(), Value::Null)).collect();
    payload.insert("fiat_currency".into(), s(currency.as_str()));
    if matches!(source, Value::Null) || quality == s("missing") {
        payload.insert("pricing_quality".into(), s("missing"));
        return Ok(payload.into_iter().collect());
    }
    if matches!(quality, Value::Null) {
        quality = s(if source == s("generic_import") {
            "provider_sample"
        } else {
            "exact"
        });
    }
    let mut rate = decimal(&input.fiat_rate, "fiat_rate")?;
    let mut total = decimal(&input.fiat_value, "fiat_value")?;
    let amount = msat(&value(tx, "amount"))?
        .div(&Decimal::from(100_000_000_000i64), &CONTEXT)
        .map_err(arithmetic)?
        .abs(&CONTEXT)
        .map_err(arithmetic)?;
    if amount.is_positive() {
        if rate.is_none() {
            rate = total
                .as_ref()
                .map(|v| v.div(&amount, &CONTEXT))
                .transpose()
                .map_err(arithmetic)?;
        }
        if total.is_none() {
            total = rate
                .as_ref()
                .map(|v| v.mul(&amount, &CONTEXT))
                .transpose()
                .map_err(arithmetic)?;
        }
    }
    if rate.is_none() && total.is_none() {
        return Err(AppError::new(
            "validation",
            "pricing updates require fiat_rate or fiat_value",
        ));
    }
    if rate.as_ref().is_some_and(|n| !n.is_positive()) {
        return Err(AppError::new("validation", "fiat_rate must be positive"));
    }
    if total.as_ref().is_some_and(Decimal::is_negative) {
        return Err(AppError::new(
            "validation",
            "fiat_value must not be negative",
        ));
    }
    let external = optional(&input.external_ref, "pricing_external_ref", 500)?;
    let method = optional(&input.method, "pricing_method", 64)?
        .unwrap_or_else(|| "desktop_transaction_detail".into());
    for (field, number) in [("fiat_rate", rate), ("fiat_value", total)] {
        if let Some(number) = number {
            let exact = number.to_fixed_string().map_err(arithmetic)?;
            let float = kassiber_pytext::parse_float(&exact)
                .map_err(|_| AppError::new("validation", "invalid decimal price"))?;
            payload.insert(field.into(), Value::Float(float));
            payload.insert(format!("{field}_exact"), s(exact));
        }
    }
    let source_name = string(&source).unwrap();
    payload.extend([
        (
            "fiat_price_source".into(),
            s(match source_name {
                "fmv_provider" | "manual_rate_cache" => "rates_cache",
                "manual_override" => "manual",
                _ => "import",
            }),
        ),
        (
            "pricing_provider".into(),
            if source_name == "manual_override" {
                s("manual")
            } else {
                Value::Null
            },
        ),
        (
            "pricing_pair".into(),
            s(format!("{}-{currency}", text(tx, "asset"))),
        ),
        (
            "pricing_timestamp".into(),
            if text(tx, "confirmed_at").is_empty() {
                value(tx, "occurred_at")
            } else {
                value(tx, "confirmed_at")
            },
        ),
        ("pricing_fetched_at".into(), s(now(conn)?)),
        (
            "pricing_granularity".into(),
            if quality == s("exact") {
                s("exact")
            } else {
                Value::Null
            },
        ),
        ("pricing_method".into(), s(method)),
        (
            "pricing_external_ref".into(),
            external.map_or(Value::Null, s),
        ),
        ("pricing_source_kind".into(), source),
        ("pricing_quality".into(), quality),
    ]);
    Ok(payload.into_iter().collect())
}

pub(super) fn update(conn: &Connection, scope: &Scope<'_>, input: &GroupedUpdate) -> Result<Value> {
    let tx = transaction(conn, scope.profile_id, &input.transaction)?;
    let mut updates = BTreeMap::<String, Value>::new();
    if let Some(note) = &input.note {
        if !matches!(note, Value::Null) {
            let note = string(note)
                .ok_or_else(|| AppError::new("validation", "note must be a string or null"))?;
            if note.chars().count() > 20_000 {
                return Err(AppError::new(
                    "validation",
                    "note cannot exceed 20000 characters",
                ));
            }
        }
        updates.insert("note".into(), note.clone());
    }
    let labels = clean_tags(input.tags.as_ref())?;
    if let Some(excluded) = &input.excluded {
        let excluded = boolean(excluded, "excluded")?;
        if excluded == Value::Bool(true) {
            guard_exclusion(conn, scope.profile_id, text(&tx, "id"))?;
        }
        if !matches!(excluded, Value::Null) {
            updates.insert("excluded".into(), excluded);
        }
    }
    if let Some(taxable) = &input.taxable {
        updates.insert("taxability_override".into(), boolean(taxable, "taxable")?);
    }
    for (raw, field, column, options) in [
        (
            &input.review_status,
            "review_status",
            "review_status",
            &["completed", "pending", "failed", "review"][..],
        ),
        (
            &input.at_regime,
            "at_regime",
            "at_regime_override",
            &["alt", "neu", "outside"][..],
        ),
        (
            &input.at_category,
            "at_category",
            "at_category_override",
            &[
                "income_general",
                "income_capital_yield",
                "neu_gain",
                "neu_loss",
                "neu_swap",
                "alt_spekulation",
                "alt_taxfree",
                "none",
            ][..],
        ),
        (&input.kind, "kind", "kind_override", KINDS),
    ] {
        if let Some(raw) = raw {
            updates.insert(column.into(), choice(raw, field, options)?);
        }
    }
    if let Some(kind) = updates.get("kind_override").and_then(string) {
        let direction = if [
            "donation",
            "gift",
            "lost",
            "sell",
            "spend",
            "stolen",
            "withdrawal",
        ]
        .contains(&kind)
        {
            "outbound"
        } else {
            "inbound"
        };
        if text(&tx, "direction") != direction {
            return Err(AppError::new("validation", format!("kind '{kind}' can only be applied to {direction} transactions"))
                .with_hint("Income kinds classify what arrived; disposal kinds classify what left. Check the transaction's direction.")
                .with_details(dict([("kind", s(kind)), ("direction", value(&tx, "direction"))])));
        }
    }
    if let Some(input) = &input.pricing {
        updates.extend(pricing(conn, scope, &tx, input)?);
    }
    let source = history::source(&input.source)?;
    let reason = input
        .reason
        .as_ref()
        .map(kassiber_pytext::strip_py)
        .filter(|reason| reason.code_points().next().is_some());
    if reason
        .as_ref()
        .is_some_and(|reason| reason.code_points().count() > 1000)
    {
        return Err(AppError::new(
            "validation",
            "reason cannot exceed 1000 characters",
        ));
    }
    let mut changes = Vec::new();
    // Audit field order is independent of input JSON and SQL column order.
    for &(field, _, _) in history::FIELDS {
        let column = match field {
            "kind" => "kind_override",
            "taxable" => "taxability_override",
            "at_regime" => "at_regime_override",
            "at_category" => "at_category_override",
            "fiat_rate" => "fiat_rate_exact",
            "fiat_value" => "fiat_value_exact",
            _ => field,
        };
        let after = if field == "tags" {
            labels.as_ref().map(|labels| {
                Value::List(canonical_tags(labels.clone()).into_iter().map(s).collect())
            })
        } else {
            updates.get(column).cloned()
        };
        if let Some(after) = after {
            let before = history::state(conn, &tx, field)?;
            if json(&before)? != json(&after)? {
                changes.push((field, before, after));
            }
        }
    }
    let changed = changes
        .iter()
        .any(|(field, _, _)| *field != "pricing_fetched_at");
    let event = if changed {
        s(atomic(conn, || {
            if changes.iter().any(|(field, _, _)| *field == "tags") {
                let ids = labels
                    .unwrap_or_default()
                    .iter()
                    .map(|label| ensure_tag(conn, scope, label))
                    .collect::<Result<Vec<_>>>()?;
                conn.execute(
                    "DELETE FROM transaction_tags WHERE transaction_id=?",
                    [text(&tx, "id")],
                )
                .map_err(error)?;
                for id in ids {
                    conn.execute(
                        "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                        [text(&tx, "id"), &id],
                    )
                    .map_err(error)?;
                }
            }
            // One statement, as in Python, so row triggers (the chain-index
            // clock among them) fire once per save rather than per column.
            if !updates.is_empty() {
                let assignments = updates
                    .keys()
                    .map(|column| format!("{column}=?"))
                    .collect::<Vec<_>>()
                    .join(",");
                let mut values = updates
                    .values()
                    .map(cursor::bind)
                    .collect::<Result<Vec<_>>>()?;
                values.push(kassiber_store::rusqlite::types::Value::Text(
                    text(&tx, "id").into(),
                ));
                conn.execute(
                    &format!("UPDATE transactions SET {assignments} WHERE id=?"),
                    kassiber_store::rusqlite::params_from_iter(values),
                )
                .map_err(error)?;
            }
            let reason = reason
                .as_ref()
                .map(|reason| {
                    reason.as_str().ok_or_else(|| {
                        AppError::new(
                            "internal_error",
                            kassiber_pytext::utf8_error(reason).unwrap(),
                        )
                    })
                })
                .transpose()?;
            append_fields(conn, scope, &tx, &changes, &source, reason)
        })?)
    } else {
        Value::Null
    };
    let mut result = record(conn, &transaction(conn, scope.profile_id, text(&tx, "id"))?)?;
    if let Value::Dict(fields) = &mut result {
        fields.extend([
            ("updated".into(), Value::Bool(changed)),
            ("history_event_id".into(), event),
        ]);
    }
    Ok(result)
}
