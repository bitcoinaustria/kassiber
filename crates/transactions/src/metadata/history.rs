//! Append-only edit history, filtered browsing and forward-edit reverts.
use super::*;
use kassiber_envelope::redaction::redact_secret_value;
use kassiber_envelope::{py_str, truthy};
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue};

pub struct HistoryOptions {
    pub transaction: Option<String>,
    pub wallet: Option<String>,
    pub source: Option<String>,
    pub field_family: Option<String>,
    pub field: Option<String>,
    pub pricing_only: bool,
    pub ai_only: bool,
    pub stale_only: bool,
    pub start: Option<String>,
    pub end: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub include_stale: bool,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            transaction: None,
            wallet: None,
            source: None,
            field_family: None,
            field: None,
            pricing_only: false,
            ai_only: false,
            stale_only: false,
            start: None,
            end: None,
            cursor: None,
            limit: None,
            include_stale: true,
        }
    }
}

fn supported() -> Value {
    let mut names: Vec<_> = FIELDS.iter().map(|f| f.0).collect();
    names.sort();
    Value::List(names.into_iter().map(s).collect())
}

pub(super) fn source(raw: &str) -> Result<String> {
    let clean = lower(strip(if raw.is_empty() { "cli" } else { raw }));
    if !["cli", "gui", "ai_tool"].contains(&clean.as_str()) {
        return Err(
            AppError::new("validation", "transaction edit source is not supported").with_details(
                dict([
                    ("source", s(raw)),
                    (
                        "supported",
                        Value::List(vec![s("ai_tool"), s("cli"), s("gui")]),
                    ),
                ]),
            ),
        );
    }
    Ok(clean)
}

fn load(raw: &Value) -> Value {
    if matches!(raw, Value::Null) {
        return Value::Null;
    }
    string(raw)
        .and_then(|s| kassiber_pytext::loads(s).ok())
        .unwrap_or_else(|| raw.clone())
}

pub(super) fn state(conn: &Connection, tx: &Value, field: &str) -> Result<Value> {
    Ok(match field {
        "tags" => Value::List(
            canonical_tags(
                tags(conn, text(tx, "id"))?
                    .iter()
                    .map(|row| text(row, "label").to_owned()),
            )
            .into_iter()
            .map(s)
            .collect(),
        ),
        "excluded" => Value::Bool(int(tx, "excluded") != 0),
        "taxable" => {
            if matches!(get(tx, "taxability_override"), None | Some(Value::Null)) {
                Value::Null
            } else {
                Value::Bool(int(tx, "taxability_override") != 0)
            }
        }
        "kind" => value(tx, "kind_override"),
        "at_regime" => value(tx, "at_regime_override"),
        "at_category" => value(tx, "at_category_override"),
        "pricing_review_accepted" => value(tx, "pricing_review_accepted_price"),
        "fiat_rate" | "fiat_value" => {
            let exact = value(tx, &format!("{field}_exact"));
            if !matches!(&exact, Value::Null) && exact != s("") {
                exact
            } else if let Some(Value::Float(n)) = get(tx, field) {
                s(general_float(*n))
            } else {
                Value::Null
            }
        }
        _ => value(tx, field),
    })
}

