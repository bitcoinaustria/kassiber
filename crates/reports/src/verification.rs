use super::*;
use artifact_csv::list;
use std::collections::BTreeMap;

pub(super) fn number(row: &Value, key: &str) -> f64 {
    match value(row, key) {
        Value::Float(n) => *n,
        Value::Int(n) => n.to_string().parse().unwrap(),
        _ => 0.0,
    }
}
fn detail(raw: &Value) -> String {
    if !truthy(raw) {
        return String::new();
    }
    let raw = py_str(raw);
    let result = match kassiber_pytext::loads(&raw) {
        Ok(Value::Dict(fields)) => fields
            .iter()
            .map(|(k, v)| {
                let kassiber_pyjson::Key::Str(k) = k else {
                    unreachable!()
                };
                format!("{}={}", k.as_str().unwrap(), py_str(v))
            })
            .collect::<Vec<_>>()
            .join("; "),
        Ok(v) => py_str(&v),
        Err(_) => raw,
    };
    result.chars().take(200).collect()
}
pub(super) fn project(conn: &Connection, profile: &Value) -> Result<Value> {
    let id = text(profile, "id");
    let tags = rows(
        conn,
        r#"
        SELECT
            tt.transaction_id,tg.code
        FROM transaction_tags tt
        JOIN tags tg ON tg.id=tt.tag_id
        JOIN transactions t ON t.id=tt.transaction_id
        WHERE t.profile_id=? AND t.excluded=0
        ORDER BY tg.code
        "#,
        [&id],
    )?;
    let mut tag_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in tags {
        tag_map
            .entry(text(&row, "transaction_id"))
            .or_default()
            .push(text(&row, "code"));
    }
    let ledger = rows(
        conn,
        r#"
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id AS internal_tx_id,
            COALESCE(NULLIF(t.external_id,''),
            je.transaction_id) AS transaction_id,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis,0) AS cost_basis,
            COALESCE(je.proceeds,0) AS proceeds,
            COALESCE(je.gain_loss,0) AS gain_loss,
            COALESCE(je.description,'') AS description,
            COALESCE(je.pricing_source_kind,'') AS pricing_source,
            COALESCE(je.pricing_quality,'') AS pricing_quality,
            CASE WHEN COALESCE(t.taxability_override,1)!=0 AND COALESCE(je.at_category,'')!='neu_swap' THEN 1 ELSE 0 END AS taxable
        FROM journal_entries je
        JOIN wallets w ON w.id=je.wallet_id
        LEFT JOIN transactions t ON t.id=je.transaction_id
        WHERE je.profile_id=?
        ORDER BY je.occurred_at ASC,
            je.created_at ASC,
            je.id ASC
        "#,
        [&id],
    )?;
    let (mut acquisitions, mut disposals) = (Vec::new(), Vec::new());
    for row in ledger {
        let qty = export_completion::integer(value(&row, "quantity"))?;
        let qty = if qty < 0.into() { -qty } else { qty };
        let quantity = decimal(&Value::Int(qty.clone()))?
            .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
            .map_err(arithmetic)?;
        let mut common = dict(
            [
                "occurred_at",
                "wallet",
                "transaction_id",
                "asset",
                "entry_type",
                "description",
                "pricing_source",
                "pricing_quality",
                "taxable",
            ]
            .map(|k| (k, value(&row, k).clone())),
        );
        replace(&mut common, "quantity_msat", Value::Int(qty));
        replace(&mut common, "quantity", float(&quantity)?);
        replace(
            &mut common,
            "tags",
            s(tag_map
                .get(&text(&row, "internal_tx_id"))
                .map(|v| v.join(", "))
                .unwrap_or_default()),
        );
        let kind = text(&row, "entry_type");
        if ["acquisition", "income", "transfer_in"].contains(&kind.as_str()) {
            for key in ["fiat_value", "gain_loss"] {
                replace(&mut common, key, Value::Float(number(&row, key)));
            }
            acquisitions.push(common);
        } else if ["disposal", "fee", "transfer_fee", "transfer_out"].contains(&kind.as_str()) {
            for key in ["proceeds", "cost_basis", "gain_loss"] {
                replace(&mut common, key, Value::Float(number(&row, key)));
            }
            replace(
                &mut common,
                "gain_loss_kassiber",
                Value::Float(number(&row, "gain_loss")),
            );
            disposals.push(common);
        }
    }
    let quarantines = rows(
        conn,
        r#"
        SELECT
            jq.reason,jq.detail_json,
            COALESCE(NULLIF(t.external_id,''),jq.transaction_id) AS transaction_id,
            t.occurred_at,
            t.asset,
            t.amount,
            COALESCE(t.description,'') AS description
        FROM journal_quarantines jq
        LEFT JOIN transactions t ON t.id=jq.transaction_id
        WHERE jq.profile_id=?
        ORDER BY t.occurred_at ASC,jq.transaction_id ASC
        "#,
        [&id],
    )?
    .iter()
    .map(|row| {
        let mut result =
            dict(["transaction_id", "reason", "description"].map(|k| (k, value(row, k).clone())));
        for key in ["occurred_at", "asset"] {
            replace(
                &mut result,
                key,
                if truthy(value(row, key)) {
                    value(row, key).clone()
                } else {
                    s("")
                },
            );
        }
        replace(
            &mut result,
            "amount",
            if matches!(value(row, "amount"), Value::Null) {
                s("")
            } else {
                float(&btc(int(row, "amount"))?)?
            },
        );
        replace(&mut result, "detail", s(detail(value(row, "detail_json"))));
        Ok(result)
    })
    .collect::<Result<Vec<_>>>()?;
    let attachments = rows(
        conn,
        r#"
        SELECT
            t.occurred_at,
            w.label AS wallet,
            COALESCE(t.external_id,'') AS transaction_id,
            t.asset,
            a.attachment_type AS kind,
            COALESCE(a.label,'') AS label,
            COALESCE(a.original_filename,'') AS filename,
            COALESCE(a.source_url,'') AS url
        FROM attachments a
        JOIN transactions t ON t.id=a.transaction_id
        JOIN wallets w ON w.id=t.wallet_id
        WHERE a.profile_id=? AND t.excluded=0
        ORDER BY t.occurred_at ASC,
            a.created_at ASC
        "#,
        [&id],
    )?
    .iter()
    .map(|row| {
        let label = text(row, "label");
        let label = kassiber_pytext::strip(&label);
        let url = text(row, "url");
        let url = kassiber_pytext::strip(&url);
        let filename = text(row, "filename");
        let filename = kassiber_pytext::strip(&filename);
        let is_url = text(row, "kind").to_lowercase() == "url";
        let name = if is_url {
            if !label.is_empty() {
                label
            } else if !url.is_empty() {
                url
            } else {
                "(link)"
            }
        } else if !filename.is_empty() {
            filename
        } else if !label.is_empty() {
            label
        } else {
            "(file)"
        };
        let mut result = dict(
            ["occurred_at", "wallet", "transaction_id", "asset"].map(|k| {
                (
                    k,
                    if truthy(value(row, k)) {
                        value(row, k).clone()
                    } else {
                        s("")
                    },
                )
            }),
        );
        for (key, v) in [
            ("type", if is_url { "url" } else { "file" }),
            ("name", name),
            ("url", if is_url { url } else { "" }),
            ("reference", if is_url { url } else { filename }),
        ] {
            replace(&mut result, key, s(v));
        }
        result
    })
    .collect::<Vec<_>>();
    let mut assets: BTreeMap<String, Value> = BTreeMap::new();
    for row in list(&holdings::report(conn, profile, true)?) {
        let bucket = assets
            .entry(text(row, "asset"))
            .or_insert_with(|| dict::<&str>([]));
        for key in ["quantity", "cost_basis", "market_value", "unrealized_pnl"] {
            let n = number(bucket, key) + number(row, key);
            replace(bucket, key, Value::Float(n));
        }
    }
    for row in list(&capital_gains(conn, profile)?) {
        let bucket = assets
            .entry(text(row, "asset"))
            .or_insert_with(|| dict::<&str>([]));
        let n = number(bucket, "realized_gain") + number(row, "gain_loss");
        replace(bucket, "realized_gain", Value::Float(n));
    }
    let cache_exists: bool = conn
        .query_row(
            r#"
        SELECT
            EXISTS(SELECT 1
        FROM sqlite_master
        WHERE type='table' AND name='rates_cache')
        "#,
            [],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let mut asset_rows = Vec::new();
    for (asset, mut row) in assets {
        let quantity = number(&row, "quantity");
        let basis = number(&row, "cost_basis");
        let market = number(&row, "market_value");
        let mut source = String::new();
        let mut timestamp = String::new();
        if get(&row, "quantity").is_some() {
            source = "transaction price (no market quote)".into();
            if let Some(pair) =
                kassiber_rates::transaction_rate_pair(&asset, &text(profile, "fiat_currency"))
                    .filter(|_| cache_exists)
            {
                match kassiber_rates::latest(conn, &pair) {
                    Ok(rate) => {
                        if truthy(value(&rate, "source")) {
                            source = text(&rate, "source");
                        }
                        if truthy(value(&rate, "timestamp")) {
                            timestamp = text(&rate, "timestamp");
                        }
                    }
                    Err(e) if e.code == "not_found" => (),
                    Err(e) => return Err(e),
                }
            }
        }
        replace(&mut row, "asset", s(asset));
        replace(
            &mut row,
            "rate",
            Value::Float(if quantity != 0.0 {
                market / quantity
            } else {
                0.0
            }),
        );
        replace(&mut row, "rate_source", s(source));
        replace(&mut row, "rate_as_of", s(timestamp));
        for key in [
            "quantity",
            "cost_basis",
            "market_value",
            "unrealized_pnl",
            "realized_gain",
        ] {
            let n = number(&row, key);
            replace(&mut row, key, Value::Float(n));
        }
        for (key, original) in [
            ("holdings_qty_kassiber", "quantity"),
            ("cost_basis_kassiber", "cost_basis"),
            ("market_value_kassiber", "market_value"),
            ("unrealized_kassiber", "unrealized_pnl"),
            ("realized_gain_kassiber", "realized_gain"),
        ] {
            let n = number(&row, original);
            replace(&mut row, key, Value::Float(n));
        }
        replace(
            &mut row,
            "avg_cost_kassiber",
            Value::Float(if quantity != 0.0 {
                basis / quantity
            } else {
                0.0
            }),
        );
        asset_rows.push(row);
    }
    Ok(dict([
        ("acquisitions", Value::List(acquisitions)),
        ("disposals", Value::List(disposals)),
        ("quarantines", Value::List(quarantines)),
        ("attachments", Value::List(attachments)),
        ("asset_rows", Value::List(asset_rows)),
    ]))
}
