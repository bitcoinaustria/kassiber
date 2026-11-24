//! Bank CSV exports: one generic reader. Header names (German and English
//! bank exports) find the columns; a column map names them when they do not.
//! Every value is read from the file; nothing ambiguous is guessed.
use super::{
    account_id, collapse, fiat, invalid, iso_date, Money, ParsedStatement, Result, StatementLine,
};
use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;

/// Rows searched for the header line (bank preambles are short).
const HEADER_SEARCH_ROWS: usize = 40;

/// Each field with its recognized headers (normalized, space-separated).
const FIELDS: &[(&str, &str)] = &[
    ("account", ""),
    ("booking_date", "buchungsdatum buchungstag bookingdate transactiondate completeddate buchung datum date"),
    ("value_date", "valuta valutadatum wertstellung wertstellungsdatum valuedate"),
    ("amount", "betrag umsatz amount"),
    ("debit", "soll debit belastung ausgang"),
    ("credit", "haben credit gutschrift eingang"),
    ("direction", "sollhaben debitcredit creditdebit"),
    ("currency", "waehrung wkz waehrungscode currency"),
    ("counterparty", "auftraggeberempfaenger empfaengerauftraggeber beguenstigterzahlungspflichtiger zahlungsempfaenger zahlungsbeteiligter empfaenger auftraggeber partnername counterparty payee name"),
    ("counterparty_iban", "partneriban counterpartyiban empfaengeriban ibanempfaenger ibanauftraggeber kontonummeriban iban"),
    ("remittance", "verwendungszweck zahlungsreferenz buchungstext paymentreference reference description purpose"),
    ("reference", "bankreferenz buchungsreferenz bankreference transactionid"),
];
const OPTIONS: &[&str] = &["decimal", "date_format"];
const DATE_FORMATS: &str =
    "YYYY-MM-DD YYYY/MM/DD DD.MM.YYYY DD.MM.YY DD/MM/YYYY MM/DD/YYYY DD-MM-YYYY MM-DD-YYYY";

/// Lower case, umlauts folded, only letters and digits.
fn normalize(header: &str) -> String {
    let mut out = String::new();
    for c in header.trim().chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// A currency named in an amount header: `Betrag (EUR)`, `Amount in EUR`.
fn header_currency(header: &str) -> Option<(String, String)> {
    let trimmed = header.trim();
    let (stem, code) = if let Some(open) = trimmed.rfind('(') {
        let close = trimmed[open..].find(')')? + open;
        (&trimmed[..open], &trimmed[open + 1..close])
    } else {
        let (stem, code) = trimmed.rsplit_once(' ')?;
        (stem.strip_suffix(" in").or(stem.strip_suffix(" In"))?, code)
    };
    let code = code.trim();
    (code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()))
        .then(|| (normalize(stem), code.to_owned()))
}

/// CSV rows with one delimiter; quotes may span lines.
struct Table {
    rows: Vec<Vec<String>>,
    /// A quoted field is still open at the end of the file: it swallowed
    /// every row after it, and only the rows before it are returned.
    open: bool,
    /// The first row (index) with text after a closing quote (`"10"0`): the
    /// field's value can't be read.
    after_quote: Option<usize>,
}

