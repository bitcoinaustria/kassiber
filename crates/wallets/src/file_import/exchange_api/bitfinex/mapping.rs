//! Bitfinex rows to records, shared by the API sync and the Reports CSV
//! import so both write the same records and ids. Trades and movements make
//! the records. Every BTC ledger entry must be a known description in the
//! exchange wallet that pairs one-to-one with the trade, fee or movement it
//! books; anything else refuses the import by name, so a trade can never
//! slip through as a deposit or withdrawal. docs/reference/bitfinex.md.
use super::super::*;

/// Products and sums of exact inputs are not rounded; the process context
/// (32 digits) would round a long amount × price.
const EXACT: Context = Context {
    prec: 1000,
    rounding: Rounding::HalfEven,
};
/// The fiat quotes Bitfinex lists BTC against. Stablecoins are not fiat.
const FIAT: [&str; 4] = ["EUR", "GBP", "JPY", "USD"];
/// A ledger entry and the trade it books are at most this far apart (s).
const TRADE_WINDOW: i64 = 60;
/// A withdrawal fee entry lies between a day before the withdrawal started
/// and a day after it completed.
const FEE_WINDOW: i64 = 86_400;
const REVIEW: &str = "Kassiber imports only BTC spot trades against USD, EUR, GBP or JPY and BTC deposits and withdrawals of the Bitfinex exchange wallet (no margin, funding, derivatives or other coins); record this entry yourself (generic ledger import) after checking it in Bitfinex.";
const COMPLETE: &str = "Import the Bitfinex Reports trades, movements and ledgers exports together (one Reports zip), or sync with a read-only API key.";

/// One trade (`/v2/auth/r/trades/hist` or a Reports trades row).
pub(in crate::file_import::exchange_api) struct Trade {
    pub id: String,
    pub symbol: String,
    pub at: PyStr,
    /// Signed (positive bought the base currency), as the fee (negative charges).
    pub amount: Decimal,
    pub price: Decimal,
    pub order_type: Option<String>,
    pub fee: Decimal,
    pub fee_currency: String,
    pub raw: Py,
}
/// One deposit or withdrawal (API or Reports movements row).
pub(in crate::file_import::exchange_api) struct Movement {
    pub id: String,
    pub currency: String,
    pub rail: Option<String>,
    pub started: PyStr,
    pub at: PyStr,
    pub status: String,
    /// Signed: positive deposits; the fees are negative.
    pub amount: Decimal,
    pub fees: Decimal,
    pub transaction: String,
    pub raw: Py,
}
/// One ledger entry (API or Reports ledgers row).
pub(in crate::file_import::exchange_api) struct LedgerEntry {
    pub id: String,
    pub currency: String,
    pub wallet: String,
    pub at: PyStr,
    pub amount: Decimal,
    pub description: String,
    pub raw: Py,
}
/// `None` is an export a CSV import lacks; the API supplies all three.
pub(in crate::file_import::exchange_api) struct Sources<'a> {
    pub ledger: Option<&'a [LedgerEntry]>,
    pub trades: Option<&'a [Trade]>,
    pub movements: Option<&'a [Movement]>,
    /// The CSV has no order type: a trade is spot only when an
    /// exchange-wallet ledger entry books it. Its ledgers export also ends at
    /// its last row, where the API's ledger read reaches the present.
    pub spot_proof_required: bool,
}

/// Counts a row in assets other than BTC: Kassiber reads only BTC from
/// Bitfinex, in either scope, so the contract counts them as `unread`.
fn leave(mapper: &mut Mapper, codes: [&str; 2]) {
    let codes: Vec<&str> = codes
        .into_iter()
        .map(str::trim)
        .filter(|code| !code.is_empty() && !code.ends_with("F0"))
        .collect();
    mapper.admit(&codes);
}

