//! `fetch_binance_records`: four signed history reads, then
//! `normalize_binance_records`. In `crypto` scope the reads cover every coin
//! in Binance's time windows, plus Convert history
//! (docs/reference/imports.md#binance-api-and-supplemental-csv).
use super::*;
use assets::{Scope, Tally};

/// Binance.com spot trading launched in July 2017; Python reads from there.
const START_MS: &str = "1498867200000";
const START: u64 = 1_498_867_200_000;
const DAY: u64 = 86_400_000;
/// A window that is still full at this span cannot be read completely.
const SPLIT_FLOOR: u64 = 60_000;
/// Convert history weighs 3,000 of the account's 180,000 per minute, and
/// withdrawal history allows 10 requests a second: reads of either are
/// spaced so one sync stays inside both limits.
const CONVERT_PACE: Duration = Duration::from_millis(if cfg!(test) { 0 } else { 1_100 });
const WITHDRAW_PACE: Duration = Duration::from_millis(if cfg!(test) { 0 } else { 120 });

pub(super) fn fetch(client: &Client<'_>) -> Result<Fetched, AppError> {
    let end = now()?.as_millis().to_string();
    let fiat = signed_get(
        client,
        "/sapi/v1/fiat/payments",
        &[
            ("transactionType", "0"),
            ("beginTime", START_MS),
            ("endTime", &end),
        ],
    )?;
    let fiat = attribute_get(&fiat, "data")?;
    if client.scope == Scope::Crypto {
        let end: u64 = end.parse().map_err(|_| limit())?;
        let mut reads = Reads {
            client,
            pages: 0,
            rows: 0,
        };
        let deposits = reads.offset("/sapi/v1/capital/deposit/hisrec", end, Duration::ZERO)?;
        let withdrawals = reads.offset("/sapi/v1/capital/withdraw/history", end, WITHDRAW_PACE)?;
        let dividends = reads.split("/sapi/v1/asset/assetDividend", end, Split::DIVIDENDS)?;
        let converts = reads.split("/sapi/v1/convert/tradeFlow", end, Split::CONVERTS)?;
        return normalize(
            &[
                (Kind::Fiat, fiat),
                (Kind::Deposit, deposits),
                (Kind::Withdrawal, withdrawals),
                (Kind::Dividend, dividends),
                (Kind::Convert, converts),
            ],
            Scope::Crypto,
        );
    }
    let deposits = signed_get(
        client,
        "/sapi/v1/capital/deposit/hisrec",
        &[("coin", "BTC"), ("startTime", START_MS), ("endTime", &end)],
    )?;
    let withdrawals = signed_get(
        client,
        "/sapi/v1/capital/withdraw/history",
        &[("coin", "BTC"), ("startTime", START_MS), ("endTime", &end)],
    )?;
    let dividends = signed_get(
        client,
        "/sapi/v1/asset/assetDividend",
        &[
            ("asset", "BTC"),
            ("startTime", START_MS),
            ("endTime", &end),
            ("limit", "500"),
        ],
    )?;
    let dividends = attribute_get(&dividends, "rows")?;
    normalize(
        &[
            (Kind::Fiat, fiat),
            (Kind::Deposit, deposits),
            (Kind::Withdrawal, withdrawals),
            (Kind::Dividend, dividends),
        ],
        Scope::Bitcoin,
    )
}

/// `[start, end]` windows of at most `span` milliseconds, oldest first,
/// from Binance's launch to `end`.
fn windows(end: u64, span: u64) -> Vec<(u64, u64)> {
    let (mut windows, mut start) = (vec![], START);
    while start <= end {
        let stop = (start + span - 1).min(end);
        windows.push((start, stop));
        start = stop + 1;
    }
    windows
}

/// A history read without a cursor: each window is read whole, and a full
/// window is halved until every part fits.
struct Split {
    span: u64,
    limit: usize,
    field: &'static str,
    pace: Duration,
}
impl Split {
    /// Dividends: at most 180 days apart, 500 rows, no paging.
    const DIVIDENDS: Split = Split {
        span: 179 * DAY,
        limit: 500,
        field: "rows",
        pace: Duration::ZERO,
    };
    /// Convert history: at most 30 days apart, 1,000 rows, `moreData` when
    /// the window holds more.
    const CONVERTS: Split = Split {
        span: 29 * DAY,
        limit: 1000,
        field: "list",
        pace: CONVERT_PACE,
    };
}

