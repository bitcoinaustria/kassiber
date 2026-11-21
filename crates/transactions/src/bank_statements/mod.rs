//! Bank statements as source-of-funds evidence (docs/reference/bank-statements.md).
//!
//! The parsers are pure: they read the given bytes and nothing else. A parsed
//! line is evidence of what a bank booked; it never becomes a transaction,
//! journal row or balance. Refusals name a row, column or field and never
//! quote the file.
mod camt;
mod csv;

pub mod money;
pub mod store;

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
pub use money::Money;

pub type Result<T> = std::result::Result<T, AppError>;

/// Statement files above this size are refused before parsing.
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// More lines than this in one file are refused.
pub const MAX_LINES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Camt053,
    Csv,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Camt053 => "camt053",
            Format::Csv => "csv",
        }
    }

    /// `auto` (or nothing) detects; anything else must be a known name.
    pub fn choose(raw: Option<&str>) -> Result<Option<Format>> {
        match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("auto") => Ok(None),
            Some("camt053" | "camt.053" | "camt") => Ok(Some(Format::Camt053)),
            Some("csv") => Ok(Some(Format::Csv)),
            Some("mt940" | "mt-940") => Err(mt940_refused()),
            Some(other) => Err(invalid(format!("Unsupported statement format '{other}'"))
                .with_hint("Choose one of: auto, camt053, csv.")),
        }
    }
}

fn mt940_refused() -> AppError {
    invalid("MT940 statements are not supported")
        .with_hint("Download the statement as CAMT.053 (XML) or CSV from your bank.")
        .with_details(dict([("reason", s("bank_statement_mt940"))]))
}

