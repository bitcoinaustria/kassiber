//! Austrian rules: the markers Kassiber writes into `notes`, lot regimes,
//! the Spekulationsfrist, and disposal classification.
//!
//! This ports the owner-authored `rp2/plugin/country/at.py`. Every parse can
//! fail with RP2's message, and callers consult the results in the order RP2
//! evaluates them, so the first failure is RP2's first failure.

use std::cmp::Ordering;
use std::fmt::Write as _;

use crate::cursor::Fragment;
use crate::decimal::Decimal;
use crate::entry::{Entry, TransactionType};
use crate::error::{EngineError, EngineResult};
use crate::methods::LotView;
use crate::num::{cmp13, zero};
use crate::time::{CivilDate, Timestamp};

/// `AT_NEU_CUTOFF`: 2021-03-01 00:00 Europe/Vienna (CET), which is
/// 2021-02-28T23:00:00Z, in microseconds since the Unix epoch. Lots acquired
/// strictly before it are Altvermögen.
pub(crate) const NEU_CUTOFF_US: i64 = 1_614_553_200_000_000;

/// `AT_DEFAULT_POOL`: the pool of a note without an `at_pool=` value.
pub(crate) const DEFAULT_POOL: &str = "default";

const REGIME_MARKER_ALT: &str = "at_regime=alt";
const REGIME_MARKER_NEU: &str = "at_regime=neu";
const POOL_MARKER: &str = "at_pool=";
const SWAP_MARKER: &str = "at_swap_link=";

/// An Austrian holding regime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Regime {
    /// Altvermögen: acquired before the cutoff; FIFO at lot basis.
    Alt,
    /// Neuvermögen: per-pool moving average.
    Neu,
}

impl Regime {
    fn as_str(self) -> &'static str {
        match self {
            Regime::Alt => "alt",
            Regime::Neu => "neu",
        }
    }
}

/// `AtDisposalCategory`: the semantic bucket of one gain/loss row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtCategory {
    IncomeGeneral,
    IncomeCapitalYield,
    NeuGain,
    NeuLoss,
    NeuSwap,
    AltSpekulation,
    AltTaxfree,
}

impl AtCategory {
    /// The enum value Kassiber stores.
    pub fn as_str(self) -> &'static str {
        match self {
            AtCategory::IncomeGeneral => "income_general",
            AtCategory::IncomeCapitalYield => "income_capital_yield",
            AtCategory::NeuGain => "neu_gain",
            AtCategory::NeuLoss => "neu_loss",
            AtCategory::NeuSwap => "neu_swap",
            AtCategory::AltSpekulation => "alt_spekulation",
            AtCategory::AltTaxfree => "alt_taxfree",
        }
    }
}

/// The marker tokens of `notes`: split on runs of space, tab, LF, and
/// comma, dropping empty tokens. No other character separates.
fn tokens(notes: &str) -> impl Iterator<Item = &str> {
    notes
        .split([' ', '\t', '\n', ','])
        .filter(|token| !token.is_empty())
}

/// `_regime_from_notes`: the explicit regime, if exactly one marker gives
/// one. Conflicting or repeated markers fail.
pub(crate) fn regime_from_notes(notes: &str) -> EngineResult<Option<Regime>> {
    let regimes: Vec<Regime> = tokens(notes)
        .filter_map(|token| match token {
            REGIME_MARKER_ALT => Some(Regime::Alt),
            REGIME_MARKER_NEU => Some(Regime::Neu),
            _ => None,
        })
        .collect();
    let Some(&first) = regimes.first() else {
        return Ok(None);
    };
    if regimes.iter().any(|&regime| regime != first) {
        let names: Vec<&str> = regimes.iter().map(|regime| regime.as_str()).collect();
        return Err(EngineError::value(format!(
            "Conflicting `at_regime` markers in notes: {}. Only one of `at_regime=alt` or `at_regime=neu` is allowed per transaction.",
            py_str_list(&names)
        )));
    }
    if regimes.len() > 1 {
        return Err(EngineError::value(format!(
            "Duplicate `at_regime={}` markers in notes; only one is allowed per transaction.",
            first.as_str()
        )));
    }
    Ok(Some(first))
}

/// `_marker_value`: the text after `marker` in the single token starting
/// with it. More than one such token fails.
fn marker_value<'a>(notes: &'a str, marker: &str) -> EngineResult<Option<&'a str>> {
    let matches: Vec<&str> = tokens(notes)
        .filter(|token| token.starts_with(marker))
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [token] => Ok(Some(&token[marker.len()..])),
        _ => Err(EngineError::value(format!(
            "Duplicate `{marker}` markers in notes; only one is allowed per transaction. Found: {}",
            py_str_list(&matches)
        ))),
    }
}

