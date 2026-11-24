//! Binance's Transaction History export (Orders → Transaction History →
//! Export, "Generate all statements"): `User_ID,UTC_Time,Account,Operation,
//! Coin,Change,Remark`, one row per balance change of every Binance wallet.
//! It is Binance's complete ledger, spot trades included, so the CSV is the
//! way to import a Binance account completely
//! (docs/reference/imports.md#binance-transaction-history-csv).
//!
//! The rows of one spot trade share their second and account; the two sides
//! of a move between the user's Binance wallets share their user, coin and
//! second; each other row is its own event. The export has no ids, so a
//! record's identity is the row's own content (`binance:csv:...`), numbered
//! for identical rows.
use super::*;
use std::collections::BTreeMap;

pub(in crate::file_import) const HEADERS: [&str; 7] = [
    "User_ID",
    "UTC_Time",
    "Account",
    "Operation",
    "Coin",
    "Change",
    "Remark",
];
const EXPORT: &str = "Export the transaction history from Binance (Orders → Transaction History → Export, all statements) and import each file unchanged.";
const REVIEW: &str = "Record this row yourself with the generic CSV/JSON import after checking it in Binance; a swap as two rows sharing a swap_ref.";

/// What a Binance operation means: the one place that maps an operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    /// A leg of a spot trade or Convert: grouped by second and account.
    Trade,
    /// A trade's fee row.
    Fee,
    Deposit,
    Withdrawal,
    /// A reward, as this record kind.
    Reward(&'static str),
    /// A move between the user's own Binance wallets or products: left out
    /// when its debit and credit are both in the export.
    Internal,
    /// Margin, futures, P2P and other products Kassiber does not book.
    Refused(&'static str),
}

/// Binance's operation names, as its exports write them.
fn op(name: &str) -> Option<Op> {
    Some(match name {
        "Buy"
        | "Sell"
        | "Transaction Buy"
        | "Transaction Spend"
        | "Transaction Revenue"
        | "Transaction Sold"
        | "Transaction Related"
        | "Binance Convert"
        | "Large OTC trading"
        | "Small Assets Exchange BNB" => Op::Trade,
        "Fee" | "Transaction Fee" => Op::Fee,
        "Deposit" | "Fiat Deposit" => Op::Deposit,
        "Withdraw" | "Fiat Withdraw" => Op::Withdrawal,
        "Simple Earn Flexible Interest"
        | "Simple Earn Locked Rewards"
        | "Savings Interest"
        | "POS savings interest"
        | "Staking Rewards"
        | "ETH 2.0 Staking Rewards"
        | "Launchpool Interest"
        | "Launchpool Airdrop"
        | "BNB Vault Rewards"
        | "Distribution"
        | "Airdrop Assets"
        | "Commission History"
        | "Referral Kickback"
        | "Commission Rebate"
        | "Cashback Voucher"
        | "Simple Earn Flexible Airdrop"
        | "Simple Earn Locked Airdrop" => Op::Reward("income"),
        "Super BNB Mining" | "Pool Distribution" => Op::Reward("mining"),
        "Transfer Between Main and Funding Wallet"
        | "Transfer Between Spot Account and UM Futures Account"
        | "Transfer Between Main Account/Futures and Margin Account"
        | "Simple Earn Flexible Subscription"
        | "Simple Earn Flexible Redemption"
        | "Simple Earn Locked Subscription"
        | "Simple Earn Locked Redemption"
        | "Savings purchase"
        | "Savings Principal redemption"
        | "POS savings purchase"
        | "POS savings redemption"
        | "Staking Purchase"
        | "Staking Redemption"
        | "ETH 2.0 Staking"
        | "Launchpool Subscription"
        | "Launchpool Redemption" => Op::Internal,
        "P2P Trading" | "C2C Transfer" => {
            Op::Refused("is a P2P trade, whose payment happened outside Binance")
        }
        _ if name.contains("Margin") || name.contains("Futures") || name.contains("Isolated") => {
            Op::Refused("is a margin or futures entry; margin and derivatives are out of scope")
        }
        "Realize profit and loss"
        | "Funding Fee"
        | "Insurance Fund Compensation"
        | "Liquidation"
        | "Options Premium"
        | "Options Fee" => Op::Refused("is a derivatives entry; derivatives are out of scope"),
        _ => return None,
    })
}

/// Accounts whose rows are spot holdings; any other account (Margin,
/// Futures, Options) is out of scope.
const ACCOUNTS: [&str; 5] = ["Spot", "Funding", "Earn", "Pool", "Card"];

/// One read row.
struct Line {
    /// 1-based data row, for messages.
    number: usize,
    /// Unix milliseconds, UTC.
    at: i64,
    user: String,
    time: String,
    account: String,
    operation: String,
    coin: String,
    change: Decimal,
    raw: Py,
}
impl Line {
    fn shown(&self) -> String {
        format!(
            "Binance CSV row {} ({} {} {} {})",
            self.number, self.time, self.account, self.operation, self.coin
        )
    }
    fn refuse(&self, reason: &str, hint: &str) -> AppError {
        AppError::new("validation", format!("{} {reason}", self.shown())).with_hint(hint.to_owned())
    }
}

/// Every row of one export as events, oldest first.
pub(in crate::file_import) fn load(file: &str, mapper: &mut Mapper) -> Result<(), AppError> {
    parse(&csv::rows(&csv::read_sig(file)?)?, mapper)
}

fn parse(rows: &[csv::Row], mapper: &mut Mapper) -> Result<(), AppError> {
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let present: Vec<&str> = first
        .headers
        .iter()
        .map(|h| kassiber_pytext::strip(h))
        .collect();
    if present != HEADERS {
        return Err(AppError::new(
            "validation",
            format!(
                "Binance CSV has columns {}, not {}",
                present.join(","),
                HEADERS.join(",")
            ),
        )
        .with_hint(EXPORT));
    }
    let mut lines = vec![];
    for (index, row) in rows.iter().enumerate() {
        if row.rest.is_some() {
            return Err(AppError::new(
                "validation",
                format!("Binance CSV row {} has more fields than headers", index + 1),
            ));
        }
        lines.push(line(index + 1, row)?);
    }
    // The export lists the newest first; records are stored oldest first,
    // rows of one second in the order the export gives them.
    lines.reverse();
    lines.sort_by_key(|line| line.at);
    // The rows of one trade: same second, same account. The sides of one
    // internal move: same user, coin and second.
    let mut events: Vec<Vec<usize>> = vec![];
    let mut keyed: BTreeMap<(bool, i64, String, String), usize> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let key = match op(&line.operation) {
            Some(Op::Trade | Op::Fee) => (true, line.at, line.account.clone(), String::new()),
            Some(Op::Internal) => (false, line.at, line.user.clone(), line.coin.clone()),
            _ => {
                events.push(vec![index]);
                continue;
            }
        };
        match keyed.get(&key) {
            Some(&event) => events[event].push(index),
            None => {
                keyed.insert(key, events.len());
                events.push(vec![index]);
            }
        }
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for event in events {
        let group: Vec<&Line> = event.iter().map(|&i| &lines[i]).collect();
        let first = group[0];
        let trading = matches!(op(&first.operation), Some(Op::Trade | Op::Fee));
        // A trade is filtered by the coins it trades, never by its fee: a
        // BTC fee of another coin's trade is booked on its own.
        let coins: Vec<&str> = group
            .iter()
            .filter(|line| !trading || op(&line.operation) != Some(Op::Fee))
            .map(|line| line.coin.as_str())
            .collect();
        let fees_only = if mapper.admit(&coins) {
            false
        } else if trading
            && group.iter().any(|line| {
                op(&line.operation) == Some(Op::Fee)
                    && assets::class(&line.coin) == assets::Class::Bitcoin
            })
        {
            true
        } else {
            // Left out with its trade, a fee row's coin is counted too.
            if trading {
                for line in group
                    .iter()
                    .filter(|line| op(&line.operation) == Some(Op::Fee))
                {
                    mapper.keeps(&line.coin, assets::class(&line.coin));
                }
            }
            continue;
        };
        if let Some(line) = group
            .iter()
            .find(|line| !ACCOUNTS.contains(&line.account.as_str()))
        {
            return Err(line.refuse(
                &format!(
                    "is in Binance's {} account, which Kassiber does not book",
                    line.account
                ),
                REVIEW,
            ));
        }
        match op(&first.operation) {
            Some(Op::Trade | Op::Fee) => trade(&group, fees_only, mapper, &mut seen)?,
            Some(Op::Deposit) => single(first, true, "deposit", mapper, &mut seen)?,
            Some(Op::Withdrawal) => single(first, false, "withdrawal", mapper, &mut seen)?,
            Some(Op::Reward(kind)) => {
                if !first.change.is_positive() {
                    return Err(first.refuse("does not credit a positive amount", REVIEW));
                }
                single(first, true, kind, mapper, &mut seen)?
            }
            Some(Op::Internal) => internal(&group, mapper)?,
            Some(Op::Refused(reason)) => return Err(first.refuse(reason, REVIEW)),
            None => return Err(first.refuse("has an operation Kassiber does not map", REVIEW)),
        }
    }
    Ok(())
}

/// One CSV row, read strictly: a `YYYY-MM-DD HH:MM:SS` UTC time, a coin
/// code and a plain signed decimal change.
fn line(number: usize, row: &csv::Row) -> Result<Line, AppError> {
    let columns = exchange::Columns::new(row);
    let text = |name: &str| columns.opt(&[name]).unwrap_or_default();
    let (user, time, account, operation, coin, change) = (
        text("User_ID"),
        text("UTC_Time"),
        text("Account"),
        text("Operation"),
        asset(&s(text("Coin").as_str())),
        text("Change"),
    );
    let bad = |what: &str| {
        AppError::new("validation", format!("Binance CSV row {number} has {what}"))
            .with_hint(EXPORT)
    };
    let shaped = time.len() == 19
        && time.as_bytes()[10] == b' '
        && time
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit());
    let at = shaped
        .then(|| kassiber_pytext::iso_datetime(&time))
        .flatten()
        .filter(|parsed| parsed.offset_us.is_none())
        .map(|parsed| parsed.utc_us().div_euclid(1_000) - 719_162 * 86_400_000)
        .ok_or_else(|| bad(&format!("time '{time}', not YYYY-MM-DD HH:MM:SS in UTC")))?;
    if coin.is_empty() || !ticker(&coin) {
        return Err(bad(&format!("coin '{coin}', which is not a ticker")));
    }
    let change = strict_decimal(&change, true)
        .ok_or_else(|| bad(&format!("change '{change}', which is not a plain decimal")))?;
    Ok(Line {
        number,
        at,
        user,
        time,
        account,
        operation,
        coin,
        change,
        raw: kassiber_pytext::loads(&columns.json().to_string()).unwrap_or(Py::Null),
    })
}

