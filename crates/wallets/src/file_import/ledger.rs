use super::river::{abs, arithmetic, nonnegative, positive, repr, truth, zero};
use super::*;
mod xlsx;
pub struct ColumnMap {
    pub(super) raw: Json,
    type_order: Vec<String>,
}
impl ColumnMap {
    pub fn from_python(value: &Py) -> Result<Self, AppError> {
        let raw: Json = serde_json::from_str(
            &kassiber_pyjson::dumps(value, &Default::default())
                .map_err(|e| AppError::new("internal_error", e.to_string()))?,
        )
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        let type_order = match kassiber_envelope::get(value, "type_map") {
            Some(Py::Dict(entries)) => entries
                .iter()
                .filter_map(|(key, _)| match key {
                    kassiber_pyjson::Key::Str(s) => s.as_str().map(str::to_owned),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        };
        Ok(Self { raw, type_order })
    }
    pub fn from_json(value: &Json) -> Self {
        Self::from_python(&py(value)).unwrap()
    }
}
pub fn map_path(file: &str) -> Result<std::path::PathBuf, AppError> {
    kassiber_paths::Context::from_env()
        .and_then(|c| c.expand_user(std::path::Path::new(file)))
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}
pub fn read_map(file: &str) -> Result<String, AppError> {
    let path = map_path(file)?;
    csv::read_text(&path.to_string_lossy()).map_err(|e| {
        if e.message.starts_with("'utf-8'") {
            e
        } else {
            AppError::new(
                "not_found",
                format!("Could not read column map file: {}", path.display()),
            )
            .with_hint("Pass inline JSON, or check the @path.")
        }
    })
}

fn tables() -> &'static Json {
    static TABLES: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| serde_json::from_str(include_str!("ledger_tables.json")).unwrap())
}
fn strings(key: &str) -> Vec<&'static str> {
    tables()[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}