/// A row of either source: a field is an API array index and a CSV column.
pub(in crate::file_import::exchange_api) trait Fields {
    fn id(&self) -> Result<String, AppError>;
    fn text(&self, field: (usize, &str)) -> String;
    fn number(&self, field: (usize, &str), optional: bool) -> Result<Decimal, AppError>;
    fn time(&self, field: (usize, &str)) -> Result<PyStr, AppError>;
    /// A field only the API has (order type, rail).
    fn api_only(&self, at: usize) -> Option<String>;
    fn raw(&self) -> Py;
}
/// `[ID, SYMBOL, MTS, _, EXEC_AMOUNT, EXEC_PRICE, ORDER_TYPE, _, _, FEE, FEE_CURRENCY]`.
pub(in crate::file_import::exchange_api) fn read_trade(f: &impl Fields) -> Result<Trade, AppError> {
    Ok(Trade {
        id: f.id()?,
        symbol: f.text((1, "PAIR")),
        at: f.time((2, "DATE"))?,
        amount: f.number((4, "AMOUNT"), false)?,
        price: f.number((5, "PRICE"), false)?,
        order_type: f.api_only(6),
        fee: f.number((9, "FEE"), true)?,
        fee_currency: f.text((10, "FEE CURRENCY")),
        raw: f.raw(),
    })
}
/// `[ID, CURRENCY, CURRENCY_NAME, _, _, MTS_STARTED, MTS_UPDATED, _, _,
/// STATUS, _, _, AMOUNT, FEES, ..., TRANSACTION_ID (20)]`.
pub(in crate::file_import::exchange_api) fn read_movement(
    f: &impl Fields,
) -> Result<Movement, AppError> {
    Ok(Movement {
        id: f.id()?,
        currency: f.text((1, "CURRENCY")),
        rail: f.api_only(2),
        started: f.time((5, "STARTED"))?,
        at: f.time((6, "UPDATED"))?,
        status: f.text((9, "STATUS")),
        amount: f.number((12, "AMOUNT"), false)?,
        fees: f.number((13, "FEES"), true)?,
        transaction: f.text((20, "TRANSACTION ID")),
        raw: f.raw(),
    })
}
/// `[ID, CURRENCY, WALLET, MTS, _, AMOUNT, BALANCE, _, DESCRIPTION]`.
pub(in crate::file_import::exchange_api) fn read_ledger(
    f: &impl Fields,
) -> Result<LedgerEntry, AppError> {
    Ok(LedgerEntry {
        id: f.id()?,
        currency: f.text((1, "CURRENCY")),
        wallet: f.text((2, "WALLET")),
        at: f.time((3, "DATE"))?,
        amount: f.number((5, "AMOUNT"), false)?,
        description: f.text((8, "DESCRIPTION")),
        raw: f.raw(),
    })
}

fn seconds(at: &PyStr) -> i64 {
    let parsed = at.as_str().and_then(kassiber_pytext::iso_datetime);
    parsed.map_or(0, |p| p.utc_us().div_euclid(1_000_000))
}
fn whole_msat(value: &Decimal) -> bool {
    exact_msat(&value.copy_abs()).is_some()
}
fn shown(text: &str) -> String {
    text.trim().chars().take(80).collect()
}
/// `Bitfinex transaction '<what> <id>' (type '<kind>') <reason>`.
fn refuse(what: &str, id: &str, kind: &str, reason: impl std::fmt::Display) -> AppError {
    let row = format!("{what} {id}");
    refusal("Bitfinex", &row, &shown(kind), reason, REVIEW)
}
fn bad(what: &str, id: &str, problem: &str) -> AppError {
    malformed("Bitfinex", &format!("{what} {id}"), problem)
}
/// BTC against `other` when it is not a fiat quote; `None` otherwise.
fn not_fiat(base: &str, quote: &str) -> Option<String> {
    let fiat = base == "BTC" && FIAT.contains(&quote);
    let other = if base == "BTC" { quote } else { base };
    (!fiat && (base == "BTC" || quote == "BTC"))
        .then(|| format!("against {}, not fiat", shown(other)))
}

/// `BTC/USD`, `tBTCUSD` or `tBTCF0:USTF0` as base and quote.
fn base_quote(symbol: &str) -> Option<(String, String)> {
    let symbol = symbol.trim();
    let (base, quote) = match symbol.strip_prefix('t') {
        _ if symbol.contains('/') => symbol.split_once('/')?,
        Some(pair) if pair.contains(':') => pair.split_once(':')?,
        Some(pair) if pair.len() == 6 && pair.is_ascii() => pair.split_at(3),
        _ => return None,
    };
    Some((base.to_owned(), quote.to_owned()))
}