/// The bounded windowed reads of one `crypto`-scope sync.
struct Reads<'a, 'b> {
    client: &'a Client<'b>,
    pages: usize,
    rows: usize,
}
impl Reads<'_, '_> {
    fn get(&mut self, path: &str, params: &[(&str, &str)], pace: Duration) -> Result<Py, AppError> {
        self.pages += 1;
        if self.pages > MAX_PAGES {
            return Err(limit());
        }
        if self.pages > 1 && !pace.is_zero() {
            if Instant::now() + pace >= self.client.deadline {
                return Err(limit());
            }
            std::thread::sleep(pace);
        }
        signed_get(self.client, path, params)
    }

    fn keep(&mut self, rows: usize) -> Result<(), AppError> {
        self.rows += rows;
        if self.rows > MAX_ROWS {
            return Err(limit());
        }
        Ok(())
    }

    /// Deposit and withdrawal history: 89-day windows (Binance allows less
    /// than 90), each paged by `offset` in pages of 1,000 until one is short.
    fn offset(&mut self, path: &str, end: u64, pace: Duration) -> Result<Py, AppError> {
        let mut rows = vec![];
        for (start, stop) in windows(end, 89 * DAY) {
            let mut offset = 0usize;
            loop {
                let (start, stop, at) = (start.to_string(), stop.to_string(), offset.to_string());
                let page = self.get(
                    path,
                    &[
                        ("startTime", &start),
                        ("endTime", &stop),
                        ("offset", &at),
                        ("limit", "1000"),
                    ],
                    pace,
                )?;
                let Py::List(page) = page else {
                    return Err(AppError::new(
                        "protocol_error",
                        format!("Binance {path} response was not a JSON list"),
                    ));
                };
                self.keep(page.len())?;
                let short = page.len() < 1000;
                rows.extend(page);
                if short {
                    break;
                }
                offset += 1000;
            }
        }
        Ok(Py::List(rows))
    }

    /// A read with no cursor (`Split`): a window that comes back full is
    /// read again as two halves, down to a minute; a minute that is still
    /// full fails closed rather than drop rows.
    fn split(&mut self, path: &str, end: u64, how: Split) -> Result<Py, AppError> {
        let mut rows = vec![];
        let mut pending: Vec<(u64, u64)> = windows(end, how.span).into_iter().rev().collect();
        while let Some((start, stop)) = pending.pop() {
            let limit = how.limit.to_string();
            let reply = self.get(
                path,
                &[
                    ("startTime", &start.to_string()),
                    ("endTime", &stop.to_string()),
                    ("limit", &limit),
                ],
                how.pace,
            )?;
            let page = match &reply {
                Py::Dict(_) => match at(&reply, how.field) {
                    Py::Null => vec![],
                    Py::List(page) => page.clone(),
                    _ => {
                        return Err(AppError::new(
                            "protocol_error",
                            format!("Binance {path} response {} was not a JSON list", how.field),
                        ))
                    }
                },
                _ => {
                    return Err(AppError::new(
                        "protocol_error",
                        format!("Binance {path} response was not a JSON object"),
                    ))
                }
            };
            if page.len() >= how.limit || at(&reply, "moreData") == &Py::Bool(true) {
                if stop - start < SPLIT_FLOOR {
                    return Err(AppError::new(
                        "protocol_error",
                        format!(
                            "Binance {path} holds more than {} rows within one minute; Kassiber cannot read them all",
                            how.limit
                        ),
                    ));
                }
                let middle = start + (stop - start) / 2;
                pending.push((middle + 1, stop));
                pending.push((start, middle));
                continue;
            }
            self.keep(page.len())?;
            rows.extend(page);
        }
        Ok(Py::List(rows))
    }
}

/// Restrictions that only let a key read.
const READ_FLAGS: [&str; 2] = ["enableReading", "enableFixReadOnly"];