fn py(value: &Json) -> Py {
    kassiber_pytext::loads(&value.to_string()).unwrap()
}
fn cell(value: &Json) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let value = kassiber_envelope::py_str(&py(value));
    let value = kassiber_pytext::strip(&value);
    (!value.is_empty()).then(|| value.to_owned())
}
fn key(value: &str) -> String {
    kassiber_pytext::casefold(
        &value
            .split(|c| kassiber_pytext::is_space(u32::from(c)))
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}
fn fail(message: impl Into<String>, hint: &str) -> AppError {
    AppError::new("validation", message).with_hint(hint)
}
fn detail<const N: usize>(error: AppError, entries: [(&str, Json); N]) -> AppError {
    error.with_details(dict(entries.map(|(key, value)| (key, py(&value)))))
}
fn code(value: &str) -> String {
    let value = kassiber_pytext::upper(kassiber_pytext::strip(value));
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return value.to_ascii_lowercase();
    }
    match value.as_str() {
        "L-BTC" => "LBTC".into(),
        _ => value,
    }
}
fn crypto(value: &str) -> bool {
    ["BTC", "XBT", "LBTC", "SAT", "SATS"].contains(&value)
}
fn canonical(value: &str) -> &str {
    if ["XBT", "SAT", "SATS"].contains(&value) {
        "BTC"
    } else {
        value
    }
}
fn btc(value: PriceNumber, asset: &str) -> Result<PriceNumber, AppError> {
    let value = abs(value)?;
    if ["SAT", "SATS"].contains(&asset) {
        arithmetic(&value, &PriceNumber::parse("100000000")?, '/')
    } else {
        Ok(value)
    }
}
fn number(value: &Json, field: &str, index: usize) -> Result<Option<PriceNumber>, AppError> {
    tax_platform::number(cell(value), field, &format!("Ledger row {index}")).map_err(|mut error| {
        if error.code == "validation" {
            error.message = format!(
                "Ledger row {index}: {field} is not a number ({})",
                kassiber_envelope::py_repr(&py(value))
            );
        }
        error
    })
}
fn get<'a>(row: &'a csv::Row, names: &[&str]) -> &'a Json {
    for name in names {
        let name = key(name);
        if let Some((_, value)) = row
            .headers
            .iter()
            .rev()
            .find_map(|h| (key(h) == name).then_some((h, &row.data[h])))
        {
            if !value.is_null() && value != "" {
                return value;
            }
        }
    }
    &Json::Null
}
fn record(row: csv::Row, index: usize) -> Result<csv::Row, AppError> {
    let label = format!("Ledger row {index}");
    if let Some(direction) = cell(&row.data["__unreadable_direction"]) {
        return Err(fail(format!("{label}: unknown direction '{direction}'"),"Kassiber could not tell whether this row is incoming or outgoing. Map the direction column's values with column_map.type_map, or map a Type column instead."));
    }
    let kind_text = cell(get(&row, &["Type", "Transaction Type", "Kind"])).ok_or_else(|| {
        fail(
            format!("{label}: missing Type"),
            "Set a Type such as Buy, Sell, Deposit, Withdrawal, Income, Mining, or Gift sent.",
        )
    })?;
    let kind_key = kassiber_pytext::lower(&kind_text)
        .split(|c| kassiber_pytext::is_space(u32::from(c)))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let entry = &tables()["types"][&kind_key];
    if entry.is_null() {
        return Err(fail(
            format!("{label}: unknown Type '{kind_text}'"),
            &format!("Use one of: {}.", strings("display").join(", ")),
        ));
    }
    let incoming = cell(get(
        &row,
        &["Received Asset", "Received Cur.", "Buy Asset", "Buy Cur."],
    ))
    .map(|s| code(&s));
    let outgoing = cell(get(
        &row,
        &["Sent Asset", "Sent Cur.", "Sell Asset", "Sell Cur."],
    ))
    .map(|s| code(&s));
    let received = number(
        get(&row, &["Received Amount", "Buy Amount"]),
        "Received Amount",
        index,
    )?;
    let sent = number(
        get(&row, &["Sent Amount", "Sell Amount"]),
        "Sent Amount",
        index,
    )?;
    let fee = number(get(&row, &["Fee Amount", "Fee"]), "Fee Amount", index)?;
    let fee_asset = cell(get(&row, &["Fee Asset", "Fee Cur."])).map(|s| code(&s));
    let explicit = number(
        get(
            &row,
            &[
                "Fiat Value",
                "Value (fiat)",
                "Buy Value (fiat)",
                "Sell Value (fiat)",
            ],
        ),
        "Fiat Value",
        index,
    )?;
    let inbound = incoming.as_deref().is_some_and(crypto) && received.is_some();
    let outbound = outgoing.as_deref().is_some_and(crypto) && sent.is_some();
    if inbound && outbound {
        return Err(fail(
            format!("{label}: both sides are Bitcoin — crypto-to-crypto rows are not supported"),
            "Record one Bitcoin leg per row and price it with a fiat amount or a Fiat Value.",
        ));
    }
    if !inbound && !outbound {
        return Err(fail(format!("{label}: no Bitcoin leg found (expected BTC, LBTC, or SATS on the Received or Sent side)"),"Put the Bitcoin amount + asset on the Received side (inbound) or the Sent side (outbound)."));
    }
    let (direction, original, amount, cash_asset, cash) = if inbound {
        (
            "inbound",
            incoming.as_deref().unwrap(),
            received.unwrap(),
            outgoing,
            sent,
        )
    } else {
        (
            "outbound",
            outgoing.as_deref().unwrap(),
            sent.unwrap(),
            incoming,
            received,
        )
    };
    let cash_asset = cash_asset.filter(|s| !crypto(s));
    let cash = cash.filter(|_| cash_asset.is_some());
    if let Some(asset) = cash_asset
        .as_deref()
        .filter(|s| !tax_platform::FIAT.contains(s))
    {
        return Err(fail(format!("{label}: unrecognized cash currency '{asset}'"),"Use a supported fiat code such as EUR, USD, or CHF on the cash side. For a separately valued Bitcoin leg, omit the opposing asset and amount and enter its reviewed Fiat Value in the book currency."));
    }
    let amount = btc(amount, original)?;
    let asset = canonical(original);
    let type_direction = entry[0].as_str().unwrap();
    if type_direction != direction {
        return Err(fail(format!("{label}: Type '{kind_text}' is {type_direction} but the Bitcoin leg is {direction}"),"Inbound Types (Buy/Deposit/Income/…) need the Bitcoin amount on the Received side; outbound Types (Sell/Withdrawal/Spend/Gift sent/…) on the Sent side."));
    }
    let (mut network_fee, mut cash_fee) = (zero(), zero());
    if let Some(fee) = fee {
        let fee = abs(fee)?;
        if positive(&fee)? {
            if fee_asset.as_deref().is_none_or(crypto) {
                network_fee = btc(fee, fee_asset.as_deref().unwrap_or(original))?;
            } else {
                let fee_asset = fee_asset.as_deref().unwrap();
                if cash.is_none() || cash_asset.is_none() {
                    return Err(fail(format!("{label}: a fiat fee ({fee_asset}) needs a matching cash leg"),"Put network fees in BTC (Fee Asset BTC or blank), or add the Sent/Received fiat amount this fee applies to."));
                }
                if Some(fee_asset) != cash_asset.as_deref() {
                    return Err(fail(format!("{label}: fee currency {fee_asset} does not match the trade currency {}",cash_asset.as_deref().unwrap()),"Use the same currency for the fee and the row's fiat amount."));
                }
                cash_fee = fee;
            }
        }
    }
    let priced = cash.is_some() && cash_asset.is_some();
    let value = if let Some(cash) = cash {
        let base = abs(cash)?;
        Some(if inbound {
            arithmetic(&base, &cash_fee, '+')?
        } else {
            nonnegative(arithmetic(&base, &cash_fee, '-')?)?
        })
    } else {
        explicit.map(abs).transpose()?
    };
    let rate = if let Some(value) = &value {
        if truth(&amount) && positive(&amount)? {
            Some(arithmetic(value, &amount, '/')?)
        } else {
            None
        }
    } else {
        None
    };
    let occurred = tax_platform::date(cell(get(&row, &["Date", "Timestamp", "Time"])), index)?;
    let txid = cell(get(&row, &["Tx-ID", "TxID", "Txid", "Transaction ID"]));
    let counterparty = cell(get(&row, &["Counterparty", "Exchange", "Platform"]));
    let hash = cell(get(
        &row,
        &[
            "Payment Hash",
            "PaymentHash",
            "Lightning Payment Hash",
            "LN Payment Hash",
        ],
    ));
    let hash_source = cell(get(
        &row,
        &[
            "Payment Hash Source",
            "PaymentHash Source",
            "Payment Hash Origin",
        ],
    ))
    .or_else(|| hash.as_ref().map(|_| "generic_ledger".into()));
    let funding = cell(get(
        &row,
        &[
            "Swap Refund Funding Tx-ID",
            "Swap Refund Funding TxID",
            "Refund Funding Tx-ID",
            "Refund Funding TxID",
            "HTLC Funding TxID",
        ],
    ));
    let vout = cell(get(
        &row,
        &[
            "Swap Refund Funding Vout",
            "Refund Funding Vout",
            "HTLC Funding Vout",
        ],
    ));
    Ok(csv::Row {
        data: json!({"txid":txid.clone().unwrap_or_default(),"occurred_at":occurred,"direction":direction,"asset":asset,"amount":repr(&amount),"fee":repr(&network_fee),"fiat_value":value.as_ref().map(repr),"fiat_rate":rate.as_ref().map(repr),"fiat_currency":if priced {cash_asset.as_deref()} else {None},"pricing_source_kind":if priced {Some("exchange_execution")} else {None},"pricing_provider":if priced {counterparty.as_deref()} else {None},"pricing_pair":if priced {Some(format!("{asset}-{}",cash_asset.as_deref().unwrap()))} else {None},"pricing_method":if priced {Some("generic_ledger")} else {None},"pricing_external_ref":txid,"pricing_quality":if priced {Some("exact")} else {None},"kind":entry[1],"description":cell(get(&row,&["Note","Comment","Description"])),"counterparty":counterparty,"payment_hash":hash,"payment_hash_source":hash_source,"swap_refund_funding_txid":funding,"swap_refund_funding_vout":vout,"raw_json":config::stored_json(&row.data)}),
        headers: vec![],
        rest: None,
        original: None,
    })
}
fn header_asset(value: Option<&str>) -> Option<String> {
    let value = kassiber_pytext::casefold(value.unwrap_or_default());
    let tokens: Vec<_> = value
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .collect();
    if tokens.join(" ").contains("liquid bitcoin") {
        return Some("LBTC".into());
    }
    tokens
        .iter()
        .rev()
        .find_map(|s| tables()["header_assets"][*s].as_str().map(str::to_owned))
}
fn header_role(value: &str) -> Option<&'static str> {
    let value = kassiber_pytext::casefold(value);
    let tokens: Vec<_> = value
        .split(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit())
        .collect();
    for (table, role) in [("received_tokens", "received"), ("sent_tokens", "sent")] {
        if strings(table).iter().any(|s| tokens.contains(s)) {
            return Some(role);
        }
    }
    None
}
fn usable(plan: &Json) -> bool {
    cell(&plan["date"]).is_some()
        && ["received", "sent", "amount"]
            .iter()
            .any(|k| cell(&plan[k]).is_some())
}
fn known_hint() -> String {
    format!("Known fields: {}.", strings("plan_fields").join(", "))
}
pub fn column_map_hint() -> String {
    known_hint()
}
pub fn validate_map(mapping: &ColumnMap, headers: &[String]) -> Result<Json, AppError> {
    let raw = mapping.raw.as_object().ok_or_else(|| {
        fail(
            "column_map must be an object mapping ledger fields to column names",
            &known_hint(),
        )
    })?;
    let unknown: Vec<_> = raw
        .keys()
        .filter(|s| !strings("plan_fields").contains(&s.as_str()))
        .collect();
    if !unknown.is_empty() {
        return Err(detail(
            fail("column_map contains unsupported fields", &known_hint()),
            [("unsupported", json!(unknown))],
        ));
    }
    let mut plan = json!({});
    for field in strings("asset_fields") {
        if let Some(value) = raw.get(field) {
            let value = cell(value);
            let normalized = value.as_deref().map(code);
            if normalized
                .as_deref()
                .is_some_and(|s| !strings("hint_codes").contains(&s))
            {
                return Err(detail(fail(format!("column_map.{field} is not an asset Kassiber can denominate a column in"),"Use a Bitcoin rail (BTC, LBTC, SATS) or a fiat currency code."),[("field", json!(field)), ("value", json!(value)), ("supported", json!(strings("hint_codes")))]));
            }
            plan[field] = json!(normalized);
        }
    }
    if let Some(value) = raw.get("type_map") {
        let mut map = json!({});
        if !value.is_null() {
            let values = value.as_object().ok_or_else(|| fail("column_map.type_map must be an object mapping this file's Type values to ledger Types","Example: {\"Kauf\": \"Buy\", \"Verkauf\": \"Sell\"}."))?;
            for source in &mapping.type_order {
                let target = &values[source];
                let label = cell(target);
                let source_key = key(source);
                if source_key.is_empty() || label.is_none() {
                    return Err(detail(
                        AppError::new(
                            "validation",
                            "column_map.type_map entries must both be non-empty strings",
                        ),
                        [("entry", json!({source:target}))],
                    ));
                }
                let label = label.unwrap();
                let canonical = kassiber_pytext::lower(&label)
                    .split(|c| kassiber_pytext::is_space(u32::from(c)))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if tables()["types"][&canonical].is_null() {
                    return Err(detail(fail(format!("column_map.type_map maps '{source}' to unknown ledger Type '{label}'"),&format!("Map onto a Type the tax engine recognizes: {}.",tables()["types"].as_object().unwrap().keys().cloned().collect::<Vec<_>>().join(", "))),[("source", json!(source)), ("target", json!(label))]));
                }
                map[&source_key] = json!(label);
            }
        }
        plan["type_map"] = if map.as_object().unwrap().is_empty() {
            Json::Null
        } else {
            map
        };
    }
    let known: Vec<_> = headers.iter().filter(|s| !s.is_empty()).collect();
    let mut claimed = BTreeMap::new();
    for field in strings("columns") {
        if let Some(value) = raw.get(field) {
            let column = cell(value);
            if let Some(column) = &column {
                if !known.is_empty() && !known.contains(&column) {
                    return Err(detail(
                        fail(
                            format!("column_map.{field} names a column that is not in this file"),
                            "Use one of the file's own header values.",
                        ),
                        [
                            ("field", json!(field)),
                            ("column", json!(column)),
                            ("headers", json!(known)),
                        ],
                    ));
                }
                if let Some(previous) = claimed.insert(column.clone(), field) {
                    return Err(detail(
                        fail(
                            "column_map maps one column to two different fields",
                            "Each source column may fill only one ledger field.",
                        ),
                        [
                            ("column", json!(column)),
                            ("fields", json!([previous, field])),
                        ],
                    ));
                }
            }
            plan[field] = json!(column);
        }
    }
    for (field, source) in [
        ("received_header_asset", "received"),
        ("sent_header_asset", "sent"),
        ("amount_header_asset", "amount"),
        ("fee_header_asset", "fee"),
    ] {
        let from = header_asset(plan[source].as_str());
        if plan.get(field).is_none() {
            plan[field] = json!(from);
        } else if from.is_some() && plan[field] != json!(from) {
            return Err(detail(fail(format!("column_map.{field} contradicts the column's own header"),&format!("Column {} already states {}. Drop the hint, or map a separate asset column instead.",kassiber_envelope::py_repr(&py(&plan[source])),from.as_deref().unwrap())),[("field", json!(field)), ("column", plan[source].clone()), ("header_asset", json!(from)), ("supplied", plan[field].clone())]));
        }
    }
    if !usable(&plan) {
        return Err(detail(
            fail(
                "column_map needs at least a date column and one amount column",
                "Map 'date' plus one of 'amount', or 'received' / 'sent'.",
            ),
            [(
                "mapped",
                json!(plan
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(_, v)| !v.is_null() && **v != json!("") && **v != json!({}))
                    .map(|(k, _)| k)
                    .collect::<Vec<_>>()),
            )],
        ));
    }
    Ok(plan)
}
fn infer(header: &[String]) -> (Json, Json) {
    let mut used = std::collections::BTreeSet::new();
    let mut detected = vec![];
    let mut plan = json!({});
    for field in [
        "date",
        "received",
        "sent",
        "amount",
        "type",
        "direction",
        "received_asset",
        "sent_asset",
        "asset",
        "fee",
        "fee_asset",
        "fiat_currency",
        "fiat_value",
        "fiat_rate",
        "note",
        "txid",
        "payment_hash",
        "payment_hash_source",
        "swap_refund_funding_txid",
        "swap_refund_funding_vout",
        "counterparty",
    ] {
        let skip = field == "amount" && !plan["received"].is_null() && !plan["sent"].is_null()
            || field == "direction" && !plan["type"].is_null();
        let aliases = tables()["aliases"][field].as_array().unwrap();
        let mut column = if skip {
            None
        } else {
            header.iter().find(|h| {
                !h.is_empty() && !used.contains(*h) && aliases.iter().any(|a| a == &key(h))
            })
        };
        if column.is_none() && ["received", "sent"].contains(&field) {
            column = header.iter().find(|h| {
                !used.contains(*h)
                    && header_role(h) == Some(field)
                    && header_asset(Some(h)).is_some()
            });
        }
        if let Some(column) = column {
            used.insert(column.clone());
            detected.push(json!({"column":column,"field":field}));
        }
        plan[field] = json!(column);
    }
    for (field, source) in [
        ("received_header_asset", "received"),
        ("sent_header_asset", "sent"),
        ("amount_header_asset", "amount"),
        ("fee_header_asset", "fee"),
    ] {
        plan[field] = json!(header_asset(plan[source].as_str()));
    }
    (plan, json!(detected))
}
fn type_value(value: Option<String>, direction: Option<&str>, cash: bool, plan: &Json) -> String {
    if let Some(value) = value {
        let k = key(&value);
        return plan["type_map"][&k]
            .as_str()
            .or(tables()["type_values"][&k].as_str())
            .unwrap_or(&value)
            .to_owned();
    }
    match (cash, direction) {
        (true, Some("inbound")) => "Buy",
        (true, _) => "Sell",
        (false, Some("inbound")) => "Deposit",
        _ => "Withdrawal",
    }
    .into()
}
fn remap(row: csv::Row, plan: &Json) -> Result<csv::Row, AppError> {
    let cell = |field: &str| plan[field].as_str().and_then(|col| cell(&row.data[col]));
    let raw = |field: &str| {
        plan[field]
            .as_str()
            .map(|col| &row.data[col])
            .unwrap_or(&Json::Null)
    };
    let num = |field: &str| number(raw(field), "x", 0).ok().flatten();
    let mut asset = cell("asset")
        .or_else(|| plan["amount_header_asset"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "BTC".into());
    let received_asset = cell("received_asset")
        .or_else(|| plan["received_header_asset"].as_str().map(str::to_owned))
        .unwrap_or_else(|| asset.clone());
    let sent_asset = cell("sent_asset")
        .or_else(|| plan["sent_header_asset"].as_str().map(str::to_owned))
        .unwrap_or_else(|| asset.clone());
    let kind = cell("type");
    let mut direction = None;
    let mut btc_cell = None;
    let mut both = false;
    let mut cash = false;
    let mut unreadable = None;
    if !plan["received"].is_null() || !plan["sent"].is_null() {
        let received = num("received");
        let sent = num("sent");
        for value in [&received, &sent].into_iter().flatten() {
            if repr(value).contains("sNaN") {
                return Err(decimal_error(DecimalError::InvalidOperation));
            }
        }
        let (r, s) = (
            received.as_ref().is_some_and(truth),
            sent.as_ref().is_some_and(truth),
        );
        if r && !s {
            direction = Some("inbound");
            btc_cell = cell("received");
        } else if s && !r {
            direction = Some("outbound");
            btc_cell = cell("sent");
        } else if r && s {
            both = true;
            let (r, s) = (crypto(&code(&received_asset)), crypto(&code(&sent_asset)));
            if r && !s {
                direction = Some("inbound");
                btc_cell = cell("received");
                cash = true;
            } else if s && !r {
                direction = Some("outbound");
                btc_cell = cell("sent");
                cash = true;
            }
        }
    } else if !plan["amount"].is_null() {
        let value = num("amount");
        if kind.is_some() {
            let mapped = type_value(kind.clone(), None, false, plan);
            let mapped = kassiber_pytext::lower(&mapped)
                .split(|c| kassiber_pytext::is_space(u32::from(c)))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            direction = tables()["types"][mapped][0].as_str();
            btc_cell = if let Some(value) = value.filter(|_| direction.is_some()) {
                Some(fixed(&abs(value)?))
            } else {
                cell("amount")
            };
        } else if !plan["direction"].is_null() {
            let raw_direction = cell("direction");
            let k = key(raw_direction.as_deref().unwrap_or_default());
            direction = if strings("inbound").contains(&k.as_str()) {
                Some("inbound")
            } else if strings("outbound").contains(&k.as_str()) {
                Some("outbound")
            } else {
                None
            };
            btc_cell = value
                .map(abs)
                .transpose()?
                .as_ref()
                .map(fixed)
                .or_else(|| cell("amount"));
            if direction.is_none() {
                unreadable = Some(raw_direction.unwrap_or_else(|| "(empty)".into()));
            }
        } else if let Some(value) = value {
            direction = Some(if positive(&value)? || !truth(&value) {
                "inbound"
            } else {
                "outbound"
            });
            btc_cell = Some(fixed(&abs(value)?));
        } else {
            direction = Some("inbound");
            btc_cell = cell("amount");
        }
    }
    let mut out = json!({"Date":cell("date").unwrap_or_default(),"Type":type_value(kind,direction,cash,plan)});
    if both {
        for (field, code) in [("received", &received_asset), ("sent", &sent_asset)] {
            if !plan[field].is_null() {
                let side = if field == "received" {
                    "Received"
                } else {
                    "Sent"
                };
                out[format!("{side} Asset")] = json!(code);
                out[format!("{side} Amount")] = json!(cell(field));
            }
        }
    } else {
        if !plan["received"].is_null() && direction == Some("inbound") {
            asset = received_asset;
        } else if !plan["sent"].is_null() && direction == Some("outbound") {
            asset = sent_asset;
        }
        if let Some(value) = unreadable {
            out["__unreadable_direction"] = json!(value);
        }
        let currency = cell("fiat_currency");
        let mut value = cell("fiat_value");
        if value.is_none() && !plan["fiat_rate"].is_null() && btc_cell.is_some() {
            if let (Some(rate), Some(amount)) = (
                num("fiat_rate"),
                number(&json!(btc_cell), "x", 0).ok().flatten(),
            ) {
                value = Some(fixed(&arithmetic(&rate, &amount, '*')?));
            }
        }
        let (side, opposing) = if direction == Some("outbound") {
            ("Sent", "Received")
        } else {
            ("Received", "Sent")
        };
        out[format!("{side} Asset")] = json!(asset);
        out[format!("{side} Amount")] = json!(btc_cell);
        if let Some(value) = value {
            if let Some(currency) = currency {
                out[format!("{opposing} Asset")] = json!(currency);
                out[format!("{opposing} Amount")] = json!(value);
            } else {
                out["Fiat Value"] = json!(value);
            }
        }
    }
    if let Some(fee) = cell("fee") {
        out["Fee Amount"] = json!(fee);
        if let Some(code) =
            cell("fee_asset").or_else(|| plan["fee_header_asset"].as_str().map(str::to_owned))
        {
            out["Fee Asset"] = json!(code);
        }
    }
    for (field, target) in [
        ("txid", "Tx-ID"),
        ("payment_hash", "Payment Hash"),
        ("payment_hash_source", "Payment Hash Source"),
        ("swap_refund_funding_txid", "Swap Refund Funding Tx-ID"),
        ("swap_refund_funding_vout", "Swap Refund Funding Vout"),
        ("note", "Note"),
        ("counterparty", "Counterparty"),
    ] {
        if !plan[field].is_null() {
            out[target] = json!(cell(field));
        }
    }
    Ok(csv::Row {
        headers: out.as_object().unwrap().keys().cloned().collect(),
        data: out,
        rest: None,
        original: None,
    })
}
fn fixed(value: &PriceNumber) -> String {
    match value {
        PriceNumber::Finite(n) => n.to_fixed_string().unwrap_or_else(|_| repr(value)),
        _ => repr(value),
    }
}
fn nonempty(row: &[Json]) -> bool {
    row.iter().any(|v| cell(v).is_some())
}
fn has_header(row: &[Json]) -> bool {
    let keys: Vec<_> = row.iter().filter_map(cell).map(|s| key(&s)).collect();
    ["type", "transaction type", "kind"]
        .iter()
        .any(|s| keys.iter().any(|k| k == s))
        && ["date", "timestamp", "time"]
            .iter()
            .any(|s| keys.iter().any(|k| k == s))
}
fn native(row: &[Json]) -> bool {
    has_header(row)
        && row.iter().filter_map(cell).any(|s| {
            [
                "received amount",
                "buy amount",
                "sent amount",
                "sell amount",
            ]
            .contains(&key(&s).as_str())
        })
}
fn read(file: &str) -> Result<Vec<Vec<Json>>, AppError> {
    if !std::path::Path::new(file).exists() {
        return Err(
            AppError::new("not_found", format!("Import file not found: {file}"))
                .with_hint("Check the file path."),
        );
    }
    let ext = std::path::Path::new(file)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if ["xlsx", "xlsm"].contains(&ext.as_str()) {
        xlsx::read(file)
    } else {
        platform_csv::raw_rows(file).map(|rows| {
            rows.into_iter()
                .map(|r| r.into_iter().map(Json::String).collect())
                .collect()
        })
    }
}
type SourceRows = (Vec<csv::Row>, Json, bool, Vec<String>, bool);
fn source(rows: &[Vec<Json>], mapping: Option<&ColumnMap>) -> Result<SourceRows, AppError> {
    let header_index = rows.iter().position(|r| nonempty(r));
    let header_row = header_index.map(|i| rows[i].as_slice()).unwrap_or_default();
    let headers: Vec<_> = header_row
        .iter()
        .map(|v| cell(v).unwrap_or_default())
        .collect();
    let native = mapping.is_none() && native(header_row);
    let (plan, detected) = if native {
        (Json::Null, Json::Null)
    } else if let Some(map) = mapping {
        (validate_map(map, &headers)?, Json::Null)
    } else {
        infer(&headers)
    };
    let mut widths = BTreeMap::new();
    for row in rows.iter().skip(header_index.map_or(0, |i| i + 1)) {
        let width = row.iter().filter(|v| cell(v).is_some()).count();
        if width > 0 {
            *widths.entry(width).or_insert(0) += 1;
        }
    }
    let preamble = widths
        .iter()
        .max_by_key(|(w, n)| (**n, **w))
        .is_some_and(|(w, _)| header_row.iter().filter(|v| cell(v).is_some()).count() < *w);
    if !native && !usable(&plan) {
        return Err(detail(AppError::new("ledger_unrecognized","Could not recognize the columns in this file.").with_hint("Download the import template (it already has the right columns), fill it in, and import that — or map the columns yourself with column_map."),[("headers", json!(headers))]));
    }
    let mut result = vec![];
    for row in rows
        .iter()
        .skip(header_index.map_or(0, |i| i + 1))
        .filter(|r| nonempty(r))
    {
        let data = Json::Object(
            headers
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.is_empty())
                .map(|(i, s)| (s.clone(), row.get(i).cloned().unwrap_or(json!(""))))
                .collect(),
        );
        let row = csv::Row {
            data,
            headers: headers.clone(),
            rest: None,
            original: None,
        };
        result.push(if native { row } else { remap(row, &plan)? });
    }
    Ok((result, detected, native, headers, preamble))
}
pub(super) fn load(file: &str, mapping: Option<&ColumnMap>) -> Result<Vec<csv::Row>, AppError> {
    let rows = read(file)?;
    let (rows, _, _, _, _) = source(&rows, mapping)?;
    let records = rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| record(row, i + 1))
        .collect::<Result<Vec<_>, _>>()?;
    if records.is_empty() {
        return Err(fail(
            "Ledger file has a header but no transaction rows to import",
            "Add at least one transaction row below the header.",
        ));
    }
    Ok(records)
}
/// `ui.wallets.ledger_preview`: the desktop uploads bytes, not a path. Like
/// Python, they are previewed from a private temporary copy carrying the
/// upload's extension, removed afterwards; nothing is persisted.
pub fn preview_upload(
    bytes: &[u8],
    extension: &str,
    mapping: Option<&ColumnMap>,
    limit: usize,
) -> Result<Py, AppError> {
    let temp = super::document::files::Temp::new("ledger-preview")?;
    let path = temp.path().join(format!("upload{extension}"));
    std::fs::write(&path, bytes).map_err(super::document::files::io)?;
    preview(&path.to_string_lossy(), mapping, limit)
}

