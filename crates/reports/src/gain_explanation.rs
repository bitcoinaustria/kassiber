use super::*;
use kassiber_store::rusqlite::{params, params_from_iter, types::Value as Sql};
use std::collections::{BTreeSet, VecDeque};

const FIELDS: [&str; 3] = ["cost_basis_exact", "proceeds_exact", "gain_loss_exact"];
const PRICING: [&str; 12] = [
    "fiat_currency",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_pair",
    "pricing_provider",
    "pricing_quality",
    "pricing_source_kind",
    "pricing_timestamp",
];

pub(super) fn profile_number(profile: &Value, key: &str) -> Result<Value> {
    let raw = value(profile, key);
    let parsed = if !truthy(raw) {
        0.into()
    } else {
        match raw {
            Value::Int(n) => n.clone(),
            Value::Bool(n) => i64::from(*n).into(),
            Value::Float(n) if n.is_finite() => {
                kassiber_pytext::parse_int(&format!("{:.0}", n.trunc()))
                    .map_err(|_| AppError::new("internal_error", "invalid journal integer"))?
            }
            Value::Float(n) => {
                return Err(AppError::new(
                    "internal_error",
                    if n.is_nan() {
                        "cannot convert float NaN to integer"
                    } else {
                        "cannot convert float infinity to integer"
                    },
                ));
            }
            Value::Str(t) => {
                kassiber_pytext::parse_int(t.as_str().unwrap_or_default()).map_err(|_| {
                    AppError::new(
                        "internal_error",
                        format!(
                            "invalid literal for int() with base 10: {}",
                            kassiber_pytext::repr_py(t)
                        ),
                    )
                })?
            }
            _ => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                        kassiber_envelope::type_name(raw)
                    ),
                ));
            }
        }
    };
    Ok(Value::Int(parsed))
}

pub(super) fn reference(conn: &Connection, profile: &Value, entry: &Value) -> Result<Value> {
    let database: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get(0),
        )
        .ok();
    validate_capital_identity(database.as_deref(), true)?;
    Ok(dict([
        ("database_id", s(database.unwrap())),
        ("workspace_id", value(profile, "workspace_id").clone()),
        ("profile_id", value(profile, "id").clone()),
        (
            "input_version",
            profile_number(profile, "journal_input_version")?,
        ),
        ("processed_at", value(profile, "last_processed_at").clone()),
        ("entry_id", entry.clone()),
    ]))
}

