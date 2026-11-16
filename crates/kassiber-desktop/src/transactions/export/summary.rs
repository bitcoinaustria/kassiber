//! The human preview: the few columns of an export's first rows that say what the file holds
//! (date, wallet, direction, amount, value, transaction), read from `ui.exports.preview`'s
//! sheet by column name, so each format (Kassiber's own, CoinTracking, Blockpit, BIP 329) maps
//! onto the same small table.

use serde_json::Value;

use super::Format;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dir {
    In,
    Out,
    #[default]
    Other,
}

/// One summarised row. Strings are display-ready except `amount` (the file's own digits) and
/// `value` (a number when it parsed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub date: String,
    pub wallet: String,
    pub dir: Option<Dir>,
    /// What the file calls the movement (`inbound`, `Deposit`, a label's type).
    pub kind: String,
    pub amount: String,
    pub asset: String,
    pub value: Option<f64>,
    pub currency: String,
    pub txid: String,
    /// BIP 329: the label text.
    pub label: String,
}

fn idx(columns: &[String], names: &[&str]) -> Option<usize> {
    names
        .iter()
        .find_map(|n| columns.iter().position(|c| c.eq_ignore_ascii_case(n)))
}

fn cell(row: &Value, i: Option<usize>) -> String {
    i.and_then(|i| row.get(i))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned()
}

/// A preview number (`3,197.60`, `255.808`) as f64; empty or unparsable is None.
pub fn number(s: &str) -> Option<f64> {
    let t: String = s
        .chars()
        .filter(|c| *c != ',' && *c != '\'' && !c.is_whitespace())
        .collect();
    if t.is_empty() { None } else { t.parse().ok() }
}

/// The app's `YYYY-MM-DD HH:MM` from an RFC 3339 stamp, `YYYY-MM-DD HH:MM:SS` or Blockpit's
/// `DD.MM.YYYY HH:MM:SS`.
pub fn app_date(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 10 && b[2] == b'.' && b[5] == b'.' {
        let rest = s.get(10..).unwrap_or("").trim();
        let iso = format!("{}-{}-{} {}", &s[6..10], &s[3..5], &s[0..2], rest);
        return crate::format::short_date(Some(iso.trim()));
    }
    if s.is_empty() {
        String::new()
    } else {
        crate::format::short_date(Some(s))
    }
}

/// `first6…last4` for a transaction id (a 64-hex txid fits a 6 rem column).
pub fn short_ref(id: &str) -> String {
    let c: Vec<char> = id.chars().collect();
    if c.len() <= 14 {
        id.to_owned()
    } else {
        format!(
            "{}…{}",
            c[..6].iter().collect::<String>(),
            c[c.len() - 4..].iter().collect::<String>()
        )
    }
}

fn dir_of(s: &str) -> Dir {
    match s.to_ascii_lowercase().as_str() {
        "inbound" | "incoming" | "in" | "deposit" | "income" | "receive" | "received" => Dir::In,
        "outbound" | "outgoing" | "out" | "withdrawal" | "spend" | "send" | "sent" => Dir::Out,
        _ => Dir::Other,
    }
}

/// The first `n` rows of `sheet` summarised for `format`.
pub fn lines(format: Format, sheet: &Value, n: usize) -> Vec<Line> {
    let columns: Vec<String> = sheet["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| c.as_str().unwrap_or("").to_owned())
                .collect()
        })
        .unwrap_or_default();
    let rows = sheet["rows"].as_array().cloned().unwrap_or_default();
    let c = |names: &[&str]| idx(&columns, names);
    let mut out = Vec::new();
    for r in rows.iter().take(n) {
        let mut l = Line::default();
        match format {
            Format::Labels => {
                l.kind = cell(r, c(&["type"]));
                l.txid = cell(r, c(&["ref"]));
                l.label = cell(r, c(&["label"]));
            }
            Format::CoinTracking => {
                l.date = app_date(&cell(r, c(&["Date"])));
                l.wallet = cell(r, c(&["Exchange"]));
                l.kind = cell(r, c(&["Type"]));
                let buy = cell(r, c(&["Buy Amount"]));
                let (amount, asset, value, dir) = if !buy.is_empty() {
                    (
                        buy,
                        cell(r, c(&["Buy Currency"])),
                        cell(r, c(&["Buy Value in Account Currency"])),
                        Dir::In,
                    )
                } else {
                    let sell = cell(r, c(&["Sell Amount"]));
                    let dir = if sell.is_empty() {
                        dir_of(&l.kind)
                    } else {
                        Dir::Out
                    };
                    (
                        sell,
                        cell(r, c(&["Sell Currency"])),
                        cell(r, c(&["Sell Value in Account Currency"])),
                        dir,
                    )
                };
                l.dir = Some(dir);
                l.amount = amount;
                l.asset = asset;
                l.value = number(&value);
                l.txid = cell(r, c(&["Tx-ID"]));
            }
            Format::Blockpit => {
                l.date = app_date(&cell(r, c(&["Date (UTC)"])));
                l.wallet = cell(r, c(&["Integration Name"]));
                l.kind = cell(r, c(&["Label"]));
                let inc = cell(r, c(&["Incoming Amount"]));
                if !inc.is_empty() {
                    l.amount = inc;
                    l.asset = cell(r, c(&["Incoming Asset"]));
                    l.dir = Some(Dir::In);
                } else {
                    l.amount = cell(r, c(&["Outgoing Amount"]));
                    l.asset = cell(r, c(&["Outgoing Asset"]));
                    l.dir = Some(if l.amount.is_empty() {
                        Dir::Other
                    } else {
                        Dir::Out
                    });
                }
                l.txid = cell(r, c(&["Trx. ID (optional)", "Trx. ID"]));
            }
            Format::Xlsx | Format::Csv => {
                l.date = app_date(&cell(r, c(&["Occurred At", "Date"])));
                l.wallet = cell(r, c(&["Wallet"]));
                l.kind = cell(r, c(&["Direction"]));
                l.dir = Some(dir_of(&l.kind));
                l.amount = cell(r, c(&["Amount"]));
                l.asset = cell(r, c(&["Asset"]));
                l.value = number(&cell(r, c(&["Fiat Value", "Value"])));
                l.currency = cell(r, c(&["Fiat Currency"]));
                l.txid = cell(r, c(&["Transaction ID", "Txid"]));
            }
        }
        out.push(l);
    }
    out
}

