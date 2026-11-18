//! Bitfinex Reports exports (report.bitfinex.com) as an active custodial
//! ledger: the trades, movements and ledgers CSVs, one at a time or together
//! in the Reports zip, each recognised by its exact header and mapped by the
//! API sync's rules, so both write the same records and ids. Only the
//! `YYYY-MM-DD` date format is read, as UTC. docs/reference/bitfinex.md.
use super::*;
use super::{map, mapping, Sources};
use exchange::Columns;
use kassiber_pyjson::PyStr;
use mapping::Fields as _;

/// The column sets `bfx-report` writes (`report.file.job.data.js`), sorted.
const TRADES: &str = "#,AMOUNT,DATE,FEE,FEE CURRENCY,FEE PERC,ORDER ID,PAIR,PRICE";
const MOVEMENTS: &str =
    "#,AMOUNT,CURRENCY,DESCRIPTION,FEES,NOTE,STARTED,STATUS,TRANSACTION ID,UPDATED";
const LEDGERS: &str = "#,AMOUNT,BALANCE,CURRENCY,DATE,DESCRIPTION,WALLET";

fn refused(message: &str, hint: &str) -> AppError {
    AppError::new("validation", format!("Bitfinex {message}")).with_hint(hint.to_owned())
}
const EXPORT: &str = "Export Trades, Movements and Ledgers as CSV from report.bitfinex.com and import the Reports zip, or each file as it is.";
const DATES: &str = "In Bitfinex Reports, set the date format to YYYY-MM-DD and the time zone to UTC, then export again.";

/// The import's files: the Reports zip's CSV members (at most 16), or the
/// one CSV.
fn files(file: &str) -> Result<Vec<String>, AppError> {
    let mut magic = [0u8; 4];
    let read = std::fs::File::open(file).and_then(|mut f| std::io::Read::read(&mut f, &mut magic));
    if !read.is_ok_and(|n| n == 4 && magic == *b"PK\x03\x04") {
        return Ok(vec![csv::read_text(file)?]);
    }
    let members = super::ledger::xlsx::archive(file)
        .map_err(|_| refused("Reports zip could not be read", EXPORT))?;
    let csvs: Vec<Vec<u8>> = members
        .into_iter()
        .filter(|(name, _)| name.to_ascii_lowercase().ends_with(".csv"))
        .map(|(_, bytes)| bytes)
        .collect();
    if csvs.is_empty() || csvs.len() > 16 {
        return Err(refused("Reports zip does not hold its CSV exports", EXPORT));
    }
    let utf8 = |bytes| {
        String::from_utf8(bytes).map_err(|_| refused("Reports CSV is not UTF-8 text", EXPORT))
    };
    let texts = csvs.into_iter().map(utf8);
    texts
        .map(|text| Ok(text?.trim_start_matches('\u{feff}').to_owned()))
        .collect()
}

/// The export's rows as events, mapped by the API sync's rules (Kassiber
/// reads only BTC from Bitfinex; other assets are counted).
pub(super) fn load(file: &str, mapper: &mut Mapper) -> Result<(), AppError> {
    let (mut trades, mut movements, mut ledger) = (None, None, None);
    for text in files(file)? {
        let mut rows = csv::rows(&text)?;
        let blank = |r: &csv::Row| {
            let values = r.data.as_object().into_iter().flat_map(|o| o.values());
            r.rest.is_none()
                && values
                    .filter_map(|v| v.as_str())
                    .all(|v| kassiber_pytext::strip(v).is_empty())
        };
        rows.retain(|r| !blank(r));
        let Some(first) = rows.first() else { continue };
        if rows.iter().any(|r| r.rest.is_some()) {
            return Err(refused(
                "CSV has a row with more fields than its header",
                EXPORT,
            ));
        }
        let mut headers: Vec<&str> = first
            .headers
            .iter()
            .map(|h| kassiber_pytext::strip(h))
            .collect();
        headers.sort_unstable();
        let columns = rows.iter().map(Columns::unstripped);
        match headers.join(",").as_str() {
            TRADES => extend(&mut trades, columns.map(|c| mapping::read_trade(&Row(&c))))?,
            MOVEMENTS => extend(
                &mut movements,
                columns.map(|c| mapping::read_movement(&Row(&c))),
            )?,
            LEDGERS => extend(&mut ledger, columns.map(|c| mapping::read_ledger(&Row(&c))))?,
            _ => {
                return Err(refused(
                    "CSV is not a Reports trades, movements or ledgers export",
                    EXPORT,
                ))
            }
        }
    }
    let sources = Sources {
        ledger: ledger.as_deref(),
        trades: trades.as_deref(),
        movements: movements.as_deref(),
        spot_proof_required: true,
    };
    map(&sources, mapper)
}
fn extend<T>(
    source: &mut Option<Vec<T>>,
    rows: impl Iterator<Item = Result<T, AppError>>,
) -> Result<(), AppError> {
    let rows = rows.collect::<Result<Vec<_>, _>>()?;
    source.get_or_insert_with(Vec::new).extend(rows);
    Ok(())
}

/// One CSV row, read strictly by column name.
struct Row<'a>(&'a Columns);
impl Row<'_> {
    fn invalid(&self, column: &str) -> AppError {
        let value: String = self.text((0, column)).chars().take(80).collect();
        AppError::new(
            "validation",
            format!("Bitfinex CSV has an invalid {column} value '{value}'"),
        )
    }
}
impl mapping::Fields for Row<'_> {
    fn id(&self) -> Result<String, AppError> {
        let value = self.text((0, "#"));
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        digits.then_some(value).ok_or_else(|| self.invalid("#"))
    }
    fn text(&self, (_, column): (usize, &str)) -> String {
        let value = self.0.get(&[column]).unwrap_or_default();
        kassiber_pytext::strip(&value).to_owned()
    }
    fn number(&self, (_, column): (usize, &str), optional: bool) -> Result<Decimal, AppError> {
        match self.text((0, column)) {
            value if optional && value.is_empty() => Ok(Decimal::zero()),
            value => strict_decimal(&value, true).ok_or_else(|| self.invalid(column)),
        }
    }
    /// `YYYY-MM-DD HH:mm:ss`, optionally `.SSS`, read as UTC, stored in
    /// whole seconds as the API's millisecond times are.
    fn time(&self, (_, column): (usize, &str)) -> Result<PyStr, AppError> {
        let value = self.text((0, column));
        let shape = |(i, b): (usize, &u8)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b' ',
            13 | 16 => *b == b':',
            19 => *b == b'.',
            _ => b.is_ascii_digit(),
        };
        let (date, clock) = (
            value.get(..10).unwrap_or(""),
            value.get(11..19).unwrap_or(""),
        );
        let iso = format!("{date}T{clock}Z");
        let shaped =
            matches!(value.len(), 19 | 23) && value.as_bytes().iter().enumerate().all(shape);
        if shaped && crate::time::timestamp(&iso).is_ok() {
            return Ok(iso.into());
        }
        let shown: String = value.chars().take(40).collect();
        let message = format!("CSV has a {column} value '{shown}' that is not YYYY-MM-DD HH:mm:ss");
        Err(refused(&message, DATES))
    }
    fn api_only(&self, _: usize) -> Option<String> {
        None
    }
    fn raw(&self) -> Py {
        kassiber_pytext::loads(&config::stored_json(&self.0.json())).unwrap_or(Py::Null)
    }
}