/// An accepted BTC trade and the ledger entries that book it and its fee.
struct Booked<'a> {
    row: &'a Trade,
    quote: String,
    btc_fee: Option<Decimal>,
    entry: [Option<String>; 2],
}

/// A BTC trade against fiat to a priced buy or sell; `None` without BTC.
fn trade(row: &Trade) -> Result<Option<(Event, Booked<'_>)>, AppError> {
    let refused = |reason: String| refuse("trade", &row.id, &row.symbol, reason);
    let Some((base, quote)) = base_quote(&row.symbol) else {
        let what = format!("a pair Kassiber cannot read ('{}')", shown(&row.symbol));
        return Err(bad("trade", &row.id, &what));
    };
    let derivative = [&base, &quote].map(|c| c.strip_suffix("F0"));
    if derivative.iter().any(Option::is_some) {
        return match derivative.contains(&Some("BTC")) {
            true => Err(refused("is a derivatives trade".into())),
            false => Ok(None),
        };
    }
    if base != "BTC" && quote != "BTC" {
        return Ok(None);
    }
    if let Some(other) = not_fiat(&base, &quote) {
        return Err(refused(format!("trades BTC {other}")));
    }
    let order = row.order_type.as_deref().map(str::trim);
    if let Some(order) = order.filter(|o| !o.to_ascii_uppercase().starts_with("EXCHANGE ")) {
        return Err(refused(format!(
            "is a margin trade (order type '{}')",
            shown(order)
        )));
    }
    if row.amount.is_zero() || !whole_msat(&row.amount) || !row.price.is_positive() {
        let what = "a zero, sub-millisatoshi or unpriced BTC amount";
        return Err(bad("trade", &row.id, what));
    }
    let (fee, currency) = (row.fee.copy_abs(), row.fee_currency.trim());
    let fee_in_btc = !fee.is_zero() && currency == "BTC";
    if row.fee.is_positive() {
        return Err(refused("credits a fee rebate".into()));
    }
    if !fee.is_zero() && !fee_in_btc && currency != quote {
        let currency = shown(currency);
        return Err(refused(format!(
            "charges its fee in {currency}, neither BTC nor {quote}"
        )));
    }
    if fee_in_btc && !whole_msat(&fee) {
        return Err(bad("trade", &row.id, "a BTC fee finer than a millisatoshi"));
    }
    let (inbound, quantity) = (row.amount.is_positive(), row.amount.copy_abs());
    let gross = quantity.mul(&row.price, &EXACT).map_err(decimal_error)?;
    // Bitfinex books the executed amount and charges the fee as its own
    // ledger entry: a buy's cost includes a fiat fee and a BTC fee leaves
    // fewer coins for the same cost; a sale's proceeds lose a fiat fee, and
    // a BTC fee leaves the account besides the coins sold.
    let zero = Decimal::zero();
    let (amount, btc_fee, value) = match (inbound, fee_in_btc) {
        (true, true) => (quantity.sub(&fee, &EXACT), zero, Ok(gross)),
        (true, false) => (Ok(quantity), zero, gross.add(&fee, &EXACT)),
        (false, true) => (Ok(quantity), fee.clone(), Ok(gross)),
        (false, false) => (Ok(quantity), zero, gross.sub(&fee, &EXACT)),
    };
    let amount = amount.map_err(decimal_error)?;
    let value = value.map_err(decimal_error)?;
    if !amount.is_positive() || !value.is_positive() {
        return Err(refused("leaves nothing after its fee".into()));
    }
    let record = Event::from_entry(Entry {
        provider: "Bitfinex",
        external_id: format!("bitfinex:trade:{}", row.id).into(),
        occurred_at: Some(row.at.clone()),
        inbound,
        asset: "BTC".into(),
        amount: PriceNumber::Finite(amount),
        fee: PriceNumber::Finite(btc_fee),
        kind: if inbound { "buy" } else { "sell" },
        raw: dict([("trade", row.raw.clone())]),
        description: None,
        pricing: Some(Pricing {
            fiat_currency: quote.clone(),
            fiat_value: canonical(&value)?,
            fiat_rate: None,
            method: "bitfinex_trade",
            external_ref: row.id.clone().into(),
        }),
    });
    let btc_fee = fee_in_btc.then_some(fee);
    Ok(Some((
        record,
        Booked {
            row,
            quote,
            btc_fee,
            entry: [None, None],
        },
    )))
}