/// One booked line, as read from the file.
#[derive(Clone, Debug, PartialEq)]
pub struct StatementLine {
    /// 1-based position in the file (CSV row, CAMT entry).
    pub position: usize,
    /// The own account this line was booked on, normalized.
    pub account: Option<String>,
    pub booking_date: String,
    pub value_date: Option<String>,
    /// Signed: outgoing transfers are negative.
    pub amount: Money,
    pub currency: String,
    pub counterparty_name: Option<String>,
    /// Normalized (no spaces, upper case). Stored masked only.
    pub counterparty_iban: Option<String>,
    pub remittance: Option<String>,
    pub bank_reference: Option<String>,
    pub end_to_end_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedStatement {
    pub format: Option<Format>,
    /// The own account when every line names the same one.
    pub account: Option<String>,
    pub currency: Option<String>,
    pub bank_name: Option<String>,
    pub bank_bic: Option<String>,
    pub statement_id: Option<String>,
    pub lines: Vec<StatementLine>,
    /// Position and Kassiber's reason for each row that is no booked line.
    pub skipped: Vec<(usize, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct ParseOptions {
    pub format: Option<Format>,
    /// CSV column map (see the reference).
    pub column_map: Option<Value>,
    /// Currency for files that state none.
    pub currency: Option<String>,
    /// The user's own account for files that do not name it per line.
    pub account: Option<String>,
}

pub(crate) fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// Parse one statement file. Fails closed on anything ambiguous.
pub fn parse(bytes: &[u8], options: &ParseOptions) -> Result<ParsedStatement> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(invalid(format!(
            "Statement file is larger than {} MiB",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    refuse_utf16(bytes)?;
    let text = decode(bytes);
    let default_currency = options
        .currency
        .as_deref()
        .map(|c| fiat(c, "--currency"))
        .transpose()?;
    let format = options.format.unwrap_or_else(|| detect(&text));
    if format != Format::Csv && options.column_map.is_some() {
        return Err(invalid("A column map applies only to CSV statements"));
    }
    if options.format.is_none() && looks_like_mt940(&text) {
        return Err(mt940_refused());
    }
    let mut parsed = match format {
        Format::Camt053 => camt::parse(&text)?,
        Format::Csv => csv::parse(&text, options.column_map.as_ref(), default_currency)?,
    };
    parsed.format = Some(format);
    // Every line belongs to one of the user's own accounts: the file names
    // it, or the user states it. Never guessed.
    let stated = options
        .account
        .as_deref()
        .map(|a| account_id(a).ok_or_else(|| invalid("--account is empty")))
        .transpose()?;
    for line in &mut parsed.lines {
        match (&line.account, &stated) {
            (None, Some(stated)) => line.account = Some(stated.clone()),
            (None, None) => {
                return Err(invalid(format!(
                    "Line {}: the statement does not name the account it belongs to",
                    line.position
                ))
                .with_hint(
                    "Pass --account with the IBAN, account number or a label for this account.",
                )
                .with_details(dict([("reason", s("bank_statement_account_required"))])))
            }
            (Some(found), Some(stated)) if found != stated => {
                return Err(invalid(format!(
                    "Line {}: the statement names a different account than --account",
                    line.position
                )))
            }
            _ => {}
        }
    }
    let mut accounts: Vec<&String> = parsed
        .lines
        .iter()
        .filter_map(|l| l.account.as_ref())
        .collect();
    accounts.sort();
    accounts.dedup();
    parsed.account = match accounts.as_slice() {
        [one] => Some((*one).clone()),
        _ => None,
    };
    if parsed.lines.len() > MAX_LINES {
        return Err(invalid(format!(
            "Statement has more than {MAX_LINES} lines; split it into smaller files"
        )));
    }
    if parsed.lines.is_empty() {
        return Err(invalid("The statement contains no booked lines")
            .with_details(dict([("skipped", Value::Int(parsed.skipped.len().into()))])));
    }
    Ok(parsed)
}

/// A UTF-16 file (BOM, or NUL bytes near the start) is refused by name rather
/// than misread as Latin-1.
fn refuse_utf16(bytes: &[u8]) -> Result<()> {
    let head = &bytes[..bytes.len().min(4096)];
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) || head.contains(&0) {
        return Err(
            invalid("The statement file is UTF-16 encoded, which Kassiber does not read")
                .with_hint(
                    "Export or save the statement as UTF-8 (or Latin-1) and import it again.",
                )
                .with_details(dict([("reason", s("bank_statement_encoding"))])),
        );
    }
    Ok(())
}

/// UTF-8 (BOM optional), else Latin-1.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    }
}

fn detect(text: &str) -> Format {
    if text.trim_start().starts_with('<') {
        Format::Camt053
    } else {
        Format::Csv
    }
}

fn looks_like_mt940(text: &str) -> bool {
    let tagged = |tag: &str| text.lines().any(|l| l.trim_start().starts_with(tag));
    tagged(":20:") && tagged(":61:")
}

/// The fiat currencies Kassiber prices in (`kassiber_transactions::FIAT_CODES`):
/// any other code, a crypto asset or stablecoin included, is refused naming
/// where it was found. Only a code the user typed (`--currency`) is shown:
/// an error never quotes the statement file.
pub(crate) fn fiat(raw: &str, place: &str) -> Result<String> {
    let code = raw.trim().to_ascii_uppercase();
    if crate::FIAT_CODES.contains(&code.as_str()) {
        return Ok(code);
    }
    let typed = place.starts_with("--");
    let shown = if typed
        && (3..=5).contains(&code.len())
        && code.bytes().all(|b| b.is_ascii_alphabetic())
    {
        format!(" ({code})")
    } else {
        String::new()
    };
    Err(invalid(format!(
        "{place}: the currency{shown} is not a supported fiat currency"
    ))
    .with_hint("Bank statements are read in fiat only; Kassiber does not track other assets.")
    .with_details(dict([("reason", s("bank_statement_currency"))])))
}

/// `YYYY-MM-DD` when the calendar date exists.
pub(crate) fn iso_date(year: i32, month: u32, day: u32) -> Option<String> {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    ((1900..=2999).contains(&year) && (1..=days).contains(&day))
        .then(|| format!("{year:04}-{month:02}-{day:02}"))
}

/// Whitespace collapsed; empty becomes `None`.
pub(crate) fn collapse(raw: &str) -> Option<String> {
    let joined = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

/// An account identifier without spaces, upper-cased.
pub(crate) fn account_id(raw: &str) -> Option<String> {
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    (!compact.is_empty()).then(|| compact.to_ascii_uppercase())
}

/// IBAN shape: two letters, two digits, up to 30 alphanumerics.
pub fn is_iban(id: &str) -> bool {
    let b = id.as_bytes();
    (15..=34).contains(&b.len())
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..].iter().all(u8::is_ascii_alphanumeric)
}

/// The display form of an account: an IBAN keeps its country code and last
/// four characters in groups of four (`AT** **** **** **** 1234`); any other
/// account number keeps only its last four characters.
pub fn mask_account(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    let keep_tail = 4.min(chars.len());
    let head = if is_iban(id) { 2 } else { 0 };
    let masked: Vec<char> = chars
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if i < head || i >= chars.len() - keep_tail {
                *c
            } else {
                '*'
            }
        })
        .collect();
    if head == 0 {
        return masked.into_iter().collect();
    }
    masked
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The account's fixed short form for generated, stored text: `••` and its
/// last four characters.
pub fn account_short(id: &str) -> String {
    let chars: Vec<char> = id.chars().filter(char::is_ascii_alphanumeric).collect();
    let tail: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    format!("\u{2022}\u{2022}{tail}")
}

/// ISO 13616 check digits (mod 97 == 1).
pub fn iban_checksum_ok(candidate: &str) -> bool {
    if candidate.len() < 15 || !candidate.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    let upper = candidate.to_ascii_uppercase();
    let rearranged = format!("{}{}", &upper[4..], &upper[..4]);
    rearranged.chars().fold(0u32, |r, c| {
        let v = c.to_digit(36).expect("alphanumeric");
        if v >= 10 {
            (r * 100 + v) % 97
        } else {
            (r * 10 + v) % 97
        }
    }) == 1
}

/// Bank free text as an output shows it: every IBAN in it (compact or in
/// groups of four, valid check digits) masked like an account. Other
/// account numbers are not recognized (docs/reference/bank-statements.md#privacy).
pub fn mask_ibans(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < chars.len() {
        let starts = (i == 0 || !chars[i - 1].is_ascii_alphanumeric())
            && chars.len() >= i + 4
            && chars[i..i + 2].iter().all(char::is_ascii_alphabetic)
            && chars[i + 2..i + 4].iter().all(char::is_ascii_digit);
        if starts {
            // Alphanumerics from here, single spaces allowed between them.
            let mut compact = String::new();
            let mut ends = Vec::new();
            let mut j = i;
            while j < chars.len() && compact.len() < 34 {
                if chars[j].is_ascii_alphanumeric() {
                    compact.push(chars[j]);
                    ends.push(j + 1);
                    j += 1;
                } else if chars[j] == ' '
                    && chars.get(j + 1).is_some_and(char::is_ascii_alphanumeric)
                {
                    j += 1;
                } else {
                    break;
                }
            }
            for len in (15..=compact.len()).rev() {
                let end = ends[len - 1];
                let boundary = chars.get(end).is_none_or(|c| !c.is_ascii_alphanumeric());
                if boundary && iban_checksum_ok(&compact[..len]) {
                    out.push_str(&mask_account(&compact[..len].to_ascii_uppercase()));
                    i = end;
                    continue 'scan;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn shown(text: Option<&str>) -> Value {
    text.map_or(Value::Null, |t| s(mask_ibans(t)))
}

/// The summary a dry run or import returns: free text with IBANs masked,
/// amounts and dates exact.
pub fn line_preview(line: &StatementLine) -> Value {
    dict([
        ("position", Value::Int(line.position.into())),
        ("booking_date", s(line.booking_date.as_str())),
        (
            "value_date",
            line.value_date.as_deref().map_or(Value::Null, s),
        ),
        ("amount", s(line.amount.canonical())),
        ("currency", s(line.currency.as_str())),
        (
            "counterparty_name",
            shown(line.counterparty_name.as_deref()),
        ),
        (
            "counterparty_iban_masked",
            line.counterparty_iban
                .as_deref()
                .map_or(Value::Null, |v| s(mask_account(v))),
        ),
        ("remittance", shown(line.remittance.as_deref())),
        ("bank_reference", shown(line.bank_reference.as_deref())),
    ])
}
