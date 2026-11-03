//! Local metadata edits, with Python-compatible append-only audit records.
use std::collections::{BTreeMap, BTreeSet};

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_pytext::{lower, strip};
use kassiber_store::rusqlite::{params, Connection};

use crate::sql::{atomic, error, invalidate, now, rows, string, uuid, Result, Scope};

mod bip329;
pub use bip329::{
    export_records as bip329_export_records, managed_export as managed_bip329_export, Bip329,
};
pub(crate) mod cursor;
mod grouped;
mod history;
pub(crate) mod records;
pub(crate) mod time;
pub use grouped::{GroupedUpdate, PricingUpdate};
pub use history::HistoryOptions;
pub use records::RecordOptions;

pub enum Command {
    Update(Box<GroupedUpdate>),
    Bip329(Bip329),
    Tags,
    CreateTag {
        code: String,
        label: String,
    },
    Get {
        transaction: String,
    },
    Note {
        transaction: String,
        note: Option<String>,
        reason: Option<String>,
    },
    Tag {
        transaction: String,
        tag: String,
        remove: bool,
        reason: Option<String>,
    },
    Excluded {
        transaction: String,
        excluded: bool,
        reason: Option<String>,
    },
    Kind {
        transaction: String,
        kind: Option<String>,
        reason: Option<String>,
    },
    Stale,
    Records(RecordOptions),
    History(HistoryOptions),
    Revert {
        transaction: String,
        event: String,
        field: Option<String>,
        source: String,
        reason: Option<String>,
    },
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    get(row, key).and_then(string).unwrap_or_default()
}
fn value(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}
fn int(row: &Value, key: &str) -> i64 {
    match get(row, key) {
        Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}
fn integer(value: &Value) -> Result<kassiber_pyjson::BigInt> {
    use kassiber_pyjson::BigInt;
    let invalid = || {
        AppError::new(
            "internal_error",
            format!(
                "invalid literal for int() with base 10: {}",
                kassiber_envelope::py_repr(value)
            ),
        )
    };
    let integer: BigInt = match value {
        Value::Int(n) => n.clone(),
        Value::Bool(n) => u8::from(*n).into(),
        Value::Float(n) if n.is_finite() => {
            format!("{:.0}", n.trunc()).parse().map_err(|_| invalid())?
        }
        Value::Str(raw) => {
            kassiber_pytext::parse_int(raw.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?
        }
        Value::Bytes(raw) => kassiber_pytext::parse_int(
            std::str::from_utf8(raw)
                .ok()
                .filter(|text| text.is_ascii())
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?,
        Value::Float(n) => {
            return Err(AppError::new(
                "internal_error",
                if n.is_nan() {
                    "cannot convert float NaN to integer"
                } else {
                    "cannot convert float infinity to integer"
                },
            ))
        }
        _ => {
            return Err(AppError::new(
                "internal_error",
                format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kassiber_envelope::type_name(value)
            ),
            ))
        }
    };
    Ok(integer)
}
fn msat(value: &Value) -> Result<kassiber_pydecimal::Decimal> {
    integer(value)?
        .to_string()
        .parse()
        .map_err(|error: kassiber_pydecimal::DecimalError| {
            AppError::new("internal_error", error.to_string())
        })
}
fn code(label: &str) -> Result<String> {
    let code = lower(strip(label)).replace(' ', "-");
    if code.is_empty() {
        return Err(AppError::new("app_error", "Code cannot be empty"));
    }
    Ok(code)
}

fn transaction(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    let exact = rows(
        conn,
        "SELECT * FROM transactions WHERE profile_id=? AND id=?",
        [profile, reference],
    )?;
    if let Some(row) = exact.into_iter().next() {
        return Ok(row);
    }
    let found = rows(conn, "SELECT * FROM transactions WHERE profile_id=? AND external_id=? ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 2", [profile, reference])?;
    if found.len() > 1 {
        return Err(AppError::new("ambiguous_reference", format!("Transaction external_id '{reference}' matches multiple transactions"))
            .with_hint("Use the Kassiber transaction id from `transactions list` or narrow the command to an unambiguous row."));
    }
    found
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("app_error", format!("Transaction '{reference}' not found")))
}

fn tags(conn: &Connection, tx: &str) -> Result<Vec<Value>> {
    rows(conn, "SELECT t.code,t.label FROM transaction_tags tt JOIN tags t ON t.id=tt.tag_id WHERE tt.transaction_id=? ORDER BY t.code ASC", [tx])
}

fn canonical_tags(labels: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for label in labels {
        let label = strip(&label);
        if !label.is_empty() && seen.insert(lower(label)) {
            result.push(label.to_owned());
        }
    }
    result.sort_by_key(|label| lower(label));
    result
}

fn ensure_tag(conn: &Connection, scope: &Scope<'_>, label: &str) -> Result<String> {
    let normalized = code(label)?;
    let found = rows(
        conn,
        "SELECT id FROM tags WHERE profile_id=? AND code=?",
        [scope.profile_id, &normalized],
    )?;
    if let Some(row) = found.first() {
        return Ok(text(row, "id").into());
    }
    let id = uuid(conn)?;
    conn.execute(
        "INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",
        params![
            id,
            scope.workspace_id,
            scope.profile_id,
            normalized,
            label,
            now(conn)?
        ],
    )
    .map_err(error)?;
    Ok(id)
}

fn json(value: &Value) -> Result<String> {
    dumps(
        value,
        &Options {
            sort_keys: true,
            separators: Some((",".into(), ":".into())),
            ..Options::default()
        },
    )
    .map_err(|e| AppError::new("validation", e.to_string()))
}

fn append(
    conn: &Connection,
    scope: &Scope<'_>,
    tx: &Value,
    field: &str,
    before: &Value,
    after: &Value,
    reason: Option<&str>,
) -> Result<String> {
    append_fields(
        conn,
        scope,
        tx,
        &[(field, before.clone(), after.clone())],
        "cli",
        reason,
    )
}

fn append_fields(
    conn: &Connection,
    scope: &Scope<'_>,
    tx: &Value,
    changes: &[(&str, Value, Value)],
    source: &str,
    reason: Option<&str>,
) -> Result<String> {
    let profile = rows(
        conn,
        "SELECT * FROM profiles WHERE id=?",
        [scope.profile_id],
    )?
    .remove(0);
    let id = uuid(conn)?;
    let version = int(&profile, "journal_input_version");
    conn.execute("INSERT INTO transaction_edit_events(id,workspace_id,profile_id,transaction_id,wallet_id,transaction_external_id,transaction_occurred_at,source,reason,changed_at,journal_input_version,journal_input_version_after,last_processed_input_version,last_processed_at,last_processed_tx_count) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![id, scope.workspace_id, scope.profile_id, text(tx,"id"), string(&value(tx,"wallet_id")), string(&value(tx,"external_id")), string(&value(tx,"occurred_at")), source, reason, now(conn)?, version, version+1, int(&profile,"last_processed_input_version"), string(&value(&profile,"last_processed_at")), int(&profile,"last_processed_tx_count")]).map_err(error)?;
    for (field, before, after) in changes {
        let diff = if *field == "tags" {
            let list = |v: &Value| match v {
                Value::List(items) => items
                    .iter()
                    .map(|v| (lower(string(v).unwrap_or_default()), v.clone()))
                    .collect::<BTreeMap<_, _>>(),
                _ => BTreeMap::new(),
            };
            let before_set = list(before);
            let after_set = list(after);
            dict([
                (
                    "added",
                    Value::List(
                        after_set
                            .iter()
                            .filter(|(k, _)| !before_set.contains_key(*k))
                            .map(|(_, v)| v.clone())
                            .collect(),
                    ),
                ),
                (
                    "removed",
                    Value::List(
                        before_set
                            .iter()
                            .filter(|(k, _)| !after_set.contains_key(*k))
                            .map(|(_, v)| v.clone())
                            .collect(),
                    ),
                ),
                ("before", before.clone()),
                ("after", after.clone()),
            ])
        } else {
            dict::<&str>([])
        };
        conn.execute("INSERT INTO transaction_edit_fields(id,event_id,field,before_value,after_value,diff_json) VALUES(?,?,?,?,?,?)", params![uuid(conn)?, id, field, json(before)?, json(after)?, json(&diff)?]).map_err(error)?;
    }
    invalidate(conn, scope.profile_id)?;
    Ok(id)
}

fn clean_reason(reason: Option<&str>) -> Result<Option<&str>> {
    let reason = reason.map(strip).filter(|s| !s.is_empty());
    if reason.is_some_and(|s| s.chars().count() > 1000) {
        return Err(AppError::new(
            "validation",
            "reason cannot exceed 1000 characters",
        ));
    }
    Ok(reason)
}

fn guard_exclusion(conn: &Connection, profile: &str, tx: &str) -> Result<()> {
    let found = rows(conn, "SELECT component.id AS component_id,term.legacy_source_id AS review_id,term.term_kind FROM custody_component_legs leg JOIN custody_components component ON component.id=leg.component_id LEFT JOIN custody_component_economic_terms term ON term.component_id=component.id AND (term.source_leg_id=leg.id OR term.target_leg_id=leg.id) WHERE component.profile_id=? AND component.state='active' AND COALESCE(leg.anchor_transaction_id,leg.transaction_id)=? ORDER BY component.created_at,component.id,term.ordinal LIMIT 1", [profile, tx])?;
    if let Some(row) = found.first() {
        let id = if text(row, "review_id").is_empty() {
            text(row, "component_id")
        } else {
            text(row, "review_id")
        };
        return Err(AppError::new("conflict",format!("Transaction belongs to active custody review {id}; excluding it would orphan reviewed custody evidence."))
            .with_hint("Reopen or supersede the custody review before excluding this transaction.")
            .with_details(dict([("review_id",s(id)),("component_id",value(row,"component_id")),("term_kind",value(row,"term_kind"))])));
    }
    Ok(())
}

fn record(conn: &Connection, tx: &Value) -> Result<Value> {
    let wallet = rows(
        conn,
        "SELECT id,label FROM wallets WHERE id=?",
        [text(tx, "wallet_id")],
    )?
    .into_iter()
    .next()
    .unwrap_or(Value::Null);
    let btc = |key| -> Result<Value> {
        Ok(Value::Float(
            format!("{}e-11", msat(&value(tx, key))?)
                .parse()
                .map_err(|_| AppError::new("validation", "invalid msat amount"))?,
        ))
    };
    let mut fields = vec![
        ("transaction_id", value(tx, "id")),
        ("external_id", s(text(tx, "external_id"))),
        ("occurred_at", value(tx, "occurred_at")),
        ("direction", value(tx, "direction")),
        ("asset", value(tx, "asset")),
        ("amount", btc("amount")?),
        ("amount_msat", Value::Int(integer(&value(tx, "amount"))?)),
        ("fee", btc("fee")?),
        ("fee_msat", Value::Int(integer(&value(tx, "fee"))?)),
        ("counterparty", s(text(tx, "counterparty"))),
        ("wallet_id", s(text(&wallet, "id"))),
        ("wallet_label", s(text(&wallet, "label"))),
        ("note", s(text(tx, "note"))),
        ("excluded", Value::Bool(int(tx, "excluded") != 0)),
    ];
    for key in [
        "fiat_currency",
        "fiat_rate",
        "fiat_value",
        "fiat_rate_exact",
        "fiat_value_exact",
        "pricing_source_kind",
        "pricing_quality",
        "pricing_external_ref",
        "review_status",
    ] {
        fields.push((key, value(tx, key)));
    }
    fields.extend([
        (
            "taxable",
            if matches!(get(tx, "taxability_override"), None | Some(Value::Null)) {
                Value::Null
            } else {
                Value::Bool(int(tx, "taxability_override") != 0)
            },
        ),
        ("at_regime", value(tx, "at_regime_override")),
        ("at_category", value(tx, "at_category_override")),
        ("kind", value(tx, "kind")),
        ("kind_override", value(tx, "kind_override")),
        ("tags", Value::List(tags(conn, text(tx, "id"))?)),
    ]);
    Ok(dict(fields))
}

pub fn execute(conn: &Connection, scope: &Scope<'_>, command: &Command) -> Result<Value> {
    match command {
        Command::Update(update) => grouped::update(conn, scope, update),
        Command::Tags => Ok(Value::List(rows(
            conn,
            "SELECT id,code,label,created_at FROM tags WHERE profile_id=? ORDER BY code ASC",
            [scope.profile_id],
        )?)),
        Command::CreateTag { code: raw, label } => atomic(conn, || {
            let normalized = code(raw)?;
            let id = uuid(conn)?;
            let result = conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![id,scope.workspace_id,scope.profile_id,normalized,label,now(conn)?]);
            match result {
                Err(kassiber_store::rusqlite::Error::SqliteFailure(e, _))
                    if e.code == kassiber_store::rusqlite::ErrorCode::ConstraintViolation =>
                {
                    return Err(AppError::new(
                        "conflict",
                        format!("Tag '{normalized}' already exists"),
                    )
                    .with_hint("Choose a different tag code or use the existing tag."))
                }
                result => {
                    result.map_err(error)?;
                }
            }
            Ok(rows(conn, "SELECT * FROM tags WHERE id=?", [id])?.remove(0))
        }),
        Command::Get {
            transaction: reference,
        } => record(conn, &transaction(conn, scope.profile_id, reference)?),
        Command::Bip329(command) => bip329::execute(conn, scope, command),
        Command::Stale => stale(conn, scope.profile_id),
        Command::Records(options) => records::list(conn, scope, options),
        Command::History(options) => history::list(conn, scope, options),
        Command::Revert {
            transaction,
            event,
            field,
            source,
            reason,
        } => history::revert(
            conn,
            scope,
            transaction,
            event,
            field.as_deref(),
            source,
            reason.as_deref(),
        ),
        _ => edit(conn, scope, command),
    }
}

fn edit(conn: &Connection, scope: &Scope<'_>, command: &Command) -> Result<Value> {
    let (reference, reason) = match command {
        Command::Note {
            transaction,
            reason,
            ..
        }
        | Command::Tag {
            transaction,
            reason,
            ..
        }
        | Command::Excluded {
            transaction,
            reason,
            ..
        }
        | Command::Kind {
            transaction,
            reason,
            ..
        } => (transaction, reason),
        _ => unreachable!("edit command"),
    };
    let tx = transaction(conn, scope.profile_id, reference)?;
    let mut next_tags = None;
    let mut selected_tag = String::new();
    let (field, column, before, after) = match command {
        Command::Note { note, .. } => {
            if note.as_ref().is_some_and(|s| s.chars().count() > 20000) {
                return Err(AppError::new(
                    "validation",
                    "note cannot exceed 20000 characters",
                ));
            }
            (
                "note",
                "note",
                value(&tx, "note"),
                note.as_deref().map_or(Value::Null, s),
            )
        }
        Command::Tag { tag, remove, .. } => {
            let selected = rows(conn,"SELECT * FROM tags WHERE profile_id=? AND (id=? OR lower(code)=lower(?) OR lower(label)=lower(?)) LIMIT 1",[scope.profile_id,tag,tag,tag])?.into_iter().next().ok_or_else(||AppError::new("app_error",format!("Tag '{tag}' not found")))?;
            selected_tag = text(&selected, "code").into();
            let old: Vec<_> = tags(conn, text(&tx, "id"))?
                .iter()
                .map(|row| text(row, "label").to_owned())
                .collect();
            let mut next = old.clone();
            if *remove {
                next.retain(|label| lower(label) != lower(text(&selected, "label")));
            } else {
                next.push(text(&selected, "label").into());
            }
            let mut cleaned = Vec::new();
            let mut seen = BTreeSet::new();
            for label in next {
                let label = strip(&label);
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
                    cleaned.push(label.to_owned());
                }
            }
            let before = Value::List(canonical_tags(old).into_iter().map(s).collect());
            let after = Value::List(canonical_tags(cleaned.clone()).into_iter().map(s).collect());
            next_tags = Some(cleaned);
            ("tags", "", before, after)
        }
        Command::Excluded { excluded, .. } => {
            if *excluded {
                guard_exclusion(conn, scope.profile_id, text(&tx, "id"))?;
            }
            (
                "excluded",
                "excluded",
                Value::Bool(int(&tx, "excluded") != 0),
                Value::Bool(*excluded),
            )
        }
        Command::Kind { kind, .. } => {
            let kind = kind.as_deref().map(strip).filter(|s| !s.is_empty());
            if let Some(kind) = kind {
                if kind.chars().count() > 64 {
                    return Err(AppError::new(
                        "validation",
                        "kind cannot exceed 64 characters",
                    ));
                }
                let expected = match kind {
                    "airdrop" | "buy" | "deposit" | "hardfork" | "income" | "interest"
                    | "lending_interest" | "mining" | "staking" | "wages" => "inbound",
                    "donation" | "gift" | "lost" | "sell" | "spend" | "stolen" | "withdrawal" => {
                        "outbound"
                    }
                    _ => {
                        return Err(AppError::new("validation", "kind is not supported")
                            .with_details(dict([("kind", s(kind))])))
                    }
                };
                if text(&tx, "direction") != expected {
                    return Err(AppError::new("validation",format!("kind '{kind}' can only be applied to {expected} transactions")).with_hint("Income kinds classify what arrived; disposal kinds classify what left. Check the transaction's direction.").with_details(dict([("kind",s(kind)),("direction",value(&tx,"direction"))])));
                }
            }
            (
                "kind",
                "kind_override",
                value(&tx, "kind_override"),
                kind.map_or(Value::Null, s),
            )
        }
        _ => unreachable!("edit command"),
    };
    let reason = clean_reason(reason.as_deref())?;
    let event = if before == after {
        Value::Null
    } else {
        s(atomic(conn, || {
            if let Some(labels) = next_tags {
                let ids = labels
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
            } else {
                let stored = match &after {
                    Value::Null => kassiber_store::rusqlite::types::Value::Null,
                    Value::Bool(b) => {
                        kassiber_store::rusqlite::types::Value::Integer(i64::from(*b))
                    }
                    _ => kassiber_store::rusqlite::types::Value::Text(
                        string(&after).unwrap_or_default().into(),
                    ),
                };
                conn.execute(
                    &format!("UPDATE transactions SET {column}=? WHERE id=?"),
                    params![stored, text(&tx, "id")],
                )
                .map_err(error)?;
            }
            append(conn, scope, &tx, field, &before, &after, reason)
        })?)
    };
    let updated = !matches!(event, Value::Null);
    if matches!(command, Command::Kind { .. }) {
        let mut result = record(conn, &transaction(conn, scope.profile_id, text(&tx, "id"))?)?;
        if let Value::Dict(fields) = &mut result {
            fields.extend([
                ("updated".into(), Value::Bool(updated)),
                ("history_event_id".into(), event),
            ]);
        }
        return Ok(result);
    }
    let mut fields = vec![("transaction_id", value(&tx, "id"))];
    if matches!(command, Command::Tag { .. }) {
        fields.push(("tag", s(selected_tag)));
        fields.push((
            "status",
            s(if !updated {
                "unchanged"
            } else if matches!(command, Command::Tag { remove: true, .. }) {
                "removed"
            } else {
                "added"
            }),
        ));
    } else {
        fields.push((field, after));
    }
    fields.extend([
        ("updated", Value::Bool(updated)),
        ("history_event_id", event),
    ]);
    Ok(dict(fields))
}