pub(super) fn fresh(conn: &Connection, profile: &Value) -> Result<(bool, bool)> {
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [text(profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let processed_count = profile_number(profile, "last_processed_tx_count")?;
    let input_version = profile_number(profile, "journal_input_version")?;
    let processed_version = profile_number(profile, "last_processed_input_version")?;
    let current = semantics_current(profile)
        && truthy(value(profile, "last_processed_at"))
        && processed_count == Value::Int(active.into())
        && input_version == processed_version;
    Ok((current, active != 0 && !current))
}

pub(super) fn explain(conn: &Connection, profile: &Value, raw: &Value) -> Result<Value> {
    let required = [
        "database_id",
        "workspace_id",
        "profile_id",
        "input_version",
        "processed_at",
        "entry_id",
    ];
    if !matches!(raw, Value::Dict(f) if f.len()==6)
        || required.iter().any(|k| get(raw, k).is_none())
        || !matches!(value(raw, "entry_id"), Value::Str(_))
        || !matches!(value(raw, "input_version"), Value::Int(_))
    {
        return Err(AppError::new(
            "validation",
            "A complete capital-gains result reference is required",
        ));
    }
    let expected = reference(conn, profile, value(raw, "entry_id"))?;
    if required
        .iter()
        .any(|k| value(raw, k) != value(&expected, k))
    {
        return Err(AppError::new(
            "report_explanation_stale",
            "This result belongs to another book or journal version. Reload the report.",
        ));
    }
    kassiber_setup::networks::require_accounting(conn, &text(profile, "id"))?;
    let country = kassiber_pytext::strip(&text(profile, "tax_country")).to_lowercase();
    if !crate::country::known(&country) {
        return Err(AppError::new(
            "internal_error",
            format!("Unsupported tax country '{}'", text(profile, "tax_country")),
        ));
    }
    if !fresh(conn, profile)?.0 {
        return Err(AppError::new(
            "report_explanation_stale",
            "This journal is missing or stale. Rebuild it before explaining the result.",
        ));
    }
    if !components::blockers(conn, profile)?.is_empty()
        || !quantity_issues(conn, &text(profile, "id"))?.is_empty()
    {
        return Err(AppError::new(
            "report_explanation_blocked",
            "Custody review blocks this report result",
        ));
    }
    let row = rows(conn,"SELECT je.* FROM journal_entries je LEFT JOIN transactions t ON t.id=je.transaction_id WHERE je.id=? AND je.profile_id=? AND je.entry_type IN ('disposal','income') AND COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap'",[text(raw,"entry_id"),text(profile,"id")])?.pop().ok_or_else(|| AppError::new("report_explanation_stale", "The capital-gains result is no longer available. Reload the report."))?;
    let mut totals = dict(FIELDS.map(|k| (k, value(&row, k).clone())));
    replace(
        &mut totals,
        "quantity_msat",
        Value::Int(int(&row, "quantity").unsigned_abs().into()),
    );
    let quarantines: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id=?",
            [text(profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let (status, calculation) = retained(&row);
    let mut payload = dict([
        ("reference", raw.clone()),
        ("book_label", value(profile, "label").clone()),
        ("currency", value(profile, "fiat_currency").clone()),
        ("totals", totals),
        ("quarantines", Value::Int(quarantines.into())),
        ("status", s(status)),
        ("calculation", Value::Null),
        ("custody_decisions", Value::List(vec![])),
        ("custody_truncated", Value::Bool(false)),
    ]);
    if let Some(mut calculation) = calculation {
        let mut walk = Inherited {
            conn,
            profile: text(profile, "id"),
            remaining: 100,
            retained: vec![],
            truncated: false,
        };
        walk.visit(&mut calculation, &BTreeSet::new(), 0)?;
        let (mut custody, truncated) =
            custody_context(conn, &text(profile, "id"), &row, &calculation)?;
        for relation in walk.retained {
            if !custody
                .iter()
                .any(|r| value(r, "decision_id") == value(&relation, "decision_id"))
            {
                custody.push(relation);
            }
        }
        replace(&mut payload, "calculation", calculation);
        replace(&mut payload, "custody_decisions", Value::List(custody));
        replace(
            &mut payload,
            "custody_truncated",
            Value::Bool(truncated || walk.truncated),
        );
    }
    Ok(payload)
}

fn required<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    if !matches!(v, Value::Dict(_)) {
        return Err(AppError::new(
            "internal_error",
            match v {
                Value::Str(_) => "string indices must be integers, not 'str'".into(),
                Value::List(_) => "list indices must be integers or slices, not str".into(),
                _ => format!(
                    "'{}' object is not subscriptable",
                    kassiber_envelope::type_name(v)
                ),
            },
        ));
    }
    get(v, key).ok_or_else(|| AppError::new("internal_error", format!("'{key}'")))
}

fn attribute(v: &Value, name: &str) -> AppError {
    AppError::new(
        "internal_error",
        format!(
            "'{}' object has no attribute '{name}'",
            kassiber_envelope::type_name(v)
        ),
    )
}

fn list(v: &Value) -> Option<&[Value]> {
    if let Value::List(v) = v {
        Some(v)
    } else {
        None
    }
}
pub(crate) fn compact(raw: &str) -> Option<Value> {
    let encoded = kassiber_pytext::loads(raw).ok()?;
    let top = list(&encoded)?;
    if top.len() != 5 || !matches!(&top[0], Value::Int(n) if n==&2.into()) {
        return None;
    }
    let sources = list(&top[3])?;
    let source = |index: &Value| -> Option<Value> {
        if matches!(index, Value::Null) {
            return Some(Value::Null);
        }
        let Value::Int(index) = index else {
            return None;
        };
        let index = usize::try_from(index.clone()).ok()?;
        let row = list(sources.get(index)?)?;
        if row.len() != 19 || !matches!(&row[6], Value::Int(_)) {
            return None;
        }
        Some(dict([
            ("asset", row[3].clone()),
            ("crypto_fee_msat", row[6].clone()),
            ("crypto_fee_msat_exact", s(py_str(&row[6]))),
            ("engine_event_id", row[1].clone()),
            ("fiat_fee_exact", row[5].clone()),
            ("occurred_at", row[2].clone()),
            (
                "pricing",
                dict(PRICING.iter().zip(&row[7..]).map(|(k, v)| (*k, v.clone()))),
            ),
            ("spot_price_exact", row[4].clone()),
            ("transaction_id", row[0].clone()),
        ]))
    };
    let mut decoded = vec![];
    for f in list(&top[4])? {
        let f = list(f)?;
        if f.len() != 10
            || !matches!(&f[0], Value::Int(_))
            || !matches!(&f[7], Value::Int(_) | Value::Null)
        {
            return None;
        }
        decoded.push(dict([
            ("acquisition_basis_exact", f[6].clone()),
            ("acquisition_quantity_msat", f[7].clone()),
            (
                "acquisition_quantity_msat_exact",
                if matches!(&f[7], Value::Null) {
                    Value::Null
                } else {
                    s(py_str(&f[7]))
                },
            ),
            ("basis_authority", f[5].clone()),
            ("cost_basis_exact", f[1].clone()),
            ("event", source(&f[8])?),
            ("gain_loss_exact", f[3].clone()),
            ("lot", source(&f[9])?),
            ("proceeds_exact", f[2].clone()),
            ("quantity_msat", f[0].clone()),
            ("quantity_msat_exact", s(py_str(&f[0]))),
            ("unit_basis_override_exact", f[4].clone()),
        ]));
    }
    Some(dict([
        ("engine", top[1].clone()),
        ("fragments", Value::List(decoded)),
        ("method", top[2].clone()),
        ("schema_version", Value::Int(1.into())),
    ]))
}
/// A journal entry's calculation metadata, from its compact or JSON form.
pub(crate) fn calculation(row: &Value) -> Option<Value> {
    if truthy(value(row, "calculation_compact")) {
        compact(&text(row, "calculation_compact"))
    } else if truthy(value(row, "calculation_json")) {
        kassiber_pytext::loads(&text(row, "calculation_json")).ok()
    } else {
        None
    }
}
fn retained(row: &Value) -> (&'static str, Option<Value>) {
    if (!truthy(value(row, "calculation_json")) && !truthy(value(row, "calculation_compact")))
        || FIELDS.iter().any(|k| matches!(value(row, k), Value::Null))
    {
        return ("engine_detail_unavailable", None);
    }
    let calc = if truthy(value(row, "calculation_compact")) {
        compact(&text(row, "calculation_compact"))
    } else {
        kassiber_pytext::loads(&text(row, "calculation_json")).ok()
    };
    let valid = || -> Option<Value> {
        let calc = calc?;
        let fragments = list(value(&calc, "fragments"))?;
        // A present lot and its pricing are mappings before basis is traced
        // through them (7c7fcc0a8); a fragment that is not a mapping fails
        // the field reads below.
        for f in fragments {
            match get(f, "lot") {
                None | Some(Value::Null) => {}
                Some(lot @ Value::Dict(_)) => {
                    if !matches!(get(lot, "pricing"), None | Some(Value::Dict(_))) {
                        return None;
                    }
                }
                Some(_) => return None,
            }
        }
        for field in FIELDS {
            let mut sum = Decimal::zero();
            for f in fragments {
                sum = sum.add(&strict_decimal(get(f, field)?)?, &DECIMAL).ok()?;
            }
            if sum != strict_decimal(value(row, field))? {
                return None;
            }
        }
        let mut quantity = kassiber_pytext::parse_int("0").ok()?;
        let mut floating = None;
        for f in fragments {
            match get(f, "quantity_msat")? {
                Value::Float(n) => {
                    floating = Some(
                        floating.unwrap_or_else(|| {
                            quantity.to_string().parse::<f64>().unwrap_or(f64::INFINITY)
                        }) + n,
                    )
                }
                Value::Int(n) => {
                    if let Some(sum) = &mut floating {
                        *sum += n.to_string().parse::<f64>().unwrap_or(f64::INFINITY);
                    } else {
                        quantity += n;
                    }
                }
                Value::Bool(n) => {
                    if let Some(sum) = &mut floating {
                        *sum += if *n { 1.0 } else { 0.0 };
                    } else {
                        quantity += i64::from(*n);
                    }
                }
                _ => return None,
            }
        }
        if let Some(sum) = floating {
            if !sum.is_finite() || sum.fract() != 0.0 {
                return None;
            }
            quantity = kassiber_pytext::parse_int(&format!("{sum:.0}")).ok()?;
        }
        if quantity != int(row, "quantity").unsigned_abs().into() {
            return None;
        }
        Some(calc)
    };
    match valid() {
        Some(calc) => ("available", Some(calc)),
        None => ("engine_detail_mismatch", None),
    }
}
fn strict_decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Null => None,
        Value::Bool(n) => Some(Decimal::from(i64::from(*n))),
        Value::Float(n) => {
            if !n.is_finite() {
                return None;
            }
            let bits = n.to_bits();
            let power = ((bits >> 52) & 0x7ff) as i32;
            let mantissa = bits & ((1_u64 << 52) - 1);
            let (mantissa, exponent) = if power == 0 {
                (mantissa, -1074)
            } else {
                (mantissa | (1_u64 << 52), power - 1075)
            };
            let mut coefficient = kassiber_pyjson::BigInt::from(mantissa);
            if exponent >= 0 {
                coefficient <<= exponent as usize;
            } else {
                coefficient *= kassiber_pyjson::BigInt::from(5).pow((-exponent) as u32);
            }
            Decimal::from_parts(
                n.is_sign_negative(),
                coefficient.magnitude().clone(),
                i64::from(exponent.min(0)),
            )
            .ok()
        }
        _ => kassiber_pytext::strip(&py_str(v)).parse().ok(),
    }
}

struct Inherited<'a> {
    conn: &'a Connection,
    profile: String,
    remaining: usize,
    retained: Vec<Value>,
    truncated: bool,
}
impl Inherited<'_> {
    fn visit(
        &mut self,
        calculation: &mut Value,
        ancestors: &BTreeSet<String>,
        depth: usize,
    ) -> Result<()> {
        let Value::Dict(fields) = calculation else {
            return Ok(());
        };
        let Some((_, Value::List(fragments))) =
            fields.iter_mut().find(|(k, _)| *k == "fragments".into())
        else {
            return Ok(());
        };
        for f in fragments {
            let Value::Dict(fields) = f else {
                continue;
            };
            let Some((_, lot)) = fields.iter_mut().find(|(k, _)| *k == "lot".into()) else {
                continue;
            };
            if !truthy(lot) {
                continue;
            }
            if !matches!(lot, Value::Dict(_)) {
                return Err(attribute(lot, "get"));
            }
            if let Some(pricing) = get(lot, "pricing") {
                if !matches!(pricing, Value::Dict(_)) {
                    return Err(attribute(pricing, "get"));
                }
            }
            if value(value(lot, "pricing"), "pricing_method") != &s("carrying_value") {
                continue;
            }
            let mut detail = dict([
                ("status", s("engine_detail_unavailable")),
                ("relations", Value::List(vec![])),
                ("source_calculations", Value::List(vec![])),
            ]);
            let target = text(lot, "transaction_id");
            if ancestors.contains(&target) || depth >= 8 || self.remaining == 0 {
                replace(&mut detail, "status", s("inherited_detail_truncated"));
                self.truncated = true;
            } else {
                let relations=rows(self.conn,"SELECT id AS decision_id,relation_kind AS custody_state,kind AS reason,policy,basis_state,component_id,out_transaction_id AS source_transaction_id,in_transaction_id AS target_transaction_id,out_asset AS source_asset,in_asset AS target_asset,CAST(out_amount AS TEXT) AS source_quantity_msat_exact,CAST(in_amount AS TEXT) AS target_quantity_msat_exact,CAST(swap_fee_msat AS TEXT) AS swap_fee_msat_exact,occurred_at,target_occurred_at FROM journal_custody_projection_relations WHERE profile_id=? AND in_transaction_id=? AND in_asset=? AND policy='carrying-value' AND basis_state='eligible' ORDER BY occurred_at,id LIMIT ?",params![self.profile, value_sql(value(lot,"transaction_id")), value_sql(required(lot,"asset")?),(self.remaining+1) as i64])?;
                let mut clipped = relations.len() > self.remaining;
                let mut sources = BTreeSet::new();
                let mut retained_relations = vec![];
                let mut calculations = vec![];
                for relation in relations.into_iter().take(self.remaining) {
                    if self.remaining == 0 {
                        clipped = true;
                        break;
                    }
                    self.remaining -= 1;
                    retain_ordered(&mut self.retained, relation.clone());
                    retained_relations.push(relation.clone());
                    let source = (
                        text(&relation, "source_transaction_id"),
                        text(&relation, "source_asset"),
                    );
                    if !sources.insert(source) {
                        continue;
                    }
                    let entries=rows(self.conn,"SELECT * FROM journal_entries WHERE profile_id=? AND transaction_id=? AND asset=? AND entry_type='disposal' ORDER BY occurred_at,id LIMIT ?",params![self.profile,value_sql(value(&relation,"source_transaction_id")),value_sql(value(&relation,"source_asset")),(self.remaining+1) as i64])?;
                    clipped |= entries.len() > self.remaining;
                    if entries.is_empty() {
                        calculations.push(dict([
                            (
                                "transaction_id",
                                value(&relation, "source_transaction_id").clone(),
                            ),
                            ("asset", value(&relation, "source_asset").clone()),
                            ("scope", s("whole_source_disposal")),
                            ("status", s("engine_detail_unavailable")),
                            ("calculation", Value::Null),
                            ("totals", Value::Null),
                        ]));
                    }
                    for row in entries.into_iter().take(self.remaining) {
                        if self.remaining == 0 {
                            clipped = true;
                            break;
                        }
                        self.remaining -= 1;
                        let (status, inherited) = retained(&row);
                        let mut inherited = inherited.unwrap_or(Value::Null);
                        if !matches!(inherited, Value::Null) {
                            let mut next = ancestors.clone();
                            next.insert(target.clone());
                            self.visit(&mut inherited, &next, depth + 1)?;
                        }
                        let mut totals = dict(FIELDS.map(|k| (k, value(&row, k).clone())));
                        replace(
                            &mut totals,
                            "quantity_msat_exact",
                            s(int(&row, "quantity").unsigned_abs().to_string()),
                        );
                        calculations.push(dict([
                            ("entry_id", value(&row, "id").clone()),
                            ("transaction_id", value(&row, "transaction_id").clone()),
                            ("asset", value(&row, "asset").clone()),
                            ("scope", s("whole_source_disposal")),
                            ("status", s(status)),
                            ("calculation", inherited),
                            ("totals", totals),
                        ]));
                    }
                }
                if clipped {
                    self.truncated = true;
                    replace(&mut detail, "status", s("inherited_detail_truncated"));
                } else if !calculations.is_empty() {
                    let status = if calculations
                        .iter()
                        .all(|c| value(c, "status") == &s("available"))
                    {
                        "available"
                    } else if calculations
                        .iter()
                        .any(|c| value(c, "status") == &s("engine_detail_mismatch"))
                    {
                        "engine_detail_mismatch"
                    } else {
                        "engine_detail_unavailable"
                    };
                    replace(&mut detail, "status", s(status));
                }
                replace(&mut detail, "relations", Value::List(retained_relations));
                replace(
                    &mut detail,
                    "source_calculations",
                    Value::List(calculations),
                );
            }
            replace(lot, "inherited_basis", detail);
        }
        Ok(())
    }
}
fn value_sql(v: &Value) -> Sql {
    if matches!(v, Value::Null) {
        Sql::Null
    } else {
        Sql::Text(py_str(v))
    }
}
fn retain_ordered(retained: &mut Vec<Value>, row: Value) {
    if let Some(slot) = retained
        .iter_mut()
        .find(|r| value(r, "decision_id") == value(&row, "decision_id"))
    {
        *slot = row;
    } else {
        retained.push(row);
    }
}
fn instant(v: &Value) -> Option<i64> {
    if !truthy(v) {
        None
    } else {
        kassiber_rates::time::parse(kassiber_pytext::strip(&py_str(v)))
    }
}
fn custody_context(
    conn: &Connection,
    profile: &str,
    result: &Value,
    calculation: &Value,
) -> Result<(Vec<Value>, bool)> {
    let mut starts = vec![];
    let mut ids = BTreeSet::new();
    let fragments = list(value(calculation, "fragments")).unwrap_or_default();
    for f in fragments {
        for key in ["event", "lot"] {
            let source = required(f, key)?;
            if truthy(source) {
                let tx = required(source, "transaction_id")?;
                if truthy(tx) {
                    ids.insert(py_str(tx));
                }
            }
        }
    }
    for f in fragments {
        for key in ["event", "lot"] {
            let source = required(f, key)?;
            if truthy(source) {
                if let Some(t) = instant(required(source, "occurred_at")?) {
                    starts.push(t);
                }
            }
        }
    }
    let Some(cutoff) = instant(value(result, "occurred_at")) else {
        return Ok((vec![], true));
    };
    let Some(earliest) = starts.into_iter().min() else {
        return Ok((vec![], true));
    };
    let mut pending = VecDeque::from([(text(result, "wallet_id"), text(result, "asset"), cutoff)]);
    let mut visited = std::collections::BTreeMap::new();
    let mut retained = vec![];
    let mut budget = 500usize;
    let mut truncated = false;
    let mut direct = true;
    while budget > 0 && !pending.is_empty() {
        let (wallet, asset, upper) = pending.pop_front().unwrap();
        if visited
            .get(&(wallet.clone(), asset.clone()))
            .is_some_and(|t| *t >= upper)
        {
            continue;
        }
        visited.insert((wallet.clone(), asset.clone()), upper);
        let mut predicate = "d.target_wallet_id=? AND d.target_asset=?".to_owned();
        let mut params = vec![
            Sql::Text(profile.into()),
            Sql::Text(wallet),
            Sql::Text(asset),
        ];
        if direct && !ids.is_empty() {
            let marks = vec!["?"; ids.len()].join(",");
            predicate=format!("({predicate}) OR d.source_transaction_id IN ({marks}) OR d.target_transaction_id IN ({marks})");
            for _ in 0..2 {
                params.extend(ids.iter().cloned().map(Sql::Text));
            }
        }
        direct = false;
        params.extend([
            Sql::Text(kassiber_rates::time::format(earliest).unwrap()),
            Sql::Text(kassiber_rates::time::format(upper).unwrap()),
            Sql::Integer((budget + 1) as i64),
        ]);
        let selected=rows(conn,&format!("SELECT d.decision_id,d.source_transaction_id,d.target_transaction_id,d.source_wallet_id,d.target_wallet_id,d.source_asset,d.target_asset,d.state AS custody_state,d.basis_state,d.reason,d.component_id,d.occurred_at,d.target_occurred_at FROM journal_custody_decisions d WHERE d.profile_id=? AND ({predicate}) AND julianday(COALESCE(d.target_occurred_at,d.occurred_at))>=julianday(?) AND julianday(COALESCE(d.target_occurred_at,d.occurred_at))<=julianday(?) ORDER BY julianday(COALESCE(d.target_occurred_at,d.occurred_at)) DESC,d.decision_id LIMIT ?"),params_from_iter(params))?;
        truncated |= selected.len() > budget;
        for row in selected.into_iter().take(budget) {
            budget -= 1;
            let source = instant(value(&row, "occurred_at"));
            let target = instant(if truthy(value(&row, "target_occurred_at")) {
                value(&row, "target_occurred_at")
            } else {
                value(&row, "occurred_at")
            });
            let (Some(source), Some(target)) = (source, target) else {
                truncated = true;
                continue;
            };
            if !(earliest..=upper).contains(&target) {
                continue;
            }
            if value(&row, "basis_state") == &s("eligible") && (earliest..=upper).contains(&source)
            {
                pending.push_back((
                    text(&row, "source_wallet_id"),
                    text(&row, "source_asset"),
                    source,
                ));
            }
            retain_ordered(&mut retained, row);
        }
    }
    Ok((retained, truncated || !pending.is_empty()))
}