/// The identity of a row's record, numbered for an identical row.
fn identity(key: String, seen: &mut BTreeMap<String, usize>) -> PyStr {
    let count = seen.entry(key.clone()).or_default();
    *count += 1;
    match *count {
        1 => key.into(),
        n => format!("{key}#{n}").into(),
    }
}

fn occurred(line: &Line) -> Result<PyStr, AppError> {
    Ok(utc_iso(line.at.div_euclid(1_000) as f64)?.into())
}

/// A BTC amount must be whole msat; any other coin's exactness is the
/// shared record check's.
fn exact(line: &Line, coin: &str, amount: &Decimal) -> Result<(), AppError> {
    if assets::class(coin) == assets::Class::Bitcoin && exact_msat(&amount.copy_abs()).is_none() {
        return Err(line.refuse(
            "has a BTC amount that is not a whole number of millisatoshi",
            EXPORT,
        ));
    }
    Ok(())
}

/// Each row's own amount in its coin's exact units, before rows are summed:
/// sums of finer amounts could round into whole units.
fn exact_row(line: &Line) -> Result<(), AppError> {
    exact(line, &line.coin, &line.change)?;
    if assets::class(&line.coin) == assets::Class::Crypto {
        assets::exact_units(&line.coin, "amount", &line.change.copy_abs())
            .map_err(|error| line.refuse(&format!("is not exact: {}", error.message), EXPORT))?;
    }
    Ok(())
}

