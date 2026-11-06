//! Annual Austrian mappings over stored categories; no tax interpretation.
use super::*;
use std::sync::OnceLock;

pub fn metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| kassiber_pytext::loads(include_str!("austrian_metadata.json")).unwrap())
}
fn constant(key: &str) -> Value {
    value(metadata(), key).clone()
}
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        unreachable!()
    }
}
pub fn integer(d: &Decimal) -> Result<Value> {
    Ok(Value::Int(
        kassiber_pytext::parse_int(&d.to_fixed_places(0, Rounding::Down).map_err(arithmetic)?)
            .map_err(|_| AppError::new("internal_error", "invalid integer"))?,
    ))
}
fn cents(d: &Decimal) -> Result<Value> {
    integer(
        &d.quantize(-2, &Context::new(32, Rounding::HalfUp))
            .map_err(arithmetic)?
            .mul(&Decimal::from(100), &DECIMAL)
            .map_err(arithmetic)?,
    )
}
fn add_integer(row: &mut Value, key: &str, amount: &Value) {
    let Value::Int(mut n) = value(row, key).clone() else {
        unreachable!()
    };
    let Value::Int(amount) = amount else {
        unreachable!()
    };
    n += amount;
    replace(row, key, Value::Int(n));
}
fn zero() -> Value {
    Value::Int(0.into())
}
fn mapping(category: &str) -> Option<i64> {
    match category {
        "income_general" | "income_capital_yield" => Some(172),
        "neu_gain" => Some(174),
        "neu_loss" => Some(176),
        "alt_spekulation" => Some(801),
        _ => None,
    }
}
fn section_id(category: &str) -> Option<&'static str> {
    match category {
        "neu_gain" | "neu_loss" | "neu_swap" => Some("1.1"),
        "alt_spekulation" => Some("1.3"),
        "income_capital_yield" => Some("2.1"),
        "income_general" => Some("2.2"),
        "alt_taxfree" => Some("3.1"),
        _ => None,
    }
}
fn form(code: Option<i64>, section: bool) -> Value {
    code.map(|code| {
        value(
            value(
                metadata(),
                if section {
                    "AUSTRIAN_KENNZAHL_FORM_SECTION_BY_CODE"
                } else {
                    "AUSTRIAN_KENNZAHL_FORM_BY_CODE"
                },
            ),
            &code.to_string(),
        )
        .clone()
    })
    .unwrap_or_else(|| s(""))
}
pub fn validate(profile: &Value) -> Result<()> {
    if kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase() != "at" {
        return Err(AppError::new("validation", "Austrian E 1kv export requires an Austrian tax profile")
            .with_hint("Use `profiles set --tax-country at --fiat-currency EUR` and re-run `journals process` first.")
            .with_details(dict([("tax_country", value(profile, "tax_country").clone())])));
    }
    if kassiber_pytext::strip(&text(profile, "fiat_currency")).to_uppercase() != "EUR" {
        return Err(AppError::new(
            "validation",
            "Austrian E 1kv export requires EUR journal amounts",
        )
        .with_hint(
            "Use an Austrian profile with --fiat-currency EUR and re-run `journals process`.",
        )
        .with_details(dict([(
            "fiat_currency",
            value(profile, "fiat_currency").clone(),
        )])));
    }
    Ok(())
}
fn year(args: &Value) -> Result<i64> {
    if matches!(value(args, "year"), Value::Null) {
        return Err(AppError::new(
            "validation",
            "--year is required for Austrian E 1kv export",
        ));
    }
    normalize_tax_year(value(args, "year"))
}