/// A completed BTC deposit or withdrawal, by Bitfinex's movement id; the
/// on-chain txid (lowercased) travels in `raw_json` for transfer pairing.
/// `unchecked`: a withdrawal fee the ledger could not cover, as the movement
/// states it ([`UNCHECKED`]).
fn movement(row: &Movement, unchecked: bool) -> Result<Event, AppError> {
    let inbound = row.amount.is_positive();
    let kind = if inbound { "deposit" } else { "withdrawal" };
    let exact = !row.amount.is_zero() && whole_msat(&row.amount) && whole_msat(&row.fees);
    if !exact || row.fees.is_positive() {
        let what = "a zero or sub-millisatoshi amount, or a fee that is not a charge";
        return Err(bad("movement", &row.id, what));
    }
    if inbound && !row.fees.is_zero() {
        return Err(refuse("movement", &row.id, kind, "charges a deposit fee"));
    }
    let mut raw = vec![("movement", row.raw.clone())];
    let hash = row.transaction.trim();
    if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        raw.push(("txid", s(hash.to_ascii_lowercase())));
    }
    Ok(Event::from_entry(Entry {
        provider: "Bitfinex",
        external_id: format!("bitfinex:movement:{}", row.id).into(),
        occurred_at: Some(row.at.clone()),
        inbound,
        asset: "BTC".into(),
        amount: PriceNumber::Finite(row.amount.copy_abs()),
        fee: PriceNumber::Finite(row.fees.copy_abs()),
        kind,
        raw: dict(raw),
        description: Some(match unchecked {
            true => UNCHECKED.into(),
            false => format!("Bitfinex {kind}").into(),
        }),
        pricing: None,
    }))
}

/// A margin funding payment: BTC interest credited to the funding wallet.
fn interest(row: &LedgerEntry) -> Result<Event, AppError> {
    Ok(Event::from_entry(Entry {
        provider: "Bitfinex",
        external_id: format!("bitfinex:ledger:{}", row.id).into(),
        occurred_at: Some(row.at.clone()),
        inbound: true,
        asset: "BTC".into(),
        amount: PriceNumber::Finite(row.amount.clone()),
        fee: PriceNumber::Finite(Decimal::zero()),
        kind: "lending_interest",
        raw: dict([("ledger", row.raw.clone())]),
        description: Some("Bitfinex margin funding payment".into()),
        pricing: None,
    }))
}

/// The description, and review tag, of a withdrawal whose fee comes from
/// the movement alone: the ledger read does not reach its date.
const UNCHECKED: &str =
    "Bitfinex withdrawal (fee from movements, not cross-checked against the ledger)";
pub(in crate::file_import::exchange_api) const UNCHECKED_TAG: &str = "bitfinex-fee-unchecked";
/// Tags the stored rows of `rows` that carry [`UNCHECKED`].
pub(in crate::file_import::exchange_api) fn tag_unchecked(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    rows: &[csv::Row],
    ids: &mut Ids<'_>,
) -> Result<(), AppError> {
    let unchecked = rows.iter().filter(|r| r.data["description"] == UNCHECKED);
    let external: Vec<&str> = unchecked.filter_map(|r| r.data["txid"].as_str()).collect();
    if external.is_empty() {
        return Ok(());
    }
    let find = "SELECT id FROM tags WHERE profile_id=? AND code=?";
    let found = conn.query_row(find, [profile, UNCHECKED_TAG], |r| r.get(0));
    let tag: String = match found.optional().map_err(database_error)? {
        Some(tag) => tag,
        None => {
            let (id, at) = (ids.next(conn)?, ids.time(conn)?);
            let add = "INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) SELECT ?,workspace_id,id,?,?,? FROM profiles WHERE id=?";
            let label = "Bitfinex fee from movements, not cross-checked against the ledger";
            let values = params![id, UNCHECKED_TAG, label, at, profile];
            conn.execute(add, values).map_err(database_error)?;
            id
        }
    };
    let link = "INSERT OR IGNORE INTO transaction_tags(transaction_id,tag_id) SELECT id,? FROM transactions WHERE wallet_id=? AND external_id=?";
    for external_id in external {
        conn.execute(link, [tag.as_str(), wallet, external_id])
            .map_err(database_error)?;
    }
    Ok(())
}

