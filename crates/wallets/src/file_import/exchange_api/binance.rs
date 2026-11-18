//! `fetch_binance_records`: the Python reference's BTC history reads
//! (card buys, BTC deposits, withdrawals and dividends), read in the time
//! windows Binance allows, plus Convert orders and the spot BTC balance
//! (docs/reference/imports.md#binance-api-and-supplemental-csv). Binance
//! serves spot trades only per market, so the API never reads them: every
//! sync says so, and its BTC balance check shows what they moved. The
//! Transaction History CSV export is Binance's complete ledger.
use super::*;
use assets::Class;

pub(super) mod history;

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
/// What the balance check compares: the spot account's BTC, free plus locked
/// in open orders (`GET /api/v3/account`); Funding, Earn and other wallets
/// are not in it.
const BALANCE_SCOPE: &str = "spot_free_plus_locked";
/// Every sync's `next_step`: what the API never reads.
const SPOT_TRADES: &str = "Kassiber does not read Binance spot trades through the API (Binance lists them only per market, so no read can prove them complete): import Binance's Transaction History CSV export instead, which is the complete ledger, and compare provider_balance";

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "binance",
    provider: "Binance",
    reads: contract::Reads::BitcoinOnly,
    default_url: "https://api.binance.com",
    key_only: false,
    fetch,
    probe,
    skipped: None,
    files: &[contract::registry::FileFormat {
        format: Format::BinanceHistory,
        reads: contract::Reads::AllAssets,
        load: history::load,
    }],
    shared_ids: false,
};

fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    let end = now()?.as_millis().to_string();
    let mut reads = Reads {
        client,
        pages: 0,
        rows: 0,
    };
    let fiat = reads.fiat(&end)?;
    let end: u64 = end.parse().map_err(|_| limit())?;
    let btc = [("coin", "BTC")];
    let deposits = reads.offset("/sapi/v1/capital/deposit/hisrec", end, Duration::ZERO, &btc)?;
    let withdrawals = reads.offset(
        "/sapi/v1/capital/withdraw/history",
        end,
        WITHDRAW_PACE,
        &btc,
    )?;
    let dividends = reads.split(
        "/sapi/v1/asset/assetDividend",
        end,
        Split::DIVIDENDS,
        &[("asset", "BTC")],
    )?;
    let converts = reads.split("/sapi/v1/convert/tradeFlow", end, Split::CONVERTS, &[])?;
    normalize(
        &[
            (History::Fiat, fiat),
            (History::Deposit, deposits),
            (History::Withdrawal, withdrawals),
            (History::Dividend, dividends),
            (History::Convert, converts),
        ],
        mapper,
    )?;
    mapper.notice(SPOT_TRADES.into());
    // Read last, so the balance is never older than the history.
    Ok(Some(balance(client)?))
}

/// `GET /api/v3/account`: the spot account's BTC, free plus locked, each
/// whole msat by its own digits. An account without a BTC entry holds none.
fn balance(client: &Client<'_>) -> Result<Balance, AppError> {
    let payload = signed_get(client, "/api/v3/account", &[("omitZeroBalances", "true")])?;
    let as_of = utc_iso(now()?.as_secs() as f64)?;
    let protocol = |message: &str| AppError::new("protocol_error", message.to_owned());
    let Py::List(rows) = at(&payload, "balances") else {
        return Err(protocol("Binance account balances were not a JSON list"));
    };
    let mut btc = rows.iter().filter(|row| asset(at(row, "asset")) == "BTC");
    let mut msat: i64 = 0;
    if let Some(row) = btc.next() {
        for key in ["free", "locked"] {
            let value = match at(row, key) {
                Py::Str(text) => text.as_str().and_then(|text| strict_decimal(text, false)),
                _ => None,
            }
            .ok_or_else(|| protocol("Binance BTC balance is not a plain decimal"))?;
            let part = exact_msat(&value)
                .ok_or_else(|| protocol("Binance BTC balance is finer than a millisatoshi"))?;
            msat = msat
                .checked_add(part)
                .ok_or_else(|| protocol("Binance BTC balance is out of range"))?;
        }
    }
    if btc.next().is_some() {
        return Err(protocol("Binance listed its BTC balance twice"));
    }
    Ok(Balance {
        msat: Ok(msat),
        as_of,
        scope: BALANCE_SCOPE,
    })
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
    paging: Paging,
}

