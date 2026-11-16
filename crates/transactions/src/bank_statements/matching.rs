//! Exact suggestions pairing outgoing bank lines with Bitcoin purchases:
//! same fiat currency, the exact purchase value, booked in the date window,
//! and no other candidate on either side. Anything else (two candidates,
//! partial amounts, fees, splits) is left unmatched with a named reason.
//! Suggestions are computed on demand; only decisions are stored.
use super::{invalid, Money, Result};
use crate::sql::{now, rows, Scope};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// A bank line may be booked this many days before the purchase ...
pub const DAYS_BEFORE: i64 = 10;
/// ... or this many days after it (value-date and time-zone slack).
pub const DAYS_AFTER: i64 = 1;

/// Exchange identities, their names and the words that name them.
pub const EXCHANGES: &[(&str, &str, &[&str])] = &[
    ("bitpanda", "Bitpanda", &["bitpanda"]),
    ("kraken", "Kraken", &["kraken", "payward"]),
    ("coinbase", "Coinbase", &["coinbase"]),
    ("binance", "Binance", &["binance"]),
    ("bitstamp", "Bitstamp", &["bitstamp"]),
    ("bitvavo", "Bitvavo", &["bitvavo"]),
    (
        "bitcoinde",
        "Bitcoin.de",
        &["bitcoin de", "bitcoinde", "futurum"],
    ),
    ("coinfinity", "Coinfinity", &["coinfinity"]),
    (
        "bullbitcoin",
        "Bull Bitcoin",
        &["bull bitcoin", "bullbitcoin"],
    ),
    ("21bitcoin", "21bitcoin", &["21bitcoin", "21 bitcoin"]),
    (
        "pocket",
        "Pocket",
        &["pocket bitcoin", "pocketbitcoin", "pocket"],
    ),
    ("strike", "Strike", &["strike"]),
    ("river", "River", &["river financial", "river"]),
    ("relai", "Relai", &["relai"]),
];

/// The first curated exchange the text names, by whole words.
pub fn named_exchange(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let haystack = format!(" {} ", words.join(" "));
    EXCHANGES
        .iter()
        .find(|(_, _, aliases)| aliases.iter().any(|a| haystack.contains(&format!(" {a} "))))
        .map(|(id, _, _)| *id)
}

pub fn exchange_name(id: &str) -> &'static str {
    EXCHANGES
        .iter()
        .find(|(key, _, _)| *key == id)
        .map_or("an exchange", |(_, name, _)| *name)
}

/// Days since 1970-01-01 of a `YYYY-MM-DD` prefix.
pub fn day_number(date: &str) -> Option<i64> {
    let (y, m, d): (i64, i64, i64) = (
        date.get(..4)?.parse().ok()?,
        date.get(5..7)?.parse().ok()?,
        date.get(8..10)?.parse().ok()?,
    );
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    Some(era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468)
}

#[derive(Clone, Debug)]
pub struct Line {
    pub id: String,
    pub booking_date: String,
    /// Absolute value of an outgoing line.
    pub amount: Money,
    pub currency: String,
    /// The curated exchange its counterparty or remittance names.
    pub exchange: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Buy {
    pub id: String,
    pub occurred_at: String,
    pub amount_msat: i64,
    pub currency: String,
    pub value: Money,
    pub exchange: Option<&'static str>,
    pub wallet_label: String,
}

/// Pairs the user decided: accepted lines/buys leave the pool, rejected
/// pairs are never suggested together again.
#[derive(Default)]
pub struct Decisions {
    pub accepted_lines: BTreeSet<String>,
    pub accepted_buys: BTreeSet<String>,
    pub rejected: BTreeSet<(String, String)>,
}

/// One exact pair and why, or an item left unmatched and why.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Pair {
        line: String,
        buy: String,
        reasons: Vec<String>,
    },
    Unmatched {
        id: String,
        line: bool,
        reason: &'static str,
    },
}