/// The desktop key check: `GET /sapi/v1/account/apiRestrictions` says what
/// the key may do. It must read, and every other `enable*`/`permits*` flag
/// (trading, margin, futures, options, withdrawals, transfers, and any flag
/// Binance adds later) must be off.
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    let restrictions = signed_get(client, "/sapi/v1/account/apiRestrictions", &[])?;
    let Py::Dict(flags) = &restrictions else {
        return Err(AppError::new(
            "protocol_error",
            "Binance API restrictions were not a JSON object",
        ));
    };
    if at(&restrictions, "enableReading") != &Py::Bool(true) {
        return Err(AppError::new(
            "auth_error",
            "This Binance API key cannot read account history",
        )
        .with_hint("Turn on Enable Reading for the key in Binance API Management."));
    }
    let mut granted: Vec<String> = flags
        .iter()
        .map(|(name, value)| (lossy(&key_text(name)), value))
        .filter(|(name, value)| {
            (name.starts_with("enable") || name.starts_with("permits"))
                && !READ_FLAGS.contains(&name.as_str())
                && **value == Py::Bool(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !granted.is_empty() {
        granted.sort();
        return Err(AppError::new(
            "auth_error",
            "This Binance API key can do more than read; Kassiber only accepts read-only keys",
        )
        .with_hint(
            "Edit the key's restrictions in Binance API Management so only Enable Reading is on, or create a new read-only key.",
        )
        .with_details(dict([(
            "permissions",
            Py::List(granted.into_iter().map(s).collect()),
        )])));
    }
    Ok(Some(true))
}

/// `normalize_binance_records(payload)` for an assembled payload dict.
#[cfg(test)]
pub(super) fn normalize_payload(payload: &Py) -> Result<Vec<Py>, AppError> {
    let value = |key| get(payload, key).cloned().unwrap_or(Py::List(vec![]));
    normalize(
        &[
            (Kind::Fiat, value("fiat_payments")),
            (Kind::Deposit, value("deposits")),
            (Kind::Withdrawal, value("withdrawals")),
            (Kind::Dividend, value("dividends")),
        ],
        Scope::Bitcoin,
    )
    .map(|fetched| fetched.records)
}

/// `payload.get(key, [])` on whatever the server returned.
fn attribute_get(payload: &Py, key: &str) -> Result<Py, AppError> {
    match payload {
        Py::Dict(_) => Ok(get(payload, key).cloned().unwrap_or(Py::List(vec![]))),
        other => Err(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(other)
            ),
        )),
    }
}

/// `_binance_signed_get`: the query in insertion order, a fresh millisecond
/// timestamp and an HMAC-SHA256 signature over exactly that query.
fn signed_get(client: &Client<'_>, path: &str, params: &[(&str, &str)]) -> Result<Py, AppError> {
    let timestamp = now()?.as_millis().to_string();
    let query = params
        .iter()
        .copied()
        .chain([("timestamp", timestamp.as_str())])
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let signature = hmac_sha256_hex(client.secret.as_bytes(), query.as_bytes());
    let url = format!("{}{path}?{query}&signature={signature}", client.base);
    client.read(
        "GET",
        &url,
        &[("Accept", "application/json"), ("X-MBX-APIKEY", client.key)],
        None,
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Fiat,
    Deposit,
    Withdrawal,
    Dividend,
    Convert,
}

/// `normalize_binance_records(payload)`: each history's rows in order, as
/// `for row in payload.get(key, []) or []` iterates them. Rows in an asset
/// the scope leaves out are counted.
fn normalize(payload: &[(Kind, Py)], scope: Scope) -> Result<Fetched, AppError> {
    let mut records = vec![];
    let mut tally = Tally::default();
    for (kind, rows) in payload {
        let rows = match rows {
            Py::List(rows) => rows.as_slice(),
            // Iterating a dict or str yields keys or characters, never rows.
            Py::Dict(_) | Py::Str(_) => &[],
            other if !truthy(other) => &[],
            other => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "'{}' object is not iterable",
                        kassiber_envelope::type_name(other)
                    ),
                ))
            }
        };
        for row in rows.iter().filter(|row| matches!(row, Py::Dict(_))) {
            let mut admit = |value: &Py, what: &str| admit(value, what, scope, &mut tally);
            match kind {
                Kind::Fiat => records.extend(fiat_payment(row, &mut admit)?),
                Kind::Deposit => records.extend(transfer(row, true, scope, &mut admit)?),
                Kind::Withdrawal => records.extend(transfer(row, false, scope, &mut admit)?),
                Kind::Dividend => records.extend(dividend(row, &mut admit)?),
                Kind::Convert => records.extend(convert(row, scope, &mut tally)?),
            }
        }
    }
    Ok(Fetched {
        records,
        skipped: None,
        tally: Some(tally),
        reported: None,
    })
}

