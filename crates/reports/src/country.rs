//! The book's tax country, as the report readers ask about it.
//!
//! Each caller keeps the reference's own reading of the stored value
//! (stripped or not, lower case) and passes the result here; an empty value
//! is the generic country.

pub use kassiber_tax_countries::{Country, Rules};

/// The registry entry of an already normalized `tax_country`.
pub fn entry(normalized: &str) -> Option<&'static Country> {
    kassiber_tax_countries::lookup(if normalized.is_empty() {
        kassiber_tax_countries::DEFAULT_CODE
    } else {
        normalized
    })
}

/// Whether the registry knows the country.
pub fn known(normalized: &str) -> bool {
    entry(normalized).is_some()
}

/// Whether the book follows Austria's rules (Alt/Neu, Kennzahlen, exit tax).
pub fn austrian(normalized: &str) -> bool {
    entry(normalized).is_some_and(|entry| entry.rules == Rules::Austria)
}

/// Whether the book's calendar is Europe/Vienna.
pub fn vienna(normalized: &str) -> bool {
    entry(normalized).is_some_and(|entry| entry.timezone == kassiber_tax_countries::VIENNA)
}

/// The book's report calendar: `None` for UTC, whose dates the readers
/// take from the stored text as the reference does, else the zone whose
/// local dates cut the book's years (`crate::calendar::local`).
pub fn zone(normalized: &str) -> Option<&'static str> {
    entry(normalized)
        .map(|entry| entry.timezone)
        .filter(|zone| *zone != kassiber_tax_countries::UTC)
}
