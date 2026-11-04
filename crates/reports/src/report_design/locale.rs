//! Report language: one string table per language and the matching number
//! and date formats. Austrian books read German (`1.234,56`, `15.03.2025`);
//! every other book reads English (`1,234.56`, `2025-03-15`). PDF text uses
//! these; XLSX headers use the strings with numeric cells; CSV ignores both.
use kassiber_pydecimal::{Decimal, Rounding};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    /// The book's tax country selects the language.
    pub fn for_country(country: &str) -> Lang {
        if country.trim().eq_ignore_ascii_case("at") {
            Lang::De
        } else {
            Lang::En
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
        }
    }
    fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::En => super::strings::EN,
            Lang::De => super::strings::DE,
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
        self.table()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
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
            Lang::De => ('.', ','),
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
            Lang::De => format!("{}\u{a0}%", self.number(value, 1)),
        }
    }
    /// `2025-03-15` or `15.03.2025` from an ISO date or timestamp.
    pub fn date(self, iso: &str) -> String {
        let Some(day) = iso.get(..10).filter(|d| d.as_bytes().get(4) == Some(&b'-')) else {
            return iso.to_owned();
        };
        match self {
            Lang::En => day.to_owned(),
            Lang::De => format!("{}.{}.{}", &day[8..10], &day[5..7], &day[..4]),
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
        match (self, month_index(iso)) {
            (Lang::De, Some(m)) => format!("{} {}", MONTHS_DE[m], &iso[..4]),
            _ => iso.get(..7).unwrap_or(iso).to_owned(),
        }
    }
    /// A short month for chart axes: `2025-03` or `Mär 25`.
    pub fn month_short(self, iso: &str) -> String {
        match (self, month_index(iso)) {
            (Lang::De, Some(m)) => format!("{} {}", MONTHS_DE_SHORT[m], &iso[2..4]),
            _ => iso.get(..7).unwrap_or(iso).to_owned(),
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
}
