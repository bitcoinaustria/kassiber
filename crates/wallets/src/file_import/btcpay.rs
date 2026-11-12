use super::*;
use kassiber_envelope::{get, truthy as py_truthy, type_name};
use kassiber_pyjson::Options;

fn py_str(value: &Py) -> String {
    match value {
        Py::Str(text) => text
            .code_points()
            .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        other => kassiber_envelope::py_str(other),
    }
}

fn value<'a>(row: &'a Py, key: &str) -> &'a Py {
    get(row, key).unwrap_or(&Py::Null)
}

fn case_points(text: &kassiber_pyjson::PyStr, lower: bool) -> kassiber_pyjson::PyStr {
    let convert = |text: &str| {
        if lower {
            kassiber_pytext::lower(text)
        } else {
            kassiber_pytext::upper(text)
        }
    };
    let mut points = Vec::new();
    let mut fragment = String::new();
    for point in text.code_points() {
        if let Some(character) = char::from_u32(point) {
            fragment.push(character);
        } else {
            points.extend(convert(&fragment).chars().map(u32::from));
            fragment.clear();
            points.push(point);
        }
    }
    points.extend(convert(&fragment).chars().map(u32::from));
    kassiber_pyjson::PyStr::from_code_points(points).unwrap()
}

fn labels(input: &Py) -> Vec<Py> {
    let label = |item: &Py| {
        let text = match item {
            Py::Str(text) => kassiber_pytext::strip_py(text),
            other => kassiber_pyjson::PyStr::from(kassiber_pytext::strip(&py_str(other))),
        };
        text.code_points().next().is_some().then_some(Py::Str(text))
    };
    match input {
        Py::Null => vec![],
        Py::Dict(items) => items
            .iter()
            .filter_map(|(key, item)| {
                let text = if matches!(item, Py::Dict(_)) {
                    value(item, "text")
                } else {
                    item
                };
                if *text == Py::Null {
                    let kassiber_pyjson::Key::Str(key) = key else {
                        return None;
                    };
                    label(&Py::Str(key.clone()))
                } else {
                    label(text)
                }
            })
            .collect(),
        Py::List(items) => items
            .iter()
            .filter_map(|item| {
                let text = if matches!(item, Py::Dict(_)) {
                    value(item, "text")
                } else {
                    item
                };
                (*text != Py::Null).then(|| label(text)).flatten()
            })
            .collect(),
        Py::Str(text) => {
            let points = text.code_points().collect::<Vec<_>>();
            points
                .split(|p| *p == u32::from(','))
                .filter_map(|points| {
                    label(&Py::Str(
                        kassiber_pyjson::PyStr::from_code_points(points.iter().copied()).unwrap(),
                    ))
                })
                .collect()
        }
        other => py_str(other)
            .split(',')
            .filter_map(|v| label(&s(v)))
            .collect(),
    }
}

pub(super) fn record(row: Py) -> Result<csv::Row, AppError> {
    if !matches!(row, Py::Dict(_)) {
        return Err(AppError::new(
            "internal_error",
            format!("'{}' object has no attribute 'items'", type_name(&row)),
        ));
    }
    let currency = asset(&py_str(
        get(&row, "Currency")
            .filter(|v| py_truthy(v))
            .unwrap_or(&s("BTC")),
    ));
    let currency_binding = match value(&row, "Currency") {
        Py::Str(text) if text.has_surrogates() => {
            Py::Str(case_points(&kassiber_pytext::strip_py(text), false))
        }
        _ => s(currency.clone()),
    };
    let currency = py_str(&currency_binding);
    let amount = value(&row, "Amount");
    if *amount == Py::Null {
        return Err(AppError::new(
            "app_error",
            "BTCPay export is missing Amount",
        ));
    }
    let original_amount = amount;
    let mut amount = kassiber_pytext::strip(&py_str(amount)).to_owned();
    let suffix = if currency.is_empty() {
        "BTC"
    } else {
        &currency
    };
    for (index, suffix) in [
        suffix.to_owned(),
        kassiber_pytext::lower(suffix),
        suffix.to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        if let Py::Str(text) = &currency_binding {
            if text.has_surrogates() {
                let Py::Str(original_amount) = original_amount else {
                    continue;
                };
                let exact = if index == 1 {
                    case_points(text, true)
                } else {
                    text.clone()
                };
                if !kassiber_pytext::strip_py(original_amount)
                    .code_points()
                    .collect::<Vec<_>>()
                    .ends_with(&exact.code_points().collect::<Vec<_>>())
                {
                    continue;
                }
            }
        }
        if let Some(prefix) = amount.strip_suffix(&suffix) {
            amount = kassiber_pytext::strip(prefix).into();
            break;
        }
    }
    let amount = PriceNumber::parse(&amount)?;
    let outbound = match &amount {
        PriceNumber::Finite(n) => n.is_negative() && !n.is_zero(),
        PriceNumber::Special(n) if n.contains("NaN") => {
            return Err(decimal_error(DecimalError::InvalidOperation))
        }
        PriceNumber::Special(n) => n.starts_with('-'),
    };
    let amount = river::repr(&river::abs(amount)?);
    let comment = value(&row, "Comment").clone();
    let normalized = dict([
        (
            "txid",
            get(&row, "TransactionId")
                .filter(|v| py_truthy(v))
                .unwrap_or(value(&row, "Transaction Id"))
                .clone(),
        ),
        ("occurred_at", value(&row, "Timestamp").clone()),
        (
            "confirmed_at",
            get(&row, "confirmed_at")
                .filter(|v| py_truthy(v))
                .unwrap_or(value(&row, "ConfirmedAt"))
                .clone(),
        ),
        (
            "direction",
            s(if outbound { "outbound" } else { "inbound" }),
        ),
        ("asset", currency_binding),
        ("amount", s(amount)),
        ("fee", s("0")),
        ("amount_includes_fee", Py::Bool(outbound)),
        ("fiat_rate", Py::Null),
        ("fiat_value", Py::Null),
        ("kind", s(if outbound { "withdrawal" } else { "deposit" })),
        (
            "description",
            if py_truthy(&comment) {
                comment.clone()
            } else {
                s("Imported from BTCPay")
            },
        ),
        ("counterparty", Py::Null),
        ("_btcpay_comment", comment),
        ("_btcpay_labels", Py::List(labels(value(&row, "Labels")))),
        (
            "raw_json",
            s(kassiber_pyjson::dumps(
                &row,
                &Options {
                    sort_keys: true,
                    ..Default::default()
                },
            )
            .map_err(|e| AppError::new("internal_error", e.to_string()))?),
        ),
    ]);
    Ok(csv::Row {
        data: generic_json::projection(&normalized),
        headers: vec![],
        rest: None,
        original: Some(normalized),
    })
}

