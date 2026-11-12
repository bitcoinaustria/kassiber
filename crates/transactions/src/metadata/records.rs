use super::*;
use kassiber_store::rusqlite::{params_from_iter, types::Value as SqlValue};

#[derive(Default)]
pub struct RecordOptions {
    pub wallet: Option<String>,
    pub tag: Option<String>,
    pub has_note: Option<bool>,
    pub excluded: Option<bool>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

pub(crate) fn wallet(conn: &Connection, profile: &str, reference: Option<&str>) -> Result<String> {
    let Some(reference) = reference.filter(|s| !s.is_empty()) else {
        return Ok(String::new());
    };
    let normalized = strip(reference);
    if let Some(row) = rows(
        conn,
        "SELECT id FROM wallets WHERE profile_id=? AND id=? LIMIT 1",
        [profile, normalized],
    )?
    .first()
    {
        return Ok(text(row, "id").into());
    }
    let found = rows(conn, "SELECT w.id,w.label,a.code AS account_code FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=? AND lower(w.label)=lower(?) ORDER BY w.label ASC,w.id ASC", [profile, normalized])?;
    if found.len() > 1 {
        return Err(AppError::new(
            "validation",
            format!("Wallet label '{reference}' is ambiguous"),
        )
        .with_hint("Use the wallet id instead of the non-unique label.")
        .with_details(dict([("matches", Value::List(found))])));
    }
    found
        .first()
        .map(|row| text(row, "id").to_owned())
        .ok_or_else(|| AppError::new("not_found", format!("Wallet '{reference}' not found")))
}

pub(super) fn list(conn: &Connection, scope: &Scope<'_>, options: &RecordOptions) -> Result<Value> {
    let limit = cursor::limit(options.limit, 100)?;
    let start = time::timestamp(options.start.as_deref(), "start")?;
    let end = time::timestamp(options.end.as_deref(), "end")?;
    let wallet = wallet(conn, scope.profile_id, options.wallet.as_deref())?;
    let tag = if let Some(reference) = options.tag.as_deref().filter(|s| !s.is_empty()) {
        let found = rows(conn, "SELECT id FROM tags WHERE profile_id=? AND (id=? OR lower(code)=lower(?) OR lower(label)=lower(?)) LIMIT 1", [scope.profile_id,reference,reference,reference])?;
        found
            .first()
            .map(|row| text(row, "id").to_owned())
            .ok_or_else(|| AppError::new("app_error", format!("Tag '{reference}' not found")))?
    } else {
        String::new()
    };
    let filters = dict([
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("wallet_id", s(wallet.as_str())),
        ("tag_id", s(tag.as_str())),
        (
            "has_note",
            options.has_note.map_or(Value::Null, Value::Bool),
        ),
        (
            "excluded",
            options.excluded.map_or(Value::Null, Value::Bool),
        ),
        ("start", s(start.as_str())),
        ("end", s(end.as_str())),
    ]);
    let mut conditions = vec!["t.profile_id=?".to_owned()];
    let mut args = vec![SqlValue::Text(scope.profile_id.into())];
    for (clause, val) in [("t.wallet_id=?",wallet),("EXISTS(SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id=t.id AND tt.tag_id=?)",tag)] {
        if !val.is_empty() { conditions.push(clause.into()); args.push(SqlValue::Text(val)); }
    }
    for (op, val) in [(">=", start), ("<=", end)] {
        if !val.is_empty() {
            conditions.push(time::predicate(conn, "t.occurred_at", op)?);
            args.push(SqlValue::Text(val));
        }
    }
    if let Some(has_note) = options.has_note {
        conditions.push(
            if has_note {
                "t.note IS NOT NULL AND t.note!=''"
            } else {
                "(t.note IS NULL OR t.note='')"
            }
            .into(),
        );
    }
    if let Some(excluded) = options.excluded {
        conditions.push("t.excluded=?".into());
        args.push(SqlValue::Integer(i64::from(excluded)));
    }
    const KEYS: &[&str] = &["occurred_at", "created_at", "id"];
    if let Some(row) = cursor::decode(options.cursor.as_deref(), &filters, KEYS, false)? {
        conditions.push("(t.occurred_at,t.created_at,t.id)<(?,?,?)".into());
        for key in KEYS {
            args.push(cursor::bind(&value(&row, key))?);
        }
    }
    args.push(SqlValue::Integer(limit + 1));
    let mut page = rows(conn,&format!("SELECT t.* FROM transactions t WHERE {} ORDER BY t.occurred_at DESC,t.created_at DESC,t.id DESC LIMIT ?",conditions.join(" AND ")),params_from_iter(args))?;
    let more = page.len() > limit as usize;
    page.truncate(limit as usize);
    let next = if more {
        cursor::encode(page.last().unwrap(), &filters, KEYS)?
    } else {
        Value::Null
    };
    let mut records = Vec::new();
    for tx in page {
        let mut row = record(conn, &tx)?;
        if let Value::Dict(fields) = &mut row {
            fields.retain(|(k, _)| {
                ![
                    "fiat_currency",
                    "fiat_rate",
                    "fiat_value",
                    "fiat_rate_exact",
                    "fiat_value_exact",
                    "pricing_source_kind",
                    "pricing_quality",
                    "pricing_external_ref",
                    "review_status",
                    "taxable",
                    "at_regime",
                    "at_category",
                    "kind",
                    "kind_override",
                ]
                .iter()
                .any(|key| k == &kassiber_pyjson::Key::from(*key))
            });
        }
        records.push(row);
    }
    Ok(dict([
        ("records", Value::List(records)),
        ("next_cursor", next),
        ("has_more", Value::Bool(more)),
        ("limit", Value::Int(limit.into())),
    ]))
}
