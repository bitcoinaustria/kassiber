use super::*;
use artifact_csv::{File, Prepared};
use std::collections::{BTreeMap, BTreeSet};
use year_handoff::{classify, Asset, Facts};

pub(super) fn platform(raw: &Value) -> Result<&'static str> {
    let original = py_str(raw);
    let raw = if truthy(raw) {
        original.clone()
    } else {
        String::new()
    };
    match kassiber_pytext::lower(kassiber_pytext::strip(&raw)).as_str() {
        "cointracking" => Ok("cointracking"),
        "blockpit" => Ok("blockpit"),
        _ => Err(AppError::new(
            "validation",
            format!("Unknown export format '{}'", original),
        )
        .with_hint("Use one of: blockpit, cointracking.")),
    }
}

const CT: &[&str] = &[
    "Type",
    "Buy Amount",
    "Buy Currency",
    "Sell Amount",
    "Sell Currency",
    "Fee",
    "Fee Currency",
    "Exchange",
    "Trade-Group",
    "Comment",
    "Date",
    "Tx-ID",
    "Buy Value in Account Currency",
    "Sell Value in Account Currency",
];
const BP: &[&str] = &[
    "Date (UTC)",
    "Integration Name",
    "Label",
    "Outgoing Asset",
    "Outgoing Amount",
    "Incoming Asset",
    "Incoming Amount",
    "Fee Asset (optional)",
    "Fee Amount (optional)",
    "Comment (optional)",
    "Trx. ID (optional)",
];