/// The ledger descriptions Kassiber knows, each a fixed grammar (lowercased,
/// single spaces) ending `on wallet <the entry's own wallet>`.
#[derive(Debug, PartialEq)]
pub(in crate::file_import::exchange_api) enum Described {
    /// `(fee, base, quote, amount, price)` of `Exchange <amount> <BASE> for
    /// <QUOTE> @ <price>`, or of `Trading fees for <amount> <BASE>
    /// (<BASE><QUOTE>) @ <price> ...` when `fee`.
    Trade(bool, String, String, Decimal, Decimal),
    /// `Deposit [(<NAME>)] #<id>`, `<Name> Withdrawal #<id>`, `Cancel... #<id>`.
    Movement(String),
    /// `<Name> Withdrawal [express] fee`.
    WithdrawalFee,
    /// `Margin Funding Payment`.
    FundingPayment,
    /// `(amount, from, to)` of `Transfer of <amount> BTC from wallet <A> to <B>`.
    Transfer(Decimal, String, String),
    Unknown,
}
pub(in crate::file_import::exchange_api) fn describe(description: &str, wallet: &str) -> Described {
    use Described::Unknown;
    let lowered = kassiber_pytext::lower(description);
    let tokens: Vec<&str> = lowered.split_whitespace().collect();
    let at = tokens.len().saturating_sub(3);
    if tokens[at..] != ["on", "wallet", wallet] {
        return Unknown;
    }
    let id = |token: &str| {
        let id = token.strip_prefix('#').filter(|id| !id.is_empty())?;
        let digits = id.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| Described::Movement(id.to_owned()))
    };
    let traded = |fee, amount: &str, base: &str, quote: Option<&str>, price: &str| {
        let numbers = (strict_decimal(amount, true), strict_decimal(price, true));
        let (base, quote) = (
            base.to_ascii_uppercase(),
            quote.unwrap_or("").trim_start_matches(':'),
        );
        match numbers {
            (Some(amount), Some(price)) if !quote.is_empty() => {
                Described::Trade(fee, base, quote.to_ascii_uppercase(), amount, price)
            }
            _ => Unknown,
        }
    };
    match &tokens[..at] {
        ["exchange", amount, base, "for", quote, "@", price] => {
            traded(false, amount, base, Some(quote), price)
        }
        ["trading", "fees", "for", amount, base, pair, "@", price, ..] => {
            let quote = pair.strip_prefix('(').and_then(|p| p.strip_suffix(')'));
            traded(
                true,
                amount,
                base,
                quote.and_then(|p| p.strip_prefix(*base)),
                price,
            )
        }
        [first, .., last] if first.starts_with("cancel") => id(last).unwrap_or(Unknown),
        ["deposit", name @ .., last]
            if name.first().is_none_or(|n| n.starts_with('('))
                && name.last().is_none_or(|n| n.ends_with(')')) =>
        {
            id(last).unwrap_or(Unknown)
        }
        [.., "withdrawal", "fee"] | [.., "withdrawal", "express", "fee"] => {
            Described::WithdrawalFee
        }
        [.., "withdrawal", last] => id(last).unwrap_or(Unknown),
        ["margin", "funding", "payment"] => Described::FundingPayment,
        ["transfer", "of", amount, "btc", "from", "wallet", from, "to", to] => {
            match strict_decimal(amount, true) {
                Some(amount) => Described::Transfer(amount, (*from).into(), (*to).into()),
                None => Unknown,
            }
        }
        _ => Unknown,
    }
}