/// Binance's renamed coins, from its own listing history.
const RENAMES: [(&str, &str); 3] = [("BCC", "BCH"), ("HSR", "HC"), ("RPX", "PHX")];

/// The canonical code of a Binance coin, or `None` when it is no ticker.
fn code(value: &Py) -> Option<String> {
    let code = normalized(value);
    let code = RENAMES
        .iter()
        .find(|(old, _)| *old == code)
        .map_or(code, |(_, new)| (*new).to_owned());
    (code == "LBTC" || ticker(&code)).then_some(code)
}

/// The asset a row in coin `value` is imported as, or `None` when the scope
/// leaves it out (counted) or it is fiat. Bitcoin is recognised exactly as
/// the reference does; a coin that is no ticker is refused in `crypto`
/// scope and left out, as before, in `bitcoin` scope.
fn admit(
    value: &Py,
    what: &str,
    scope: Scope,
    tally: &mut Tally,
) -> Result<Option<String>, AppError> {
    let currency = asset(value);
    if is_bitcoin(&currency) {
        return Ok(Some(currency));
    }
    match code(value) {
        Some(code) => {
            let class = assets::class(&code);
            Ok(tally.admit(scope, &code, class).then_some(code))
        }
        None if scope == Scope::Bitcoin => Ok(None),
        None => Err(unmapped(value, what)),
    }
}
fn unmapped(value: &Py, what: &str) -> AppError {
    AppError::new(
        "validation",
        format!(
            "Binance {what} has coin '{}', which Kassiber cannot map exactly",
            lossy(&text(value))
        ),
    )
    .with_hint("Review this row through the generic ledger.")
}

/// `record(entry)` after the exact-unit check names the row.
fn emit(entry: Entry, what: &str) -> Result<Py, AppError> {
    exact(&entry, what)?;
    record(entry)
}

/// `str(value or "").strip().casefold()`.
fn folded(value: &Py) -> String {
    let text = if truthy(value) {
        lossy(&text(value))
    } else {
        String::new()
    };
    kassiber_pytext::casefold(kassiber_pytext::strip(&text))
}

type Admit<'a> = dyn FnMut(&Py, &str) -> Result<Option<String>, AppError> + 'a;

/// `normalize_binance_fiat_payment(row)`.
fn fiat_payment(row: &Py, admit: &mut Admit) -> Result<Option<Py>, AppError> {
    if folded(at(row, "status")) != "completed" {
        return Ok(None);
    }
    let reference = either(row, &["orderNo", "orderId"]);
    let what = format!("fiat payment '{}'", lossy(&text(reference)));
    let Some(currency) = admit(at(row, "cryptoCurrency"), &what)? else {
        return Ok(None);
    };
    let fiat_currency = asset(at(row, "fiatCurrency"));
    if !is_fiat(&fiat_currency) {
        return Err(AppError::new(
            "validation",
            "Binance fiat payment is not fiat-denominated",
        ));
    }
    let amount = decimal(at(row, "obtainAmount"), "0")?;
    let fiat_value = abs(decimal(at(row, "sourceAmount"), "0")?)?;
    let occurred_at = epoch(at(row, "createTime"), true)?;
    let reference = if truthy(reference) {
        text(reference)
    } else {
        occurred_at.clone().unwrap_or_else(|| "None".into())
    };
    let price = at(row, "price");
    let fiat_rate = if *price == Py::Null || *price == s("") {
        None
    } else {
        Some(decimal(price, "0")?)
    };
    emit(
        Entry {
            provider: "Binance",
            external_id: concat(&["binance:".into(), reference.clone()]),
            occurred_at,
            inbound: true,
            asset: currency,
            amount,
            fee: PriceNumber::Finite(Decimal::zero()),
            kind: "buy",
            raw: row.clone(),
            description: None,
            pricing: Some(Pricing {
                fiat_currency,
                fiat_value,
                fiat_rate,
                method: "binance_api",
                external_ref: reference,
            }),
        },
        &what,
    )
    .map(Some)
}