fn stale(conn: &Connection, profile: &str) -> Result<Value> {
    let profile_row = rows(conn, "SELECT * FROM profiles WHERE id=?", [profile])?.remove(0);
    let version = int(&profile_row, "last_processed_input_version");
    let summary = rows(conn,"SELECT COUNT(*) AS count,MAX(changed_at) AS latest FROM transaction_edit_events WHERE profile_id=? AND journal_input_version_after>?",params![profile,version])?.remove(0);
    let fields = rows(conn,"SELECT f.field,COUNT(*) AS count FROM transaction_edit_events e JOIN transaction_edit_fields f ON f.event_id=e.id WHERE e.profile_id=? AND e.journal_input_version_after>? GROUP BY f.field",params![profile,version])?;
    let sources = rows(conn,"SELECT source,COUNT(*) AS count FROM transaction_edit_events WHERE profile_id=? AND journal_input_version_after>? GROUP BY source",params![profile,version])?;
    let mut families = BTreeMap::<&str, i64>::new();
    for row in &fields {
        let field = text(row, "field");
        let family = if [
            "review_status",
            "taxable",
            "at_regime",
            "at_category",
            "kind",
        ]
        .contains(&field)
        {
            "tax"
        } else if [
            "fiat_currency",
            "fiat_rate",
            "fiat_value",
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
        ]
        .contains(&field)
        {
            "pricing"
        } else {
            "metadata"
        };
        *families.entry(family).or_default() += int(row, "count");
    }
    Ok(dict([
        ("edit_count", value(&summary, "count")),
        ("latest_changed_at", value(&summary, "latest")),
        (
            "source_counts",
            dict(
                sources
                    .iter()
                    .map(|row| (text(row, "source"), value(row, "count"))),
            ),
        ),
        (
            "family_counts",
            dict(families.into_iter().map(|(k, v)| (k, Value::Int(v.into())))),
        ),
        (
            "field_counts",
            dict(
                fields
                    .iter()
                    .map(|row| (text(row, "field"), value(row, "count"))),
            ),
        ),
        ("last_processed_input_version", Value::Int(version.into())),
        (
            "last_processed_at",
            value(&profile_row, "last_processed_at"),
        ),
    ]))
}
