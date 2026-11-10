//! Report language: one string table per locale and the matching number
//! and date formats. Austrian books read Austrian German (`1.234,56`,
//! `15.03.2025`, `Jänner`), German books Germany's German (`Januar`), Swiss
//! books Swiss German (`1'234.56`, no ß), Spanish books Spanish
//! (`1.234,56`, `15/03/2025`), Japanese books Japanese (`1,234.56`,
//! `2025/03/15`); every other book reads English (`1,234.56`, `2025-03-15`). PDF text uses these; XLSX headers use the
//! strings with numeric cells; CSV ignores both.
use kassiber_pydecimal::{Decimal, Rounding};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    /// Austrian German.
    De,
    /// Germany's German: its own strings over the shared German table.
    DeDe,
    /// Swiss German: its own strings over the shared German table.
    DeCh,
    /// Spanish (Spain): `1.234,56`, `15/03/2025`.
    Es,
    /// Japanese: `1,234.56`, `2025/03/15`.
    Ja,
}

impl Lang {
    /// The book's tax country selects the language.
    pub fn for_country(country: &str) -> Lang {
        match kassiber_tax_countries::lookup(&country.trim().to_ascii_lowercase())
            .map(|entry| entry.locale)
        {
            Some(kassiber_tax_countries::Locale::DeAt) => Lang::De,
            Some(kassiber_tax_countries::Locale::DeDe) => Lang::DeDe,
            Some(kassiber_tax_countries::Locale::DeCh) => Lang::DeCh,
            Some(kassiber_tax_countries::Locale::Es) => Lang::Es,
            Some(kassiber_tax_countries::Locale::Ja) => Lang::Ja,
            _ => Lang::En,
        }
    }
    /// The registry's language.
    fn registry(self) -> kassiber_tax_countries::Language {
        match self {
            Lang::En => kassiber_tax_countries::Language::En,
            Lang::De | Lang::DeDe | Lang::DeCh => kassiber_tax_countries::Language::De,
            Lang::Es => kassiber_tax_countries::Language::Es,
            Lang::Ja => kassiber_tax_countries::Language::Ja,
        }
    }
    /// Whether the report reads German (any locale).
    pub fn german(self) -> bool {
        matches!(self, Lang::De | Lang::DeDe | Lang::DeCh)
    }
    /// The book's tax country as the scope names it: the registry's name of
    /// the stored code, the generic name for anything else.
    pub fn country_name(self, country: &str) -> &'static str {
        kassiber_tax_countries::lookup(country)
            .or_else(|| kassiber_tax_countries::lookup(kassiber_tax_countries::DEFAULT_CODE))
            .map_or("", |entry| entry.name(self.registry()))
    }
    /// The kicker above a report title: `Kassiber`, or `Kassiber · <country>`
    /// for a book of a real country.
    pub fn kicker(self, country: &str) -> String {
        match kassiber_tax_countries::lookup(country).filter(|entry| entry.iso.is_some()) {
            Some(entry) => format!("{} · {}", self.t("doc.kicker"), entry.name(self.registry())),
            None => self.t("doc.kicker").into(),
        }
    }
    /// The preview notice every rendering of a preview country's report
    /// carries.
    pub fn preview_notice(self, country: &str) -> Option<&'static str> {
        kassiber_tax_countries::lookup(country)?.preview_notice(self.registry())
    }
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De | Lang::DeDe | Lang::DeCh => "de",
            Lang::Es => "es",
            Lang::Ja => "ja",
        }
    }
    /// The locale's own strings, read before its base table.
    fn overlay(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::En | Lang::De | Lang::Es | Lang::Ja => &[],
            Lang::DeDe => super::strings::DE_DE,
            Lang::DeCh => super::strings::DE_CH,
        }
    }
    fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::En => super::strings::EN,
            Lang::De | Lang::DeDe | Lang::DeCh => super::strings::DE,
            Lang::Es => super::strings_es::ES,
            Lang::Ja => super::strings_ja::JA,
        }
    }
    /// The string for `key`. Both tables carry every key (a test checks);
    /// a missing key is a bug and fails the export instead of guessing.
    pub fn t(self, key: &str) -> &'static str {
        self.get(key)
            .unwrap_or_else(|| panic!("report string {key:?} missing for {}", self.code()))
    }
    /// The string for a key built from data (a method or status name), when
    /// the table has one.
    pub fn get(self, key: &str) -> Option<&'static str> {
        self.overlay()
            .iter()
            .chain(self.table())
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            // Another country's layer keys are English-only outside the
            // English and German tables; no book of this language renders
            // them.
            .or_else(|| {
                (matches!(self, Lang::Es | Lang::Ja) && foreign_key(self, key))
                    .then(|| Lang::En.get(key))
                    .flatten()
            })
    }
    /// `t` with `{name}` placeholders filled.
    pub fn tf(self, key: &str, args: &[(&str, &str)]) -> String {
        let mut out = self.t(key).to_owned();
        for (name, value) in args {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
    fn separators(self) -> (char, char) {
        match self {
            Lang::En => (',', '.'),
            Lang::De | Lang::DeDe => ('.', ','),
            Lang::Es => ('.', ','),
            Lang::Ja => (',', '.'),
            // The Swiss apostrophe, as U+0027 (`1'234.56`).
            Lang::DeCh => ('\'', '.'),
        }
    }

    /// Display text of an exact number: rounded half-even, grouped, with a
    /// true minus sign; never `−0.00`.
    pub fn number(self, value: &Decimal, places: u32) -> String {
        let fixed = value
            .to_fixed_places(places, Rounding::HalfEven)
            .unwrap_or_else(|_| value.to_canonical());
        let (negative, body) = match fixed.strip_prefix('-') {
            Some(rest) => (rest.chars().any(|c| c.is_ascii_digit() && c != '0'), rest),
            None => (false, fixed.as_str()),
        };
        let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
        let (group, decimal) = self.separators();
        let mut out = String::new();
        if negative {
            out.push('\u{2212}');
        }
        out.push_str(&grouped(whole, group));
        if !fraction.is_empty() {
            out.push(decimal);
            out.push_str(fraction);
        }
        out
    }
    pub fn integer(self, value: i128) -> String {
        let text = grouped(&value.unsigned_abs().to_string(), self.separators().0);
        if value < 0 {
            format!("\u{2212}{text}")
        } else {
            text
        }
    }
    pub fn percent(self, value: &Decimal) -> String {
        match self {
            Lang::En => format!("{}%", self.number(value, 1)),
            Lang::Ja => format!("{}%", self.number(value, 1)),
            Lang::De | Lang::DeDe | Lang::DeCh | Lang::Es => {
                format!("{}\u{a0}%", self.number(value, 1))
            }
        }
    }
    /// `2025-03-15` or `15.03.2025` from an ISO date or timestamp.
    pub fn date(self, iso: &str) -> String {
        let Some(day) = iso.get(..10).filter(|d| d.as_bytes().get(4) == Some(&b'-')) else {
            return iso.to_owned();
        };
        match self {
            Lang::En => day.to_owned(),
            Lang::De | Lang::DeDe | Lang::DeCh => {
                format!("{}.{}.{}", &day[8..10], &day[5..7], &day[..4])
            }
            Lang::Es => format!("{}/{}/{}", &day[8..10], &day[5..7], &day[..4]),
            Lang::Ja => format!("{}/{}/{}", &day[..4], &day[5..7], &day[8..10]),
        }
    }
    /// `2025-03-15 14:05 UTC` or `15.03.2025 14:05 UTC`.
    pub fn date_time(self, iso: &str) -> String {
        match iso.get(11..16) {
            Some(time) if iso.len() >= 16 => format!("{} {time} UTC", self.date(iso)),
            _ => self.date(iso),
        }
    }
    /// A month (`2025-03`): ISO in English, spelled out in German.
    pub fn month(self, iso: &str) -> String {
        match (self.months(), month_index(iso)) {
            // Spanish names the month "de" its year (`marzo de 2025`).
            (Some((months, _)), Some(m)) if self == Lang::Es => {
                format!("{} de {}", months[m], &iso[..4])
            }
            (Some((months, _)), Some(m)) => format!("{} {}", months[m], &iso[..4]),
            // Japanese months are numbered (`2025年3月`).
            (None, Some(m)) if self == Lang::Ja => format!("{}年{}月", &iso[..4], m + 1),
            _ => iso.get(..7).unwrap_or(iso).to_owned(),
        }
    }
    /// A short month for chart axes: `2025-03` or `Mär 25`.
    pub fn month_short(self, iso: &str) -> String {
        match (self.months(), month_index(iso)) {
            (Some((_, short)), Some(m)) => format!("{} {}", short[m], &iso[2..4]),
            (None, Some(m)) if self == Lang::Ja => format!("{}/{:02}", &iso[2..4], m + 1),
            _ => iso.get(..7).unwrap_or(iso).to_owned(),
        }
    }
    /// The spelled-out month names and their short forms; English keeps
    /// ISO months.
    fn months(self) -> Option<(&'static [&'static str; 12], &'static [&'static str; 12])> {
        match self {
            Lang::En => None,
            Lang::De => Some((&MONTHS_DE, &MONTHS_DE_SHORT)),
            Lang::DeDe | Lang::DeCh => Some((&MONTHS_DE_DE, &MONTHS_DE_DE_SHORT)),
            Lang::Es => Some((&MONTHS_ES, &MONTHS_ES_SHORT)),
            Lang::Ja => None,
        }
    }
}