/// `pool_id_from_notes`: the `at_pool=` value, or the default pool when it
/// is absent or empty.
pub(crate) fn pool_id(notes: &str) -> EngineResult<String> {
    Ok(match marker_value(notes, POOL_MARKER)? {
        None | Some("") => DEFAULT_POOL.to_owned(),
        Some(pool) => pool.to_owned(),
    })
}

/// `has_swap_link`: whether any token starts with `at_swap_link=`.
pub(crate) fn has_swap_link(notes: &str) -> bool {
    tokens(notes).any(|token| token.starts_with(SWAP_MARKER))
}

/// `swap_link_id`: the non-empty `at_swap_link=` value.
pub(crate) fn swap_link_id(notes: &str) -> EngineResult<Option<String>> {
    Ok(match marker_value(notes, SWAP_MARKER)? {
        None | Some("") => None,
        Some(id) => Some(id.to_owned()),
    })
}

/// `_validated_swap_link_id`: the swap id of a marked transaction that is
/// eligible for swap neutrality. An empty id fails, even on an explicit Alt
/// transaction; an explicit Alt transaction has none.
pub(crate) fn validated_swap_link_id(entry: &Entry) -> EngineResult<Option<String>> {
    if !has_swap_link(&entry.notes) {
        return Ok(None);
    }
    let Some(id) = swap_link_id(&entry.notes)? else {
        return Err(EngineError::value(format!(
            "Empty `at_swap_link=` marker on transaction. The id is required so RP2 can pair the outgoing leg with the incoming leg. Event: {}",
            entry.text
        )));
    };
    if regime_from_notes(&entry.notes)? == Some(Regime::Alt) {
        return Ok(None);
    }
    Ok(Some(id))
}

/// `classify_lot_regime` for a lot without a `from_lot` chain (universal
/// application): the explicit marker, else Alt iff acquired before the
/// cutoff instant.
pub(crate) fn lot_regime(lot: &Entry) -> EngineResult<Regime> {
    Ok(match regime_from_notes(&lot.notes)? {
        Some(regime) => regime,
        None if lot.ts.us < NEU_CUTOFF_US => Regime::Alt,
        None => Regime::Neu,
    })
}

/// The Europe/Vienna calendar date of `ts`, which the request must carry.
fn vienna_date(ts: &Timestamp) -> EngineResult<CivilDate> {
    ts.vienna_date.ok_or_else(|| {
        EngineError::request(format!(
            "timestamp {:?} has no Europe/Vienna date for the Austrian rules",
            ts.raw
        ))
    })
}

/// `_add_one_calendar_year` (§ 108 BAO): the same day next year, or
/// 28 February when the start is a 29 February.
fn add_one_calendar_year(start: CivilDate) -> EngineResult<CivilDate> {
    let year = start.year + 1;
    if year > 9999 {
        return Err(EngineError::value(format!("year {year} is out of range")));
    }
    let day = if start.month == 2 && start.day == 29 {
        28
    } else {
        start.day
    };
    Ok(CivilDate {
        year,
        month: start.month,
        day,
    })
}

/// `_within_spekulationsfrist`: whether the disposal's Vienna date is on or
/// before the acquisition's first Vienna anniversary.
fn within_spekulationsfrist(acquisition: &Timestamp, disposal: &Timestamp) -> EngineResult<bool> {
    let acquired = vienna_date(acquisition)?;
    let disposed = vienna_date(disposal)?;
    Ok(disposed <= add_one_calendar_year(acquired)?)
}

/// `classify_disposal` for one fragment with its computed gain.
pub(crate) fn classify(
    fragment: &Fragment,
    fiat_gain: &Decimal,
    entries: &[Entry],
    lots: &[LotView],
) -> EngineResult<AtCategory> {
    let event = &entries[fragment.event];
    let Some(lot) = fragment.lot else {
        return Ok(match event.transaction_type {
            TransactionType::Staking | TransactionType::Interest => AtCategory::IncomeCapitalYield,
            _ => AtCategory::IncomeGeneral,
        });
    };
    let lot = &entries[lots[lot].entry];
    match lot_regime(lot)? {
        Regime::Neu => {
            if validated_swap_link_id(event)?.is_some() {
                Ok(AtCategory::NeuSwap)
            } else if cmp13(fiat_gain, &zero())? == Ordering::Less {
                Ok(AtCategory::NeuLoss)
            } else {
                Ok(AtCategory::NeuGain)
            }
        }
        Regime::Alt => {
            if within_spekulationsfrist(&lot.ts, &event.ts)? {
                Ok(AtCategory::AltSpekulation)
            } else {
                Ok(AtCategory::AltTaxfree)
            }
        }
    }
}