/// Days the bank booked before the purchase, when inside the window.
fn days_before(line: &Line, buy: &Buy) -> Option<i64> {
    let before = day_number(&buy.occurred_at)? - day_number(&line.booking_date)?;
    (-DAYS_AFTER..=DAYS_BEFORE)
        .contains(&before)
        .then_some(before)
}

fn reasons(line: &Line, buy: &Buy, before: i64) -> Vec<String> {
    let mut out = vec![
        format!("same currency {}", buy.currency),
        format!(
            "bank amount equals the purchase value {}",
            buy.value.canonical()
        ),
        match before {
            0 => "booked the same day as the purchase".to_owned(),
            1 => "booked 1 day before the purchase".to_owned(),
            -1 => "booked 1 day after the purchase".to_owned(),
            n => format!("booked {n} days before the purchase"),
        },
    ];
    if let Some(exchange) = line.exchange {
        out.push(format!(
            "counterparty or reference names {}",
            exchange_name(exchange)
        ));
        if buy.exchange == Some(exchange) {
            out.push(format!(
                "matches the purchase's {} wallet",
                exchange_name(exchange)
            ));
        }
    }
    out
}

/// Exact one-to-one pairs; every other open purchase, and every line with
/// several candidates, is unmatched with its reason.
pub fn suggest(lines: &[Line], buys: &[Buy], decisions: &Decisions) -> Vec<Outcome> {
    let lines: Vec<&Line> = lines
        .iter()
        .filter(|l| !decisions.accepted_lines.contains(&l.id))
        .collect();
    let buys: Vec<&Buy> = buys
        .iter()
        .filter(|b| !decisions.accepted_buys.contains(&b.id))
        .collect();
    let candidate = |l: &Line, b: &Buy| {
        let open = !decisions.rejected.contains(&(l.id.clone(), b.id.clone()));
        (open && l.currency == b.currency && l.amount == b.value)
            .then(|| days_before(l, b))
            .flatten()
    };
    let for_line = |l: &Line| buys.iter().filter(|b| candidate(l, b).is_some()).count();
    let mut out = Vec::new();
    for b in &buys {
        let found: Vec<&&Line> = lines.iter().filter(|l| candidate(l, b).is_some()).collect();
        match found.as_slice() {
            [] => out.push(Outcome::Unmatched {
                id: b.id.clone(),
                line: false,
                reason: "no_exact_amount",
            }),
            [l] if for_line(l) == 1 => out.push(Outcome::Pair {
                line: l.id.clone(),
                buy: b.id.clone(),
                reasons: reasons(l, b, candidate(l, b).unwrap_or(0)),
            }),
            _ => out.push(Outcome::Unmatched {
                id: b.id.clone(),
                line: false,
                reason: "ambiguous",
            }),
        }
    }
    for l in lines.iter().filter(|l| for_line(l) > 1) {
        out.push(Outcome::Unmatched {
            id: l.id.clone(),
            line: true,
            reason: "ambiguous",
        });
    }
    out
}

/// The id of a group of lines and purchases, independent of their order.
pub fn group_key<A: AsRef<str>, B: AsRef<str>>(lines: &[A], buys: &[B]) -> String {
    let sorted = |items: Vec<&str>| {
        let mut items = items;
        items.sort_unstable();
        items.join(",")
    };
    let mut digest = Sha256::new();
    digest.update(sorted(lines.iter().map(AsRef::as_ref).collect()).as_bytes());
    digest.update(b"|");
    digest.update(sorted(buys.iter().map(AsRef::as_ref).collect()).as_bytes());
    digest.finalize()[..8]
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// Wallet kinds that are themselves an exchange account.
fn wallet_exchange(kind: &str, label: &str) -> Option<&'static str> {
    match kind {
        "pocketbitcoin" => Some("pocket"),
        "bitpanda" | "kraken" | "coinbase" | "binance" | "bullbitcoin" | "coinfinity"
        | "21bitcoin" | "river" | "strike" => EXCHANGES
            .iter()
            .find(|(id, _, _)| *id == kind)
            .map(|(id, _, _)| *id),
        _ => named_exchange(label),
    }
}