/// A source's rows with each Bitfinex id once: an identical repeat (the
/// same export twice, a page boundary) collapses, another refuses.
fn unique<'a, T>(rows: Option<&'a [T]>, what: &str, key: fn(&T) -> (&str, &Py)) -> Rows<'a, T> {
    let mut seen: BTreeMap<&str, &Py> = BTreeMap::new();
    let mut kept = vec![];
    for row in rows.unwrap_or(&[]) {
        let (id, raw) = key(row);
        match seen.insert(id, raw) {
            Some(first) if first != raw => return Err(bad(what, id, "two different contents")),
            Some(_) => {}
            None => kept.push(row),
        }
    }
    Ok(kept)
}
type Rows<'a, T> = Result<Vec<&'a T>, AppError>;

type Slot<'a> = (&'a str, &'a mut Option<String>);
/// A transfer leg: the entry, its (amount, from, to) and the leg it pairs with.
type Leg<'a> = (&'a LedgerEntry, (Decimal, String, String), Option<String>);
/// Books `entry` into the one slot `fits` offers: no fit, several fits, or
/// a slot another entry already holds refuses the import.
fn book(entry: &LedgerEntry, fits: Vec<Slot>, none: &str) -> Result<(), AppError> {
    let refused = |reason: String| refuse("ledger entry", &entry.id, &entry.description, reason);
    let ids: Vec<&str> = fits.iter().map(|(id, _)| *id).collect();
    let [(_, slot)] = <[_; 1]>::try_from(fits).map_err(|fits| match fits.len() {
        0 => refused(none.into()),
        n => refused(format!(
            "fits {n} ({}); Kassiber cannot tell which",
            ids.join(", ")
        )),
    })?;
    match slot.replace(entry.id.clone()) {
        Some(other) => Err(refused(format!("books what ledger entry {other} books"))),
        None => Ok(()),
    }
}