fn rows(text: &str, delimiter: char) -> Table {
    let (mut rows, mut row, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let (mut closed, mut after_quote) = (false, None);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => {
                    quoted = false;
                    closed = true;
                }
                c => field.push(c),
            }
            continue;
        }
        if closed && c != delimiter && !matches!(c, '\r' | '\n' | ' ') {
            after_quote.get_or_insert(rows.len());
        }
        match c {
            '"' if field.trim().is_empty() => {
                field.clear();
                quoted = true;
            }
            c if c == delimiter => {
                closed = false;
                row.push(std::mem::take(&mut field));
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                closed = false;
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !quoted && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    Table {
        rows,
        open: quoted,
        after_quote,
    }
}

/// Column indices per field; `remittance` may name several.
#[derive(Default)]
struct Plan {
    columns: Vec<(&'static str, Vec<usize>)>,
    header_currency: Option<String>,
}

impl Plan {
    fn one(&self, field: &str) -> Option<usize> {
        self.all(field).first().copied()
    }
    fn all(&self, field: &str) -> &[usize] {
        self.columns
            .iter()
            .find(|(f, _)| *f == field)
            .map_or(&[], |(_, c)| c.as_slice())
    }
    fn usable(&self) -> bool {
        self.one("booking_date").is_some()
            && (self.one("amount").is_some()
                || self.one("debit").is_some() && self.one("credit").is_some())
    }
}

/// The plan the headers suggest, and the first ambiguity: a field two
/// columns could be is refused (when the row is a usable header), never
/// resolved by picking one.
fn detect_plan(header: &[String]) -> (Plan, Option<String>) {
    let mut plan = Plan::default();
    let mut ambiguity = None;
    let mut stems: Vec<String> = header.iter().map(|h| normalize(h)).collect();
    for (i, h) in header.iter().enumerate() {
        if let Some((stem, code)) =
            header_currency(h).filter(|(stem, _)| FIELDS[3].1.split(' ').any(|a| a == stem))
        {
            if plan.header_currency.as_ref().is_some_and(|c| *c != code) {
                ambiguity.get_or_insert("Amount headers name different currencies".to_owned());
            }
            stems[i] = stem;
            plan.header_currency.get_or_insert(code);
        }
    }
    let mut used = vec![false; header.len()];
    for (field, aliases) in FIELDS {
        let mut found: Vec<usize> = (0..header.len())
            .filter(|i| !used[*i] && aliases.split(' ').any(|a| !a.is_empty() && a == stems[*i]))
            .collect();
        if *field != "remittance" && found.len() > 1 {
            let columns: Vec<String> = found.iter().map(|i| (i + 1).to_string()).collect();
            ambiguity.get_or_insert(format!(
                "Several columns could be {field}: columns {}",
                columns.join(", ")
            ));
            found.truncate(1);
        }
        for i in &found {
            used[*i] = true;
        }
        if !found.is_empty() {
            plan.columns.push((field, found));
        }
    }
    (plan, ambiguity)
}

fn ambiguous(message: String) -> kassiber_envelope::AppError {
    invalid(message)
        .with_hint(
            "Pass a column map naming the column to use by its number, e.g. {\"amount\": 3}.",
        )
        .with_details(dict([("reason", s("bank_csv_ambiguous_columns"))]))
}

/// A map names the file's own headers, or a column by its 1-based number.
/// `None` when this row lacks a named header.
fn mapped_plan(header: &[String], map: &[(String, Value)]) -> Result<Option<Plan>> {
    let mut plan = Plan::default();
    let mut claimed: Vec<usize> = Vec::new();
    for (key, value) in map.iter().filter(|(k, _)| !OPTIONS.contains(&k.as_str())) {
        let field = FIELDS.iter().map(|(f, _)| *f).find(|f| f == key).ok_or_else(|| {
            invalid(format!("Unknown column-map field '{key}'"))
                .with_hint("Fields: account, booking_date, value_date, amount, debit, credit, direction, currency, counterparty, counterparty_iban, remittance, reference; options: decimal, date_format.")
        })?;
        let names = match value {
            Value::List(items) if field == "remittance" => items.clone(),
            other => vec![other.clone()],
        };
        let mut indices = Vec::new();
        for name in names {
            let index = match name {
                Value::Int(n) => match usize::try_from(n)
                    .ok()
                    .filter(|n| (1..=header.len()).contains(n))
                {
                    Some(n) => n - 1,
                    None => return Ok(None),
                },
                Value::Str(t) => {
                    let name = t.as_str().unwrap_or_default().trim().to_owned();
                    let matches: Vec<usize> = (0..header.len())
                        .filter(|i| header[*i].trim() == name)
                        .collect();
                    match matches.as_slice() {
                        [] => return Ok(None),
                        [one] => *one,
                        _ => {
                            return Err(invalid(format!(
                                "The column the map names for {field} appears {} times",
                                matches.len()
                            ))
                            .with_hint("Name the column by its number instead, e.g. 3.")
                            .with_details(dict([("reason", s("bank_csv_ambiguous_columns"))])))
                        }
                    }
                }
                _ => {
                    return Err(invalid(format!(
                        "Column-map field '{key}' must name a header or a column number"
                    )))
                }
            };
            if claimed.contains(&index) {
                return Err(invalid(format!("Column {} is mapped twice", index + 1)));
            }
            claimed.push(index);
            indices.push(index);
        }
        plan.columns.push((field, indices));
    }
    plan.header_currency = plan
        .one("amount")
        .and_then(|i| header_currency(&header[i]))
        .map(|(_, code)| code);
    if !plan.usable() {
        return Err(invalid(
            "A column map needs booking_date plus amount, or both debit and credit",
        ));
    }
    Ok(Some(plan))
}

/// The columns a map names only by number need this many cells; `None`
/// when any mapped field names a header, which checks the row itself.
fn positional_width(map: &[(String, Value)]) -> Option<usize> {
    let mut width = 0;
    for (_, value) in map.iter().filter(|(k, _)| !OPTIONS.contains(&k.as_str())) {
        let items = match value {
            Value::List(items) => items.clone(),
            other => vec![other.clone()],
        };
        for item in items {
            let Value::Int(n) = item else { return None };
            width = width.max(usize::try_from(n).ok()?);
        }
    }
    (width > 0).then_some(width)
}

/// Table, header row and plan: the first delimiter under which
/// an early row reads as a usable header.
fn choose(text: &str, map: &[(String, Value)]) -> Result<(Vec<Vec<String>>, usize, Plan)> {
    let mapped = map.iter().any(|(k, _)| !OPTIONS.contains(&k.as_str()));
    for delimiter in [';', '\t', ','] {
        let Table {
            rows: table,
            open,
            after_quote,
        } = rows(text, delimiter);
        for index in 0..table.len().min(HEADER_SEARCH_ROWS) {
            let header = &table[index];
            if header.len() < 2 {
                continue;
            }
            let plan = match mapped {
                true => mapped_plan(header, map)?,
                false => match detect_plan(header) {
                    (plan, Some(message)) if plan.usable() => return Err(ambiguous(message)),
                    (plan, _) => Some(plan),
                },
            };
            if let Some(plan) = plan.filter(Plan::usable) {
                if open {
                    return Err(invalid(
                        "A quoted field is never closed, so the rows after it can't be read",
                    )
                    .with_hint("Export the statement again unchanged.")
                    .with_details(dict([("reason", s("bank_csv_unterminated_quote"))])));
                }
                // Rows above the header are never read; from it on, a value
                // with text after its closing quote is refused, not guessed.
                if let Some(row) = after_quote.filter(|row| *row >= index) {
                    return Err(invalid(format!(
                        "Row {} has text after a closing quote, so its value can't be read",
                        row + 1
                    ))
                    .with_hint("Export the statement again unchanged.")
                    .with_details(dict([("reason", s("bank_csv_text_after_quote"))])));
                }
                // A map of column numbers can't tell a header from a data
                // row: when this delimiter skipped a row another delimiter
                // splits into every mapped column, the file's own delimiter
                // may be the other one (punctuation inside a description),
                // and reading on would take the wrong cells as amounts.
                if let Some(width) = mapped.then(|| positional_width(map)).flatten() {
                    let skipped = [';', '\t', ',']
                        .into_iter()
                        .filter(|other| *other != delimiter)
                        .any(|other| {
                            rows(text, other)
                                .rows
                                .iter()
                                .take(index)
                                .any(|row| row.len() >= width)
                        });
                    if skipped {
                        return Err(invalid(
                            "The file's delimiter is ambiguous for a column map of column numbers",
                        )
                        .with_hint("Name the columns by their header text in the column map, e.g. {\"booking_date\": \"Date\", \"amount\": \"Amount\"}.")
                        .with_details(dict([("reason", s("bank_csv_ambiguous_delimiter"))])));
                    }
                }
                return Ok((table, index, plan));
            }
        }
    }
    Err(match mapped {
        true => invalid("No row in the file has every column the map names"),
        false => invalid("The CSV columns were not recognized as a bank statement").with_hint(
            "Pass a column map naming at least booking_date and amount (or debit and credit).",
        ),
    }
    .with_details(dict([("reason", s("bank_csv_unrecognized"))])))
}

/// Three numeric parts of a date cell and their separator (time ignored).
fn parts(raw: &str) -> Option<(char, Vec<&str>)> {
    let date = raw.trim().split([' ', 'T']).next()?;
    let separator = date.chars().find(|c| matches!(c, '.' | '/' | '-'))?;
    let p: Vec<&str> = date.split(separator).collect();
    let numeric = p
        .iter()
        .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
    (p.len() == 3 && numeric).then_some((separator, p))
}

/// A cell that starts like a date (a digit, then a date separator) without
/// reading as one: a transaction row whose date is malformed, never a footer
/// label to skip.
fn date_like(raw: &str) -> bool {
    let date = raw.trim().split([' ', 'T']).next().unwrap_or("");
    date.starts_with(|c: char| c.is_ascii_digit()) && date.contains(['.', '/', '-'])
}

/// A date cell in one named format.
fn read_date(raw: &str, format: &str) -> Option<String> {
    let (separator, p) = parts(raw)?;
    let n = |v: &str| v.parse::<u32>().ok();
    let year = |v: &str, len: usize| (v.len() == len).then(|| v.parse::<i32>().ok()).flatten();
    let (y, m, d) = match (format, separator) {
        ("YYYY-MM-DD", '-') | ("YYYY/MM/DD", '/') => (year(p[0], 4)?, n(p[1])?, n(p[2])?),
        ("DD.MM.YYYY", '.') | ("DD/MM/YYYY", '/') | ("DD-MM-YYYY", '-') => {
            (year(p[2], 4)?, n(p[1])?, n(p[0])?)
        }
        ("DD.MM.YY", '.') => (2000 + year(p[2], 2)?, n(p[1])?, n(p[0])?),
        ("MM/DD/YYYY", '/') | ("MM-DD-YYYY", '-') => (year(p[2], 4)?, n(p[0])?, n(p[1])?),
        _ => return None,
    };
    iso_date(y, m, d)
}

/// The one date format every date-shaped cell fits; day/month ambiguity
/// no row settles is refused.
fn decide_dates(cells: &[String], column: usize) -> Result<Option<&'static str>> {
    let cells: Vec<&String> = cells.iter().filter(|c| parts(c).is_some()).collect();
    if cells.is_empty() {
        return Ok(None);
    }
    let fitting: Vec<&str> = DATE_FORMATS
        .split(' ')
        .filter(|f| cells.iter().all(|c| read_date(c, f).is_some()))
        .collect();
    match fitting.as_slice() {
        [one] => Ok(Some(one)),
        [] => Err(invalid(format!(
            "Column {column} mixes date formats or holds invalid dates"
        ))
        .with_hint("Set date_format in the column map, e.g. \"DD.MM.YYYY\".")),
        _ => Err(invalid(format!(
            "Dates in column {column} could be day-first or month-first and no row decides"
        ))
        .with_hint("Set date_format in the column map: \"DD/MM/YYYY\" or \"MM/DD/YYYY\".")),
    }
}

/// An amount cell: sign, digits with `.`/`,`, and an optional currency code.
struct Cell {
    negative: bool,
    digits: String,
    currency: Option<String>,
}

/// `None` when the cell is not an amount; a code that is no fiat currency
/// is refused.
fn cell(raw: &str, row: usize) -> Result<Option<Cell>> {
    let mut text = raw.trim().to_owned();
    let letters: String = text.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let currency = match letters.len() {
        0 => None,
        3 => Some(fiat(&letters, &format!("Row {row}"))?),
        _ => return Ok(None),
    };
    text.retain(|c| !c.is_ascii_alphabetic());
    let mut text = text.trim().to_owned();
    let mut negative = false;
    if let Some(rest) = text.strip_suffix('-').or_else(|| text.strip_prefix('-')) {
        negative = true;
        text = rest.trim().to_owned();
    } else if let Some(rest) = text.strip_prefix('+') {
        text = rest.trim().to_owned();
    }
    let digits_only = !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b',');
    Ok(digits_only.then_some(Cell {
        negative,
        digits: text,
        currency,
    }))
}