/// The purchase value: the exact fiat value, else the exact rate times the
/// exact BTC amount. `None` when neither is exact.
fn purchase_value(row: &Value, msat: i64) -> Option<Money> {
    let text = |k: &str| crate::documents::text(row, k).trim().to_owned();
    if let Some(value) = Money::parse(&text("fiat_value_exact")).filter(|v| !v.is_negative()) {
        return Some(value);
    }
    let btc = Money::parse(&format!(
        "{}.{:011}",
        msat / 100_000_000_000,
        msat % 100_000_000_000
    ))?;
    Money::parse(&text("fiat_rate_exact"))?.times(btc)
}

/// Inbound BTC buys with an exact fiat value, and the ids of those without.
pub(crate) fn buys(conn: &Connection, scope: &Scope<'_>) -> Result<(Vec<Buy>, Vec<Value>)> {
    let (mut found, mut unpriced) = (Vec::new(), Vec::new());
    for row in rows(
        conn,
        "SELECT t.id,t.occurred_at,t.amount,t.fiat_currency,t.fiat_value_exact,t.fiat_rate_exact,w.label AS wallet_label,w.kind AS wallet_kind FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? AND t.excluded=0 AND t.direction='inbound' AND upper(t.asset)='BTC' AND lower(coalesce(nullif(t.kind_override,''),t.kind,''))='buy' ORDER BY t.occurred_at,t.id",
        [scope.profile_id],
    )? {
        let text = |k: &str| crate::documents::text(&row, k).to_owned();
        let msat = match get(&row, "amount") {
            Some(Value::Int(n)) => i64::try_from(n.clone()).unwrap_or(0),
            _ => 0,
        };
        let currency = super::fiat(&text("fiat_currency"), "purchase").ok();
        match (purchase_value(&row, msat), currency) {
            (Some(value), Some(currency)) => found.push(Buy {
                id: text("id"),
                occurred_at: text("occurred_at"),
                amount_msat: msat,
                currency,
                value,
                exchange: wallet_exchange(&text("wallet_kind"), &text("wallet_label")),
                wallet_label: text("wallet_label"),
            }),
            _ => unpriced.push(s(text("id"))),
        }
    }
    Ok((found, unpriced))
}

pub(crate) fn decisions(conn: &Connection, scope: &Scope<'_>) -> Result<Decisions> {
    let mut out = Decisions::default();
    if !super::store::tables_present(conn)? {
        return Ok(out);
    }
    for row in rows(
        conn,
        "SELECT d.line_id,d.transaction_id,d.state FROM bank_statement_line_decisions d JOIN transactions t ON t.id=d.transaction_id AND t.profile_id=d.profile_id WHERE d.profile_id=?",
        [scope.profile_id],
    )? {
        let text = |k: &str| crate::documents::text(&row, k).to_owned();
        if text("state") == "accepted" {
            out.accepted_lines.insert(text("line_id"));
            out.accepted_buys.insert(text("transaction_id"));
        } else {
            out.rejected.insert((text("line_id"), text("transaction_id")));
        }
    }
    Ok(out)
}