/// `normalize_binance_transfer(row, inbound=...)`. Another asset's row keys
/// on Binance's own record id, since one chain transaction may carry
/// several coins.
fn transfer(
    row: &Py,
    inbound: bool,
    scope: Scope,
    admit: &mut Admit,
) -> Result<Option<Py>, AppError> {
    let direction = if inbound { "deposit" } else { "withdrawal" };
    let what = format!(
        "{direction} '{}'",
        lossy(&text(either(row, &["id", "txId", "withdrawOrderId"])))
    );
    let coin = either(row, &["coin", "asset"]);
    if is_bitcoin(&asset(coin)) {
        // The reference's filter, which the Python vectors pin for BTC.
        let status = folded(at(row, "status"));
        if !status.is_empty()
            && !["1", "6", "success", "completed", "credited", "confirming"]
                .contains(&status.as_str())
        {
            return Ok(None);
        }
    } else {
        match settled(at(row, "status"), inbound) {
            Some(true) => {}
            Some(false) => return Ok(None),
            None if scope == Scope::Bitcoin => return Ok(None),
            None => {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "Binance {what} has status '{}', which Kassiber does not know",
                        lossy(&text(at(row, "status")))
                    ),
                ))
            }
        }
    }
    let Some(currency) = admit(coin, &what)? else {
        return Ok(None);
    };
    let amount = decimal(at(row, "amount"), "0")?;
    let fee = decimal(either(row, &["transactionFee", "fee"]), "0")?;
    let occurred_at = epoch(
        either(row, &["insertTime", "applyTime", "successTime"]),
        true,
    )?;
    let record_id = str_or_none(at(row, "id")).filter(|_| !is_bitcoin(&currency));
    let external_id = match record_id {
        Some(id) => concat(&[format!("binance:{direction}:").into(), id]),
        None => str_or_none(either(row, &["txId", "id", "withdrawOrderId"])).unwrap_or_else(|| {
            concat(&[
                "binance:".into(),
                occurred_at.clone().unwrap_or_else(|| "None".into()),
                format!(":{currency}:{}", repr(&amount)).into(),
            ])
        }),
    };
    emit(
        Entry {
            provider: "Binance",
            external_id,
            occurred_at,
            inbound,
            asset: currency,
            amount,
            fee: if inbound {
                PriceNumber::Finite(Decimal::zero())
            } else {
                fee
            },
            kind: direction,
            raw: row.clone(),
            description: Some(
                if inbound {
                    "Binance deposit"
                } else {
                    "Binance withdrawal"
                }
                .into(),
            ),
            pricing: None,
        },
        &what,
    )
    .map(Some)
}

/// Whether another coin's deposit or withdrawal is final, by Binance's
/// documented status codes: a deposit is credited at 1 (success) or 6
/// (credited, not yet withdrawable) and still open or failed at 0, 2, 7 and
/// 8; a withdrawal is complete at 6 and open, cancelled, rejected or failed
/// at 0 to 5. `None` for any other value.
fn settled(status: &Py, inbound: bool) -> Option<bool> {
    let status = match status {
        Py::Int(_) | Py::Str(_) => lossy(&text(status)),
        _ => return None,
    };
    match (inbound, status.as_str()) {
        (true, "1" | "6") | (false, "6") => Some(true),
        (true, "0" | "2" | "7" | "8") | (false, "0" | "1" | "2" | "3" | "4" | "5") => Some(false),
        _ => None,
    }
}

/// `normalize_binance_dividend(row)`: income (or mining), unpriced.
fn dividend(row: &Py, admit: &mut Admit) -> Result<Option<Py>, AppError> {
    let id = either(row, &["id", "tranId"]);
    let what = format!("dividend '{}'", lossy(&text(id)));
    let Some(currency) = admit(either(row, &["asset", "coinName"]), &what)? else {
        return Ok(None);
    };
    let amount = decimal(either(row, &["amount", "profitAmount"]), "0")?;
    if is_zero(&amount)? {
        return Ok(None);
    }
    // The reference falls back to a time for BTC; another asset needs
    // Binance's own record id, or two coins paid at once would collide.
    if !is_bitcoin(&currency) && !truthy(id) {
        return Err(AppError::new(
            "validation",
            format!("Binance {currency} dividend has no id or tranId"),
        ));
    }
    let reference = text(either(row, &["id", "tranId", "time", "divTime"]));
    let occurred_at = epoch(either(row, &["divTime", "time"]), true)?;
    let info = str_or_none(either(row, &["enInfo", "type"]));
    let mining = info
        .as_ref()
        .is_some_and(|info| kassiber_pytext::casefold(&lossy(info)).contains("mining"));
    let description = match &info {
        Some(info) => concat(&["Binance income - ".into(), info.clone()]),
        None => "Binance income".into(),
    };
    emit(
        Entry {
            provider: "Binance",
            external_id: concat(&["binance:".into(), reference]),
            occurred_at,
            inbound: true,
            asset: currency,
            amount,
            fee: PriceNumber::Finite(Decimal::zero()),
            kind: if mining { "mining" } else { "income" },
            raw: row.clone(),
            description: Some(description),
            pricing: None,
        },
        &what,
    )
    .map(Some)
}