fn event<'a>(
    classification: &str,
    direction: &str,
    kind: &'a str,
    fiat: bool,
    conversion: bool,
    non_taxable: bool,
) -> (Option<&'a str>, Option<&'static str>) {
    let fallback = if direction == "inbound" {
        "deposit"
    } else {
        "withdrawal"
    };
    if classification.contains('+') {
        return if classification
            .split('+')
            .any(|c| ["income", "acquisition", "disposal"].contains(&c))
        {
            (None, Some("mixed"))
        } else {
            (Some(fallback), Some("swap"))
        };
    }
    if conversion && ["disposal", "acquisition"].contains(&classification) {
        return if non_taxable {
            (None, Some("non_taxable_conversion"))
        } else {
            (
                Some(if classification == "disposal" {
                    "sell"
                } else {
                    "buy"
                }),
                Some("conversion"),
            )
        };
    }
    if non_taxable && classification == "income" {
        return (Some("income_non_taxable"), None);
    }
    if non_taxable && classification == "disposal" {
        return (Some("payment_non_taxable"), None);
    }
    match classification {
        "internal_transfer" => (Some(fallback), None),
        "swap" => (Some(fallback), Some("swap")),
        "income" => (
            Some(match kind {
                "mining" | "mining_reward" => "mining",
                "staking" => "staking",
                "airdrop" => "airdrop",
                "interest" | "lending_interest" => "interest",
                _ => "income",
            }),
            None,
        ),
        "acquisition" => (
            Some(if kind == "buy" && fiat {
                "buy"
            } else {
                "deposit"
            }),
            None,
        ),
        "disposal" => (
            Some(if kind == "sell" && fiat {
                "sell"
            } else {
                "payment"
            }),
            None,
        ),
        "fee" => (Some("fee"), None),
        _ => (Some(fallback), Some("unbooked")),
    }
}
fn number(amount: Option<&Decimal>) -> Result<String> {
    let Some(amount) = amount else {
        return Ok(String::new());
    };
    let text = amount
        .normalize(&DECIMAL)
        .and_then(|d| d.to_fixed_string())
        .map_err(arithmetic)?;
    Ok(if text == "-0" || text.is_empty() {
        "0".into()
    } else {
        text
    })
}
fn timestamp(raw: &str) -> Result<String> {
    let raw_z = raw.replace('Z', "+00:00");
    let moment = (raw_z == kassiber_pytext::strip(&raw_z))
        .then(|| kassiber_rates::time::parse(&raw_z))
        .flatten()
        .ok_or_else(|| {
            AppError::new(
                "internal_error",
                format!(
                    "Invalid isoformat string: {}",
                    kassiber_pytext::repr_str(&raw_z)
                ),
            )
        })?;
    kassiber_rates::time::format(moment)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))
}
fn contains(range: &Value, raw: &str, possible: bool) -> Result<bool> {
    if matches!(range, Value::Null) {
        return Ok(true);
    }
    if possible && undated(raw) {
        return Ok(true);
    }
    if raw.is_empty() {
        return Ok(false);
    }
    let moment = kassiber_rates::time::parse(&timestamp(raw)?).unwrap();
    Ok(
        kassiber_rates::time::parse(&text(range, "start")).unwrap() <= moment
            && moment < kassiber_rates::time::parse(&text(range, "end")).unwrap(),
    )
}
fn undated(raw: &str) -> bool {
    raw.is_empty() || raw == "1970-01-01T00:00:00Z"
}
fn scope(wallets: &[kassiber_wallets::Wallet]) -> String {
    if wallets.len() > 3 {
        format!("{} wallets", wallets.len())
    } else {
        wallets
            .iter()
            .map(|w| w.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(super) fn prepare(conn: &Connection, profile: &Value, args: &Value) -> Result<Prepared> {
    let (output, metadata) = table(conn, profile, args)?;
    let count = output.len() - 1;
    Ok(Prepared {
        files: vec![File {
            name: "platform.csv".into(),
            sheet: String::new(),
            bytes: artifact_csv::encode(&output),
            rows: count,
        }],
        metadata,
        completion: None,
    })
}

/// The import file's header and rows with the export's metadata, unencoded.
pub(super) fn table(
    conn: &Connection,
    profile: &Value,
    args: &Value,
) -> Result<(Vec<Vec<Value>>, Value)> {
    let target = platform(value(args, "platform"))?;
    require_context(conn, profile, false)?;
    export_request::freshness(conn, profile)?;
    let id = text(profile, "id");
    let refs = match value(args, "wallets") {
        Value::Null => {
            if truthy(value(args, "wallet")) {
                Some(vec![value(args, "wallet").clone()])
            } else {
                None
            }
        }
        Value::List(refs) => Some(refs.clone()),
        _ => return Err(AppError::new("validation", "wallets must be a list")),
    };
    let mut wallets = Vec::new();
    if let Some(refs) = refs {
        if refs.is_empty() {
            return Err(AppError::new(
                "validation",
                "Select at least one wallet to export",
            ));
        }
        for reference in refs {
            let wallet = kassiber_wallets::resolve(conn, &id, &py_str(&reference))?;
            if !wallets
                .iter()
                .any(|w: &kassiber_wallets::Wallet| w.id == wallet.id)
            {
                wallets.push(wallet);
            }
        }
    }
    let range = value(args, "export_range");
    let mut blocking = Vec::new();
    for row in rows(conn,"SELECT jq.transaction_id,jq.reason,t.occurred_at,t.wallet_id,COALESCE(t.external_id,'') AS external_id,w.label AS wallet FROM journal_quarantines jq JOIN transactions t ON t.id=jq.transaction_id JOIN wallets w ON w.id=t.wallet_id WHERE jq.profile_id=? ORDER BY t.occurred_at,jq.transaction_id",[&id])? {
        if (wallets.is_empty() || wallets.iter().any(|w| w.id == text(&row,"wallet_id"))) && contains(range,&text(&row,"occurred_at"),true)? { blocking.push(row); }
    }
    if !blocking.is_empty() {
        return Err(AppError::new("transaction_export_quarantined", "This export is blocked by quarantined transactions.")
            .with_hint("Resolve or exclude every quarantined transaction in this scope, then process journals again. The target tool would otherwise receive a guessed event.")
            .with_details(dict([("wallet",s(scope(&wallets))), ("count",Value::Int(blocking.len().into())), ("transactions",Value::List(blocking.iter().take(20).map(|r| dict([
                ("transaction_ref",value(r,"transaction_id").clone()), ("transaction_id",value(r,"external_id").clone()), ("wallet",value(r,"wallet").clone()), ("occurred_at",value(r,"occurred_at").clone()), ("reason",value(r,"reason").clone())
            ])).collect()))])));
    }
    let wallet = if wallets.len() == 1 {
        Some(wallets[0].id.as_str())
    } else {
        None
    };
    for group in ["t.asset", "w.label,t.asset"] {
        rows(conn,&format!("SELECT SUM(CASE WHEN t.direction='inbound' THEN t.amount ELSE 0 END),SUM(CASE WHEN t.direction='outbound' THEN t.amount ELSE 0 END),SUM(t.fee) FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 AND (?2 IS NULL OR t.wallet_id=?2) GROUP BY {group}"), kassiber_store::rusqlite::params![id,wallet])?;
    }
    let mut observed = rows(conn,"SELECT t.*,w.label AS wallet,COALESCE(t.external_id,'') AS transaction_id FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at,t.created_at,t.id",[&id])?;
    if let Some(wallet) = wallet {
        observed.retain(|r| text(r, "wallet_id") == wallet);
    }
    let exported = transaction_csv::report_rows(conn, profile, wallet)?;
    let mut facts: BTreeMap<String, Facts> = BTreeMap::new();
    let mut values: BTreeMap<(String, String), Decimal> = BTreeMap::new();
    for row in rows(conn,"SELECT transaction_id,entry_type,at_category,COALESCE(fiat_value_exact,fiat_value) AS fiat_value,COALESCE(proceeds_exact,proceeds) AS proceeds FROM journal_entries WHERE profile_id=?",[&id])? {
        let tx = text(&row,"transaction_id");
        let kind = text(&row,"entry_type");
        let category = text(&row,"at_category");
        let fact = facts.entry(tx.clone()).or_default();
        fact.types.insert(if kind == "disposal" && category == "neu_swap" { "neutral_disposal".into() } else { kind.clone() });
        if truthy(value(&row,"at_category")) { fact.categories.insert(category); }
        if let Some(key) = match kind.as_str() { "income" => Some("income"), "disposal" => Some("proceeds"), "acquisition" => Some("cost"), _ => None } {
            let current = values.entry((tx,key.into())).or_insert_with(Decimal::zero);
            *current = current.add(&decimal(value(&row,if key == "proceeds" { "proceeds" } else { "fiat_value" }))?, &DECIMAL).map_err(arithmetic)?;
        }
    }
    let mut links: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut swaps = BTreeSet::new();
    let mut conversions = BTreeSet::new();
    for row in rows(conn,"SELECT * FROM journal_custody_projection_relations WHERE profile_id=? AND relation_kind IN ('move','conversion','direct_payout')",[&id])? {
        let legs = [text(&row,"out_transaction_id"),text(&row,"in_transaction_id")];
        let conversion = text(&row,"relation_kind") != "move";
        let swap = conversion && legs.iter().any(|tx| facts.get(tx).is_some_and(|f| f.categories.contains("neu_swap")));
        for (key,tx) in ["out_transaction_id","in_transaction_id"].into_iter().zip(legs) {
            if !truthy(value(&row,key)) { continue; }
            links.entry(tx.clone()).or_default().insert(text(&row,"id").chars().take(12).collect());
            if conversion { conversions.insert(tx.clone()); }
            if swap { swaps.insert(tx); }
        }
    }
    let currency = kassiber_pytext::upper(&text(profile, "fiat_currency"));
    let mut output = vec![if target == "cointracking" { CT } else { BP }
        .iter()
        .map(|h| s(*h))
        .collect()];
    let mut warnings: BTreeMap<&str, usize> = BTreeMap::new();
    let mut tickerless = BTreeSet::new();
    let mut unsupported = Vec::new();
    let mut no_date = 0;
    let mut has_value = false;
    for (source, exported) in observed.iter().zip(exported.iter()) {
        if !wallets.is_empty() && !wallets.iter().any(|w| w.label == text(source, "wallet")) {
            continue;
        }
        let occurred = text(source, "occurred_at");
        if !contains(range, &occurred, false)? {
            no_date += usize::from(undated(&occurred));
            continue;
        }
        let tx = text(source, "id");
        let kind = text(
            source,
            if truthy(value(source, "kind_override")) {
                "kind_override"
            } else {
                "kind"
            },
        );
        let kind = kassiber_pytext::lower(kassiber_pytext::strip(&kind)).replace(['-', ' '], "_");
        let empty = Facts::default();
        let classification = classify(facts.get(&tx).unwrap_or(&empty), swaps.contains(&tx));
        let cash = transaction_csv::optional(source, "fiat_value_exact", "fiat_value")?;
        let fiat =
            cash.is_some() && kassiber_pytext::upper(&text(exported, "fiat_currency")) == currency;
        let (event, warning) = event(
            &classification,
            &text(source, "direction"),
            &kind,
            fiat,
            conversions.contains(&tx) && !swaps.contains(&tx),
            value(source, "taxability_override") == &Value::Int(0.into()),
        );
        let Some(event) = event else {
            unsupported.push(dict([
                ("transaction_ref", s(tx)),
                ("transaction_id", value(source, "transaction_id").clone()),
                ("wallet", value(source, "wallet").clone()),
                ("occurred_at", value(source, "occurred_at").clone()),
                ("classification", s(classification)),
                ("reason", warning.map(s).unwrap_or(Value::Null)),
            ]));
            continue;
        };
        if let Some(warning) = warning {
            *warnings.entry(warning).or_default() += 1;
        }
        let asset = Asset::new(&text(source, "asset"));
        let code = if asset.id == "bitcoin:btc" {
            "BTC".into()
        } else if asset.id == "liquid:lbtc" {
            "LBTC".into()
        } else if asset.chain == "liquid" {
            let code = asset.id.split_once(':').unwrap().1.to_owned();
            tickerless.insert(code.clone());
            code
        } else {
            asset.code.clone()
        };
        let input = serde_json::json!({"direction":text(source,"direction"),"amount":int(source,"amount"),"fee":int(source,"fee"),"amount_includes_fee":truthy(value(source,"amount_includes_fee"))});
        let amounts = kassiber_custody::components::review::boundary_amounts(&input)
            .map_err(|e| AppError::new(e.code, e.message))?;
        let amount = |key: &str| {
            amounts[key]
                .as_i64()
                .map(kassiber_pyjson::BigInt::from)
                .unwrap_or_else(|| kassiber_pytext::parse_int(&amounts[key].to_string()).unwrap())
        };
        let outbound = kassiber_pytext::lower(kassiber_pytext::strip(&text(source, "direction")))
            == "outbound";
        let amount_text = asset.exact(&amount(if event == "fee" {
            "wallet_movement_msat"
        } else {
            "principal_msat"
        }))?;
        let fee = if outbound && int(source, "fee") != 0 && event != "fee" {
            asset.exact(&amount("fee_msat"))?
        } else {
            String::new()
        };
        let key = if [
            "income",
            "mining",
            "staking",
            "airdrop",
            "interest",
            "income_non_taxable",
        ]
        .contains(&event)
        {
            "income"
        } else if ["sell", "payment", "payment_non_taxable"].contains(&event) {
            "proceeds"
        } else if event == "buy" && warning == Some("conversion") {
            "cost"
        } else {
            ""
        };
        let recorded = values.get(&(tx.clone(), key.into())).cloned();
        let fmv = if !matches!(value(exported, "fmv"), Value::Null) {
            Some(decimal(value(exported, "fmv"))?)
        } else {
            None
        };
        let value_text = number(recorded.as_ref().or(fmv.as_ref()))?;
        let fiat_amount = if warning == Some("conversion") {
            value_text.clone()
        } else if fiat {
            number(cash.as_ref())?
        } else {
            String::new()
        };
        has_value |= !value_text.is_empty();
        let mut note = format!("Kassiber {classification}");
        if let Some(refs) = links.get(&tx) {
            note += &format!(
                "; pair {}",
                refs.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        let description = text(exported, "description");
        let description = kassiber_pytext::strip(&description);
        let comment = if description.is_empty() {
            note
        } else {
            format!("{description} · {note}")
        };
        let occurred = timestamp(&occurred)?;
        let date = format!(
            "{}{} {}",
            occurred[..4].parse::<u32>().unwrap(),
            &occurred[4..10],
            &occurred[11..19]
        );
        let tx = if truthy(value(source, "transaction_id")) {
            text(source, "transaction_id")
        } else {
            tx
        };
        let inbound = [
            "buy",
            "income",
            "mining",
            "staking",
            "airdrop",
            "interest",
            "deposit",
            "income_non_taxable",
        ]
        .contains(&event);
        let (buy_amount, buy_code, sell_amount, sell_code) = if inbound {
            (
                amount_text.as_str(),
                code.as_str(),
                if event == "buy" {
                    fiat_amount.as_str()
                } else {
                    ""
                },
                if event == "buy" {
                    currency.as_str()
                } else {
                    ""
                },
            )
        } else {
            (
                if event == "sell" {
                    fiat_amount.as_str()
                } else {
                    ""
                },
                if event == "sell" {
                    currency.as_str()
                } else {
                    ""
                },
                amount_text.as_str(),
                code.as_str(),
            )
        };
        let fee_code = if fee.is_empty() { "" } else { code.as_str() };
        let row = if target == "cointracking" {
            let label = match event {
                "buy" | "sell" => "Trade",
                "income" => "Income",
                "mining" => "Mining",
                "staking" => "Staking",
                "airdrop" => "Airdrop",
                "interest" => "Interest Income",
                "payment" => "Spend",
                "fee" => "Other Fee",
                "deposit" => "Deposit",
                "withdrawal" => "Withdrawal",
                "income_non_taxable" => "Income (non taxable)",
                "payment_non_taxable" => "Expense (non taxable)",
                _ => unreachable!(),
            };
            vec![
                s(label),
                s(buy_amount),
                s(buy_code),
                s(sell_amount),
                s(sell_code),
                s(fee.as_str()),
                s(fee_code),
                value(source, "wallet").clone(),
                s("Kassiber"),
                s(comment),
                s(date),
                s(tx),
                s(if inbound { &value_text } else { "" }),
                s(if inbound { "" } else { &value_text }),
            ]
        } else {
            let label = match event {
                "buy" | "sell" => "Trade",
                "income" => "Income",
                "mining" => "Mining",
                "staking" => "Staking",
                "airdrop" => "Airdrop",
                "interest" => "Interest",
                "payment" => "Payment",
                "fee" => "Fee",
                "deposit" => "Deposit",
                "withdrawal" => "Withdrawal",
                "income_non_taxable" => "Non-Taxable (In)",
                "payment_non_taxable" => "Non-Taxable (Out)",
                _ => unreachable!(),
            };
            let date = format!(
                "{}.{}.{} {}",
                &occurred[8..10],
                &occurred[5..7],
                occurred[..4].parse::<u32>().unwrap(),
                &occurred[11..19]
            );
            vec![
                s(date),
                value(source, "wallet").clone(),
                s(label),
                s(sell_code),
                s(sell_amount),
                s(buy_code),
                s(buy_amount),
                s(fee_code),
                s(fee.as_str()),
                s(comment),
                s(tx),
            ]
        };
        output.push(row);
    }
    if !unsupported.is_empty() {
        return Err(AppError::new("platform_export_unsupported", "Some transactions have no import row in this format that keeps their tax result")
            .with_hint("Rows that are part own-wallet transfer and part payment or income, and conversions reviewed as not taxable, cannot be written without changing the tax result. Export these wallets as Kassiber Excel or CSV, or enter the rows in the target tool by hand.")
            .with_details(dict([("count",Value::Int(unsupported.len().into())),("transactions",Value::List(unsupported.into_iter().take(20).collect()))])));
    }
    let mut warning_text = Vec::new();
    for (key, count) in warnings {
        warning_text.push(s(match key {
            "conversion" => format!("{count} legs of taxable conversions are exported as a sale and a purchase at the booked proceeds and cost."),
            "swap" => format!("{count} swap legs between own wallets are exported as withdrawal/deposit; link them as a conversion in the target tool."),
            "unbooked" => format!("{count} rows have no journal entry and are exported as plain deposit/withdrawal."),
            _ => unreachable!(),
        }));
    }
    if !tickerless.is_empty() {
        warning_text.push(s(format!(
            "Assets without a ticker are exported by asset id ({}); map them in the target tool.",
            tickerless.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    if has_value {
        warning_text.push(s(format!("Value columns are in {currency}; they assume the target tool uses the same account currency.")));
    }
    if no_date > 0 {
        warning_text.push(s(format!("{} without a known date could belong to this period but {} not in this file. Export All time to include undated transactions.",if no_date == 1 { "1 transaction".into() } else { format!("{no_date} transactions") },if no_date == 1 { "is" } else { "are" })));
    }
    let count = output.len() - 1;
    Ok((
        output,
        dict([
            ("format", s("csv")),
            ("platform", s(target)),
            (
                "platform_label",
                s(if target == "cointracking" {
                    "CoinTracking CSV"
                } else {
                    "Blockpit CSV"
                }),
            ),
            ("wallet", s(scope(&wallets))),
            ("rows", Value::Int(count.into())),
            ("warnings", Value::List(warning_text)),
            ("range", range.clone()),
        ]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dump(conn: &Connection) -> Value {
        let tables = rows(conn,"SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",[]).unwrap();
        assert_eq!(tables.len(), 113);
        dict(tables.iter().map(|t| {
            let name = text(t, "name");
            (
                name.clone(),
                Value::List(rows(conn, &format!("SELECT * FROM \"{name}\""), []).unwrap()),
            )
        }))
    }
    fn check(names: &[&str]) {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/platform_csv.json")).unwrap();
        for name in names {
            assert!(
                vectors
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["name"] == *name),
                "missing vector {name}"
            );
        }
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        for case in vectors
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| names.is_empty() || names.contains(&c["name"].as_str().unwrap()))
        {
            conn.execute_batch("SAVEPOINT platform_fixture").unwrap();
            conn.execute_batch(include_str!("../tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = dump(conn);
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let path = root.path().join("platform.csv");
            let result = crate::export::export(
                conn,
                "reports.export-platform-csv",
                &args,
                &path,
                &kassiber_setup::SystemSources,
            );
            let actual = match result {
                Ok(result) => {
                    assert!(get(&result, "report_snapshot").is_none());
                    let bytes = std::fs::read(&path).unwrap();
                    assert_eq!(
                        String::from_utf8(bytes).unwrap(),
                        case["content"].as_str().unwrap(),
                        "{} bytes",
                        case["name"]
                    );
                    let Value::Dict(fields) = result else {
                        panic!()
                    };
                    dict([(
                        "ok",
                        Value::Dict(
                            fields
                                .into_iter()
                                .filter(|(k, _)| k != &"file".into())
                                .collect(),
                        ),
                    )])
                }
                Err(error) => {
                    assert!(!path.exists(), "{} published on refusal", case["name"]);
                    dict([(
                        "error",
                        dict([
                            ("code", s(error.code)),
                            ("message", s(error.message)),
                            ("hint", error.hint.map(s).unwrap_or(Value::Null)),
                            ("details", error.details.unwrap_or(Value::Null)),
                            ("retryable", Value::Bool(error.retryable)),
                        ]),
                    )])
                }
            };
            let actual: serde_json::Value = serde_json::from_str(
                &kassiber_pyjson::dumps(&actual, &Default::default()).unwrap(),
            )
            .unwrap();
            assert_eq!(actual, case["expected"], "{} payload", case["name"]);
            assert_eq!(dump(conn), before, "{} tables", case["name"]);
            conn.execute_batch("ROLLBACK TO platform_fixture; RELEASE platform_fixture")
                .unwrap();
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    #[test]
    fn platform_csv_python_vectors_and_112_tables() {
        check(&[]);
    }
    #[test]
    fn platform_csv_base_bytes() {
        check(&["cointracking-base", "blockpit-base"]);
    }
    #[test]
    fn platform_csv_event_bytes() {
        check(&[
            "cointracking-event-buy",
            "blockpit-event-sell",
            "cointracking-foreign-fiat",
            "cointracking-event-mining",
            "blockpit-event-staking",
            "cointracking-event-airdrop",
            "blockpit-event-interest",
            "cointracking-non-taxable-income",
            "blockpit-conversion-buy",
            "cointracking-transfer",
            "blockpit-neutral",
        ]);
    }
    #[test]
    fn platform_csv_amount_bytes() {
        check(&[
            "cointracking-fee",
            "blockpit-fee-included",
            "cointracking-fee-only-included",
            "blockpit-one-msat",
            "cointracking-exact",
            "blockpit-overflow-movement",
        ]);
    }
    #[test]
    fn platform_csv_scope_and_barrier_bytes() {
        check(&[
            "cointracking-stale",
            "blockpit-freshness",
            "cointracking-quarantined",
            "cointracking-quarantine-undated",
            "blockpit-non-taxable-conversion",
            "cointracking-mixed-income",
            "blockpit-quarantine-scope-Cold",
            "cointracking-quarantine-scope-Hot",
            "blockpit-empty-date-range",
            "cointracking-short-year",
            "blockpit-offset",
            "cointracking-issued-assets",
            "blockpit-price-outside-wallet",
            "cointracking-unicode-asset",
            "blockpit-unicode-bare",
            "blockpit-namespaced",
            "cointracking-unicode",
        ]);
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    struct Fixed;
    impl kassiber_setup::Sources for Fixed {
        fn now(&self, _: &Connection) -> Result<String> {
            Ok("2026-01-02T03:04:05Z".into())
        }
        fn uuid(&self, _: &Connection) -> Result<String> {
            panic!()
        }
    }
    #[test]
    fn platform_csv_requests_keep_validation_ranges_and_native_publication() {
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(include_str!("../tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(
            "DELETE FROM journal_quarantines; DELETE FROM journal_entries WHERE id='swap';",
        )
        .unwrap();
        let exports = root.path().join("exports");
        for platform in ["cointracking", "blockpit"] {
            let kind = "ui.transactions.export_platform_csv";
            assert!(export_request::handles(kind));
            let args = dict([
                ("platform", s(format!(" {platform} "))),
                ("wallets", Value::List(vec![s("Hot")])),
                ("range", s("custom")),
                ("from", s("2024-01-01")),
                ("to", s("2024-12-31")),
                ("timezone", s("UTC")),
            ]);
            let result = export_request::execute(conn, kind, &args, &exports, &Fixed).unwrap();
            assert_eq!(text(&result, "platform"), platform);
            assert_eq!(text(&result, "scope"), "transactions");
            assert_eq!(text(&result, "wallet"), "Hot");
            assert_eq!(text(value(&result, "range"), "label"), "2024");
            let first = format!("kassiber-{platform}-2024-20260102-030405.csv");
            assert_eq!(text(&result, "filename"), first);
            let bytes = std::fs::read(exports.join("reports").join(first)).unwrap();
            assert_eq!(value(&result, "bytes"), &Value::Int(bytes.len().into()));
            assert!(get(&result, "report_snapshot").is_none());
            std::fs::create_dir(
                exports
                    .join("reports")
                    .join(format!("kassiber-{platform}-2024-20260102-030405-2.csv")),
            )
            .unwrap();
            let again = export_request::execute(conn, kind, &args, &exports, &Fixed).unwrap();
            assert_eq!(
                text(&again, "filename"),
                format!("kassiber-{platform}-2024-20260102-030405-3.csv")
            );
            let path = root.path().join(format!("{platform}.csv"));
            let cli = dict([
                ("file", s(path.to_str().unwrap())),
                ("wallets", Value::List(vec![s("Hot")])),
                ("export_format", s(format!("{platform}-csv"))),
                ("from_day", s("2024-01-01")),
                ("to_day", s("2024-12-31")),
                ("timezone", s("UTC")),
            ]);
            let cli_result =
                export_request::execute(conn, "transactions.export", &cli, &exports, &Fixed)
                    .unwrap();
            assert_eq!(text(&cli_result, "platform"), platform);
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        for (raw, expected) in [
            (
                r#"{}"#,
                "ui.transactions.export_platform_csv requires a platform",
            ),
            (
                r#"{"platform":false}"#,
                "ui.transactions.export_platform_csv requires a platform",
            ),
            (r#"{"platform":"bad"}"#, "Unknown export format 'bad'"),
            (
                r#"{"platform":"bad","range":""}"#,
                "ui.transactions.export_platform_csv range must not be empty",
            ),
            (
                r#"{"platform":"blockpit","wallet":"Cold","wallets":[]}"#,
                "ui.transactions.export_platform_csv takes wallet or wallets, not both",
            ),
            (
                r#"{"platform":"blockpit","wallets":42}"#,
                "ui.transactions.export_platform_csv wallets must be an array of strings",
            ),
            (
                r#"{"platform":"blockpit","timezone":42}"#,
                "ui.transactions.export_platform_csv timezone must be a string",
            ),
        ] {
            let error = export_request::execute(
                conn,
                "ui.transactions.export_platform_csv",
                &kassiber_pytext::loads(raw).unwrap(),
                &exports,
                &Fixed,
            )
            .unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(error.message, expected);
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM filed_report_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        let empty = kassiber_store::Book::create(&root.path().join("empty"), None).unwrap();
        for (raw, message) in [
            (
                r#"{}"#,
                "ui.transactions.export_platform_csv requires a platform",
            ),
            (r#"{"platform":"bad"}"#, "Unknown export format 'bad'"),
            (
                r#"{"platform":"blockpit","wallets":42}"#,
                "ui.transactions.export_platform_csv wallets must be an array of strings",
            ),
        ] {
            let error = export_request::execute(
                empty.connection(),
                "ui.transactions.export_platform_csv",
                &kassiber_pytext::loads(raw).unwrap(),
                &exports,
                &Fixed,
            )
            .unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(error.message, message);
        }
    }
}
