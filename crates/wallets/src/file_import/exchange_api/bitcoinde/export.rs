//! Bitcoin.de's BTC account statement CSV, an active custodial ledger: `;`,
//! dot decimals, German wall time, German or English headings, mapped by the
//! API sync's own [`records`] so both store a booking once (bitcoinde.md).
use super::*;
use super::{classify, plain, records, Line, Terms, Type};

/// Canonical columns and their `|`-separated headings (compared trimmed and
/// lowercased): the German export, the current and the original English one.
const COLUMNS: &str = "\
date: datum|date
type: typ|type|booking type
currency: währung|currency
reference: referenz|reference
address: btc-adresse|adresse|btc-address
rate: kurs|rate|price
rate_unit: einheit (kurs)|unit (rate)
btc_before: btc vor gebühr|crypto vor gebühr|btc before fee|btc incl. fee
fiat_before: menge vor gebühr|amount before fee
fiat_before_unit: einheit (menge vor gebühr)|unit (amount before fee)
btc_after: btc nach bitcoin.de-gebühr|crypto nach bitcoin.de-gebühr|btc excl. bitcoin.de fee
fiat_after: menge nach bitcoin.de-gebühr|amount after bitcoin.de-fee
fiat_after_unit: einheit (menge nach bitcoin.de-gebühr)|unit (amount after bitcoin.de-fee)
fiat_after_partner: eur nach bitcoin.de- und fidor-gebühr|eur excl. bitcoin.de and fidor fee
cashflow: zu- / abgang|incoming / outgoing
balance: kontostand|balance|account balance
comment: kommentar|comment";
const REQUIRED: &str =
    "date type currency reference btc_before btc_after fiat_after fiat_after_unit cashflow balance";
/// Per-coin heading endings: `ETH-Adresse`, `ETH vor Gebühr`, ... name the coin.
const COIN_HEADINGS: &str = "-adresse|-address| vor gebühr| nach bitcoin.de-gebühr| before fee| incl. fee| excl. bitcoin.de fee";
const HINT: &str = "Download the BTC account statement from Bitcoin.de (mein Bitcoin.de → Kontoauszug → BTC, type all, whole period → Kontoauszug als CSV herunterladen) and import that file unchanged.";

fn refuse(message: String) -> AppError {
    AppError::new("validation", message).with_hint(HINT)
}

/// The statement's bookings as events, mapped by the API sync's own rules.
/// Kassiber reads only BTC from Bitcoin.de, in either scope: another coin's
/// statement or row is counted as unread, rows per coin.
pub(super) fn load(file: &str, mapper: &mut Mapper) -> Result<(), AppError> {
    let rows = csv::rows_delimited(&csv::read_sig(file)?, ';')?;
    let Some(first) = rows.first() else {
        return Ok(());
    };
    let mut lines = vec![];
    if let Some(coin) = statement_coin(&first.headers) {
        for _ in &rows {
            mapper.admit(&[&coin]);
        }
    } else {
        let columns = columns(&first.headers)?;
        for (number, row) in (2..).zip(&rows) {
            let currency = columns.get("currency").and_then(|h| row.data.get(h));
            let currency = kassiber_pytext::strip(currency.and_then(Json::as_str).unwrap_or(""));
            match kassiber_pytext::upper(currency) {
                coin if !coin.is_empty() && coin != "BTC" => {
                    mapper.admit(&[&coin]);
                }
                _ => lines.push(line(number, row, &columns)?),
            }
        }
    }
    for event in records(&lines)?.records {
        mapper.push(event)?;
    }
    Ok(())
}
/// The coin another coin's statement names in its headings (`ETH-Adresse`,
/// `ETH vor Gebühr`, ...), upper-cased.
fn statement_coin(headers: &[String]) -> Option<String> {
    headers.iter().find_map(|heading| {
        let folded = kassiber_pytext::lower(kassiber_pytext::strip(heading));
        let coin = COIN_HEADINGS
            .split('|')
            .find_map(|ending| folded.strip_suffix(ending))
            .filter(|coin| !coin.is_empty() && coin.chars().all(|c| c.is_ascii_alphanumeric()))?;
        let ours = ["btc", "crypto", "menge", "amount", "eur"].contains(&coin);
        (!ours).then(|| coin.to_ascii_uppercase())
    })
}