/// How an answer says whether its window holds more rows than it returned.
#[derive(Clone, Copy)]
enum Paging {
    /// `total`: the window's row count.
    Total,
    /// `moreData`: true when rows were left out.
    MoreData,
}
impl Split {
    /// Dividends: at most 180 days apart, 500 rows, no paging; `total` is
    /// the window's count.
    const DIVIDENDS: Split = Split {
        span: 179 * DAY,
        limit: 500,
        field: "rows",
        pace: Duration::ZERO,
        paging: Paging::Total,
    };
    /// Convert history: at most 30 days apart, 1,000 rows, `moreData` when
    /// the window holds more.
    const CONVERTS: Split = Split {
        span: 29 * DAY,
        limit: 1000,
        field: "list",
        pace: CONVERT_PACE,
        paging: Paging::MoreData,
    };
}

/// A count Binance states (`total`): a non-negative integer.
fn stated_count(value: &Py) -> Option<usize> {
    match value {
        Py::Int(count) => count.to_string().parse().ok(),
        _ => None,
    }
}

fn unpaged(path: &str, field: &str) -> AppError {
    AppError::new(
        "protocol_error",
        format!("Binance {path} answered without a valid {field}; Kassiber cannot tell whether it read every row"),
    )
}

/// The bounded windowed reads of one sync.
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

    /// Card and bank buys: pages of 500 (Binance's most) until the answer's
    /// `total` is read. An answer without a `total`, or one that ends
    /// before it, refuses.
    fn fiat(&mut self, end: &str) -> Result<Py, AppError> {
        const PATH: &str = "/sapi/v1/fiat/payments";
        let mut rows = vec![];
        for page in 1.. {
            let number = page.to_string();
            let reply = self.get(
                PATH,
                &[
                    ("transactionType", "0"),
                    ("beginTime", START_MS),
                    ("endTime", end),
                    ("page", number.as_str()),
                    ("rows", "500"),
                ],
                Duration::ZERO,
            )?;
            let data = attribute_get(&reply, "data")?;
            let total = stated_count(at(&reply, "total")).ok_or_else(|| unpaged(PATH, "total"))?;
            let Py::List(data) = data else {
                return Err(AppError::new(
                    "protocol_error",
                    format!("Binance {PATH} response data was not a JSON list"),
                ));
            };
            self.keep(data.len())?;
            let empty = data.is_empty();
            rows.extend(data);
            if rows.len() >= total {
                break;
            }
            if empty {
                return Err(AppError::new(
                    "protocol_error",
                    format!(
                        "Binance {PATH} stated {total} rows but returned {}",
                        rows.len()
                    ),
                ));
            }
        }
        Ok(Py::List(rows))
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
    fn offset(
        &mut self,
        path: &str,
        end: u64,
        pace: Duration,
        filter: &[(&str, &str)],
    ) -> Result<Py, AppError> {
        let mut rows = vec![];
        for (start, stop) in windows(end, 89 * DAY) {
            let mut offset = 0usize;
            loop {
                let (start, stop, at) = (start.to_string(), stop.to_string(), offset.to_string());
                let mut params = filter.to_vec();
                params.extend([
                    ("startTime", start.as_str()),
                    ("endTime", stop.as_str()),
                    ("offset", at.as_str()),
                    ("limit", "1000"),
                ]);
                let page = self.get(path, &params, pace)?;
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

    /// A read with no cursor (`Split`): a window that comes back full, or
    /// states more rows than it returned, is read again as two halves, down
    /// to a minute; a minute that is still full fails closed rather than
    /// drop rows, and so does an answer without its paging field.
    fn split(
        &mut self,
        path: &str,
        end: u64,
        how: Split,
        filter: &[(&str, &str)],
    ) -> Result<Py, AppError> {
        let mut rows = vec![];
        let mut pending: Vec<(u64, u64)> = windows(end, how.span).into_iter().rev().collect();
        while let Some((start, stop)) = pending.pop() {
            let limit = how.limit.to_string();
            let (from, to) = (start.to_string(), stop.to_string());
            let mut params = filter.to_vec();
            params.extend([
                ("startTime", from.as_str()),
                ("endTime", to.as_str()),
                ("limit", limit.as_str()),
            ]);
            let reply = self.get(path, &params, how.pace)?;
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
            let more = match how.paging {
                Paging::Total => {
                    stated_count(at(&reply, "total")).ok_or_else(|| unpaged(path, "total"))?
                        > page.len()
                }
                Paging::MoreData => match at(&reply, "moreData") {
                    Py::Bool(more) => *more,
                    _ => return Err(unpaged(path, "moreData")),
                },
            };
            if page.len() >= how.limit || more {
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
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
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
pub(super) fn normalize_payload(payload: &Py, mapper: &mut Mapper) -> Result<(), AppError> {
    let value = |key| get(payload, key).cloned().unwrap_or(Py::List(vec![]));
    normalize(
        &[
            (History::Fiat, value("fiat_payments")),
            (History::Deposit, value("deposits")),
            (History::Withdrawal, value("withdrawals")),
            (History::Dividend, value("dividends")),
        ],
        mapper,
    )
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
enum History {
    Fiat,
    Deposit,
    Withdrawal,
    Dividend,
    Convert,
}

/// `normalize_binance_records(payload)`: each history's rows in order, as
/// `for row in payload.get(key, []) or []` iterates them.
fn normalize(payload: &[(History, Py)], mapper: &mut Mapper) -> Result<(), AppError> {
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
            let event = match kind {
                History::Fiat => fiat_payment(row, mapper)?,
                History::Deposit => transfer(row, true, mapper)?,
                History::Withdrawal => transfer(row, false, mapper)?,
                History::Dividend => dividend(row, mapper)?,
                History::Convert => convert(row, mapper)?,
            };
            if let Some(event) = event {
                mapper.push(event)?;
            }
        }
    }
    Ok(())
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

/// `normalize_binance_fiat_payment(row)`.
fn fiat_payment(row: &Py, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    if folded(at(row, "status")) != "completed" {
        return Ok(None);
    }
    let currency = asset(at(row, "cryptoCurrency"));
    if !mapper.admit(&[&currency]) {
        return Ok(None);
    }
    let fiat_currency = fiat_code(asset(at(row, "fiatCurrency")));
    if !is_fiat(&fiat_currency) {
        return Err(AppError::new(
            "validation",
            "Binance fiat payment is not fiat-denominated",
        ));
    }
    let amount = decimal(at(row, "obtainAmount"), "0")?;
    let fiat_value = abs(decimal(at(row, "sourceAmount"), "0")?)?;
    let occurred_at = epoch(at(row, "createTime"), true)?;
    let reference = either(row, &["orderNo", "orderId"]);
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
    let external_id = concat(&["binance:".into(), reference.clone()]);
    Ok(Some(Event {
        occurred_at,
        raw: row.clone(),
        pricing: Some(Pricing {
            fiat_currency,
            fiat_value,
            fiat_rate,
            method: "binance_api",
            external_ref: reference,
        }),
        ..Event::one(external_id, Kind::Trade, true, currency, amount)
    }))
}

/// Binance's documented transfer statuses: deposits 0 pending, 1 success,
/// 2 rejected, 6 credited (not yet withdrawable), 7 wrong deposit, 8
/// waiting for the user; withdrawals 0 email sent, 1 cancelled, 2 awaiting
/// approval, 3 rejected, 4 processing, 5 failure, 6 completed. `Some(true)`
/// for a settled one, `Some(false)` for one that moved nothing (yet),
/// `None` for a status Binance does not document. The Python reference's
/// words keep their reading where they name a settled transfer (`success`,
/// `completed`, and `credited` for a deposit), and a row without a status
/// (Binance always sends one) is read as it read it: settled.
fn settled(inbound: bool, status: &Py) -> Option<bool> {
    let code = match status {
        Py::Null => return Some(true),
        Py::Int(code) => code.to_string(),
        Py::Str(_) => folded(status),
        _ => return None,
    };
    match (inbound, code.as_str()) {
        (_, "" | "success" | "completed") | (true, "1" | "6" | "credited") | (false, "6") => {
            Some(true)
        }
        (true, "0" | "2" | "7" | "8") | (false, "0" | "1" | "2" | "3" | "4" | "5") => Some(false),
        _ => None,
    }
}

/// `normalize_binance_transfer(row, inbound=...)`: a settled deposit or
/// withdrawal; one Binance has not settled is left out, and one whose
/// status Binance does not document refuses the sync.
fn transfer(row: &Py, inbound: bool, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    let currency = asset(either(row, &["coin", "asset"]));
    if !mapper.admit(&[&currency]) {
        return Ok(None);
    }
    let direction = if inbound { "deposit" } else { "withdrawal" };
    match settled(inbound, at(row, "status")) {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => {
            let reference = str_or_none(either(row, &["txId", "id", "withdrawOrderId"]))
                .map_or_else(String::new, |id| format!(" '{}'", lossy(&id)));
            return Err(AppError::new(
                "validation",
                format!(
                    "Binance {direction}{reference} has status {}, which Binance does not document for a {direction}",
                    kassiber_pyjson::dumps(at(row, "status"), &Default::default())
                        .unwrap_or_default()
                ),
            )
            .with_hint("Nothing was imported. Check the transfer in Binance and record it yourself with the generic CSV/JSON import if it settled."));
        }
    }
    let amount = decimal(at(row, "amount"), "0")?;
    let fee = decimal(either(row, &["transactionFee", "fee"]), "0")?;
    let occurred_at = epoch(
        either(row, &["insertTime", "applyTime", "successTime"]),
        true,
    )?;
    let external_id =
        str_or_none(either(row, &["txId", "id", "withdrawOrderId"])).unwrap_or_else(|| {
            concat(&[
                "binance:".into(),
                occurred_at.clone().unwrap_or_else(|| "None".into()),
                format!(":{currency}:{}", repr(&amount)).into(),
            ])
        });
    let (kind, description) = match inbound {
        true => ("deposit", "Binance deposit"),
        false => ("withdrawal", "Binance withdrawal"),
    };
    Ok(Some(Event {
        occurred_at,
        fee: if inbound {
            PriceNumber::Finite(Decimal::zero())
        } else {
            fee
        },
        raw: row.clone(),
        description: Some(description.into()),
        ..Event::one(external_id, Kind::Movement(kind), inbound, currency, amount)
    }))
}

/// `normalize_binance_dividend(row)`.
fn dividend(row: &Py, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    let currency = asset(either(row, &["asset", "coinName"]));
    if !mapper.admit(&[&currency]) {
        return Ok(None);
    }
    let amount = decimal(either(row, &["amount", "profitAmount"]), "0")?;
    if is_zero(&amount)? {
        return Ok(None);
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
    let kind = Kind::Movement(if mining { "mining" } else { "income" });
    let external_id = concat(&["binance:".into(), reference]);
    Ok(Some(Event {
        occurred_at,
        raw: row.clone(),
        description: Some(description),
        ..Event::one(external_id, kind, true, currency, amount)
    }))
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

/// One Convert order (`/sapi/v1/convert/tradeFlow`). Only `SUCCESS` orders
/// are read; others are still open or failed. BTC against fiat is a buy or
/// sell priced at the fiat side (Convert's price includes its fee). An order
/// without BTC is counted as unread. BTC against another crypto asset is
/// refused in either scope: the API never books the other asset, so the BTC
/// leg alone would be a fiat-priced sale or purchase of a swap.
fn convert(row: &Py, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    if folded(at(row, "orderStatus")) != "success" {
        return Ok(None);
    }
    let Some(order) = str_or_none(at(row, "orderId")) else {
        return Err(AppError::new(
            "validation",
            "Binance convert order has no orderId",
        ));
    };
    let what = format!("convert '{}'", lossy(&order));
    let side = |asset: &str| {
        let value = at(row, asset);
        code(value).ok_or_else(|| {
            AppError::new(
                "validation",
                format!(
                    "Binance {what} has coin '{}', which Kassiber cannot map exactly",
                    lossy(&text(value))
                ),
            )
            .with_hint("Record this row yourself with the generic CSV/JSON import.")
        })
    };
    let (from, to) = (side("fromAsset")?, side("toAsset")?);
    if !mapper.admit(&[&from, &to]) {
        return Ok(None);
    }
    let (from_class, to_class) = (assets::class(&from), assets::class(&to));
    let (inbound, coin, amount, fiat, value) = match (from_class, to_class) {
        (Class::Fiat, _) => (true, &to, "toAmount", &from, "fromAmount"),
        (_, Class::Fiat) => (false, &from, "fromAmount", &to, "toAmount"),
        _ => {
            return Err(AppError::new(
                "validation",
                format!("Binance {what} swaps {from} for {to}"),
            )
            .with_hint("The Binance API reads only BTC, so it cannot book the other side of this swap. Import Binance's Transaction History CSV export into a wallet whose asset_scope is crypto instead, which books both sides."))
        }
    };
    let positive_amount = |key: &str, code: &str| -> Result<PriceNumber, AppError> {
        let amount = decimal(at(row, key), "0")?;
        if !positive(&amount)? {
            return Err(AppError::new(
                "validation",
                format!("Binance {what} has no positive {code} amount"),
            ));
        }
        Ok(amount)
    };
    let quantity = positive_amount(amount, coin)?;
    let fiat_value = positive_amount(value, fiat)?;
    let external_id = concat(&["binance:convert:".into(), order.clone()]);
    Ok(Some(Event {
        occurred_at: epoch(at(row, "createTime"), true)?,
        raw: row.clone(),
        description: Some("Binance convert".into()),
        pricing: Some(Pricing {
            fiat_currency: fiat.clone(),
            fiat_value,
            fiat_rate: None,
            method: "binance_api",
            external_ref: order,
        }),
        ..Event::one(external_id, Kind::Trade, inbound, coin.clone(), quantity)
    }))
}
