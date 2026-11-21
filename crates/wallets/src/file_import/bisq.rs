//! Bisq 1 trade history as the user's side of each trade's 2-of-2 escrow:
//! one ledger per completed BTC/fiat trade (deposit, trade, payout legs that
//! net to zero), priced from Portfolio > History and dated and paired by the
//! escrow transactions Funds > Transactions names. Anything the exports do not
//! settle is held or refused by trade id. See `docs/exchanges/bisq.md`.
use super::exchange::Columns;
use super::*;
use kassiber_pytext::zone::{self, Zone};
use kassiber_tax_countries::{calendar, Country};

pub mod holds;
use holds::Hold;

/// Canonical column, English header, German header, required.
type Column = (&'static str, &'static str, &'static str, bool);
/// `ClosedTradesView.ColumnNames`.
const TRADES: &[Column] = &[
    ("trade_id", "Trade ID", "Handels-ID", true),
    ("date", "Date/Time", "Datum/Zeit", true),
    ("market", "Market", "Markt", true),
    ("price", "Price", "Preis", true),
    ("deviation", "Deviation", "Abweichung", false),
    ("amount_btc", "Amount in BTC", "Betrag in BTC", true),
    ("volume", "Amount", "Betrag", true),
    ("currency", "Currency", "Währung", false),
    ("tx_fee", "Transaction Fee", "Transaktionsgebühr", false),
    ("trade_fee_btc", "Trade Fee BTC", "Handelsgebühr BTC", false),
    ("trade_fee_bsq", "Trade Fee BSQ", "Handelsgebühr BSQ", false),
    ("buyer_deposit", "Buyer Deposit", "Käufer-Kaution", true),
    (
        "seller_deposit",
        "Seller Deposit",
        "Verkäufer-Kaution",
        true,
    ),
    ("offer_type", "Offer type", "Angebotstyp", true),
    ("status", "Status", "Status", true),
];
/// `TransactionsView`'s exported columns.
const TRANSACTIONS: &[Column] = &[
    ("date", "Date/Time", "Datum/Zeit", true),
    ("trade_id", "Trade ID", "Handels-ID", true),
    ("details", "Details", "Details", true),
    ("txid", "Transaction ID", "Transaktions-ID", true),
    ("amount_btc", "Amount in BTC", "Betrag in BTC", true),
];
/// `portfolio.closed.completed` and `portfolio.closed.canceled`.
const COMPLETED: &[&str] = &["Completed", "Abgeschlossen"];
const CANCELED: &[&str] = &["Canceled", "Abgebrochen"];
/// `shared.buyCurrency` / `shared.sellCurrency` with the base currency BTC.
const BUY: &[&str] = &["Buy BTC", "BTC kaufen"];
const SELL: &[&str] = &["Sell BTC", "BTC verkaufen"];
/// `funds.tx.multiSigDeposit` / `funds.tx.multiSigPayout` (not translated).
const DEPOSIT: &str = "Multisig deposit";
const PAYOUT: &str = "Multisig payout";
const SATS_PER_BTC: i64 = 100_000_000;
const US_PER_SECOND: i64 = 1_000_000;
/// Days from 0001-01-01 to 1970-01-01: `crate::time` counts from the first.
const EPOCH_US: i64 = 719_162 * 86_400 * US_PER_SECOND;
const REEXPORT: &str = "Export the history again unchanged.";
const INEXACT: &str = "Kassiber shapes and prices a Bisq trade only from its exact exported amounts; export the history again unchanged, or record this trade's movements manually.";

/// An escrow leg and the one owned on-chain row its Bisq txid names.
pub(super) struct Pairing {
    pub trade: String,
    pub leg: &'static str,
    pub txid: String,
    pub external_id: String,
    pub onchain: String,
    /// The escrow leg is the outbound side (the payout).
    pub escrow_sends: bool,
    /// The part of the on-chain debit Bisq's exports say paid miner fees.
    pub fee_msat: i64,
}
#[derive(Default)]
pub(super) struct Parsed {
    pub rows: Vec<csv::Row>,
    pub held: Vec<Hold>,
    /// Trades this import shaped; their escrow txids cover any hold's.
    pub completed: Vec<String>,
    pub canceled: usize,
    pub pairings: Vec<Pairing>,
}

/// An owned on-chain observation of an escrow transaction.
struct OnChain {
    id: String,
    occurred_us: i64,
    /// The complete wallet movement: an outbound debit with its fee.
    movement_msat: i64,
}
/// The book the timeline reads: owned on-chain rows by txid, earlier legs,
/// and its tax calendar.
pub(super) struct Book<'a> {
    pub conn: &'a Connection,
    pub profile: &'a str,
    /// The escrow wallet, whose own legs are not on-chain evidence.
    pub escrow: &'a str,
    pub timezone: Option<&'a str>,
}
impl Book<'_> {
    /// Owned on-chain rows of `txid` in `direction` outside the escrow.
    fn owned(&self, txid: &str, direction: &str) -> Result<Vec<OnChain>, AppError> {
        let mut statement = self
            .conn
            .prepare("SELECT id,occurred_at,amount+CASE WHEN direction='outbound' AND COALESCE(amount_includes_fee,0)=0 THEN fee ELSE 0 END FROM transactions WHERE profile_id=? AND excluded=0 AND lower(external_id)=? AND direction=? AND asset='BTC' AND wallet_id!=? ORDER BY id")
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![self.profile, txid, direction, self.escrow], |r| {
                Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?))
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        let observed = |(id, at, movement_msat): (String, String, i64)| {
            let occurred_us = instant(&at)?;
            Ok(OnChain {
                id,
                occurred_us,
                movement_msat,
            })
        };
        rows.into_iter().map(observed).collect()
    }
    /// Another trade whose stored legs name `txid` as their escrow deposit or
    /// payout.
    fn claimed_by_other(&self, trade: &str, txid: &str) -> Result<Option<String>, AppError> {
        self.conn
            .query_row(
                "SELECT substr(external_id,6,length(external_id)-5-length(json_extract(raw_json,'$.bisq_leg'))-1) FROM transactions WHERE profile_id=?1 AND external_id LIKE 'bisq:%' AND substr(external_id,1,length(?2))!=?2 AND (json_extract(raw_json,'$.bisq_deposit_txid')=?3 OR json_extract(raw_json,'$.bisq_payout_txid')=?3) ORDER BY external_id LIMIT 1",
                params![self.profile, format!("bisq:{trade}:"), txid],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)
    }
    /// The legs of `trade` an earlier import stored anywhere in the book, by
    /// leg: wallet, direction, asset, msat, kind, date, contract time, zone,
    /// escrow txids and (trade leg) volume, price and currency.
    fn stored(&self, trade: &str) -> Result<BTreeMap<String, String>, AppError> {
        let mut statement = self
            .conn
            .prepare("SELECT substr(external_id,length(?2)+1),wallet_id||'|'||direction||'|'||asset||'|'||amount||'|'||COALESCE(kind,'')||'|'||occurred_at||'|'||COALESCE(json_extract(raw_json,'$.bisq_contract_time'),'')||'|'||COALESCE(json_extract(raw_json,'$.bisq_timezone'),'')||'|'||COALESCE(json_extract(raw_json,'$.bisq_deposit_txid'),'')||'|'||COALESCE(json_extract(raw_json,'$.bisq_payout_txid'),'')||'|'||CASE WHEN external_id=?2||'trade' THEN COALESCE(fiat_value_exact,'')||'|'||COALESCE(fiat_rate_exact,'')||'|'||COALESCE(fiat_currency,'') ELSE '' END FROM transactions WHERE profile_id=?1 AND external_id IN (?2||'deposit',?2||'trade',?2||'payout')")
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![self.profile, format!("bisq:{trade}:")], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(database_error)?
            .collect::<Result<_, _>>()
            .map_err(database_error);
        rows
    }
    fn country(&self) -> Result<Option<&'static Country>, AppError> {
        let code: Option<String> = self
            .conn
            .query_row(
                "SELECT tax_country FROM profiles WHERE id=?",
                [self.profile],
                |r| r.get(0),
            )
            .optional()
            .map_err(database_error)?;
        Ok(code.and_then(|code| kassiber_tax_countries::lookup(&code)))
    }
}