/// Each heading's canonical column; an unknown or repeated heading is
/// refused.
fn columns(headers: &[String]) -> Result<BTreeMap<&'static str, String>, AppError> {
    let mut result = BTreeMap::new();
    for heading in headers {
        let folded = kassiber_pytext::lower(kassiber_pytext::strip(heading));
        let shown = kassiber_pytext::strip(heading);
        let name = COLUMNS.lines().find_map(|line| {
            let (name, names) = line.split_once(": ")?;
            names.split('|').any(|n| n == folded).then_some(name)
        });
        let problem = match name {
            None => "an unknown column",
            Some(name) if result.insert(name, heading.clone()).is_some() => "a repeated column",
            Some(_) => continue,
        };
        return Err(refuse(format!("Bitcoin.de CSV has {problem} '{shown}'")));
    }
    let missing: Vec<&str> = REQUIRED
        .split(' ')
        .filter(|c| !result.contains_key(c))
        .collect();
    match missing.join(", ") {
        missing if missing.is_empty() => Ok(result),
        missing => Err(refuse(format!(
            "Bitcoin.de CSV is missing required columns: {missing}"
        ))),
    }
}

fn line(
    number: usize,
    row: &csv::Row,
    columns: &BTreeMap<&'static str, String>,
) -> Result<Line, AppError> {
    let position = format!("CSV line {number}");
    let bad = |what: String| refuse(format!("Bitcoin.de {position} {what}"));
    if row.rest.is_some() {
        return Err(bad("has more fields than the heading".into()));
    }
    let cell = |name: &str| -> String {
        let value = columns.get(name).and_then(|heading| row.data.get(heading));
        kassiber_pytext::strip(value.and_then(Json::as_str).unwrap_or_default()).to_owned()
    };
    let decimal = |name: &str| -> Result<Option<Decimal>, AppError> {
        match cell(name) {
            value if value.is_empty() => Ok(None),
            value => plain(&value).map(Some).ok_or_else(|| {
                bad(format!("has '{value}' in {name}; Kassiber reads only plain decimals with a decimal point"))
            }),
        }
    };
    let needed = |name: &str| decimal(name)?.ok_or_else(|| bad(format!("has no {name}")));
    // `YYYY-MM-DD HH:MM:SS`, German wall time; a time the clock change
    // skips or repeats has no single instant.
    let date = cell("date");
    let at = match date.split_once(' ') {
        Some((day, clock)) if !clock.contains('.') => {
            contract::wall_time("Europe/Berlin", day, clock).map(|ms| ms.div_euclid(1_000))
        }
        _ => Err("not YYYY-MM-DD HH:MM:SS".into()),
    }
    .map_err(|why| bad(format!("has the date '{date}': {why}")))?;
    let currency = cell("currency");
    if !currency.eq_ignore_ascii_case("BTC") {
        return Err(bad(format!("books {currency}; Kassiber imports BTC only")));
    }
    let kind = cell("type");
    if kind.is_empty() {
        return Err(bad("has no type".into()));
    }
    let trade = match classify(&kind) {
        kind @ (Type::Buy | Type::Sell) => {
            // The older Fidor express exports name the cash after the
            // bank's fee as well; that is what changed hands.
            let partner = decimal("fiat_after_partner")?;
            let quote = match partner {
                Some(_) => "EUR".to_owned(),
                None => kassiber_pytext::upper(&cell("fiat_after_unit")),
            };
            // The marketplace is spot only; the row must still name it.
            let unit = cell("rate_unit");
            let market = unit.split_whitespace().collect::<Vec<_>>().join(" ");
            if !market.eq_ignore_ascii_case(&format!("BTC / {quote}")) {
                return Err(bad(format!("is a trade whose market '{market}' is not BTC / {quote}; Kassiber prices only spot BTC trades against the cash shown")));
            }
            Some(Terms {
                kind: if kind == Type::Buy { "buy" } else { "sell" }.into(),
                btc_before: needed("btc_before")?,
                btc_after: needed("btc_after")?,
                fiat_currency: quote,
                fiat: match partner {
                    Some(partner) => partner,
                    None => needed("fiat_after")?,
                },
            })
        }
        _ => None,
    };
    let raw = row.headers.iter().map(|heading| {
        let value = row.data.get(heading).and_then(Json::as_str);
        (heading.as_str(), value.map_or(Py::Null, s))
    });
    Ok(Line {
        at,
        reference: cell("reference"),
        cashflow: needed("cashflow")?,
        balance: needed("balance")?,
        trade,
        raw: dict([("statement_row", dict(raw.collect::<Vec<_>>()))]),
        position,
        kind,
    })
}