/// Every source's events after the ledger cross-check, oldest first, to the
/// mapper, with what was skipped, by reason; the other assets are counted.
pub(in crate::file_import::exchange_api) fn map(
    sources: &Sources<'_>,
    mapper: &mut Mapper,
) -> Result<(), AppError> {
    let (mut not_btc, mut canceled, mut unfinished) = (0i64, 0i64, 0i64);
    let mut records: Vec<(PyStr, Event)> = vec![];
    let mut booked: Vec<Booked> = vec![];
    for row in unique(sources.trades, "trade", |r| (&r.id, &r.raw))? {
        match trade(row)? {
            Some((record, kept)) => {
                records.push((row.at.clone(), record));
                booked.push(kept);
            }
            None => {
                let (base, quote) = base_quote(&row.symbol).unwrap_or_default();
                leave(mapper, [&base, &quote]);
                not_btc += 1;
            }
        }
    }
    // BTC movements by id (every status) and the completed ones with the
    // fee entry the ledger books for them.
    let mut movements: BTreeMap<&str, &Movement> = BTreeMap::new();
    let mut completed: Vec<(&Movement, Option<String>)> = vec![];
    for row in unique(sources.movements, "movement", |r| (&r.id, &r.raw))? {
        if row.currency.trim() != "BTC" {
            leave(mapper, [&row.currency, ""]);
            not_btc += 1;
            continue;
        }
        let rail = row.rail.as_deref().map(str::trim);
        if let Some(rail) = rail.filter(|r| !r.eq_ignore_ascii_case("BITCOIN")) {
            let reason = format!("moves BTC over '{}', not the Bitcoin network", shown(rail));
            return Err(refuse("movement", &row.id, "movement", reason));
        }
        match row.status.trim().to_ascii_uppercase().as_str() {
            "COMPLETED" => completed.push((row, None)),
            "CANCELED" | "CANCELLED" => canceled += 1,
            // The next sync imports it once Bitfinex completes it.
            _ => unfinished += 1,
        }
        movements.insert(&row.id, row);
    }
    // Per movement id: the sum its entries book and the first entry.
    let mut evidence: BTreeMap<String, (Decimal, &str)> = BTreeMap::new();
    // Transfer legs between the exchange and funding wallets: the entry, its
    // (amount, from, to) and, for an incoming leg, the outgoing one it pairs.
    let mut legs: Vec<Leg> = vec![];
    // The first and last date the ledger read covers.
    let times = sources.ledger.unwrap_or(&[]).iter().map(|r| seconds(&r.at));
    let (first, last) = (times.clone().min(), times.max());
    for row in unique(sources.ledger, "ledger entry", |r| (&r.id, &r.raw))? {
        let wallet = row.wallet.trim().to_ascii_lowercase();
        let described = describe(&row.description, &wallet);
        let refused = |reason: String| refuse("ledger entry", &row.id, &row.description, reason);
        let needs = |export: &str, present: bool| match present {
            true => Ok(()),
            false => Err(refused(format!("needs the {export} export")).with_hint(COMPLETE)),
        };
        // A trade of BTC against anything but fiat, in any currency's ledger.
        if let Described::Trade(_, base, quote, ..) = &described {
            if let Some(other) = not_fiat(base, quote) {
                return Err(refused(format!("books a trade of BTC {other}")));
            }
        }
        if row.currency.trim() != "BTC" {
            // Its BTC side, if any, is the BTC ledger's to book.
            leave(mapper, [&row.currency, ""]);
            not_btc += 1;
            continue;
        }
        // The funding wallet only earns interest and moves BTC to and from
        // the exchange wallet; anything else there, and every other wallet,
        // is refused.
        let funding = matches!(
            described,
            Described::FundingPayment | Described::Transfer(..)
        );
        if wallet != "exchange" && !(wallet == "funding" && funding) {
            return Err(refused(format!("is in the '{}' wallet", shown(&wallet))));
        }
        if !whole_msat(&row.amount) {
            return Err(bad("ledger entry", &row.id, "a sub-millisatoshi amount"));
        }
        let (at, charged) = (seconds(&row.at), row.amount.copy_abs());
        match described {
            Described::Trade(_, base, ..) if base != "BTC" => {
                return Err(refused(format!(
                    "books a trade of {}, not BTC",
                    shown(&base)
                )));
            }
            Described::Trade(fee, _, quote, amount, price) => {
                needs("trades", sources.trades.is_some())?;
                if (fee && !row.amount.is_negative()) || (!fee && amount.copy_abs() != charged) {
                    return Err(refused("does not book what its description names".into()));
                }
                let fits = booked.iter_mut().filter(|t| {
                    let (trade, near) = (t.row, (seconds(&t.row.at) - at).abs() <= TRADE_WINDOW);
                    let fits = match fee {
                        true => {
                            trade.amount.copy_abs() == amount.copy_abs()
                                && t.btc_fee == Some(charged.clone())
                        }
                        false => trade.amount == row.amount,
                    };
                    fits && near && t.quote == quote && trade.price == price
                });
                let fits = fits.map(|t| (t.row.id.as_str(), &mut t.entry[usize::from(fee)]));
                book(row, fits.collect(), "fits no BTC trade or trade fee")?;
            }
            Described::Movement(id) => {
                needs("movements", sources.movements.is_some())?;
                let entry = evidence.entry(id).or_insert((Decimal::zero(), &row.id));
                entry.0 = entry.0.add(&row.amount, &EXACT).map_err(decimal_error)?;
            }
            Described::WithdrawalFee if row.amount.is_negative() => {
                needs("movements", sources.movements.is_some())?;
                let fits = completed.iter_mut().filter(|(m, _)| {
                    let window = seconds(&m.started) - FEE_WINDOW..=seconds(&m.at) + FEE_WINDOW;
                    m.amount.is_negative() && m.fees.copy_abs() == charged && window.contains(&at)
                });
                let fits = fits.map(|(m, slot)| (m.id.as_str(), slot)).collect();
                book(row, fits, "fits no completed withdrawal with that fee")?;
            }
            Described::FundingPayment if wallet == "funding" && row.amount.is_positive() => {
                records.push((row.at.clone(), interest(row)?));
            }
            Described::Transfer(amount, from, to)
                if [&from, &to]
                    .iter()
                    .all(|w| ["exchange", "funding"].contains(&w.as_str()))
                    && from != to
                    && amount.copy_abs() == charged
                    && row.amount.is_negative() == (wallet == from) =>
            {
                legs.push((row, (charged, from, to), None));
            }
            _ => return Err(refused("has a description Kassiber does not map".into())),
        }
    }
    // Each outgoing transfer leg pairs one-to-one with its incoming leg:
    // the same amount between the same wallets within 60 s.
    for out in legs.iter().map(|l| (l.0, l.1.clone())).collect::<Vec<_>>() {
        let (entry, key) = out;
        if entry.amount.is_negative() {
            let fits = legs.iter_mut().filter(|(e, k, _)| {
                let near = (seconds(&e.at) - seconds(&entry.at)).abs() <= TRADE_WINDOW;
                e.amount.is_positive() && *k == key && near
            });
            let fits = fits.map(|(e, _, slot)| (e.id.as_str(), slot)).collect();
            book(entry, fits, "is a transfer without its other leg")?;
        }
    }
    if let Some((lone, ..)) = legs
        .iter()
        .find(|(e, _, paired)| e.amount.is_positive() && paired.is_none())
    {
        let reason = "is a transfer without its other leg";
        return Err(refuse("ledger entry", &lone.id, &lone.description, reason));
    }
    // Each movement's entries add up to what it moved.
    for (id, (sum, first)) in &evidence {
        let refused = |reason: &str| {
            let reason = format!("refers to movement #{} {reason}", shown(id));
            refuse("ledger entry", first, "movement", reason)
        };
        let Some(movement) = movements.get(id.as_str()) else {
            return Err(refused("that Bitfinex did not list"));
        };
        let (zero, whole) = (sum.is_zero(), *sum == movement.amount);
        let balanced = match movement.status.trim().to_ascii_uppercase().as_str() {
            "COMPLETED" => whole,
            "CANCELED" | "CANCELLED" => zero,
            _ => zero || whole,
        };
        if !balanced {
            return Err(refused("whose entries do not add up to its amount"));
        }
    }
    for t in &booked {
        let refused = |reason: &str| refuse("trade", &t.row.id, &t.row.symbol, reason);
        if sources.spot_proof_required && t.entry[0].is_none() {
            let reason = "has no exchange-wallet ledger entry that shows it was a spot trade";
            return Err(refused(reason).with_hint(COMPLETE));
        }
        if t.entry[0].is_some() && t.btc_fee.is_some() && t.entry[1].is_none() {
            return Err(refused("has its ledger entry but no BTC fee entry"));
        }
    }
    // A withdrawal's fee split is proven only by its own entries adding up
    // to `AMOUNT` and a separate fee entry; whether `AMOUNT` includes `FEES`
    // is not documented, and Bitfinex books a withdrawal's entries when it is
    // requested. Only a withdrawal without any ledger entry, requested before
    // the ledger read's first entry (or with no ledger read at all), keeps the
    // fee its movement states, tagged for review.
    for (row, fee_entry) in &completed {
        let principal = evidence
            .get(&row.id)
            .is_some_and(|(sum, _)| *sum == row.amount);
        let unproven =
            row.amount.is_negative() && !row.fees.is_zero() && !(principal && fee_entry.is_some());
        let requested = seconds(&row.started);
        let before = first.is_none_or(|first| requested < first);
        if unproven && (evidence.contains_key(&row.id) || !before) {
            let after = sources.spot_proof_required && last.is_some_and(|last| requested > last);
            let reason = match after {
                true => "was requested after the ledgers export's last row; import a ledgers export that covers it",
                false => "has a fee the ledger does not show as its own entry",
            };
            return Err(refuse("movement", &row.id, "withdrawal", reason).with_hint(COMPLETE));
        }
        records.push((row.at.clone(), movement(row, unproven)?));
    }
    // Oldest first; Bitfinex lists newest first.
    records.reverse();
    records.sort_by(|a, b| a.0.cmp(&b.0));
    let skipped = dict([
        ("non_btc_entries", Py::from(not_btc)),
        ("canceled_movements", Py::from(canceled)),
        ("unfinished_movements", Py::from(unfinished)),
        ("internal_transfers", Py::from(legs.len() as i64)),
    ]);
    mapper.counted(skipped);
    for (_, event) in records {
        mapper.push(event)?;
    }
    Ok(())
}