/// The decimal mark one cell implies: `Some(None)` ambiguous (`1.000`),
/// `None` no mark.
fn vote(digits: &str) -> Option<Option<char>> {
    let (commas, dots) = (digits.matches(',').count(), digits.matches('.').count());
    let other = |mark| if mark == ',' { '.' } else { ',' };
    match (commas, dots) {
        (0, 0) => None,
        (_, 0) | (0, _) => {
            let mark = if commas > 0 { ',' } else { '.' };
            let after = digits.len() - digits.rfind(mark).unwrap() - 1;
            Some(match commas + dots {
                1 if after == 3 => None,
                1 => Some(mark),
                // Repeated marks are grouping, so the other one is decimal.
                _ => Some(other(mark)),
            })
        }
        _ => Some(Some(if digits.rfind(',') > digits.rfind('.') {
            ','
        } else {
            '.'
        })),
    }
}

/// Grouping only between groups of exactly three digits after a leading
/// group of one to three (`1.234.567,89`), never after the decimal mark.
fn read_amount(cell: &Cell, decimal: char) -> Option<Money> {
    let grouping = if decimal == ',' { '.' } else { ',' };
    let (integer, fraction) = cell
        .digits
        .split_once(decimal)
        .unwrap_or((&cell.digits, ""));
    let groups: Vec<&str> = integer.split(grouping).collect();
    let grouped = groups.len() == 1
        || (1..=3).contains(&groups[0].len()) && groups[1..].iter().all(|g| g.len() == 3);
    if !grouped || fraction.contains([',', '.']) {
        return None;
    }
    let value = Money::parse(&format!(
        "{}{}{fraction}",
        groups.concat(),
        if cell.digits.contains(decimal) {
            "."
        } else {
            ""
        }
    ))?;
    Some(if cell.negative {
        value.negated()
    } else {
        value
    })
}

