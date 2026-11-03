use super::*;

pub(super) fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    args: Option<&Value>,
    pair_reading: &dyn Fn(&str, &str) -> Result<Value>,
) -> Result<Value> {
    if scope.workspace_id.is_empty() || scope.profile_id.is_empty() {
        return Ok(dict([("transaction", Value::Null), ("query", s(""))]));
    }
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("ui snapshot args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    unknown(args, "ui.transactions.resolve", &["query"])?;
    let query = get(args, "query")
        .and_then(crate::sql::string)
        .map(strip)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| invalid("ui.transactions.resolve query must be a non-empty string"))?;
    let mut ids = vec![query.to_owned()];
    if !ids.contains(&lower(query)) {
        ids.push(lower(query));
    }
    let mut external_ids = ids.clone();
    if !external_ids.contains(&upper(query)) {
        external_ids.push(upper(query));
    }
    let mut selected = None;
    for (column, candidates) in [("t.id", ids), ("t.external_id", external_ids)] {
        let mut bindings = vec![SqlValue::Text(scope.profile_id.into())];
        bindings.extend(candidates.iter().cloned().map(SqlValue::Text));
        let order_limit = if column == "t.id" {
            ""
        } else {
            " ORDER BY t.occurred_at DESC,t.created_at DESC,t.id DESC LIMIT 1"
        };
        let sql = format!(
            "SELECT {COLUMNS},{} AS payment_method {FROM} WHERE t.profile_id=? AND {column} IN ({}){order_limit}",
            sql::PAYMENT,
            vec!["?"; candidates.len()].join(",")
        );
        selected = rows(conn, &sql, params_from_iter(bindings))?
            .into_iter()
            .next();
        if selected.is_some() {
            break;
        }
    }
    let transaction = if let Some(row) = selected {
        let current = current(conn, scope)?;
        let pairs = render::pairs(conn, std::slice::from_ref(&row), current)?;
        let pair = pairs.get(text(&row, "id"));
        let mut payload = render::row(conn, &row, pair)?;
        if let Some(pair) = pair {
            let focused = lower(text(&row, "id")) == lower(query);
            if focused {
                let sign = if text(&row, "direction") == "inbound" {
                    1
                } else {
                    -1
                };
                set(
                    &mut payload,
                    "amountSat",
                    render::sat(sign * num(&row, "amount")),
                );
                set(&mut payload, "feeSat", render::sat(num(&row, "fee")));
                set(
                    &mut payload,
                    "rate",
                    render::positive(&val(&row, "fiat_rate")),
                );
                let eur = match render::positive(&val(&row, "fiat_value")) {
                    Value::Float(n) => Value::Float(sign as f64 * n.abs()),
                    _ => Value::Null,
                };
                set(&mut payload, "eur", eur);
                set(&mut payload, "account", val(&row, "wallet"));
                set(
                    &mut payload,
                    "counter",
                    s(format!(
                        "{} {} -> {}",
                        text(pair, "label"),
                        text(pair, "out_asset"),
                        text(pair, "in_asset")
                    )),
                );
            }
            if text(pair, "kind") != "journal-derived" {
                let ranks = rows(
                    conn,
                    &queue_items(conn, "", "", "", current)?,
                    [scope.profile_id],
                )?;
                if let Some(rank) = ranks.iter().find(|rank| val(rank, "id") == val(&row, "id")) {
                    set(
                        &mut payload,
                        "queueStatus",
                        s(match num(rank, "item_rank") {
                            3 => "review",
                            2 => "failed",
                            1 => "pending",
                            _ => "completed",
                        }),
                    );
                }
            }
            let reading = pair_reading(
                text(pair, "out_transaction_id"),
                text(pair, "in_transaction_id"),
            )?;
            let mut displayed = val(&payload, "pair");
            if focused {
                set(&mut displayed, "feeSat", render::sat(num(&row, "fee")));
            }
            for key in ["reviewPairId", "pairFingerprint"] {
                set(&mut displayed, key, val(&reading, key));
            }
            set(&mut payload, "pair", displayed);
        }
        payload
    } else {
        Value::Null
    };
    Ok(dict([
        ("transaction", transaction),
        ("query", s(query)),
        ("workspaceId", s(scope.workspace_id)),
        ("profileId", s(scope.profile_id)),
    ]))
}