/// One Convert order (`/sapi/v1/convert/tradeFlow`). Only `SUCCESS` orders
/// are imported; others are still open or failed. Against fiat it is a buy
/// or sell priced at the fiat side; between two crypto assets it is a swap
/// pair, unpriced. Convert charges no separate fee: its price includes it.
pub(super) fn convert(row: &Py, scope: Scope, tally: &mut Tally) -> Result<Vec<Py>, AppError> {
    if folded(at(row, "orderStatus")) != "success" {
        return Ok(vec![]);
    }
    let Some(order) = str_or_none(at(row, "orderId")) else {
        return Err(AppError::new(
            "validation",
            "Binance convert order has no orderId",
        ));
    };
    let name = lossy(&order);
    let what = format!("convert '{name}'");
    let side = |asset: &str, amount: &str| -> Result<(String, PriceNumber), AppError> {
        let value = at(row, asset);
        let code = code(value).ok_or_else(|| unmapped(value, &what))?;
        let amount = decimal(at(row, amount), "0")?;
        if !positive(&amount)? {
            return Err(AppError::new(
                "validation",
                format!("Binance {what} has no positive {code} amount"),
            ));
        }
        Ok((code, amount))
    };
    let (from, from_amount) = side("fromAsset", "fromAmount")?;
    let (to, to_amount) = side("toAsset", "toAmount")?;
    let occurred_at = epoch(at(row, "createTime"), true)?;
    let id = |suffix: &str| concat(&["binance:convert:".into(), order.clone(), suffix.into()]);
    let entry = |inbound: bool, asset: &str, amount: &PriceNumber, kind, pricing| Entry {
        provider: "Binance",
        external_id: id(match (kind, inbound) {
            ("swap", true) => ":in",
            ("swap", false) => ":out",
            _ => "",
        }),
        occurred_at: occurred_at.clone(),
        inbound,
        asset: asset.into(),
        amount: amount.clone(),
        fee: PriceNumber::Finite(Decimal::zero()),
        kind,
        raw: row.clone(),
        description: Some("Binance convert".into()),
        pricing,
    };
    let priced = |currency: &str, value: &PriceNumber| {
        Some(Pricing {
            fiat_currency: currency.into(),
            fiat_value: value.clone(),
            fiat_rate: None,
            method: "binance_api",
            external_ref: order.clone(),
        })
    };
    use assets::Class::Fiat;
    let (from_class, to_class) = (assets::class(&from), assets::class(&to));
    let records = match (from_class, to_class) {
        (Fiat, Fiat) => vec![],
        (Fiat, class) if tally.admit(scope, &to, class) => vec![emit(
            entry(true, &to, &to_amount, "buy", priced(&from, &from_amount)),
            &what,
        )?],
        (_, Fiat) if tally.admit(scope, &from, from_class) => vec![emit(
            entry(false, &from, &from_amount, "sell", priced(&to, &to_amount)),
            &what,
        )?],
        (Fiat, _) | (_, Fiat) => vec![],
        _ => match (
            tally.admit(scope, &from, from_class),
            tally.admit(scope, &to, to_class),
        ) {
            (true, true) => {
                let reference = id("");
                let out = entry(false, &from, &from_amount, "swap", None);
                let into = entry(true, &to, &to_amount, "swap", None);
                exact(&out, &what)?;
                exact(&into, &what)?;
                vec![swap_leg(out, &reference)?, swap_leg(into, &reference)?]
            }
            (false, false) => vec![],
            _ => return Err(AppError::new(
                "validation",
                format!("Binance {what} swaps {from} for {to}"),
            )
            .with_hint(
                "A swap between Bitcoin and another asset needs the wallet's crypto asset scope.",
            )),
        },
    };
    Ok(records)
}