// Python format(float, ".12g"). Rust's scientific formatter rounds the exact
// binary value to even, then general formatting chooses fixed/exponent layout.
fn general_float(n: f64) -> String {
    if !n.is_finite() {
        return kassiber_pyjson::float_repr(n);
    }
    let scientific = format!("{n:.11e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    if !(-4..12).contains(&exponent) {
        format!(
            "{}e{}{:02}",
            mantissa.trim_end_matches('0').trim_end_matches('.'),
            if exponent < 0 { '-' } else { '+' },
            exponent.unsigned_abs()
        )
    } else {
        let out = format!("{n:.precision$}", precision = (11 - exponent) as usize);
        if out.contains('.') {
            out.trim_end_matches('0').trim_end_matches('.').into()
        } else {
            out
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn revert(
    conn: &Connection,
    scope: &Scope<'_>,
    reference: &str,
    event: &str,
    field: Option<&str>,
    raw_source: &str,
    reason: Option<&str>,
) -> Result<Value> {
    let tx = transaction(conn, scope.profile_id, reference)?;
    if rows(
        conn,
        "SELECT id FROM transaction_edit_events WHERE profile_id=? AND transaction_id=? AND id=?",
        [scope.profile_id, text(&tx, "id"), event],
    )?
    .is_empty()
    {
        return Err(
            AppError::new("not_found", "transaction edit event not found")
                .with_hint("Use the event id from transaction history."),
        );
    }
    let mut fields = rows(
        conn,
        "SELECT * FROM transaction_edit_fields WHERE event_id=? ORDER BY field ASC,id ASC",
        [event],
    )?;
    if let Some(field) = field.filter(|s| !s.is_empty()) {
        if !FIELDS.iter().any(|f| f.0 == field) {
            return Err(
                AppError::new("validation", "history field is not supported")
                    .with_details(dict([("field", s(field))])),
            );
        }
        fields.retain(|row| text(row, "field") == field);
        if fields.is_empty() {
            return Err(
                AppError::new("validation", "history event did not change that field")
                    .with_details(dict([("event_id", s(event)), ("field", s(field))])),
            );
        }
    }
    let mut changes = Vec::new();
    let mut updates = Vec::new();
    let mut next_tags = None;
    let mut reverted = Vec::new();
    for row in &fields {
        let field = text(row, "field");
        let before = state(conn, &tx, field)?;
        let old = load(&value(row, "before_value"));
        let old = if field == "tags" {
            let labels = match old {
                Value::List(labels) => labels.iter().map(py_str).collect(),
                _ => Vec::new(),
            };
            next_tags = Some(labels);
            Value::List(
                canonical_tags(next_tags.as_ref().unwrap().clone())
                    .into_iter()
                    .map(s)
                    .collect(),
            )
        } else if field == "excluded" {
            Value::Bool(truthy(&old))
        } else {
            old
        };
        if field != "tags" {
            let column = match field {
                "taxable" => "taxability_override",
                "kind" => "kind_override",
                "at_regime" => "at_regime_override",
                "at_category" => "at_category_override",
                "pricing_review_accepted" => "pricing_review_accepted_price",
                _ if FIELDS.iter().any(|f| f.0 == field) => field,
                _ => {
                    return Err(
                        AppError::new("validation", "history field cannot be reverted")
                            .with_details(dict([("field", s(field))])),
                    )
                }
            };
            if field == "fiat_rate" || field == "fiat_value" {
                // `float(finite_decimal(before_value, field))` (d444f4db5).
                let float =
                    if matches!(&old, Value::Null) {
                        Value::Null
                    } else {
                        let number = kassiber_pydecimal::finite_price(&py_str(&old))
                            .map_err(|error| AppError::new("validation", error.message(field)))?;
                        Value::Float(kassiber_pytext::parse_float(&number.to_string()).map_err(
                            |_| AppError::new("internal_error", "invalid stored decimal"),
                        )?)
                    };
                updates.push((column.to_owned(), float));
                updates.push((format!("{field}_exact"), old.clone()));
            } else {
                updates.push((column.to_owned(), old.clone()));
            }
        }
        if json(&before)? != json(&old)? {
            changes.push((field, before, old));
        }
        reverted.push(s(field));
    }
    if updates
        .iter()
        .any(|(column, after)| column == "excluded" && truthy(after))
    {
        guard_exclusion(conn, scope.profile_id, text(&tx, "id"))?;
    }
    let source = source(raw_source)?;
    let default_reason = format!("Reverted transaction edit {event}");
    let reason = clean_reason(Some(
        reason.filter(|s| !s.is_empty()).unwrap_or(&default_reason),
    ))?;
    let changed = !changes.is_empty() && changes.iter().any(|(f, _, _)| *f != "pricing_fetched_at");
    if changed
        && updates.iter().any(|(column, _)| {
            [
                "fiat_rate",
                "fiat_rate_exact",
                "fiat_value",
                "fiat_value_exact",
            ]
            .contains(&column.as_str())
        })
    {
        // History reverts bypass the ordinary pricing payload normalizer.
        validate_transaction_price(text(&tx, "id"), &|field| {
            updates
                .iter()
                .rev()
                .find(|(column, _)| column == field)
                .map_or_else(|| value(&tx, field), |(_, after)| after.clone())
        })?;
    }
    let new_event = if changed {
        s(atomic(conn, || {
            if changes.iter().any(|(f, _, _)| *f == "tags") {
                let ids = next_tags
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
            // One statement, as Python's audited update, so row triggers
            // fire once per revert rather than once per column.
            if !updates.is_empty() {
                let assignments = updates
                    .iter()
                    .map(|(column, _)| format!("{column}=?"))
                    .collect::<Vec<_>>()
                    .join(",");
                let mut values = updates
                    .iter()
                    .map(|(_, after)| cursor::bind(after))
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
            changes.sort_by_key(|(field, _, _)| {
                FIELDS.iter().position(|f| f.0 == *field).unwrap_or(999)
            });
            append_fields(conn, scope, &tx, &changes, &source, reason)
        })?)
    } else {
        Value::Null
    };
    reverted.sort_by_key(py_str);
    Ok(dict([
        (
            "transaction",
            record(conn, &transaction(conn, scope.profile_id, text(&tx, "id"))?)?,
        ),
        ("updated", Value::Bool(changed)),
        ("reverted_event_id", s(event)),
        ("history_event_id", new_event),
        ("reverted_fields", Value::List(reverted)),
    ]))
}

fn label(field: &str, value: &Value) -> String {
    if matches!(value, Value::Null) {
        return "Empty".into();
    }
    if field == "tags" {
        return match value {
            Value::List(items) if !items.is_empty() => {
                items.iter().map(py_str).collect::<Vec<_>>().join(", ")
            }
            _ => "No tags".into(),
        };
    }
    if field == "excluded" {
        return if truthy(value) {
            "Excluded from reports"
        } else {
            "Included in reports"
        }
        .into();
    }
    if field == "taxable" {
        return if truthy(value) {
            "Taxable"
        } else {
            "Not taxable"
        }
        .into();
    }
    let text = py_str(value);
    if field == "pricing_review_accepted" {
        return match text.split_once(':') {
            Some((kind, amount)) if !amount.is_empty() => format!(
                "Accepted {} {amount}",
                if kind == "rate" {
                    "price per unit"
                } else {
                    "value"
                }
            ),
            _ => text,
        };
    }
    if let Some((_, _, label)) = VALUE_LABELS
        .iter()
        .find(|(f, v, _)| *f == field && *v == text)
    {
        return (*label).into();
    }
    if let Value::Bool(b) = value {
        return if *b { "Yes" } else { "No" }.into();
    }
    text
}

fn field_payload(row: &Value) -> Result<Value> {
    let field = text(row, "field");
    let before = load(&value(row, "before_value"));
    let after = load(&value(row, "after_value"));
    let diff = load(&value(row, "diff_json"));
    let diff = if truthy(&diff) {
        diff
    } else {
        dict::<&str>([])
    };
    let redacted_before = redact_secret_value(&before);
    let redacted_after = redact_secret_value(&after);
    let redacted_diff = redact_secret_value(&diff);
    let changed = json(&before)? != json(&redacted_before)?
        || json(&after)? != json(&redacted_after)?
        || json(&diff)? != json(&redacted_diff)?;
    let (_, name, family) = FIELDS
        .iter()
        .find(|(name, _, _)| *name == field)
        .copied()
        .unwrap_or((field, field, "metadata"));
    Ok(dict([
        ("id", value(row, "id")),
        ("field", s(field)),
        ("label", s(name)),
        ("family", s(family)),
        ("before_value", redacted_before.clone()),
        ("after_value", redacted_after.clone()),
        ("before_label", s(label(field, &redacted_before))),
        ("after_label", s(label(field, &redacted_after))),
        ("diff", redacted_diff),
        ("redacted", Value::Bool(changed)),
    ]))
}

fn summary(fields: &[Value]) -> String {
    if fields.iter().any(|f| text(f, "family") == "pricing") {
        return "Pricing provenance updated".into();
    }
    if let Some(tags) = fields.iter().find(|f| text(f, "field") == "tags") {
        let diff = value(tags, "diff");
        let mut parts = Vec::new();
        for (key, verb) in [("added", "added"), ("removed", "removed")] {
            if let Some(Value::List(items)) = get(&diff, key).filter(|v| truthy(v)) {
                parts.push(format!(
                    "{verb} {}",
                    items.iter().map(py_str).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        return format!(
            "Tags updated{}",
            if parts.is_empty() {
                String::new()
            } else {
                format!(": {}", parts.join("; "))
            }
        );
    }
    if fields.len() == 1 {
        if text(&fields[0], "field") == "note" {
            return "Note updated".into();
        }
        if text(&fields[0], "field") == "excluded" {
            return text(&fields[0], "after_label").into();
        }
    }
    format!(
        "Updated {}",
        fields
            .iter()
            .map(|f| text(f, "label"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn event_payload(conn: &Connection, row: &Value, version: i64) -> Result<Value> {
    let field_rows = rows(
        conn,
        "SELECT * FROM transaction_edit_fields WHERE event_id=? ORDER BY field ASC,id ASC",
        [text(row, "id")],
    )?;
    let fields = field_rows
        .iter()
        .map(field_payload)
        .collect::<Result<Vec<_>>>()?;
    let families: BTreeSet<_> = fields
        .iter()
        .map(|f| text(f, "family").to_owned())
        .collect();
    let anchor = dict([
        (
            "journal_input_version",
            Value::Int(int(row, "journal_input_version").into()),
        ),
        (
            "journal_input_version_after",
            Value::Int(int(row, "journal_input_version_after").into()),
        ),
        (
            "last_processed_input_version",
            Value::Int(int(row, "last_processed_input_version").into()),
        ),
        ("last_processed_at", value(row, "last_processed_at")),
        (
            "last_processed_tx_count",
            Value::Int(int(row, "last_processed_tx_count").into()),
        ),
        (
            "stale_for_reports",
            Value::Bool(int(row, "journal_input_version_after") > version),
        ),
    ]);
    let btc = |key| match get(row, key) {
        Some(Value::Int(n)) => format!("{n}e-11")
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let tx = dict([
        ("id", value(row, "transaction_id")),
        ("external_id", s(text(row, "transaction_external_id"))),
        ("occurred_at", s(text(row, "transaction_occurred_at"))),
        ("direction", s(text(row, "direction"))),
        ("asset", s(text(row, "asset"))),
        ("amount", btc("amount")),
        ("amount_msat", value(row, "amount")),
        ("fee", btc("fee")),
        ("fee_msat", value(row, "fee")),
        ("counterparty", s(text(row, "counterparty"))),
    ]);
    Ok(dict([
        ("id", value(row, "id")),
        ("transaction_id", value(row, "transaction_id")),
        (
            "transaction_external_id",
            s(text(row, "transaction_external_id")),
        ),
        (
            "transaction_occurred_at",
            s(text(row, "transaction_occurred_at")),
        ),
        ("wallet_id", s(text(row, "wallet_id"))),
        ("wallet_label", s(text(row, "wallet_label"))),
        ("source", value(row, "source")),
        (
            "source_label",
            s(match text(row, "source") {
                "cli" => "CLI",
                "gui" => "Desktop",
                "ai_tool" => "Assistant",
                other => other,
            }),
        ),
        ("reason", s(text(row, "reason"))),
        ("changed_at", value(row, "changed_at")),
        ("summary", s(summary(&fields))),
        (
            "families",
            Value::List(families.into_iter().map(s).collect()),
        ),
        ("fields", Value::List(fields)),
        ("report_anchor", anchor),
        ("transaction", tx),
    ]))
}

/// `history_for_transaction_ids`: every edit event of the selected
/// transactions, oldest first, grouped by transaction id in sorted order.
pub(super) fn for_transactions(
    conn: &Connection,
    profile_id: &str,
    version: i64,
    ids: &[String],
) -> Result<Vec<(String, Vec<Value>)>> {
    let unique: BTreeSet<&String> = ids.iter().collect();
    if unique.is_empty() {
        return Ok(Vec::new());
    }
    let mut bind = vec![SqlValue::Text(profile_id.into())];
    bind.extend(unique.iter().map(|id| SqlValue::Text((*id).clone())));
    let found = rows(conn,&format!("SELECT e.*,w.label AS wallet_label,t.direction,t.asset,t.amount,t.fee,t.counterparty FROM transaction_edit_events e LEFT JOIN wallets w ON w.id=e.wallet_id LEFT JOIN transactions t ON t.id=e.transaction_id WHERE e.profile_id=? AND e.transaction_id IN ({}) ORDER BY e.changed_at ASC,e.id ASC",vec!["?";unique.len()].join(",")),params_from_iter(bind))?;
    let mut grouped: Vec<(String, Vec<Value>)> = unique
        .iter()
        .map(|id| ((*id).clone(), Vec::new()))
        .collect();
    for row in &found {
        let id = text(row, "transaction_id").to_owned();
        let payload = event_payload(conn, row, version)?;
        match grouped.iter_mut().find(|(key, _)| *key == id) {
            Some((_, events)) => events.push(payload),
            None => grouped.push((id, vec![payload])),
        }
    }
    Ok(grouped)
}

pub(super) fn list(
    conn: &Connection,
    scope: &Scope<'_>,
    options: &HistoryOptions,
) -> Result<Value> {
    let limit = cursor::limit(options.limit, 50)?;
    let profile = rows(
        conn,
        "SELECT * FROM profiles WHERE id=?",
        [scope.profile_id],
    )?
    .remove(0);
    let version = int(&profile, "last_processed_input_version");
    let start = time::timestamp(options.start.as_deref(), "start")?;
    let end = time::timestamp(options.end.as_deref(), "end")?;
    let transaction_id = options
        .transaction
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|reference| {
            transaction(conn, scope.profile_id, reference).map(|tx| text(&tx, "id").to_owned())
        })
        .transpose()?
        .unwrap_or_default();
    let wallet_id = records::wallet(conn, scope.profile_id, options.wallet.as_deref())?;
    let raw_source = if options.ai_only {
        Some("ai_tool")
    } else {
        options.source.as_deref()
    };
    let clean_source = raw_source
        .filter(|s| !s.is_empty())
        .map(source)
        .transpose()?
        .unwrap_or_default();
    let clean_family = lower(strip(options.field_family.as_deref().unwrap_or_default()));
    let selected_family = if options.pricing_only {
        "pricing"
    } else {
        &clean_family
    };
    if !selected_family.is_empty() && !["metadata", "tax", "pricing"].contains(&selected_family) {
        return Err(
            AppError::new("validation", "field_family is not supported").with_details(dict([
                ("field_family", s(selected_family)),
                (
                    "supported",
                    Value::List(vec![s("metadata"), s("pricing"), s("tax")]),
                ),
            ])),
        );
    }
    let clean_field = strip(options.field.as_deref().unwrap_or_default());
    if options.field.as_ref().is_some_and(|s| !s.is_empty())
        && !FIELDS.iter().any(|f| f.0 == clean_field)
    {
        return Err(
            AppError::new("validation", "history field is not supported").with_details(dict([
                ("field", s(options.field.as_deref().unwrap())),
                ("supported", supported()),
            ])),
        );
    }
    let mut clauses = vec!["e.profile_id=?".to_owned()];
    let mut args = vec![SqlValue::Text(scope.profile_id.into())];
    for (clause, v) in [
        ("e.transaction_id=?", &transaction_id),
        ("e.wallet_id=?", &wallet_id),
        ("e.source=?", &clean_source),
    ] {
        if !v.is_empty() {
            clauses.push(clause.into());
            args.push(SqlValue::Text(v.clone()));
        }
    }
    for (op, v) in [(">=", start.as_str()), ("<=", end.as_str())] {
        if !v.is_empty() {
            clauses.push(time::predicate(conn, "e.changed_at", op)?);
            args.push(SqlValue::Text(v.into()));
        }
    }
    let selected: Vec<_> = FIELDS
        .iter()
        .filter(|f| {
            if !clean_field.is_empty() {
                f.0 == clean_field
            } else {
                !selected_family.is_empty() && f.2 == selected_family
            }
        })
        .map(|f| f.0)
        .collect();
    if !selected.is_empty() {
        clauses.push(format!("EXISTS(SELECT 1 FROM transaction_edit_fields f WHERE f.event_id=e.id AND f.field IN ({}))",vec!["?";selected.len()].join(",")));
        args.extend(selected.into_iter().map(|f| SqlValue::Text(f.into())));
    }
    if options.stale_only {
        clauses.push("e.journal_input_version_after>?".into());
        args.push(SqlValue::Integer(version));
    }
    let filters = dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("transaction_id", s(transaction_id.as_str())),
        ("wallet_id", s(wallet_id.as_str())),
        ("source", s(clean_source.as_str())),
        ("field_family", s(clean_family.as_str())),
        ("field", s(clean_field)),
        ("pricing_only", Value::Bool(options.pricing_only)),
        ("ai_only", Value::Bool(options.ai_only)),
        ("stale_only", Value::Bool(options.stale_only)),
        ("start", s(start.as_str())),
        ("end", s(end.as_str())),
    ]);
    const KEYS: &[&str] = &["changed_at", "id"];
    if let Some(row) = cursor::decode(options.cursor.as_deref(), &filters, KEYS, true)? {
        clauses.push("(e.changed_at,e.id)<(?,?)".into());
        for key in KEYS {
            args.push(cursor::bind(&value(&row, key))?);
        }
    }
    args.push(SqlValue::Integer(limit + 1));
    let mut page = rows(conn,&format!("SELECT e.*,w.label AS wallet_label,t.direction,t.asset,t.amount,t.fee,t.counterparty FROM transaction_edit_events e LEFT JOIN wallets w ON w.id=e.wallet_id LEFT JOIN transactions t ON t.id=e.transaction_id WHERE {} ORDER BY e.changed_at DESC,e.id DESC LIMIT ?",clauses.join(" AND ")),params_from_iter(args))?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next = if more {
        cursor::encode(page.last().unwrap(), &filters, KEYS)?
    } else {
        Value::Null
    };
    let mut public_filters = filters;
    if let Value::Dict(fields) = &mut public_filters {
        fields.drain(..2);
    }
    let mut result = dict([
        (
            "events",
            Value::List(
                page.iter()
                    .map(|row| event_payload(conn, row, version))
                    .collect::<Result<_>>()?,
            ),
        ),
        ("next_cursor", next),
        ("has_more", Value::Bool(more)),
        ("limit", Value::Int(limit.into())),
        ("filters", public_filters),
    ]);
    if options.include_stale {
        if let Value::Dict(fields) = &mut result {
            fields.push(("stale".into(), stale(conn, scope.profile_id)?));
        }
    }
    Ok(result)
}

// Frozen presentation labels from the Python reference, in audit field order.
pub(super) const FIELDS: &[(&str, &str, &str)] = &[
    ("note", "Note", "metadata"),
    ("tags", "Tags", "metadata"),
    ("excluded", "Report inclusion", "metadata"),
    ("review_status", "Review status", "tax"),
    ("taxable", "Taxable", "tax"),
    ("at_regime", "Austrian regime", "tax"),
    ("at_category", "Austrian category", "tax"),
    ("kind", "Recorded as", "tax"),
    ("fiat_currency", "Fiat currency", "pricing"),
    ("fiat_rate", "Price per BTC", "pricing"),
    ("fiat_value", "Fiat value", "pricing"),
    ("fiat_price_source", "Legacy price source", "pricing"),
    ("pricing_source_kind", "Pricing source", "pricing"),
    ("pricing_quality", "Pricing quality", "pricing"),
    (
        "pricing_review_accepted",
        "Approximate price accepted",
        "pricing",
    ),
    ("pricing_provider", "Pricing provider", "pricing"),
    ("pricing_pair", "Pricing pair", "pricing"),
    ("pricing_timestamp", "Pricing timestamp", "pricing"),
    ("pricing_granularity", "Pricing granularity", "pricing"),
    ("pricing_method", "Pricing method", "pricing"),
    (
        "pricing_external_ref",
        "Pricing evidence reference",
        "pricing",
    ),
    ("pricing_fetched_at", "Pricing captured at", "pricing"),
];

const VALUE_LABELS: &[(&str, &str, &str)] = &[
    ("review_status", "completed", "Completed"),
    ("review_status", "pending", "Pending"),
    ("review_status", "failed", "Failed"),
    ("review_status", "review", "Needs review"),
    ("at_regime", "alt", "Altbestand"),
    ("at_regime", "neu", "Neubestand"),
    ("at_regime", "outside", "Outside §27b"),
    ("at_category", "income_general", "General income"),
    ("at_category", "income_capital_yield", "Capital yield"),
    ("at_category", "neu_gain", "Neubestand gain"),
    ("at_category", "neu_loss", "Neubestand loss"),
    ("at_category", "neu_swap", "Neubestand swap"),
    ("at_category", "alt_spekulation", "Altbestand speculation"),
    ("at_category", "alt_taxfree", "Altbestand tax-free"),
    ("at_category", "none", "Not reportable"),
    ("pricing_source_kind", "generic_import", "Generic import"),
    ("pricing_source_kind", "wallet_export", "Wallet export"),
    (
        "pricing_source_kind",
        "exchange_execution",
        "Exchange execution",
    ),
    (
        "pricing_source_kind",
        "btcpay_wallet_export",
        "BTCPay wallet export",
    ),
    ("pricing_source_kind", "btcpay_invoice", "BTCPay invoice"),
    ("pricing_source_kind", "btcpay_payment", "BTCPay payment"),
    ("pricing_source_kind", "btcpay_payout", "BTCPay payout"),
    ("pricing_source_kind", "manual_override", "Manual override"),
    (
        "pricing_source_kind",
        "manual_rate_cache",
        "Manual rate cache",
    ),
    (
        "pricing_source_kind",
        "fmv_provider",
        "Provider fair-market price",
    ),
    ("pricing_quality", "exact", "Exact"),
    ("pricing_quality", "provider_sample", "Provider sample"),
    ("pricing_quality", "coarse_fallback", "Coarse fallback"),
    ("pricing_quality", "missing", "Missing"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpython_metadata_timestamp_and_legacy_float_vectors() {
        let data =
            kassiber_pytext::loads(include_str!("../../tests/data/metadata-vectors.json")).unwrap();
        let Value::List(times) = get(&data, "timestamps").unwrap() else {
            panic!()
        };
        for row in times {
            match time::timestamp(Some(text(row, "input")), "start") {
                Ok(actual) => {
                    assert_eq!(value(row, "code"), Value::Null, "{row:?}");
                    assert_eq!(actual, text(row, "value"), "{row:?}");
                }
                Err(error) => {
                    assert_eq!(s(error.code), value(row, "code"), "{row:?}");
                    assert_eq!(error.message, text(row, "value"), "{row:?}");
                }
            }
        }
        let Value::List(floats) = get(&data, "floats").unwrap() else {
            panic!()
        };
        for row in floats {
            let Value::Float(n) = get(row, "value").unwrap() else {
                panic!()
            };
            assert_eq!(general_float(*n), text(row, "formatted"), "{n:?}");
        }
    }
}