/// The sides of one move between the user's Binance wallets: each debit
/// matches a credit of the same amount, or the move refuses by name, since
/// a side the export does not hold may have left Binance.
fn internal(group: &[&Line], mapper: &mut Mapper) -> Result<(), AppError> {
    let mut credits: Vec<&Line> = group
        .iter()
        .copied()
        .filter(|line| line.change.is_positive())
        .collect();
    let mut unmatched = vec![];
    for line in group.iter().filter(|line| !line.change.is_positive()) {
        let amount = line.change.copy_abs();
        match credits.iter().position(|credit| credit.change == amount) {
            Some(credit) => {
                credits.remove(credit);
            }
            None => unmatched.push(*line),
        }
    }
    unmatched.extend(credits);
    if let Some(line) = unmatched.into_iter().min_by_key(|line| line.number) {
        return Err(line.refuse(
            &format!(
                "moves {} between Binance wallets, and the export holds no matching row of the other side",
                line.coin
            ),
            REVIEW,
        ));
    }
    for _ in group {
        mapper.internal();
    }
    Ok(())
}

/// A deposit, withdrawal or reward: one row, one leg.
fn single(
    line: &Line,
    inbound: bool,
    kind: &'static str,
    mapper: &mut Mapper,
    seen: &mut BTreeMap<String, usize>,
) -> Result<(), AppError> {
    if line.change.is_zero() || line.change.is_negative() == inbound {
        return Err(line.refuse("moves the other way than its operation says", REVIEW));
    }
    exact(line, &line.coin, &line.change)?;
    let key = format!(
        "binance:csv:{}:{}:{}:{}:{}",
        line.time, line.account, line.operation, line.coin, line.change
    );
    let amount = PriceNumber::Finite(line.change.copy_abs());
    mapper.push(Event {
        occurred_at: Some(occurred(line)?),
        raw: line.raw.clone(),
        description: Some(format!("Binance {}", line.operation).into()),
        ..Event::one(
            identity(key, seen),
            Kind::Movement(kind),
            inbound,
            line.coin.clone(),
            amount,
        )
    })
}

