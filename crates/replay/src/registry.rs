//! Tax settings parsed against the tax-country registry
//! (`kassiber-tax-countries`) instead of hand-written lists, so a country,
//! method or pool scope the registry learns is carried without a schema
//! edit. Each parser finds the registry constant the text spells and prints
//! that constant again, in the same spelling; the text is never copied.
use kassiber_tax_countries::{Fiat, COUNTRIES};

/// `candidate` exactly, upper-cased or lower-cased: the spelling of `text`,
/// printed from the constant.
fn spelled(candidate: &str, text: &str) -> Option<String> {
    [
        candidate.to_owned(),
        candidate.to_ascii_uppercase(),
        candidate.to_ascii_lowercase(),
    ]
    .into_iter()
    .find(|spelling| spelling == text)
}

fn first<'a>(candidates: impl Iterator<Item = &'a str>, text: &str) -> Option<String> {
    candidates
        .into_iter()
        .find_map(|candidate| spelled(candidate, text))
}

/// A registered `tax_country` code.
pub fn country(text: &str) -> Option<String> {
    first(COUNTRIES.iter().map(|c| c.code), text)
}

/// A method some registered country allows (`gains_algorithm`, stored upper
/// case), or the engine's lower-case name of one (a calculation's `method`).
pub fn method(text: &str) -> Option<String> {
    first(
        COUNTRIES.iter().flat_map(|c| c.methods.iter().copied()),
        text,
    )
}

/// A cost-basis pool scope some registered country allows.
pub fn pool_scope(text: &str) -> Option<String> {
    first(
        COUNTRIES.iter().flat_map(|c| c.pool_scopes.iter().copied()),
        text,
    )
}

/// ISO 4217 codes a profile may choose where its country lets it.
const PROFILE_FIAT: &[&str] = &[
    "EUR", "USD", "CHF", "GBP", "JPY", "CAD", "AUD", "SEK", "NOK", "DKK", "PLN", "CZK", "HUF",
    "NZD", "SGD", "HKD", "BRL", "MXN", "ZAR", "INR", "KRW", "CNY", "TRY", "RON", "BGN", "ISK",
    "ILS",
];

/// A fiat currency: every currency a registered country fixes, and the
/// codes a profile may choose.
pub fn fiat(text: &str) -> Option<String> {
    let fixed = COUNTRIES.iter().filter_map(|c| match c.fiat {
        Fiat::Fixed(code) => Some(code),
        Fiat::Profile => None,
    });
    first(fixed.chain(PROFILE_FIAT.iter().copied()), text)
}

/// A quarantine reason: one the custody and journal stages write
/// (`custody`), one of the tax engine's (`QuarantineReason::ALL`), or a
/// country's fail-closed refusal code from the registry.
pub fn quarantine_reason(custody: &[&str], text: &str) -> Option<String> {
    let engine = kassiber_tax_core::manifest::QuarantineReason::ALL
        .iter()
        .map(|reason| reason.as_str());
    let countries = COUNTRIES
        .iter()
        .filter_map(|c| c.engine.as_ref())
        .flat_map(|rules| rules.refused_receipts.iter().map(|r| r.code));
    custody
        .iter()
        .copied()
        .chain(engine)
        .chain(countries)
        .find(|reason| *reason == text)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_setting_parses_and_nothing_else_does() {
        for c in COUNTRIES {
            assert_eq!(country(c.code).as_deref(), Some(c.code));
            assert_eq!(method(c.default_method).as_deref(), Some(c.default_method));
            let engine = c.default_method.to_ascii_lowercase();
            assert_eq!(method(&engine), Some(engine.clone()));
            assert_eq!(
                pool_scope(c.default_pool_scope).as_deref(),
                Some(c.default_pool_scope)
            );
            if let Fiat::Fixed(code) = c.fiat {
                assert_eq!(fiat(code).as_deref(), Some(code));
            }
        }
        for text in [
            "",
            "atlantis",
            "MOVING_AVERAGE_AT ",
            "Fifo",
            "Eur",
            "global\u{0}",
        ] {
            assert_eq!(country(text), None, "{text:?}");
            assert_eq!(method(text), None, "{text:?}");
            assert_eq!(pool_scope(text), None, "{text:?}");
            assert_eq!(fiat(text), None, "{text:?}");
        }
    }
}