/// Suggestions for the book, optionally only those involving one purchase.
pub fn suggest_for_book(
    conn: &Connection,
    scope: &Scope<'_>,
    transaction: Option<&str>,
) -> Result<Value> {
    let (buys, unpriced) = buys(conn, scope)?;
    if let Some(tx) = transaction {
        if !buys.iter().any(|b| b.id == tx) && !unpriced.contains(&s(tx)) {
            return Err(
                AppError::new("not_found", format!("Bitcoin purchase '{tx}' not found")).with_hint(
                    "Bank lines are matched against inbound BTC transactions whose kind is buy.",
                ),
            );
        }
    }
    let mut line_rows: BTreeMap<String, Value> = BTreeMap::new();
    let mut lines = Vec::new();
    if super::store::tables_present(conn)? {
        let sql = format!("SELECT {} FROM bank_statement_lines l JOIN bank_statement_imports i ON i.id=l.import_id WHERE l.profile_id=? AND l.amount LIKE '-%' ORDER BY l.booking_date, l.id", super::store::LINE_COLUMNS);
        for row in rows(conn, &sql, [scope.profile_id])? {
            let line = super::store::StoredLine::from_row(&row);
            if let Some(amount) = Money::parse(&line.amount) {
                lines.push(Line {
                    id: line.id.clone(),
                    booking_date: line.booking_date.clone(),
                    amount: amount.abs(),
                    currency: line.currency.clone(),
                    exchange: line.named_exchange(),
                });
            }
            line_rows.insert(line.id.clone(), line.payload(&row));
        }
    }
    let by_id: BTreeMap<&str, &Buy> = buys.iter().map(|b| (b.id.as_str(), b)).collect();
    let (mut suggestions, mut unmatched) = (Vec::new(), Vec::new());
    for outcome in suggest(&lines, &buys, &decisions(conn, scope)?) {
        match outcome {
            Outcome::Pair { line, buy, reasons } if transaction.is_none_or(|tx| tx == buy) => {
                let b = by_id[buy.as_str()];
                suggestions.push(dict([
                    ("id", s(group_key(&[&line], &[&buy]))),
                    ("currency", s(b.currency.as_str())),
                    ("bank_total", s(b.value.canonical())),
                    ("purchase_total", s(b.value.canonical())),
                    ("line_ids", Value::List(vec![s(line.as_str())])),
                    ("transaction_ids", Value::List(vec![s(buy.as_str())])),
                    (
                        "lines",
                        Value::List(line_rows.get(&line).cloned().into_iter().collect()),
                    ),
                    (
                        "transactions",
                        Value::List(vec![dict([
                            ("id", s(b.id.as_str())),
                            ("occurred_at", s(b.occurred_at.as_str())),
                            ("amount_msat", Value::Int(b.amount_msat.into())),
                            ("fiat_currency", s(b.currency.as_str())),
                            ("fiat_value", s(b.value.canonical())),
                            ("wallet", s(b.wallet_label.as_str())),
                            (
                                "exchange",
                                b.exchange.map_or(Value::Null, |e| s(exchange_name(e))),
                            ),
                        ])]),
                    ),
                    (
                        "reasons",
                        Value::List(reasons.iter().map(|r| s(r.as_str())).collect()),
                    ),
                ]));
            }
            Outcome::Unmatched { id, line, reason }
                if line || transaction.is_none_or(|tx| tx == id) =>
            {
                unmatched.push(dict([
                    (if line { "line_id" } else { "transaction_id" }, s(id)),
                    ("reason", s(reason)),
                ]));
            }
            _ => {}
        }
    }
    Ok(dict([
        ("suggestions", Value::List(suggestions)),
        ("unmatched", Value::List(unmatched)),
        ("purchases_without_exact_fiat_value", Value::List(unpriced)),
        (
            "rules",
            dict([
                ("days_before", Value::Int(DAYS_BEFORE.into())),
                ("days_after", Value::Int(DAYS_AFTER.into())),
                ("exact_amount", Value::Bool(true)),
            ]),
        ),
    ]))
}