pub(super) fn load(file: &str, json: bool) -> Result<Vec<csv::Row>, AppError> {
    let rows = if json {
        let payload = csv::read_json(file, "BTCPay JSON export", true)?;
        let Py::List(rows) = payload else {
            return Err(AppError::new(
                "app_error",
                "BTCPay JSON export must be a list of transaction objects",
            ));
        };
        rows
    } else {
        csv::load(file)?
            .into_iter()
            .map(|r| kassiber_pytext::loads(&r.data.to_string()).expect("CSV JSON projection"))
            .collect()
    };
    rows.into_iter().map(record).collect()
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
    let (mut notes, mut tags, mut created) = (0, 0, 0);
    for row in rows {
        let row = row.original.as_ref().expect("BTCPay record");
        // `str(record.get("txid") or record.get("id") or "")` (464f82094):
        // the string identity observation storage uses, so a boolean source id
        // cannot reach a numeric neighbour.
        let Some(external) = [value(row, "txid"), value(row, "id")]
            .into_iter()
            .find(|v| py_truthy(v))
        else {
            continue;
        };
        let bound = match external {
            Py::Str(text) if text.has_surrogates() => {
                return Err(crate::unicode_binding_error(text))
            }
            other => py_str(other),
        };
        let existing: Option<(String, Option<String>)> = conn.query_row("SELECT id,note FROM transactions WHERE profile_id=? AND wallet_id=? AND external_id=? ORDER BY created_at DESC LIMIT 1", params![profile, wallet, bound], |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(database_error)?;
        let Some((id, note)) = existing else { continue };
        let comment = value(row, "_btcpay_comment");
        let comment = if *comment == Py::Null {
            String::new()
        } else {
            kassiber_pytext::strip(&py_str(comment)).into()
        };
        if !comment.is_empty() && note.as_ref().is_none_or(String::is_empty) {
            conn.execute("UPDATE transactions SET note=? WHERE id=?", [&comment, &id])
                .map_err(database_error)?;
            notes += 1;
        }
        if let Py::List(labels) = value(row, "_btcpay_labels") {
            for label in labels {
                if let Py::Str(text) = label {
                    if text.has_surrogates() {
                        let points = text.code_points().collect::<Vec<_>>();
                        let mut normalized = Vec::new();
                        let mut start = 0;
                        for (at, point) in points.iter().enumerate() {
                            if (0xd800..=0xdfff).contains(point) {
                                let prefix = points[start..at]
                                    .iter()
                                    .filter_map(|p| char::from_u32(*p))
                                    .collect::<String>();
                                normalized.extend(
                                    kassiber_pytext::lower(&prefix)
                                        .replace(' ', "-")
                                        .chars()
                                        .map(u32::from),
                                );
                                normalized.push(*point);
                                start = at + 1;
                            }
                        }
                        let suffix = points[start..]
                            .iter()
                            .filter_map(|p| char::from_u32(*p))
                            .collect::<String>();
                        normalized.extend(
                            kassiber_pytext::lower(&suffix)
                                .replace(' ', "-")
                                .chars()
                                .map(u32::from),
                        );
                        let code =
                            Py::Str(kassiber_pyjson::PyStr::from_code_points(normalized).unwrap());
                        generic_json::metadata(&dict([("description", code)]), "description")?;
                    }
                }
                let label = py_str(label);
                let code = kassiber_pytext::lower(&label).replace(' ', "-");
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
                        conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES(?,?,?,?,?,?)", params![id, workspace, profile, code, label, ids.time(conn)?]).map_err(database_error)?;
                        created += 1;
                        id
                    }
                };
                tags += conn
                    .execute(
                        "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) VALUES(?,?)",
                        [&id, &tag],
                    )
                    .map_err(database_error)? as i64;
            }
        }
    }
    result.extend([
        ("btcpay_notes_set", Py::from(notes)),
        ("btcpay_tags_added", Py::from(tags)),
        ("btcpay_tags_created", Py::from(created)),
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