/// Python's `repr()` of a string.
///
/// Non-ASCII characters outside the general categories Python treats as
/// unprintable (`Cc`, `Cf`, `Co`, `Zs`, `Zl`, `Zp`, and the noncharacters)
/// print as themselves. Unassigned code points, which depend on the Python
/// build's Unicode version, also print as themselves.
pub(crate) fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if ch == quote => {
                out.push('\\');
                out.push(ch);
            }
            ' '..='~' => out.push(ch),
            _ if ch.is_ascii() || !is_printable(ch) => {
                let code = u32::from(ch);
                // Writing to a String cannot fail.
                let _ = match code {
                    0..=0xff => write!(out, "\\x{code:02x}"),
                    0x100..=0xffff => write!(out, "\\u{code:04x}"),
                    _ => write!(out, "\\U{code:08x}"),
                };
            }
            _ => out.push(ch),
        }
    }
    out.push(quote);
    out
}

/// Python's `repr()` of a list of strings.
pub(crate) fn py_str_list(items: &[&str]) -> String {
    let parts: Vec<String> = items.iter().map(|item| py_repr(item)).collect();
    format!("[{}]", parts.join(", "))
}

/// Non-ASCII code points Python's `str.isprintable` rejects, as inclusive
/// ranges: controls, format characters, separators, private use, and
/// noncharacters.
const UNPRINTABLE: &[(u32, u32)] = &[
    (0x80, 0xa0),
    (0xad, 0xad),
    (0x600, 0x605),
    (0x61c, 0x61c),
    (0x6dd, 0x6dd),
    (0x70f, 0x70f),
    (0x890, 0x891),
    (0x8e2, 0x8e2),
    (0x1680, 0x1680),
    (0x180e, 0x180e),
    (0x2000, 0x200f),
    (0x2028, 0x202f),
    (0x205f, 0x2064),
    (0x2066, 0x206f),
    (0x3000, 0x3000),
    (0xe000, 0xf8ff),
    (0xfdd0, 0xfdef),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x13438),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
    (0xf0000, 0xffffd),
    (0x100000, 0x10fffd),
];

fn is_printable(ch: char) -> bool {
    let code = u32::from(ch);
    // U+xxFFFE and U+xxFFFF are noncharacters in every plane.
    if code & 0xfffe == 0xfffe {
        return false;
    }
    !UNPRINTABLE
        .iter()
        .any(|&(start, end)| (start..=end).contains(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_is_vienna_midnight() {
        let date = CivilDate {
            year: 2021,
            month: 2,
            day: 28,
        };
        let us = (date.to_days() * 86_400 + 23 * 3_600) * 1_000_000;
        assert_eq!(us, NEU_CUTOFF_US);
    }

    #[test]
    fn markers_tokenize_on_the_documented_separators() {
        assert_eq!(
            regime_from_notes("x,at_regime=alt\tat_pool=p").unwrap(),
            Some(Regime::Alt)
        );
        assert_eq!(regime_from_notes("at_regime=alt\rat_pool=x").unwrap(), None);
        assert_eq!(
            regime_from_notes("AT_REGIME=alt at_regime=ALT").unwrap(),
            None
        );
        assert_eq!(pool_id("at_pool=a=b").unwrap(), "a=b");
        assert_eq!(pool_id("at_pool= x").unwrap(), DEFAULT_POOL);
        assert!(!has_swap_link("at_swap_link_v2=x"));
        assert_eq!(swap_link_id("at_swap_link=").unwrap(), None);
        let duplicate = pool_id("at_pool=default at_pool=savings").unwrap_err();
        assert_eq!(
            duplicate.message,
            "Duplicate `at_pool=` markers in notes; only one is allowed per transaction. Found: ['at_pool=default', 'at_pool=savings']"
        );
        let conflict = regime_from_notes("at_regime=neu at_regime=alt").unwrap_err();
        assert!(conflict.message.contains("['neu', 'alt']"));
    }

    #[test]
    fn frist_anniversary_handles_the_leap_day() {
        let leap = CivilDate {
            year: 2020,
            month: 2,
            day: 29,
        };
        assert_eq!(
            add_one_calendar_year(leap).unwrap(),
            CivilDate {
                year: 2021,
                month: 2,
                day: 28
            }
        );
        let last = CivilDate {
            year: 9999,
            month: 1,
            day: 1,
        };
        assert_eq!(
            add_one_calendar_year(last).unwrap_err().message,
            "year 10000 is out of range"
        );
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(py_repr("default"), "'default'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(
            py_repr("a\\b\r\x0b\u{a0}é\u{200b}"),
            "'a\\\\b\\r\\x0b\\xa0é\\u200b'"
        );
        assert_eq!(py_str_list(&[]), "[]");
        assert_eq!(py_str_list(&["alt", "neu"]), "['alt', 'neu']");
    }
}