pub(super) fn read(
    book: &Book<'_>,
    file: &str,
    companion: Option<&str>,
) -> Result<Parsed, AppError> {
    let zone = load_zone(book.timezone)?;
    let trades = csv::rows(&csv::read_sig(file)?)?;
    let Some(companion) = companion.filter(|c| !c.is_empty()) else {
        return Err(AppError::new(
            "validation",
            "A Bisq import needs the Funds > Transactions export as well",
        )
        .with_hint(
            "Export Funds > Transactions to CSV in Bisq and pass it with --transactions next to the Portfolio > History export.",
        ));
    };
    let transactions = csv::rows(&csv::read_sig(companion)?)?;
    parse(book, &zone, trades, transactions)
}

fn missing(message: String) -> AppError {
    AppError::new("validation", message).with_hint(
        "Export Portfolio > History and Funds > Transactions with Bisq set to English or German, and import those files unchanged.",
    )
}
fn refuse(trade: &str, reason: impl std::fmt::Display, hint: &str) -> AppError {
    AppError::new("validation", format!("Bisq trade {trade} {reason}")).with_hint(hint.to_owned())
}

/// The exporting computer's IANA zone: from the system tzdata or, where a
/// computer has none (Windows), the tax-country calendar's rule for it.
fn load_zone(name: Option<&str>) -> Result<Zone, AppError> {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return Err(AppError::new(
            "validation",
            "A Bisq import needs the time zone of the computer that exported it",
        )
        .with_hint(
            "Bisq writes local time without an offset. Pass the exporting computer's IANA zone, for example --timezone Europe/Vienna.",
        ));
    };
    let tzpath = zone::tzpath(std::env::var("PYTHONTZPATH").ok().as_deref());
    load_zone_in(name, &tzpath)
}