fn text_option<'a>(map: &'a [(String, Value)], key: &str) -> Result<Option<&'a str>> {
    match map.iter().find(|(k, _)| k == key).map(|(_, v)| v) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(v)) => Ok(v.as_str()),
        Some(_) => Err(invalid(format!("Column-map option '{key}' must be text"))),
    }
}

pub(super) fn parse(
    text: &str,
    column_map: Option<&Value>,
    default_currency: Option<String>,
) -> Result<ParsedStatement> {
    let map: Vec<(String, Value)> = match column_map {
        None => Vec::new(),
        Some(Value::Dict(pairs)) => pairs
            .iter()
            .map(|(k, v)| match k {
                kassiber_pyjson::Key::Str(t) => {
                    (t.as_str().unwrap_or_default().to_owned(), v.clone())
                }
                _ => (String::new(), v.clone()),
            })
            .collect(),
        Some(_) => return Err(invalid("A column map must be a JSON object")),
    };
    let decimal_option = match text_option(&map, "decimal")? {
        None => None,
        Some(",") => Some(','),
        Some(".") => Some('.'),
        Some(_) => return Err(invalid("Column-map decimal must be \",\" or \".\"")),
    };
    let date_option = match text_option(&map, "date_format")? {
        None => None,
        Some(v) => Some(
            DATE_FORMATS
                .split(' ')
                .find(|f| f.eq_ignore_ascii_case(v.trim()))
                .ok_or_else(|| {
                    invalid("Unsupported date_format")
                        .with_hint(format!("Use one of: {}.", DATE_FORMATS.replace(' ', ", ")))
                })?,
        ),
    };
    let (table, header_index, plan) = choose(text, &map)?;
    let body: Vec<(usize, &Vec<String>)> = table
        .iter()
        .enumerate()
        .skip(header_index + 1)
        .filter(|(_, r)| r.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, r)| (i + 1, r))
        .collect();
    // A row with values past the header's last column has an unquoted
    // delimiter in a value (`1,234.56` under a comma delimiter): its cells no
    // longer line up with the headers, so it is refused, not read shifted.
    // Empty trailing cells (a delimiter at the end of a row) are harmless.
    let width = table[header_index].len();
    if let Some((row, _)) = body
        .iter()
        .find(|(_, r)| r.iter().skip(width).any(|c| !c.trim().is_empty()))
    {
        return Err(invalid(format!(
            "Row {row} has more values than the header has columns, so its values can't be matched to columns"
        ))
        .with_hint("Export the statement again unchanged; a value containing the delimiter must be quoted.")
        .with_details(dict([("reason", s("bank_csv_row_too_wide"))])));
    }
    let get = |row: &Vec<String>, column: Option<usize>| {
        column
            .and_then(|c| row.get(c))
            .map_or(String::new(), |v| v.trim().to_owned())
    };
    let at = |column: usize, field: &str| format!("column {} ({field})", column + 1);

    // Conventions decided over the whole file before any row is read.
    let booking = plan.one("booking_date").expect("usable plan");
    let value_column = plan.one("value_date");
    let date_format = match date_option {
        Some(f) => Some(f),
        None => {
            let cells: Vec<String> = body
                .iter()
                .flat_map(|(_, r)| [get(r, Some(booking)), get(r, value_column)])
                .collect();
            decide_dates(&cells, booking + 1)?
        }
    };
    let amount_columns: Vec<usize> = ["amount", "debit", "credit"]
        .iter()
        .filter_map(|f| plan.one(f))
        .collect();
    let decimal = match decimal_option {
        Some(d) => d,
        None => {
            let (mut marks, mut ambiguous) = (Vec::new(), None);
            for (row, cells) in &body {
                for column in &amount_columns {
                    match cell(&get(cells, Some(*column)), *row)
                        .ok()
                        .flatten()
                        .and_then(|c| vote(&c.digits))
                    {
                        Some(Some(mark)) if !marks.contains(&mark) => marks.push(mark),
                        Some(None) => {
                            ambiguous.get_or_insert((*row, *column));
                        }
                        _ => {}
                    }
                }
            }
            match (marks.as_slice(), ambiguous) {
                ([_, _], _) => {
                    return Err(invalid("Amounts use both a decimal comma and a decimal point")
                        .with_hint("Set decimal in the column map to \",\" or \".\"."))
                }
                ([mark], _) => *mark,
                (_, None) => '.',
                (_, Some((row, column))) => {
                    return Err(invalid(format!(
                        "Row {row}, {}: the amount could use a thousands separator or a decimal mark, and no row decides",
                        at(column, "amount")
                    ))
                    .with_hint("Set decimal in the column map to \",\" or \".\"."))
                }
            }
        }
    };

    let mut parsed = ParsedStatement::default();
    for (row_number, row) in body {
        let date_cell = get(row, Some(booking));
        let amounts: Vec<(usize, String)> = amount_columns
            .iter()
            .map(|c| (*c, get(row, Some(*c))))
            .collect();
        let Some(date) = date_format.and_then(|f| read_date(&date_cell, f)) else {
            if !date_cell.is_empty()
                && amounts.iter().any(|(_, v)| !v.is_empty())
                && (parts(&date_cell).is_some() || date_like(&date_cell))
            {
                return Err(invalid(format!(
                    "Row {row_number}, {}: the cell is not a valid date",
                    at(booking, "booking_date")
                )));
            }
            parsed.skipped.push((
                row_number,
                "no booking date (not a transaction row)".to_owned(),
            ));
            continue;
        };
        let read = |column: usize, raw: &str| -> Result<Option<(Money, Option<String>)>> {
            if raw.is_empty() {
                return Ok(None);
            }
            let refused = || {
                invalid(format!(
                    "Row {row_number}, {}: the cell is not an amount",
                    at(column, "amount")
                ))
            };
            let c = cell(raw, row_number)?.ok_or_else(refused)?;
            let value = read_amount(&c, decimal).ok_or_else(refused)?;
            Ok(Some((value, c.currency)))
        };
        let (amount, embedded) = if let Some(column) = plan.one("amount") {
            let (value, code) = read(column, &get(row, Some(column)))?.ok_or_else(|| {
                invalid(format!(
                    "Row {row_number}, {}: amount is empty",
                    at(column, "amount")
                ))
            })?;
            let value = match normalize(&get(row, plan.one("direction"))).as_str() {
                "" | "h" | "haben" | "c" | "cr" | "credit" | "gutschrift" => value,
                "s" | "soll" | "d" | "dr" | "debit" | "lastschrift" => value.abs().negated(),
                _ => {
                    return Err(invalid(format!(
                        "Row {row_number}: the debit/credit mark is not recognized"
                    )))
                }
            };
            (value, code)
        } else {
            let (debit_column, credit_column) = (
                plan.one("debit").expect("usable"),
                plan.one("credit").expect("usable"),
            );
            match (
                read(debit_column, &get(row, Some(debit_column)))?,
                read(credit_column, &get(row, Some(credit_column)))?,
            ) {
                (Some((d, code)), None) => (d.abs().negated(), code),
                (None, Some((c, code))) => (c.abs(), code),
                (Some((d, _)), Some((c, code))) if d.is_zero() => (c.abs(), code),
                (Some((d, code)), Some((c, _))) if c.is_zero() => (d.abs().negated(), code),
                (None, None) => {
                    return Err(invalid(format!(
                        "Row {row_number}: both debit and credit are empty"
                    )))
                }
                _ => {
                    return Err(invalid(format!(
                        "Row {row_number}: both debit and credit carry an amount"
                    )))
                }
            }
        };
        // Every place that states a currency must agree.
        let currency_cell = get(row, plan.one("currency"));
        let mut declared: Vec<(String, &str)> = Vec::new();
        if !currency_cell.is_empty() {
            declared.push((
                fiat(&currency_cell, &format!("Row {row_number}"))?,
                "currency column",
            ));
        }
        if let Some(code) = embedded {
            declared.push((code, "amount"));
        }
        if let Some(code) = &plan.header_currency {
            declared.push((fiat(code, "The amount header")?, "amount header"));
        }
        if let Some(code) = &default_currency {
            declared.push((code.clone(), "--currency"));
        }
        if let Some((other, source)) = declared.iter().find(|(c, _)| *c != declared[0].0) {
            return Err(invalid(format!(
                "Row {row_number}: the {} says {} but the {source} says {other}",
                declared[0].1, declared[0].0
            )));
        }
        let (currency, _) = declared.into_iter().next().ok_or_else(|| {
            invalid(format!(
                "Row {row_number}: the statement does not state a currency"
            ))
            .with_hint("Pass --currency, or map a currency column.")
        })?;
        let value_raw = get(row, value_column);
        let value_date = match value_raw.is_empty() {
            true => None,
            false => Some(
                date_format
                    .and_then(|f| read_date(&value_raw, f))
                    .ok_or_else(|| {
                        invalid(format!(
                            "Row {row_number}, {}: the cell is not a valid date",
                            at(value_column.unwrap_or(0), "value_date")
                        ))
                    })?,
            ),
        };
        let remittance: Vec<String> = plan
            .all("remittance")
            .iter()
            .filter_map(|c| collapse(&get(row, Some(*c))))
            .collect();
        parsed.lines.push(StatementLine {
            position: row_number,
            account: account_id(&get(row, plan.one("account"))),
            booking_date: date,
            value_date,
            amount,
            currency,
            counterparty_name: collapse(&get(row, plan.one("counterparty"))),
            counterparty_iban: account_id(&get(row, plan.one("counterparty_iban"))),
            remittance: collapse(&remittance.join(" ")),
            bank_reference: collapse(&get(row, plan.one("reference"))),
            end_to_end_id: None,
        });
    }
    Ok(parsed)
}
