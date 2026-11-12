//! Lists project stored authored terms selected by persisted effective memberships.
use super::*;
use kassiber_envelope::s;
use std::collections::{BTreeMap, BTreeSet};

fn terms(conn: &Connection, scope: &Scope<'_>, kind: &str) -> Result<Vec<Value>> {
    terms_with_deleted(conn, scope, kind, false)
}
fn terms_with_deleted(
    conn: &Connection,
    scope: &Scope<'_>,
    kind: &str,
    deleted: bool,
) -> Result<Vec<Value>> {
    rows(conn, "SELECT term.*, c.superseded_at,
        COALESCE(source.anchor_transaction_id,source.transaction_id) AS out_transaction_id,
        COALESCE(target.anchor_transaction_id,target.transaction_id) AS in_transaction_id,
        (SELECT SUM(a.source_amount_msat) FROM custody_component_allocations a
         WHERE a.component_id=term.component_id AND a.source_leg_id=term.source_leg_id AND a.sink_leg_id=term.target_leg_id) AS allocated_source_amount_msat,
        (SELECT SUM(a.sink_amount_msat) FROM custody_component_allocations a
         WHERE a.component_id=term.component_id AND a.source_leg_id=term.source_leg_id AND a.sink_leg_id=term.target_leg_id) AS allocated_sink_amount_msat
        FROM custody_component_economic_terms term
        JOIN custody_components c ON c.id=term.component_id
        JOIN custody_component_legs source ON source.id=term.source_leg_id
        JOIN custody_component_legs target ON target.id=term.target_leg_id
        WHERE term.profile_id=? AND term.term_kind=? AND (term.component_id IN
          (SELECT DISTINCT component_id FROM custody_component_transaction_memberships WHERE profile_id=?)
          OR (? AND c.state='superseded' AND NOT EXISTS (SELECT 1 FROM custody_components successor WHERE successor.supersedes_component_id=c.id)))
        ORDER BY term.created_at DESC,term.component_id,term.ordinal", kassiber_store::rusqlite::params![scope.profile_id,kind,scope.profile_id,deleted])
}
pub(super) fn pair_records(
    conn: &Connection,
    scope: &Scope<'_>,
    deleted: bool,
) -> Result<Vec<Value>> {
    let contexts = contexts(conn, scope)?;
    let mut result = BTreeMap::new();
    for row in terms_with_deleted(conn, scope, "transaction_pair", deleted)? {
        let mut pair = common(&row, false)?;
        append(
            &mut pair,
            "out_asset",
            value(context(&contexts, &row, "out_transaction_id")?, "asset"),
        );
        append(
            &mut pair,
            "in_asset",
            value(context(&contexts, &row, "in_transaction_id")?, "asset"),
        );
        result.insert(text(&pair, "id"), pair);
    }
    if let Value::List(rows) = sorted(result) {
        Ok(rows)
    } else {
        unreachable!()
    }
}
/// `custody_authored_migration.list_pair_review_records` for current terms:
/// one flat record per pair review, newest first.
pub fn pair_review_records(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
) -> Result<Vec<Value>> {
    let scope = Scope {
        workspace_id,
        profile_id,
    };
    let contexts = contexts(conn, &scope)?;
    let int = |value: &Value| -> Result<Value> {
        match value {
            Value::Int(_) => Ok(value.clone()),
            other => Err(AppError::new(
                "internal_error",
                format!(
                    "int() argument is not an integer: {}",
                    kassiber_envelope::py_repr(other)
                ),
            )),
        }
    };
    let mut result = BTreeMap::new();
    for row in terms(conn, &scope, "transaction_pair")? {
        let source = context(&contexts, &row, "out_transaction_id")?;
        let target = context(&contexts, &row, "in_transaction_id")?;
        let reviewed = value(&row, "reviewed_source_amount_msat");
        let allocated_out = value(&row, "allocated_source_amount_msat");
        let allocated_in = value(&row, "allocated_sink_amount_msat");
        let out_amount = if !matches!(allocated_out, Value::Null) {
            int(&allocated_out)?
        } else if !matches!(reviewed, Value::Null) {
            int(&reviewed)?
        } else {
            int(&value(source, "amount"))?
        };
        let in_amount = if matches!(allocated_in, Value::Null) {
            int(&value(target, "amount"))?
        } else {
            int(&allocated_in)?
        };
        let side = |prefix: &str, context: &Value| -> Result<Vec<(String, Value)>> {
            Ok(vec![
                (
                    format!("{prefix}_external_id"),
                    value(context, "external_id"),
                ),
                (format!("{prefix}_asset"), value(context, "asset")),
                (
                    format!("{prefix}_full_amount_msat"),
                    int(&value(context, "amount"))?,
                ),
                (
                    format!("{prefix}_occurred_at"),
                    value(context, "occurred_at"),
                ),
                (format!("{prefix}_wallet"), value(context, "wallet")),
            ])
        };
        let mut fields: Vec<(String, Value)> = vec![
            ("id".into(), value(&row, "legacy_source_id")),
            ("component_id".into(), value(&row, "component_id")),
            (
                "out_transaction_id".into(),
                s(text(&row, "out_transaction_id")),
            ),
            (
                "in_transaction_id".into(),
                s(text(&row, "in_transaction_id")),
            ),
            ("kind".into(), value(&row, "review_kind")),
            ("policy".into(), value(&row, "tax_policy")),
            ("swap_fee_msat".into(), value(&row, "swap_fee_msat")),
            ("swap_fee_kind".into(), value(&row, "swap_fee_kind")),
            ("out_amount".into(), reviewed),
            ("created_at".into(), value(&row, "created_at")),
            ("out_amount_msat".into(), out_amount),
            ("in_amount_msat".into(), in_amount),
        ];
        fields.extend(side("out", source)?);
        fields.extend(side("in", target)?);
        result.insert(
            text(&row, "legacy_source_id"),
            Value::Dict(
                fields
                    .into_iter()
                    .map(|(k, v)| (k.as_str().into(), v))
                    .collect(),
            ),
        );
    }
    if let Value::List(rows) = sorted(result) {
        Ok(rows)
    } else {
        unreachable!()
    }
}
pub(super) fn payout_records(conn: &Connection, scope: &Scope<'_>) -> Result<Vec<Value>> {
    let mut result = BTreeMap::new();
    for row in terms_with_deleted(conn, scope, "direct_swap_payout", true)? {
        let payout = common(&row, true)?;
        result.insert(text(&payout, "id"), payout);
    }
    if let Value::List(rows) = sorted(result) {
        Ok(rows)
    } else {
        unreachable!()
    }
}
fn contexts(conn: &Connection, scope: &Scope<'_>) -> Result<BTreeMap<String, Value>> {
    Ok(rows(conn, "SELECT t.id,t.external_id,t.asset,t.amount,t.occurred_at,w.label AS wallet,w.kind AS wallet_kind FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=?", [scope.profile_id])?.into_iter().map(|r| (text(&r,"id"),r)).collect())
}
fn integer(v: &Value) -> f64 {
    match v {
        Value::Int(n) => n.to_string().parse().expect("SQLite integer"),
        _ => 0.0,
    }
}
pub(super) fn btc(v: &Value) -> Value {
    // Decimal integer / 1e11 -> correctly-rounded float, without first rounding
    // the integer to f64 (SQLite permits values above 2**53).
    let negative = integer(v) < 0.0;
    let n = match v {
        Value::Int(n) => n.to_string(),
        _ => "0".into(),
    };
    let digits = n.trim_start_matches('-');
    let padded = format!("{digits:0>12}");
    let cut = padded.len() - 11;
    Value::Float(
        format!(
            "{}{}.{}",
            if negative { "-" } else { "" },
            &padded[..cut],
            &padded[cut..]
        )
        .parse()
        .expect("decimal integer"),
    )
}
fn fallback(row: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|k| get(row, k).filter(|v| !matches!(v, Value::Null)).cloned())
        .unwrap_or(Value::Null)
}
fn common(row: &Value, payout: bool) -> Result<Value> {
    let price = match get(row, "payout_fiat_value_exact") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => kassiber_pydecimal::finite_price(&py_str(v)).map(Some),
    };
    let invalid_price = price.is_err();
    let mut out = Vec::new();
    if payout {
        out.extend([
            ("invalid", Value::Bool(invalid_price)),
            (
                "validation_error",
                if invalid_price {
                    s("Invalid payout_fiat_value; delete and recreate this payout")
                } else {
                    Value::Null
                },
            ),
        ]);
    }
    out.extend([
        ("id", value(row, "legacy_source_id")),
        ("component_id", value(row, "component_id")),
        ("workspace_id", value(row, "workspace_id")),
        ("profile_id", value(row, "profile_id")),
        ("out_transaction_id", value(row, "out_transaction_id")),
    ]);
    if !payout {
        out.push(("in_transaction_id", value(row, "in_transaction_id")));
    }
    out.extend([
        ("kind", value(row, "review_kind")),
        ("policy", value(row, "tax_policy")),
    ]);
    if payout {
        out.extend([
            ("payout_asset", value(row, "payout_asset")),
            ("payout_amount", btc(&value(row, "payout_amount_msat"))),
            ("payout_amount_msat", value(row, "payout_amount_msat")),
            ("payout_occurred_at", value(row, "payout_occurred_at")),
            (
                "payout_fiat_value",
                match price.ok().flatten() {
                    None => Value::Null,
                    Some(value) => {
                        Value::Float(value.to_string().parse::<f64>().expect("finite price"))
                    }
                },
            ),
            ("payout_external_id", value(row, "payout_external_id")),
            ("counterparty", value(row, "counterparty")),
        ]);
    }
    out.extend([
        ("notes", value(row, "review_notes")),
        ("swap_fee_msat", value(row, "swap_fee_msat")),
        ("swap_fee_kind", value(row, "swap_fee_kind")),
    ]);
    if !payout {
        out.extend([
            ("confidence_at_pair", value(row, "confidence_at_review")),
            ("pair_source", value(row, "review_source")),
        ]);
    }
    out.extend([
        ("out_amount", value(row, "reviewed_source_amount_msat")),
        ("deleted_at", value(row, "superseded_at")),
        ("created_at", value(row, "created_at")),
    ]);
    Ok(dict(out))
}
fn boundary(context: &Value, amount: Value, wallet_kind: bool) -> Value {
    let mut out = vec![
        ("transaction_id", value(context, "id")),
        (
            "external_id",
            get(context, "external_id")
                .filter(|v| kassiber_envelope::truthy(v))
                .cloned()
                .unwrap_or_else(|| s("")),
        ),
        ("wallet", value(context, "wallet")),
    ];
    if wallet_kind {
        out.push(("wallet_kind", value(context, "wallet_kind")));
    }
    out.extend([("asset", value(context, "asset"))]);
    if wallet_kind {
        out.push(("occurred_at", value(context, "occurred_at")));
    }
    out.extend([
        ("amount", btc(&amount)),
        ("amount_msat", amount),
        ("full_amount", btc(&value(context, "amount"))),
        ("full_amount_msat", value(context, "amount")),
    ]);
    if !wallet_kind {
        out.push(("occurred_at", value(context, "occurred_at")));
    }
    dict(out)
}
fn append(row: &mut Value, name: &str, v: Value) {
    if let Value::Dict(fields) = row {
        fields.push((name.into(), v));
    }
}
fn sorted(rows: BTreeMap<String, Value>) -> Value {
    let mut result: Vec<_> = rows.into_values().collect();
    result.sort_by_key(|r| (text(r, "created_at"), text(r, "id")));
    result.reverse();
    Value::List(result)
}
fn context<'a>(
    contexts: &'a BTreeMap<String, Value>,
    row: &Value,
    name: &str,
) -> Result<&'a Value> {
    contexts.get(&text(row, name)).ok_or_else(|| {
        AppError::new(
            "internal_error",
            "Authored review transaction context is missing",
        )
    })
}
pub(super) fn pairs(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    let terms = terms(conn, scope, "transaction_pair")?;
    let contexts = contexts(conn, scope)?;
    let mut shapes: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>, usize)> = BTreeMap::new();
    for row in &terms {
        let shape = shapes.entry(text(row, "component_id")).or_default();
        shape.0.insert(text(row, "out_transaction_id"));
        shape.1.insert(text(row, "in_transaction_id"));
        shape.2 += 1;
    }
    // Only a book with pairs has components to validate (and a profile).
    let components = if terms.is_empty() {
        Vec::new()
    } else {
        authored_active_components(conn, scope)?
    };
    let mut result = BTreeMap::new();
    for row in terms {
        let source = context(&contexts, &row, "out_transaction_id")?;
        let target = context(&contexts, &row, "in_transaction_id")?;
        let shape = &shapes[&text(&row, "component_id")];
        let mut entry = common(&row, false)?;
        // `list_pair_reviews`: the review guidance follows the pair fields.
        let guidance = crate::pair_guidance::guidance(&components, &entry);
        if let (Value::Dict(fields), Value::Dict(guidance)) = (&mut entry, guidance) {
            fields.extend(guidance);
        }
        append(
            &mut entry,
            "component",
            dict([
                ("id", value(&row, "component_id")),
                ("source_count", Value::Int(shape.0.len().into())),
                ("sink_count", Value::Int(shape.1.len().into())),
                ("allocation_count", Value::Int(shape.2.into())),
            ]),
        );
        let out = fallback(
            &row,
            &[
                "allocated_source_amount_msat",
                "reviewed_source_amount_msat",
            ],
        );
        let out = if matches!(out, Value::Null) {
            value(source, "amount")
        } else {
            out
        };
        let inbound = fallback(&row, &["allocated_sink_amount_msat"]);
        let inbound = if matches!(inbound, Value::Null) {
            value(target, "amount")
        } else {
            inbound
        };
        append(&mut entry, "out", boundary(source, out, true));
        append(&mut entry, "in", boundary(target, inbound, true));
        result.insert(text(&row, "legacy_source_id"), entry);
    }
    Ok(sorted(result))
}
pub(super) fn payouts(conn: &Connection, scope: &Scope<'_>) -> Result<Value> {
    let contexts = contexts(conn, scope)?;
    let mut result = BTreeMap::new();
    for row in terms(conn, scope, "direct_swap_payout")? {
        let source = context(&contexts, &row, "out_transaction_id")?;
        let mut entry = common(&row, true)?;
        let out = get(&row, "reviewed_source_amount_msat")
            .filter(|v| !matches!(v, Value::Null))
            .cloned()
            .unwrap_or_else(|| value(source, "amount"));
        append(&mut entry, "out", boundary(source, out, false));
        append(
            &mut entry,
            "payout",
            dict([
                ("asset", value(&row, "payout_asset")),
                ("amount", btc(&value(&row, "payout_amount_msat"))),
                ("amount_msat", value(&row, "payout_amount_msat")),
                (
                    "occurred_at",
                    get(&row, "payout_occurred_at")
                        .filter(|v| kassiber_envelope::truthy(v))
                        .cloned()
                        .unwrap_or_else(|| value(source, "occurred_at")),
                ),
                ("external_id", value(&row, "payout_external_id")),
                ("counterparty", value(&row, "counterparty")),
            ]),
        );
        result.insert(text(&row, "legacy_source_id"), entry);
    }
    Ok(sorted(result))
}