/// [`load_zone`] with the zoneinfo directories to search.
fn load_zone_in(name: &str, tzpath: &[std::path::PathBuf]) -> Result<Zone, AppError> {
    if let Ok(zone) = zone::load(name, tzpath) {
        return Ok(zone);
    }
    match calendar::zone(name) {
        _ if name == "UTC" => Ok(Zone::utc(name)),
        Some(rule) if rule.eu_summer_time() => {
            Ok(Zone::eu(name, i64::from(rule.standard_offset_s())))
        }
        // A southern zone keeps its own summer time (Sydney, Auckland).
        Some(rule) if rule.summer_rule().is_some() => {
            let (start, end) = rule.summer_rule().expect("a summer rule");
            let posix = |change: calendar::Change, end: bool| {
                let (month, week, seconds) = change.posix(end);
                (i64::from(month), i64::from(week), i64::from(seconds))
            };
            Ok(Zone::posix_rule(
                name,
                i64::from(rule.standard_offset_s()),
                posix(start, false),
                posix(end, true),
            ))
        }
        Some(rule) => Ok(Zone::pinned(name, i64::from(rule.standard_offset_s()), &[], None)),
        None => Err(AppError::new("validation", format!("Unknown time zone '{name}'")).with_hint(
            "Use an IANA time zone name such as Europe/Vienna or UTC. Without a system time zone database (Windows) only the tax countries' zones are known: for any other, set PYTHONTZPATH to a compiled zoneinfo directory (Git for Windows ships one in usr\\share\\zoneinfo).",
        )),
    }
}

/// Refuse an export without a required column, in English or German.
fn require(rows: &[csv::Row], columns: &[Column], label: &str) -> Result<(), AppError> {
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let has = |name| {
        first
            .headers
            .iter()
            .any(|h| river::column(h) == river::column(name))
    };
    let absent = columns.iter().filter(|c| c.3 && !has(c.1) && !has(c.2));
    let absent = absent.map(|c| c.1).collect::<Vec<_>>();
    if absent.is_empty() {
        return Ok(());
    }
    Err(missing(format!(
        "Bisq {label} CSV is missing required columns: {}",
        absent.join(", ")
    )))
}

struct Row<'a> {
    columns: Columns,
    table: &'static [Column],
    id: &'a str,
}
impl Row<'_> {
    fn get(&self, key: &str) -> Option<String> {
        let column = self
            .table
            .iter()
            .find(|c| c.0 == key)
            .expect("known column");
        self.columns.opt(&[column.1, column.2])
    }
    /// A present, plain decimal amount (`fiat`: and positive), else refuse.
    fn exact(&self, key: &str, label: &str, fiat: bool) -> Result<String, AppError> {
        let text = self.get(key).unwrap_or_default();
        let positive = !fiat || text.bytes().any(|b| (b'1'..=b'9').contains(&b));
        let inexact = || refuse(self.id, format!("has no exact {label} ('{text}')"), INEXACT);
        plain(&text).filter(|_| positive).ok_or_else(inexact)
    }
    /// A present BTC amount in satoshis.
    fn sats(&self, key: &str, label: &str) -> Result<i64, AppError> {
        let text = self.exact(key, label, false)?;
        sats(&text)
            .ok_or_else(|| refuse(self.id, format!("has no exact {label} ('{text}')"), INEXACT))
    }
    /// An optional fee column: absent, blank or a plain decimal.
    fn fee(&self, key: &str) -> Result<Option<String>, AppError> {
        match self.get(key) {
            None => Ok(None),
            Some(text) => plain(&text).map(Some).ok_or_else(|| {
                refuse(self.id, format!("has an unreadable fee '{text}'"), REEXPORT)
            }),
        }
    }
}