fn month_index(iso: &str) -> Option<usize> {
    let m: usize = iso.get(5..7)?.parse().ok()?;
    (1..=12).contains(&m).then_some(m - 1)
}

/// Austrian month names (Jänner, not Januar).
const MONTHS_DE: [&str; 12] = [
    "Jänner",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONTHS_DE_SHORT: [&str; 12] = [
    "Jän", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];
/// Germany's and Switzerland's month names (Januar).
const MONTHS_DE_DE: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONTHS_DE_DE_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
];

/// Spanish month names, lower case as Spanish writes them.
const MONTHS_ES: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];
const MONTHS_ES_SHORT: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
];

/// Whether `key` belongs to another country's layer (`<code>.…` of a
/// registry country whose report language is not `lang`'s).
pub(super) fn foreign_key(lang: Lang, key: &str) -> bool {
    let Some((prefix, _)) = key.split_once('.') else {
        return false;
    };
    kassiber_tax_countries::lookup(prefix)
        .is_some_and(|entry| entry.iso.is_some() && entry.locale.language() != lang.registry())
}

fn grouped(digits: &str, separator: char) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(raw: &str) -> Decimal {
        raw.parse().unwrap()
    }
    #[test]
    fn numbers_dates_and_months_follow_the_language() {
        assert_eq!(Lang::En.number(&d("-1234.565"), 2), "\u{2212}1,234.56");
        assert_eq!(Lang::De.number(&d("-1234.565"), 2), "\u{2212}1.234,56");
        assert_eq!(Lang::De.number(&d("-0.001"), 2), "0,00");
        assert_eq!(Lang::De.percent(&d("59.25")), "59,2\u{a0}%");
        assert_eq!(Lang::En.date("2025-03-15T14:05:00Z"), "2025-03-15");
        assert_eq!(Lang::De.date("2025-03-15T14:05:00Z"), "15.03.2025");
        assert_eq!(
            Lang::De.date_time("2026-01-02T03:04:05Z"),
            "02.01.2026 03:04 UTC"
        );
        assert_eq!(Lang::De.month("2024-01"), "Jänner 2024");
        assert_eq!(Lang::De.month_short("2025-03"), "Mär 25");
        assert_eq!(Lang::En.month("2025-03"), "2025-03");
        assert_eq!(Lang::for_country(" AT "), Lang::De);
        assert_eq!(Lang::for_country("generic"), Lang::En);
    }

    #[test]
    fn germany_and_switzerland_have_their_own_german() {
        assert_eq!(Lang::for_country("de"), Lang::DeDe);
        assert_eq!(Lang::DeDe.month("2024-01"), "Januar 2024");
        assert_eq!(Lang::DeDe.month_short("2024-01"), "Jan 24");
        assert_eq!(Lang::De.month("2024-01"), "Jänner 2024");
        assert_eq!(Lang::DeDe.number(&d("-1234.565"), 2), "\u{2212}1.234,56");
        assert_eq!(Lang::DeDe.date("2025-03-15"), "15.03.2025");
        assert_eq!(Lang::DeCh.number(&d("1234567.891"), 2), "1'234'567.89");
        assert_eq!(Lang::DeCh.integer(-1234), "\u{2212}1'234");
        assert_eq!(Lang::DeCh.date("2024-12-31T23:00:00Z"), "31.12.2024");
        assert_eq!(Lang::DeCh.month("2024-03"), "März 2024");
        assert_eq!(Lang::DeCh.percent(&d("59.25")), "59.2\u{a0}%");
        assert!(Lang::DeCh.german() && Lang::DeDe.german() && !Lang::En.german());
        assert_eq!(Lang::DeCh.code(), "de");
        // A locale's own strings come first.
        assert_eq!(
            Lang::DeDe.t("method.FIFO"),
            "First in, first out (FIFO) je Wallet"
        );
        assert_eq!(Lang::De.t("method.FIFO"), "First in, first out (FIFO)");
        assert_eq!(Lang::DeCh.t("disposals.title"), "Veräusserungen");
    }
}