pub(super) fn normalize_tax_year(raw: &Value) -> Result<i64> {
    let year = match raw {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(n) => Some(i64::from(*n).into()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).ok()
        }
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Value::Str(n) => n.as_str().and_then(|n| kassiber_pytext::parse_int(n).ok()),
        _ => None,
    }
    .ok_or_else(|| AppError::new("validation", "--year must be a four-digit tax year"))?;
    if !(2009.into()..=2100.into()).contains(&year) {
        return Err(AppError::new(
            "validation",
            "--year must be a plausible four-digit tax year",
        ));
    }
    Ok(i64::try_from(year).unwrap())
}
fn detail(row: Value) -> Result<Value> {
    let category = text(&row, "at_category");
    let labels = value(metadata(), "AUSTRIAN_E1KV_CATEGORY_LABELS");
    if get(labels, &category).is_none() {
        return Err(AppError::new(
            "internal",
            "Austrian E 1kv export encountered an unsupported journal category",
        )
        .with_hint("Re-run `journals process` with the current Kassiber version.")
        .with_details(dict([("at_category", s(category))])));
    }
    let code = mapping(&category);
    let quantity_msat = int(&row, "quantity").unsigned_abs();
    let quantity = decimal(&s(quantity_msat.to_string()))?
        .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
        .map_err(arithmetic)?;
    let proceeds = decimal(value(&row, "proceeds"))?;
    let basis = decimal(value(&row, "cost_basis"))?;
    let gain = decimal(value(&row, "gain_loss"))?;
    let income = if text(&row, "entry_type") == "income" {
        gain.clone()
    } else {
        Decimal::zero()
    };
    let price = if quantity.is_zero() {
        Value::Null
    } else {
        cents(
            &if text(&row, "entry_type") == "income" {
                &income
            } else {
                &proceeds
            }
            .div(&quantity, &DECIMAL)
            .map_err(arithmetic)?,
        )?
    };
    let amount = if text(&row, "entry_type") == "income" {
        gain.clone()
    } else if code == Some(176) {
        gain.abs(&DECIMAL).map_err(arithmetic)?
    } else if matches!(code, Some(172 | 174 | 801)) {
        gain.clone()
    } else {
        Decimal::zero()
    };
    let (year, date) = calendar::vienna(&text(&row, "occurred_at"))?;
    let fallback = |a, b| {
        if truthy(value(&row, a)) {
            value(&row, a).clone()
        } else {
            value(&row, b).clone()
        }
    };
    Ok(dict([
        ("tax_year", Value::Int(year.into())),
        ("date", s(date)),
        (
            "tx_id",
            fallback("transaction_external_id", "transaction_id"),
        ),
        ("transaction_id", value(&row, "transaction_id").clone()),
        ("wallet", value(&row, "wallet").clone()),
        ("asset", value(&row, "asset").clone()),
        ("kind", fallback("transaction_kind", "entry_type")),
        ("entry_type", value(&row, "entry_type").clone()),
        ("at_category", s(category.as_str())),
        ("at_category_label", value(labels, &category).clone()),
        (
            "at_regime",
            s(if category.starts_with("neu_") {
                "neu"
            } else if category.starts_with("alt_") {
                "alt"
            } else if category.starts_with("income_") {
                "income"
            } else {
                ""
            }),
        ),
        ("qty_msat", Value::Int(quantity_msat.into())),
        ("quantity", float(&quantity)?),
        ("price_eur_cents", price),
        ("cost_basis_eur_cents", cents(&basis)?),
        ("proceeds_eur_cents", cents(&proceeds)?),
        ("gain_loss_eur_cents", cents(&gain)?),
        ("income_eur_cents", cents(&income)?),
        ("form_amount_eur_cents", cents(&amount)?),
        ("holding_period_days", Value::Null),
        (
            "kennzahl",
            code.map(|v| Value::Int(v.into())).unwrap_or(Value::Null),
        ),
        ("stored_kennzahl", value(&row, "at_kennzahl").clone()),
        ("form", form(code, false)),
        ("form_section", form(code, true)),
        (
            "note",
            if truthy(value(&row, "transaction_note")) {
                value(&row, "transaction_note").clone()
            } else {
                value(&row, "description").clone()
            },
        ),
    ]))
}
/// `_austrian_e1kv_rows`: the year's classified rows, in journal order.
pub(crate) fn e1kv_rows(conn: &Connection, profile: &Value, year: i64) -> Result<Vec<Value>> {
    let (start, end) = calendar::window(year);
    let id = text(profile, "id");
    let raw = rows(conn,"SELECT je.occurred_at,je.transaction_id,je.entry_type,je.asset,je.quantity,COALESCE(je.cost_basis,0) AS cost_basis,COALESCE(je.proceeds,0) AS proceeds,COALESCE(je.gain_loss,0) AS gain_loss,COALESCE(je.description,'') AS description,je.at_category,je.at_kennzahl,w.label AS wallet,t.external_id AS transaction_external_id,COALESCE(NULLIF(t.kind_override,''),t.kind) AS transaction_kind,t.note AS transaction_note FROM journal_entries je JOIN wallets w ON w.id=je.wallet_id LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.profile_id=? AND je.at_category IS NOT NULL AND je.at_category!='neu_swap' AND je.entry_type NOT IN ('fee','transfer_fee') AND COALESCE(t.taxability_override,1)!=0 AND je.occurred_at>=? AND je.occurred_at<? ORDER BY je.occurred_at ASC,je.created_at ASC,je.id ASC",[&id,&start,&end])?;
    let mut details = Vec::new();
    for row in raw {
        if calendar::vienna(&text(&row, "occurred_at"))?.0 == year {
            details.push(detail(row)?);
        }
    }
    Ok(details)
}
pub(crate) fn kennzahl_for_category(category: &str) -> Option<i64> {
    mapping(category)
}
pub fn report(
    conn: &Connection,
    workspace: &Value,
    profile: &Value,
    args: &Value,
) -> Result<Value> {
    let year = year(args)?;
    let (start, end) = calendar::window(year);
    let id = text(profile, "id");
    let details = e1kv_rows(conn, profile, year)?;
    let mut summaries = Vec::new();
    let mut totals = dict::<&str>([]);
    for code in [172, 174, 176, 801] {
        let mut summary = dict([
            ("kennzahl", Value::Int(code.into())),
            (
                "label",
                value(
                    value(metadata(), "AUSTRIAN_E1KV_KENNZAHL_LABELS"),
                    &code.to_string(),
                )
                .clone(),
            ),
            ("form", form(Some(code), false)),
            ("form_section", form(Some(code), true)),
            ("row_count", zero()),
            ("amount_eur_cents", zero()),
        ]);
        for row in &details {
            if value(row, "kennzahl") == &Value::Int(code.into()) {
                add_integer(&mut summary, "row_count", &Value::Int(1.into()));
                add_integer(
                    &mut summary,
                    "amount_eur_cents",
                    value(row, "form_amount_eur_cents"),
                );
            }
        }
        let Value::Dict(fields) = &summary else {
            unreachable!()
        };
        replace(
            &mut totals,
            &code.to_string(),
            Value::Dict(
                fields
                    .iter()
                    .filter(|(k, _)| *k != "kennzahl".into())
                    .cloned()
                    .collect(),
            ),
        );
        summaries.push(summary);
    }
    let mut sections = dict::<&str>([]);
    for section in list(value(metadata(), "AUSTRIAN_TAX_SECTION_ORDER")) {
        let section = py_str(section);
        let meta = value(value(metadata(), "AUSTRIAN_TAX_SECTION_METADATA"), &section);
        let mut section_totals = dict([
            ("row_count", zero()),
            ("quantity_msat", zero()),
            ("amount_eur_cents", zero()),
            ("proceeds_eur_cents", zero()),
            ("cost_basis_eur_cents", zero()),
            ("gain_loss_eur_cents", zero()),
            ("income_eur_cents", zero()),
        ]);
        let mut section_rows = Vec::new();
        for row in &details {
            if section_id(&text(row, "at_category")) == Some(section.as_str()) {
                add_integer(&mut section_totals, "row_count", &Value::Int(1.into()));
                for (dest, source) in [
                    ("quantity_msat", "qty_msat"),
                    ("amount_eur_cents", "form_amount_eur_cents"),
                    ("proceeds_eur_cents", "proceeds_eur_cents"),
                    ("cost_basis_eur_cents", "cost_basis_eur_cents"),
                    ("gain_loss_eur_cents", "gain_loss_eur_cents"),
                    ("income_eur_cents", "income_eur_cents"),
                ] {
                    add_integer(&mut section_totals, dest, value(row, source));
                }
                section_rows.push(row.clone());
            }
        }
        replace(
            &mut sections,
            &section,
            dict([
                ("section_id", s(section.as_str())),
                ("label", value(meta, "label").clone()),
                ("law", value(meta, "law").clone()),
                ("supported", value(meta, "supported").clone()),
                (
                    "status",
                    s(if truthy(value(meta, "supported")) {
                        "supported"
                    } else {
                        "not_modelled"
                    }),
                ),
                ("kennzahlen", value(meta, "kennzahlen").clone()),
                ("totals", section_totals),
                ("detail_rows", Value::List(section_rows)),
            ]),
        );
    }
    let mut quarantines = std::collections::BTreeMap::<String, i64>::new();
    for row in rows(conn,"SELECT jq.reason,t.occurred_at FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id WHERE jq.profile_id=? AND t.occurred_at>=? AND t.occurred_at<?",[&id,&start,&end])? {
        if calendar::vienna(&text(&row,"occurred_at"))?.0 == year { *quarantines.entry(if truthy(value(&row,"reason")) {text(&row,"reason")} else {String::new()}).or_default() += 1; }
    }
    let mut quarantines: Vec<_> = quarantines.into_iter().collect();
    quarantines.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mismatches = details
        .iter()
        .filter(|row| {
            !matches!(value(row, "stored_kennzahl"), Value::Null)
                && !matches!(value(row, "kennzahl"), Value::Null)
                && decimal(value(row, "stored_kennzahl")).ok()
                    != decimal(value(row, "kennzahl")).ok()
        })
        .map(|row| {
            dict([
                ("tx_id", value(row, "tx_id").clone()),
                ("at_category", value(row, "at_category").clone()),
                ("stored_kennzahl", value(row, "stored_kennzahl").clone()),
                ("export_kennzahl", value(row, "kennzahl").clone()),
            ])
        })
        .collect();
    let scope = if truthy(value(profile, "cost_basis_pool_scope")) {
        text(profile, "cost_basis_pool_scope")
    } else {
        "global".into()
    };
    let assumption = |code: &str, message: Value| {
        dict([
            ("code", s(code)),
            ("severity", s("review")),
            ("message", message),
        ])
    };
    let mut assumptions = vec![
        assumption(
            "AT-E1KV-FOREIGN-SELF-CUSTODY",
            constant("AUSTRIAN_E1KV_SELF_CUSTODY_ASSUMPTION"),
        ),
        assumption(
            "AT-E1KV-DETAIL-LIMITATION",
            constant("AUSTRIAN_E1KV_DETAIL_LIMITATION"),
        ),
        assumption(
            "AT-E1KV-KENNZAHL-REPROCESS",
            constant("AUSTRIAN_E1KV_REPROCESS_HINT"),
        ),
        assumption(
            "AT-COST-BASIS-POOL-SCOPE",
            s(format!(
                "Austrian cost basis was calculated using the reviewed '{scope}' pool scope."
            )),
        ),
    ];
    if details
        .iter()
        .any(|r| text(r, "asset").to_uppercase() == "LBTC")
    {
        assumptions.push(assumption(
            "AT-002",
            s("L-BTC is treated as Kryptowaehrung like BTC for this report period."),
        ));
    }
    if details
        .iter()
        .any(|r| text(r, "kind").to_lowercase() == "routing_income")
    {
        assumptions.push(assumption(
            "AT-001",
            s("Lightning routing fees are treated as laufende Einkuenfte at fair market value."),
        ));
    }
    assumptions.push(assumption(
        "AT-REVIEW-GATE",
        constant("AUSTRIAN_E1KV_REVIEW_GATE"),
    ));
    Ok(dict([
        ("workspace", value(workspace, "label").clone()),
        ("profile", value(profile, "label").clone()),
        ("tax_year", Value::Int(year.into())),
        ("fiat_currency", value(profile, "fiat_currency").clone()),
        ("tax_country", value(profile, "tax_country").clone()),
        ("cost_basis_pool_scope", s(scope)),
        ("form", s("E 1kv")),
        ("form_section", constant("AUSTRIAN_E1KV_FORM_SECTION")),
        ("review_gate", constant("AUSTRIAN_E1KV_REVIEW_GATE")),
        ("assumptions", Value::List(assumptions)),
        ("summary_rows", Value::List(summaries)),
        ("kennzahl_totals", totals),
        ("section_order", constant("AUSTRIAN_TAX_SECTION_ORDER")),
        ("sections", sections),
        ("rows", Value::List(details)),
        (
            "data_quality",
            dict([
                (
                    "quarantines",
                    Value::List(
                        quarantines
                            .into_iter()
                            .map(|(reason, count)| {
                                dict([("reason", s(reason)), ("count", Value::Int(count.into()))])
                            })
                            .collect(),
                    ),
                ),
                ("kennzahl_mismatches", Value::List(mismatches)),
            ]),
        ),
    ]))
}