/// `digits[.digits]` exactly as bitcoinj's MonetaryFormat writes it.
fn plain(text: &str) -> Option<String> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    (!whole.is_empty() && digits(whole) && digits(fraction) && !text.ends_with('.'))
        .then(|| text.to_owned())
}
/// At most the 21 million BTC that can exist, so every msat product of an
/// amount (`* 1000`, and the debit minus quantity) stays inside i64.
fn sats(text: &str) -> Option<i64> {
    let text = plain(text)?;
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    if fraction.len() > 8 || whole.len() > 8 {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let fraction: i64 = format!("{fraction:0<8}").parse().ok()?;
    Some(whole * SATS_PER_BTC + fraction).filter(|&n| n <= 21_000_000 * SATS_PER_BTC)
}
/// A signed amount (`Amount in BTC` of Funds > Transactions).
fn signed_sats(text: &str) -> Option<i64> {
    match text.strip_prefix('-') {
        Some(rest) => sats(rest).map(|n| -n),
        None => sats(text),
    }
}
fn btc(sats: i64) -> String {
    format!("{}.{:08}", sats / SATS_PER_BTC, sats % SATS_PER_BTC)
}
/// `Utilities.getShortId`: the id the trade history shows for a trade.
fn short_id(id: &str) -> String {
    match id.split_once('-') {
        Some((head, _)) if head.chars().count() <= 8 => head.to_owned(),
        _ => id.chars().take(8).collect(),
    }
}

/// `DisplayUtils.formatDateTime`: the exporting computer's local date and
/// time, in Java's DEFAULT style for the Bisq language. Returned as the naive
/// clock with `Z`, the convention of the other naive exports.
fn when(text: &str) -> Option<String> {
    let normalized = text.replace(['\u{202f}', '\u{a0}'], " ").replace(',', " ");
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let month = |name: &str| -> Option<u32> {
        const MONTHS: [&str; 12] = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ];
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let name = if name == "sept" { "sep" } else { name.as_str() };
        MONTHS.iter().position(|m| *m == name).map(|i| i as u32 + 1)
    };
    let number = |s: &str| -> Option<u32> {
        (!s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let split = |date: &str, separator: char| -> Option<[u32; 3]> {
        let parts = date
            .split(separator)
            .map(number)
            .collect::<Option<Vec<_>>>()?;
        parts.try_into().ok()
    };
    let ((year, mon, day), rest) = match tokens.as_slice() {
        // en_US: Mar 15, 2024 2:30:45 PM
        [m, d, y, rest @ ..] if month(m).is_some() => ((number(y)?, month(m)?, number(d)?), rest),
        // en_GB and other English regions: 15 Mar 2024 14:30:45
        [d, m, y, rest @ ..] if month(m).is_some() => ((number(y)?, month(m)?, number(d)?), rest),
        // de: 15.03.2024 14:30:45
        [date, rest @ ..] if date.contains('.') => {
            let [d, m, y] = split(date, '.')?;
            ((y, m, d), rest)
        }
        // ISO-style locales: 2024-03-15 14:30:45
        [date, rest @ ..] if date.contains('-') => {
            let [y, m, d] = split(date, '-')?;
            ((y, m, d), rest)
        }
        _ => return None,
    };
    let (clock, meridiem) = match rest {
        [clock] => (*clock, None),
        [clock, meridiem] => (*clock, Some(meridiem.replace('.', "").to_ascii_lowercase())),
        _ => return None,
    };
    let parts = clock.split(':').collect::<Vec<_>>();
    let [h, mi, s] = parts.as_slice() else {
        return None;
    };
    let (mut hour, minute, second) = (number(h)?, number(mi)?, number(s)?);
    if year < 1000 || mi.len() != 2 || s.len() != 2 {
        return None;
    }
    match meridiem.as_deref() {
        None => {}
        Some(m @ ("am" | "pm")) if (1..=12).contains(&hour) => {
            hour = hour % 12 + if m == "pm" { 12 } else { 0 };
        }
        Some(_) => return None,
    }
    let iso = format!("{year:04}-{mon:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    crate::time::timestamp(&iso).ok()
}
fn instant(text: &str) -> Result<i64, AppError> {
    crate::time::instant(text)?
        .ok_or_else(|| AppError::new("validation", format!("Invalid timestamp '{text}'")))
}

/// A local Bisq date as the one UTC instant the named zone's clock showed
/// it; a wall time a daylight-saving change skipped or repeated refuses.
fn utc(zone: &Zone, trade: &str, text: &str) -> Result<i64, AppError> {
    let Some(naive) = when(text) else {
        return Err(refuse(
            trade,
            format!("has an unreadable date '{text}'"),
            "Export again with Bisq set to English or German, and pass the time zone the exporting computer used with --timezone.",
        ));
    };
    let local = (instant(&naive)? - EPOCH_US).div_euclid(US_PER_SECOND);
    let mut found = Vec::new();
    for fold in [false, true] {
        if let Some(offset) = zone.local_offset_fold(local, fold) {
            let utc = local - offset;
            if zone.utc_offset(utc) == Some(offset) && !found.contains(&utc) {
                found.push(utc);
            }
        }
    }
    match found.as_slice() {
        [seconds] => Ok(seconds * US_PER_SECOND + EPOCH_US),
        [] => Err(refuse(
            trade,
            format!("has a date '{text}' that {} skipped (daylight saving)", zone.key),
            "Check the time zone passed with --timezone; it must be the exporting computer's.",
        )),
        _ => Err(refuse(
            trade,
            format!(
                "has a date '{text}' that {} showed twice (daylight saving); Kassiber cannot tell which",
                zone.key
            ),
            "Record this trade's movements manually, or check the time zone passed with --timezone.",
        )),
    }
}

/// Whether two instants share a UTC calendar year and the book's tax year
/// in its tax time zone.
fn same_tax_year(a: i64, b: i64, country: Option<&'static Country>) -> bool {
    let utc = |us: i64| crate::time::iso_seconds(us)[..4].to_owned();
    let local = |c: &Country, us: i64| {
        let zone = calendar::zone(c.timezone).unwrap_or(calendar::UTC);
        zone.local_date(us - EPOCH_US)
            .map(|date| c.tax_year(date))
            .ok()
    };
    utc(a) == utc(b) && country.is_none_or(|c| local(c, a).is_some_and(|y| Some(y) == local(c, b)))
}

/// One Funds > Transactions row Bisq labels as a trade's escrow transaction.
#[derive(Clone)]
struct Escrow {
    txid: String,
    /// The UTC instant (µs since 0001-01-01) of Bisq's local date.
    at_us: i64,
    sats: i64,
    raw: Json,
}
#[derive(Default)]
struct Movements {
    /// The full trade id these rows name.
    trade: String,
    deposit: Vec<Escrow>,
    payout: Vec<Escrow>,
}

/// Funds > Transactions: each trade's deposit and payout transactions, by the
/// short id the trade history shows.
fn movements(rows: &[csv::Row], zone: &Zone) -> Result<BTreeMap<String, Movements>, AppError> {
    let mut result = BTreeMap::<String, Movements>::new();
    require(rows, TRANSACTIONS, "transactions")?;
    for row in rows {
        let row = Row {
            columns: Columns::new(row),
            table: TRANSACTIONS,
            id: "",
        };
        let into_deposit = match row.get("details").unwrap_or_default().as_str() {
            DEPOSIT => true,
            PAYOUT => false,
            _ => continue,
        };
        let Some(trade) = row.get("trade_id") else {
            continue;
        };
        let txid = row.get("txid").unwrap_or_default();
        let Some(txid) = hash(&txid) else {
            return Err(missing(format!(
                "Bisq transactions CSV has an unreadable transaction id '{txid}' for trade {trade}"
            )));
        };
        let at_us = utc(zone, &trade, &row.get("date").unwrap_or_default())?;
        let amount = row.get("amount_btc").unwrap_or_default();
        let Some(sats) = signed_sats(&amount) else {
            return Err(missing(format!(
                "Bisq transactions CSV has an unreadable amount '{amount}' for trade {trade}"
            )));
        };
        let short = short_id(&trade);
        let entry = result.entry(short.clone()).or_default();
        if entry.trade.is_empty() {
            entry.trade = trade.clone();
        } else if entry.trade != trade {
            return Err(refuse(
                &short,
                format!(
                    "names two different Bisq trades ({} and {trade})",
                    entry.trade
                ),
                "Two trades share this short id. Import them separately, or record one manually.",
            ));
        }
        let list = if into_deposit {
            &mut entry.deposit
        } else {
            &mut entry.payout
        };
        // The same row twice is one fact; the same txid with another amount
        // or date stays as a second entry, so the trade is held as
        // conflicting rather than read from whichever row came first.
        if !list
            .iter()
            .any(|e| e.txid == txid && e.sats == sats && e.at_us == at_us)
        {
            list.push(Escrow {
                txid,
                at_us,
                sats,
                raw: row.columns.json(),
            });
        }
    }
    Ok(result)
}

/// A conflict between this export and the legs an earlier import stored.
fn conflict(trade: &str, what: &str) -> AppError {
    refuse(
        trade,
        format!("conflicts with its escrow legs already in the book ({what})"),
        "Bisq trade ids identify one trade; an export that changes a trade's amounts, prices or dates is not imported. Check that both exports and --timezone match the earlier import, or roll that import back first (kassiber imports rollback).",
    )
}

fn parse(
    book: &Book<'_>,
    zone: &Zone,
    rows: Vec<csv::Row>,
    transactions: Vec<csv::Row>,
) -> Result<Parsed, AppError> {
    let mut parsed = Parsed::default();
    if rows.is_empty() {
        return Ok(parsed);
    }
    require(&rows, TRADES, "trade history")?;
    let movements = movements(&transactions, zone)?;
    let country = book.country()?;
    let mut seen = BTreeMap::<String, Json>::new();
    // Escrow txid -> the trade of this export that books it.
    let mut claimed = BTreeMap::<String, String>::new();
    for row in &rows {
        let columns = Columns::new(row);
        let raw = columns.json();
        let mut row = Row {
            columns,
            table: TRADES,
            id: "",
        };
        let Some(id) = row.get("trade_id") else {
            return Err(missing(
                "Bisq trade history CSV has a row without a Trade ID".into(),
            ));
        };
        match seen.insert(id.clone(), raw.clone()) {
            Some(previous) if previous == raw => continue,
            Some(_) => {
                return Err(refuse(
                    &id,
                    "appears twice with different values",
                    "Bisq trade ids are unique in one history export; import a single unchanged export.",
                ))
            }
            None => {}
        }
        row.id = &id;
        let status = row.get("status").unwrap_or_default();
        if CANCELED.contains(&status.as_str()) {
            // A canceled offer never reached escrow; any maker fee was paid on
            // chain from the user's wallet and is observed there.
            parsed.canceled += 1;
            continue;
        }
        let market = row.get("market").unwrap_or_default();
        let (base, quote) = market.split_once('/').unwrap_or((&market, ""));
        if base != "BTC" {
            let coin = if base.is_empty() {
                market.as_str()
            } else {
                base
            };
            return Err(refuse(
                &id,
                format!("trades {coin} ({market}): altcoin-market trades are not supported yet"),
                "Filter Portfolio > History to your fiat markets before exporting, and record this trade in the generic ledger with an explicit fiat value.",
            ));
        }
        let fiat = row.get("currency").unwrap_or_else(|| quote.to_owned());
        if !tax_platform::FIAT.contains(&quote) || fiat != quote {
            return Err(refuse(
                &id,
                format!("has market '{market}' (currency '{fiat}'), not Bitcoin against a fiat currency Kassiber knows"),
                "Kassiber imports Bisq trades of Bitcoin against fiat only; record this trade in the generic ledger with an explicit fiat value.",
            ));
        }
        // Whatever this trade turns out to be, its escrow transactions are
        // what a hold must block. A trade already in the book is never held:
        // its legs would book while the hold blocked their transactions.
        let stored = book.stored(&id)?;
        let escrow_txs = movements.get(&id);
        let txids: Vec<String> = escrow_txs
            .map(|m| {
                let mut txids = Vec::new();
                for e in m.deposit.iter().chain(&m.payout) {
                    if !txids.contains(&e.txid) {
                        txids.push(e.txid.clone());
                    }
                }
                txids
            })
            .unwrap_or_default();
        let hold = |reason: &'static str| -> Result<Hold, AppError> {
            if !stored.is_empty() {
                return Err(conflict(
                    &id,
                    &format!("this export would hold it: {reason}"),
                ));
            }
            Ok(Hold {
                trade: id.clone(),
                status: status.clone(),
                reason,
                txids: txids.clone(),
            })
        };
        if !COMPLETED.contains(&status.as_str()) {
            // Mediated or arbitrated payouts differ from the contract and are
            // not in the export; anything else is not a settled trade.
            parsed.held.push(hold("not_completed")?);
            continue;
        }
        let offer = row.get("offer_type").unwrap_or_default();
        let buyer = match offer.as_str() {
            o if BUY.contains(&o) => true,
            o if SELL.contains(&o) => false,
            _ => {
                return Err(refuse(
                    &id,
                    format!("has an unrecognized offer type '{offer}'"),
                    "Export the history again with Bisq set to English or German.",
                ))
            }
        };
        let contract_us = utc(zone, &id, &row.get("date").unwrap_or_default())?;
        let contract = crate::time::iso_seconds(contract_us);
        let amount = row.sats("amount_btc", "BTC amount")?;
        if amount <= 0 {
            return Err(refuse(&id, "has no positive BTC amount", REEXPORT));
        }
        let buyer_deposit = row.sats("buyer_deposit", "buyer security deposit")?;
        let seller_deposit = row.sats("seller_deposit", "seller security deposit")?;
        let volume = row.exact("volume", "fiat amount", true)?;
        let price = row.exact("price", "price", true)?;
        let fees = json!({
            "trade_fee_btc": row.fee("trade_fee_btc")?,
            "trade_fee_bsq": row.fee("trade_fee_bsq")?,
            "transaction_fee_btc": row.fee("tx_fee")?,
        });
        // The user's side of the escrow, netting to zero by construction:
        // what they locked (inbound), traded, and got back (outbound).
        let (deposited, paid_out) = if buyer {
            (buyer_deposit, amount + buyer_deposit)
        } else {
            (amount + seller_deposit, seller_deposit)
        };
        let side = if buyer { "buy" } else { "sell" };
        let traded = if buyer { "inbound" } else { "outbound" };
        let legs: [(&'static str, &str, i64, &str); 3] = [
            ("deposit", "inbound", deposited, "deposit"),
            ("trade", traded, amount, side),
            ("payout", "outbound", paid_out, "withdrawal"),
        ];
        // The timeline comes from the escrow transactions, never the contract.
        let shapes = escrow_txs.map(|m| (m.deposit.as_slice(), m.payout.as_slice()));
        let (deposit, payout) = match shapes {
            None | Some(([], _)) => {
                parsed.held.push(hold("missing_deposit_tx")?);
                continue;
            }
            Some((_, [])) => {
                parsed.held.push(hold("missing_payout_tx")?);
                continue;
            }
            Some(([deposit], [payout])) => (deposit.clone(), payout.clone()),
            Some(_) => {
                parsed.held.push(hold("conflicting_escrow_txs")?);
                continue;
            }
        };
        // An open hold that named another escrow transaction stays until
        // the owner reviews it: this export does not explain that txid.
        let settled = std::collections::BTreeSet::from([deposit.txid.clone(), payout.txid.clone()]);
        // Kassiber's own observations of the escrow transactions: the deposit
        // must be observed (else the legs could be dated too early), and an
        // observed debit or receipt must be exactly what Bisq says.
        let deposit_rows = book.owned(&deposit.txid, "outbound")?;
        let payout_rows = book.owned(&payout.txid, "inbound")?;
        let debit_msat = -deposit.sats * 1000;
        let reason = match (deposit_rows.as_slice(), payout_rows.as_slice()) {
            _ if !holds::anchors(book.conn, book.profile, &id)?.is_subset(&settled) => {
                Some("conflicting_escrow_txs")
            }
            _ if -deposit.sats < deposited => Some("deposit_amount_mismatch"),
            _ if payout.sats != paid_out => Some("payout_amount_mismatch"),
            ([], _) => Some("deposit_not_in_book"),
            // Several owned rows of one escrow transaction do not say which
            // one is Bisq's: never paired, so never booked as settled.
            ([_, _, ..], _) => Some("deposit_debit_mismatch"),
            ([observed], _) if observed.movement_msat != debit_msat => {
                Some("deposit_debit_mismatch")
            }
            (_, [_, _, ..]) => Some("payout_receipt_mismatch"),
            (_, [observed]) if observed.movement_msat != paid_out * 1000 => {
                Some("payout_receipt_mismatch")
            }
            _ => None,
        };
        // The escrow receives no earlier than the wallet spends, the trade
        // follows the deposit by the store's smallest step (one second), and
        // the payout leaves the escrow no later than the wallet received it.
        let deposit_us = deposit_rows
            .iter()
            .map(|r| r.occurred_us)
            .fold(deposit.at_us, i64::max);
        let trade_us = deposit_us + US_PER_SECOND;
        let payout_us = payout_rows
            .iter()
            .map(|r| r.occurred_us)
            .fold(payout.at_us, i64::min);
        let reason = reason
            .or((payout_us <= trade_us).then_some("payout_precedes_deposit"))
            .or((!same_tax_year(contract_us, trade_us, country)).then_some("tax_year_boundary"));
        if let Some(reason) = reason {
            parsed.held.push(hold(reason)?);
            continue;
        }
        // An escrow transaction belongs to one trade: another trade of this
        // export or of the book naming it would book the same movement twice.
        for txid in [&deposit.txid, &payout.txid] {
            let other = match claimed.get(txid) {
                Some(other) => Some(other.clone()),
                None => book.claimed_by_other(&id, txid)?,
            };
            if let Some(other) = other {
                return Err(refuse(
                    &id,
                    format!("names escrow transaction {txid}, which trade {other} also names"),
                    "A Bisq escrow transaction belongs to one trade. Import the exports unchanged, or roll back the import that booked the other trade first (kassiber imports rollback).",
                ));
            }
            claimed.insert(txid.clone(), id.clone());
        }
        let times = [deposit_us, trade_us, payout_us].map(crate::time::iso_seconds);
        // A trade already in the book must match this export exactly: changed
        // amounts or prices refuse, and so do moved escrow dates or another
        // time zone, rather than redate legs a review may rest on.
        for (index, (leg, direction, quantity, kind)) in legs.iter().enumerate() {
            let pricing = if *leg == "trade" {
                format!("{volume}|{price}|{fiat}")
            } else {
                String::new()
            };
            let facts = format!(
                "{}|{direction}|BTC|{}|{kind}|{}|{contract}|{}|{}|{}|{pricing}",
                book.escrow,
                quantity * 1000,
                times[index],
                zone.key,
                deposit.txid,
                payout.txid
            );
            if !stored.is_empty() && stored.get(*leg) != (*quantity != 0).then_some(&facts) {
                let what = format!(
                    "its {leg} leg differs in amount, price, wallet, date, time zone or escrow transaction"
                );
                return Err(conflict(&id, &what));
            }
        }
        for (index, (leg, direction, quantity, kind)) in legs.into_iter().enumerate() {
            if quantity == 0 {
                continue;
            }
            let escrow = match leg {
                "deposit" => Some(&deposit),
                "payout" => Some(&payout),
                _ => None,
            };
            let mut stored_raw = raw.clone();
            stored_raw["bisq_leg"] = json!(leg);
            stored_raw["bisq_fees"] = fees.clone();
            stored_raw["bisq_contract_time"] = json!(contract);
            stored_raw["bisq_timezone"] = json!(zone.key);
            stored_raw["bisq_deposit_txid"] = json!(deposit.txid);
            stored_raw["bisq_payout_txid"] = json!(payout.txid);
            if let Some(escrow) = escrow {
                stored_raw["bisq_transaction"] = escrow.raw.clone();
            }
            let external_id = format!("bisq:{id}:{leg}");
            let mut data = json!({
                "txid": external_id,
                "occurred_at": times[index],
                "direction": direction,
                "asset": "BTC",
                "amount": btc(quantity),
                "fee": "0",
                "counterparty": "Bisq",
                "kind": kind,
                "raw_json": config::stored_json(&stored_raw),
            });
            if let Some(escrow) = escrow {
                data["description"] = json!(format!("Bisq {side} trade {id}: escrow {leg}"));
                let (owned, escrow_sends, fee_msat) = if leg == "deposit" {
                    (&deposit_rows, false, debit_msat - quantity * 1000)
                } else {
                    (&payout_rows, true, 0)
                };
                if let [onchain] = owned.as_slice() {
                    parsed.pairings.push(Pairing {
                        trade: id.clone(),
                        leg,
                        txid: escrow.txid.clone(),
                        external_id: external_id.clone(),
                        onchain: onchain.id.clone(),
                        escrow_sends,
                        fee_msat,
                    });
                }
            } else {
                let description = format!(
                    "Bisq {side} {} BTC for {volume} {fiat} (trade {id})",
                    btc(amount)
                );
                for (key, value) in [
                    ("fiat_rate", price.as_str()),
                    ("fiat_value", volume.as_str()),
                    ("fiat_currency", fiat.as_str()),
                    ("pricing_source_kind", "exchange_execution"),
                    ("pricing_provider", "Bisq"),
                    ("pricing_pair", &format!("BTC-{fiat}")),
                    ("pricing_timestamp", contract.as_str()),
                    ("pricing_method", "bisq_csv"),
                    ("pricing_external_ref", id.as_str()),
                    ("pricing_quality", "exact"),
                    ("description", description.as_str()),
                ] {
                    data[key] = json!(value);
                }
            }
            parsed.rows.push(csv::Row {
                data,
                headers: vec![],
                rest: None,
                original: None,
            });
        }
        parsed.completed.push(id.clone());
    }
    Ok(parsed)
}

/// The import response's Bisq fields, and whether journal inputs changed.
type Finished = (Vec<(&'static str, Py)>, bool);

/// After the legs are stored: record the holds, and pair each escrow leg
/// with the one owned row its txid names through the shared evidenced pair
/// authoring. Returns the import response's Bisq fields and whether journal
/// inputs changed.
pub(super) fn finish(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    parsed: &Parsed,
) -> Result<Finished, AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [profile],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let now = lifecycle::now(conn)?;
    let mut changed = holds::persist(conn, &workspace, profile, parsed, &now)?;
    let scope = kassiber_transfers::Scope {
        workspace_id: &workspace,
        profile_id: profile,
    };
    let (mut created, mut skipped) = (Vec::new(), Vec::new());
    for pairing in &parsed.pairings {
        let escrow: String = conn
            .query_row(
                "SELECT id FROM transactions WHERE wallet_id=? AND external_id=?",
                [wallet, &pairing.external_id],
                |r| r.get(0),
            )
            .map_err(database_error)?;
        let (out, inbound) = if pairing.escrow_sends {
            (&escrow, &pairing.onchain)
        } else {
            (&pairing.onchain, &escrow)
        };
        let describe = |extra: (&str, Py)| {
            dict([
                ("trade_id", s(pairing.trade.as_str())),
                ("leg", s(pairing.leg)),
                ("txid", s(pairing.txid.as_str())),
                extra,
            ])
        };
        let notes = format!(
            "Bisq trade {} escrow {}: Bisq's Funds > Transactions export names txid {}; miner fee {} msat",
            pairing.trade, pairing.leg, pairing.txid, pairing.fee_msat
        );
        let fee = i128::from(pairing.fee_msat);
        let system = &kassiber_transfers::SystemSources;
        match kassiber_transfers::create_evidenced(conn, &scope, out, inbound, fee, &notes, system)?
        {
            kassiber_transfers::Evidenced::Created(pair) => {
                changed = true;
                let id = kassiber_envelope::get(&pair, "id").cloned();
                created.push(describe(("pair_id", id.unwrap_or(Py::Null))));
            }
            kassiber_transfers::Evidenced::Declined(reason) => {
                skipped.push(describe(("reason", s(reason))));
            }
        }
    }
    if changed {
        invalidate_journals(conn, profile)?;
    }
    let held = parsed.held.iter().map(|held| {
        dict([
            ("trade_id", s(held.trade.as_str())),
            ("status", s(held.status.as_str())),
            ("reason", s(held.reason)),
        ])
    });
    let summary = vec![
        ("bisq_review_required", Py::List(held.collect())),
        ("bisq_canceled_skipped", Py::from(parsed.canceled as i64)),
        ("bisq_pairs_created", Py::List(created)),
        ("bisq_pairs_skipped", Py::List(skipped)),
    ];
    Ok((summary, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without tzdata (Windows) a southern registry zone keeps its own
    /// summer time: Sydney is UTC+11 in January, and its repeated autumn
    /// hour is refused as ambiguous rather than read at one offset.
    #[test]
    fn southern_zones_keep_their_summer_time_without_tzdata() {
        let sydney = load_zone_in("Australia/Sydney", &[]).unwrap();
        assert_eq!(
            utc(&sydney, "t", "Jan 15, 2025 12:00:00 PM").unwrap(),
            utc(&Zone::utc("UTC"), "t", "Jan 15, 2025 1:00:00 AM").unwrap()
        );
        assert_eq!(
            utc(&sydney, "t", "Jul 15, 2025 12:00:00 PM").unwrap(),
            utc(&Zone::utc("UTC"), "t", "Jul 15, 2025 2:00:00 AM").unwrap()
        );
        let repeated = utc(&sydney, "t", "Apr 6, 2025 2:30:00 AM").unwrap_err();
        assert!(
            repeated.message.contains("showed twice"),
            "{}",
            repeated.message
        );
    }

    #[test]
    fn reads_bisq_dates_amounts_ids_and_tax_years_exactly() {
        for (text, expected) in [
            ("Mar 15, 2024 2:30:45 PM", Some("2024-03-15T14:30:45Z")),
            (
                "Mar 15, 2024 2:30:45\u{202f}PM",
                Some("2024-03-15T14:30:45Z"),
            ),
            ("Jan 1, 2024 12:05:00 AM", Some("2024-01-01T00:05:00Z")),
            ("15.03.2024 14:30:45", Some("2024-03-15T14:30:45Z")),
            ("15 Sept 2024 09:01:02", Some("2024-09-15T09:01:02Z")),
            ("2024-03-15 14:30:45", Some("2024-03-15T14:30:45Z")),
            ("Mar 15, 2024", None),
            ("Mar 32, 2024 2:30:45 PM", None),
            ("Mar 15, 2024 13:30:45 PM", None),
            ("15/03/2024 14:30:45", None),
        ] {
            assert_eq!(when(text).as_deref(), expected, "{text}");
        }
        assert_eq!(sats("0.025"), Some(2_500_000));
        assert_eq!(sats("0.00000001"), Some(1));
        assert_eq!(sats("21000000"), Some(21_000_000 * SATS_PER_BTC));
        for text in [
            "",
            "0.000000001",
            "0,025",
            "-0.1",
            "1.",
            ".5",
            "1e-3",
            "N/A",
            "21000000.00000001",
            "99999999.99999999",
        ] {
            assert_eq!(sats(text), None, "{text}");
        }
        assert_eq!(signed_sats("-0.0061"), Some(-610_000));
        assert_eq!(btc(2_500_001), "0.02500001");
        assert_eq!(
            short_id("a1b2c3d4-0f1e-4b2a-9c3d-112233445566-1.9.9"),
            "a1b2c3d4"
        );
        assert_eq!(short_id("abcdefghij-x"), "abcdefgh");
        let at = |text: &str| crate::time::instant(text).unwrap().unwrap();
        let austria = kassiber_tax_countries::lookup("at");
        // Same UTC year, but Vienna has already reached the next year.
        assert!(same_tax_year(
            at("2024-12-31T20:00:00Z"),
            at("2024-12-31T22:30:00Z"),
            austria
        ));
        assert!(!same_tax_year(
            at("2024-12-31T20:00:00Z"),
            at("2024-12-31T23:30:00Z"),
            austria
        ));
        assert!(!same_tax_year(
            at("2024-12-31T23:59:00Z"),
            at("2025-01-01T00:00:01Z"),
            None
        ));
        // The United Kingdom's tax year starts on 6 April.
        let britain = kassiber_tax_countries::lookup("gb");
        assert!(!same_tax_year(
            at("2024-04-05T20:00:00Z"),
            at("2024-04-06T08:00:00Z"),
            britain
        ));
    }
}
