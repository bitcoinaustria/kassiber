//! Phoenix's CSV adapter and its import-authored notes/tags.
use super::*;

fn integer(row: &Json, key: &str) -> Result<kassiber_pyjson::BigInt, AppError> {
    let raw = optional(row, key).unwrap_or_else(|| "0".into());
    kassiber_pytext::parse_int(&raw)
        .map_err(|_| AppError::new("app_error", format!("Invalid Phoenix {key} '{raw}'")))
}
fn btc(value: &kassiber_pyjson::BigInt) -> Result<Decimal, AppError> {
    dec(&value.to_string())?
        .div(&dec("100000000000")?, &CTX)
        .map_err(decimal_error)
}
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    let Some(first) = rows.first() else {
        return Ok(rows);
    };
    let missing = ["date", "id", "type", "amount_msat"]
        .into_iter()
        .filter(|k| first.data.get(*k).is_none())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(AppError::new(
            "app_error",
            format!(
                "Phoenix CSV is missing required columns: {}",
                missing.join(", ")
            ),
        ));
    }
    rows.into_iter().map(record).collect()
}
fn record(row: csv::Row) -> Result<csv::Row, AppError> {
    let row = row.data;
    let kind = optional(&row, "type").unwrap_or_else(|| "unknown".into());
    let amount_signed = integer(&row, "amount_msat")?;
    let amount = btc(&amount_signed)?.abs(&CTX).map_err(decimal_error)?;
    let direction = if amount_signed < 0.into() {
        "outbound"
    } else if amount_signed > 0.into()
        || [
            "lightning_received",
            "swap_in",
            "legacy_swap_in",
            "legacy_pay_to_open",
        ]
        .contains(&kind.as_str())
    {
        "inbound"
    } else {
        "outbound"
    };
    let mining_fee = integer(&row, "mining_fee_sat")? * 1000;
    let service_fee = integer(&row, "service_fee_msat")?;
    let fee = btc(&(mining_fee + service_fee))?;
    let value = optional(&row, "amount_fiat")
        .map(|value| {
            let first = value
                .split(|c: char| kassiber_pytext::is_space(u32::from(c)))
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            match PriceNumber::parse(first)? {
                PriceNumber::Finite(v) => {
                    v.abs(&CTX).map(PriceNumber::Finite).map_err(decimal_error)
                }
                PriceNumber::Special(v) if v.contains("sNaN") => {
                    Err(decimal_error(DecimalError::InvalidOperation))
                }
                PriceNumber::Special(v) => {
                    Ok(PriceNumber::Special(v.trim_start_matches('-').to_owned()))
                }
            }
        })
        .transpose()?;
    let rate = if !amount.is_zero() {
        value
            .as_ref()
            .map(|v| match v {
                PriceNumber::Finite(v) => v
                    .div(&amount, &CTX)
                    .map(PriceNumber::Finite)
                    .map_err(decimal_error),
                PriceNumber::Special(v) => Ok(PriceNumber::Special(v.clone())),
            })
            .transpose()?
    } else {
        None
    };
    let description = optional(&row, "description");
    let payment_hash = hash(&text(&row, "payment_hash"));
    Ok(csv::Row {
        headers: vec![],
        original: None,
        data: json!({
            "txid":row.get("id"),"occurred_at":row.get("date"),"direction":direction,"asset":"BTC",
            "amount":amount.to_string(),"fee":fee.to_string(),
            "fiat_rate":rate.as_ref().map(PriceNumber::exact).transpose()?,"fiat_value":value.as_ref().map(PriceNumber::exact).transpose()?,
            "kind":kind,"description":description,"counterparty":optional(&row,"destination"),
            "payment_hash":payment_hash,"payment_hash_source":payment_hash.as_ref().map(|_|"importer"),
            "_phoenix_type":kind,"_phoenix_description":description,"raw_json":config::stored_json(&row),
        }),
        rest: None,
    })
}

pub(super) fn metadata(
    conn: &Connection,
    scope: (&str, &str),
    wallet: &str,
    rows: &[csv::Row],
    ids: &mut Ids<'_>,
    result: &mut Vec<(&str, Py)>,
    invalidated: bool,
) -> Result<(), AppError> {
    let (profile, workspace) = scope;
    let mut notes = 0;
    let mut tags = 0;
    let mut created = 0;
    for row in rows {
        let row = &row.data;
        let external = text(row, "txid");
        if external.is_empty() {
            continue;
        }
        let existing:Option<(String,Option<String>)>=conn.query_row("SELECT id,note FROM transactions WHERE profile_id=? AND wallet_id=? AND external_id=? ORDER BY created_at DESC LIMIT 1",[profile,wallet,&external],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error)?;
        let Some((id, note)) = existing else { continue };
        if let Some(description) = optional(row, "_phoenix_description")
            .filter(|_| note.as_ref().is_none_or(String::is_empty))
        {
            conn.execute(
                "UPDATE transactions SET note=? WHERE id=?",
                [description, id.clone()],
            )
            .map_err(database_error)?;
            notes += 1;
        }
        if let Some(kind) = optional(row, "_phoenix_type") {
            let code = kind.to_lowercase().replace(' ', "-");
            let tag: Option<String> = conn
                .query_row(
                    "SELECT id FROM tags WHERE profile_id=? AND code=?",
                    [profile, &code],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            let tag = match tag {
                Some(id) => id,
                None => {
                    let id = ids.next(conn)?;
                    conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)",params![id,workspace,profile,code,kind,ids.time(conn)?]).map_err(database_error)?;
                    created += 1;
                    id
                }
            };
            tags += conn
                .execute(
                    "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                    [id, tag],
                )
                .map_err(database_error)? as i64;
        }
    }
    result.extend([
        ("phoenix_notes_set", Py::from(notes)),
        ("phoenix_tags_added", Py::from(tags)),
        ("phoenix_tags_created", Py::from(created)),
    ]);
    if !invalidated && (notes > 0 || tags > 0) {
        invalidate_journals(conn, profile)?;
        result
            .iter_mut()
            .find(|(k, _)| *k == "journal_invalidated")
            .unwrap()
            .1 = Py::Bool(true);
    }
    Ok(())
}