/// Whether the summary has a fiat value column (Blockpit's template has none).
pub fn has_value(format: Format) -> bool {
    !matches!(format, Format::Blockpit | Format::Labels)
}

/// `+ ₿ 0.08000000` (BTC and L-BTC), else `+ 12.5 USDT`; the file's digits as they are.
pub fn amount_text(l: &Line) -> String {
    if l.amount.is_empty() {
        return String::new();
    }
    let sign = match l.dir {
        Some(Dir::In) => "+ ",
        Some(Dir::Out) => "\u{2212} ",
        _ => "",
    };
    let digits = l.amount.trim_start_matches(['-', '+']);
    let a = l.asset.to_ascii_uppercase();
    if a.is_empty() || a == "BTC" || a == "LBTC" || a == "L-BTC" {
        format!("{sign}₿ {digits}")
    } else {
        format!("{sign}{digits} {}", l.asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kassiber_sheet_maps_by_column_name() {
        let sheet = json!({
            "columns": ["Occurred At", "Wallet", "Transaction ID", "Direction", "Asset", "Amount", "Fiat Currency", "Fiat Value"],
            "rows": [["2019-01-15T09:20:00Z", "Treasury", "23128f54767bd0e388598d33e997a4595beb5afca72326200504d1baf6fe9a3f", "outbound", "BTC", "0.08000000", "EUR", "3,197.60"]]
        });
        let l = &lines(Format::Xlsx, &sheet, 6)[0];
        assert_eq!(l.date, "2019-01-15 09:20");
        assert_eq!(l.wallet, "Treasury");
        assert_eq!(l.dir, Some(Dir::Out));
        assert_eq!(l.value, Some(3197.60));
        assert_eq!(l.currency, "EUR");
        assert_eq!(amount_text(l), "\u{2212} ₿ 0.08000000");
        assert_eq!(short_ref(&l.txid), "23128f…9a3f");
    }

    #[test]
    fn tool_sheets_pick_the_filled_side() {
        let ct = json!({
            "columns": ["Type", "Buy Amount", "Buy Currency", "Sell Amount", "Sell Currency", "Exchange", "Date", "Tx-ID", "Buy Value in Account Currency", "Sell Value in Account Currency"],
            "rows": [["Withdrawal", "", "", "0.5", "BTC", "Cold", "2020-02-01 10:00:00", "ab", "", "4,000.5"]]
        });
        let l = &lines(Format::CoinTracking, &ct, 6)[0];
        assert_eq!(
            (l.dir, l.amount.as_str(), l.value, l.date.as_str()),
            (Some(Dir::Out), "0.5", Some(4000.5), "2020-02-01 10:00")
        );
        let bp = json!({
            "columns": ["Date (UTC)", "Integration Name", "Label", "Outgoing Asset", "Outgoing Amount", "Incoming Asset", "Incoming Amount", "Trx. ID (optional)"],
            "rows": [["15.01.2019 09:20:00", "Liquid", "Deposit", "", "", "LBTC", "0.08000000", "x"]]
        });
        let l = &lines(Format::Blockpit, &bp, 6)[0];
        assert_eq!(
            (l.dir, l.date.as_str(), amount_text(l)),
            (
                Some(Dir::In),
                "2019-01-15 09:20",
                "+ ₿ 0.08000000".to_owned()
            )
        );
    }

    #[test]
    fn labels_and_limits() {
        let s = json!({"columns": ["type", "ref", "label"], "rows": [["tx", "abc", "Rent"], ["tx", "def", "Coffee"]]});
        let ls = lines(Format::Labels, &s, 1);
        assert_eq!(ls.len(), 1);
        assert_eq!((ls[0].kind.as_str(), ls[0].label.as_str()), ("tx", "Rent"));
        assert_eq!(number(""), None);
        assert_eq!(app_date(""), "");
    }
}