/// Explicit line and purchase ids, resolved within the profile.
pub fn resolve_pairs(
    conn: &Connection,
    scope: &Scope<'_>,
    lines: &[String],
    transactions: &[String],
) -> Result<(Vec<super::store::StoredLine>, Vec<Value>)> {
    if lines.is_empty() || transactions.is_empty() {
        return Err(invalid("Name at least one bank line and one transaction"));
    }
    let tables = super::store::tables_present(conn)?;
    let mut line_rows = Vec::new();
    for id in lines {
        let sql = format!("SELECT {} FROM bank_statement_lines l JOIN bank_statement_imports i ON i.id=l.import_id WHERE l.profile_id=? AND l.id=?", super::store::LINE_COLUMNS);
        let found = if tables {
            rows(conn, &sql, params![scope.profile_id, id])?
        } else {
            Vec::new()
        };
        let row = found.first().ok_or_else(|| {
            AppError::new("not_found", format!("Bank statement line '{id}' not found"))
        })?;
        line_rows.push(super::store::StoredLine::from_row(row));
    }
    let mut tx_rows = Vec::new();
    for id in transactions {
        let found = rows(
            conn,
            "SELECT t.* FROM transactions t WHERE t.profile_id=? AND t.id=?",
            params![scope.profile_id, id],
        )?;
        tx_rows.push(
            found.into_iter().next().ok_or_else(|| {
                AppError::new("not_found", format!("Transaction '{id}' not found"))
            })?,
        );
    }
    Ok((line_rows, tx_rows))
}

/// Record `rejected` for every named pair; an accepted pair becomes
/// rejected, which withdraws the line from that source's evidence. A
/// purchase a reset removed can still have its earlier decisions rejected.
pub fn reject(
    conn: &Connection,
    scope: &Scope<'_>,
    lines: &[String],
    transactions: &[String],
) -> Result<Value> {
    crate::sql::atomic(conn, || {
        if lines.is_empty() || transactions.is_empty() {
            return Err(invalid("Name at least one bank line and one transaction"));
        }
        let tables = super::store::tables_present(conn)?;
        let exists = |sql: &str, values: &[&str]| -> Result<bool> {
            let mut bound = vec![scope.profile_id];
            bound.extend_from_slice(values);
            Ok(!rows(conn, sql, kassiber_store::rusqlite::params_from_iter(bound))?.is_empty())
        };
        const DECIDED: &str = "SELECT state FROM bank_statement_line_decisions WHERE profile_id=? AND line_id=? AND transaction_id=?";
        for line in lines {
            if !tables
                || !exists(
                    "SELECT id FROM bank_statement_lines WHERE profile_id=? AND id=?",
                    &[line],
                )?
            {
                return Err(AppError::new(
                    "not_found",
                    format!("Bank statement line '{line}' not found"),
                ));
            }
        }
        for tx in transactions {
            let mut known = exists(
                "SELECT id FROM transactions WHERE profile_id=? AND id=?",
                &[tx],
            )?;
            for line in lines {
                known = known || exists(DECIDED, &[line, tx])?;
            }
            if !known {
                return Err(AppError::new(
                    "not_found",
                    format!("Transaction '{tx}' not found"),
                ));
            }
        }
        let at = now(conn)?;
        let mut withdrawn = 0;
        for line in lines {
            for tx in transactions {
                withdrawn +=
                    exists(&format!("{DECIDED} AND state='accepted'"), &[line, tx])? as i64;
                conn.execute(
                    "INSERT INTO bank_statement_line_decisions(line_id,transaction_id,profile_id,state,source_id,group_key,reasons_json,created_at,updated_at) VALUES(?,?,?,'rejected',NULL,NULL,'[]',?,?) ON CONFLICT(line_id,transaction_id) DO UPDATE SET state='rejected', updated_at=excluded.updated_at",
                    params![line, tx, scope.profile_id, at, at],
                )
                .map_err(crate::sql::error)?;
            }
        }
        let ids = |v: &[String]| Value::List(v.iter().map(|x| s(x.as_str())).collect());
        Ok(dict([
            ("state", s("rejected")),
            ("line_ids", ids(lines)),
            ("transaction_ids", ids(transactions)),
            ("withdrawn_accepted", Value::Int(withdrawn.into())),
        ]))
    })
}