/// The rows of one second and account: one coin out and one in (fills of
/// one side of one pair summed), and fee rows. Against fiat it is a buy or
/// sell valued at the fiat that moved (net of a fiat fee); between two coins
/// two swap legs. A fee in the received coin is netted from what arrived; a
/// fee in a third coin is its own fee row, booked where the scope keeps
/// that coin and counted where not. With `fees_only` (the scope keeps no
/// traded coin, but a fee is in BTC) only the kept fee rows are booked.
fn trade(
    group: &[&Line],
    fees_only: bool,
    mapper: &mut Mapper,
    seen: &mut BTreeMap<String, usize>,
) -> Result<(), AppError> {
    let first = group[0];
    let sum = |rows: &mut BTreeMap<String, Decimal>, line: &Line| -> Result<(), AppError> {
        let total = rows.entry(line.coin.clone()).or_insert_with(Decimal::zero);
        *total = total.add(&line.change, &CTX).map_err(decimal_error)?;
        Ok(())
    };
    let (mut moved, mut fees) = (BTreeMap::new(), BTreeMap::new());
    // The directions each traded coin moved in: a coin both bought and sold
    // in one second is two trades the export does not tell apart.
    let mut directions: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
    for line in group {
        exact_row(line)?;
        match op(&line.operation) {
            Some(Op::Fee) => sum(&mut fees, line)?,
            _ => {
                let seen = directions.entry(line.coin.as_str()).or_default();
                match line.change.is_negative() {
                    true => seen.0 = true,
                    false => seen.1 = true,
                }
                sum(&mut moved, line)?
            }
        }
    }
    if let Some((coin, _)) = directions.iter().find(|(_, (out, into))| *out && *into) {
        return Err(first.refuse(
            &format!("buys and sells {coin} within one second; Kassiber cannot tell which rows belong together"),
            REVIEW,
        ));
    }
    moved.retain(|_, total| !total.is_zero());
    let (sent, received) = match (
        moved.iter().find(|(_, total)| total.is_negative()),
        moved.iter().find(|(_, total)| total.is_positive()),
    ) {
        (Some(sent), Some(received)) if moved.len() == 2 => (sent, received),
        _ => {
            let coins: Vec<&str> = moved.keys().map(String::as_str).collect();
            return Err(first.refuse(
                &format!(
                    "is one of several trades in one second ({}); Kassiber books a second's trades only when they are one pair, one coin out and one in",
                    coins.join(", ")
                ),
                REVIEW,
            ));
        }
    };
    if fees.values().any(|fee| fee.is_positive()) {
        return Err(first.refuse("has a fee row that credits the account", REVIEW));
    }
    let (sent_coin, sent_total) = (sent.0.clone(), sent.1.copy_abs());
    let (received_coin, mut received_total) = (received.0.clone(), received.1.clone());
    let mut fiat_fee = Decimal::zero();
    let mut third = vec![];
    for (coin, fee) in &fees {
        let fee = fee.copy_abs();
        if fee.is_zero() {
            continue;
        }
        if *coin == received_coin {
            received_total = received_total.sub(&fee, &CTX).map_err(decimal_error)?;
            if !received_total.is_positive() {
                return Err(first.refuse("has a fee as large as what it received", REVIEW));
            }
        } else if *coin == sent_coin || assets::class(coin) == assets::Class::Fiat {
            if assets::class(coin) != assets::Class::Fiat {
                return Err(first.refuse(
                    &format!("charges its fee in {coin}, the coin it spent; Kassiber cannot tell whether the spent amount includes it"),
                    REVIEW,
                ));
            }
            fiat_fee = fiat_fee.add(&fee, &CTX).map_err(decimal_error)?;
        } else {
            third.push((coin.clone(), fee));
        }
    }
    for (coin, amount) in [(&sent_coin, &sent_total), (&received_coin, &received_total)] {
        exact(first, coin, amount)?;
    }
    let base = format!("binance:csv:{}:{}", first.time, first.account);
    let occurred_at = occurred(first)?;
    let raw = Py::List(group.iter().map(|line| line.raw.clone()).collect());
    let event = |id: PyStr, kind: Kind, inbound: bool, coin: &str, amount: &Decimal| Event {
        occurred_at: Some(occurred_at.clone()),
        raw: raw.clone(),
        description: Some(format!("Binance trade {sent_coin} -> {received_coin}").into()),
        ..Event::one(
            id,
            kind,
            inbound,
            coin.to_owned(),
            PriceNumber::Finite(amount.clone()),
        )
    };
    let fiat = |coin: &str| assets::class(coin) == assets::Class::Fiat;
    let reference = identity(format!("{base}:{sent_coin}-{received_coin}"), seen);
    let pricing = |currency: &str, value: Decimal| {
        Some(Pricing {
            fiat_currency: currency.to_owned(),
            fiat_value: PriceNumber::Finite(value),
            fiat_rate: None,
            method: "binance_csv",
            external_ref: reference.clone(),
        })
    };
    match (fiat(&sent_coin), fiat(&received_coin)) {
        (true, true) => return Err(first.refuse("exchanges one fiat currency for another", REVIEW)),
        // The principal legs are another scope's; only the fees below.
        _ if fees_only => {}
        (true, false) => {
            // A buy: its cost is the fiat spent plus a fiat fee.
            let cost = sent_total.add(&fiat_fee, &CTX).map_err(decimal_error)?;
            mapper.push(Event {
                pricing: pricing(&sent_coin, cost),
                ..event(
                    reference.clone(),
                    Kind::Trade,
                    true,
                    &received_coin,
                    &received_total,
                )
            })?;
        }
        (false, true) => {
            // A sale: its proceeds are the fiat received net of a fiat fee.
            let proceeds = received_total.sub(&fiat_fee, &CTX).map_err(decimal_error)?;
            if !proceeds.is_positive() {
                return Err(first.refuse("has a fee as large as its proceeds", REVIEW));
            }
            mapper.push(Event {
                pricing: pricing(&received_coin, proceeds),
                ..event(
                    reference.clone(),
                    Kind::Trade,
                    false,
                    &sent_coin,
                    &sent_total,
                )
            })?;
        }
        (false, false) => {
            if !fiat_fee.is_zero() {
                return Err(first.refuse("charges a fiat fee on a swap", REVIEW));
            }
            let kept = |coin: &str, mapper: &mut Mapper| mapper.keeps(coin, assets::class(coin));
            if !(kept(&sent_coin, mapper) && kept(&received_coin, mapper)) {
                return Err(first.refuse(
                    &format!("swaps {sent_coin} for {received_coin}"),
                    &format!(
                        "Kassiber does not book one leg of a swap as a sale or purchase. To import both legs, run {} and import the file again.",
                        mapper.keep_command()
                    ),
                ));
            }
            let out = concat(&[reference.clone(), ":out".into()]);
            let into = concat(&[reference.clone(), ":in".into()]);
            mapper.push(event(
                out,
                Kind::Swap(reference.clone()),
                false,
                &sent_coin,
                &sent_total,
            ))?;
            mapper.push(event(
                into,
                Kind::Swap(reference.clone()),
                true,
                &received_coin,
                &received_total,
            ))?;
        }
    }
    for (coin, fee) in third {
        // A fee in a coin the scope does not keep is counted, as its trade
        // would be.
        if !mapper.keeps(&coin, assets::class(&coin)) {
            continue;
        }
        let id = concat(&[reference.clone(), format!(":fee:{coin}").into()]);
        mapper.push(Event {
            description: Some(format!("Binance fee in {coin}").into()),
            ..event(id, Kind::Fee, false, &coin, &fee)
        })?;
    }
    Ok(())
}