pub fn preview(file: &str, mapping: Option<&ColumnMap>, limit: usize) -> Result<Py, AppError> {
    let rows = read(file)?;
    let source = source(&rows, mapping);
    if let Err(error) = &source {
        if error.code == "ledger_unrecognized" {
            let i = rows.iter().position(|r| nonempty(r));
            let headers: Vec<_> = i
                .map(|i| {
                    rows[i]
                        .iter()
                        .map(|v| cell(v).unwrap_or_default())
                        .collect()
                })
                .unwrap_or_default();
            let (_, detected) = infer(&headers);
            let data: Vec<_> = rows
                .iter()
                .skip(i.map_or(0, |i| i + 1))
                .filter(|r| nonempty(r))
                .collect();
            let mut widths = BTreeMap::new();
            for row in &data {
                let w = row.iter().filter(|v| cell(v).is_some()).count();
                *widths.entry(w).or_insert(0) += 1;
            }
            let preamble = widths
                .iter()
                .max_by_key(|(w, n)| (**n, **w))
                .is_some_and(|(w, _)| headers.iter().filter(|s| !s.is_empty()).count() < *w);
            return Ok(dict([
                ("confident", Py::Bool(false)),
                ("detected", py(&detected)),
                ("headers", py(&json!(headers))),
                ("header_is_preamble", Py::Bool(preamble)),
                ("template", Py::Bool(false)),
                ("rows_read", Py::from(data.len() as i64)),
                ("mapped", Py::from(0)),
                ("errors", Py::from(0)),
                ("problems", Py::List(vec![])),
                ("preview", Py::List(vec![])),
                ("truncated", Py::Bool(false)),
            ]));
        }
    }
    let (records, detected, native, headers, preamble) = source?;
    let count = records.len();
    let (mut normalized, mut problems) = (vec![], vec![]);
    for (i, row) in records.into_iter().enumerate() {
        match record(row, i + 1) {
            Ok(r) => normalized.push(r),
            Err(e) if e.code != "internal_error" => problems.push(dict([
                ("row", Py::from((i + 1) as i64)),
                ("message", s(e.message)),
            ])),
            Err(e) => return Err(e),
        }
    }
    let mut maximum = None;
    for row in &normalized {
        let value = abs(PriceNumber::parse(row.data["amount"].as_str().unwrap())?)?;
        if maximum.is_none() || positive(&arithmetic(&value, maximum.as_ref().unwrap(), '-')?)? {
            maximum = Some(value);
        }
    }
    let mut preview = vec![];
    for row in normalized.iter().take(limit) {
        preview.push(dict(
            [
                "occurred_at",
                "direction",
                "asset",
                "amount",
                "fee",
                "kind",
                "fiat_currency",
                "fiat_value",
                "description",
            ]
            .map(|k| {
                (
                    k,
                    if ["amount", "fee", "fiat_value"].contains(&k) && !row.data[k].is_null() {
                        s(fixed(
                            &PriceNumber::parse(row.data[k].as_str().unwrap()).unwrap(),
                        ))
                    } else {
                        py(&row.data[k])
                    },
                )
            }),
        ));
    }
    Ok(dict([
        ("headers", py(&json!(headers))),
        ("header_is_preamble", Py::Bool(preamble)),
        (
            "amount_max",
            s(maximum
                .as_ref()
                .map(fixed)
                .unwrap_or_else(|| "0.000000".into())),
        ),
        ("template", Py::Bool(native)),
        ("rows_read", Py::from(count as i64)),
        ("mapped", Py::from(normalized.len() as i64)),
        ("errors", Py::from(problems.len() as i64)),
        (
            "problems",
            Py::List(problems.into_iter().take(limit).collect()),
        ),
        ("preview", Py::List(preview)),
        ("truncated", Py::Bool(normalized.len() > limit)),
        ("confident", Py::Bool(true)),
        ("detected", py(&detected)),
    ]))
}
