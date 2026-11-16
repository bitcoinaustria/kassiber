//! Number, money and date formatting that matches the desktop UI's `Intl` output
//! (ui-tauri/src/lib/currency.ts and overview-dashboard/model.ts) byte for byte where it matters.

const NBSP: char = '\u{a0}';

/// Locale conventions the web picks per fiat (`localeForFiat`).
#[derive(Clone, Copy)]
struct Loc {
    group: char,
    decimal: char,
    /// Symbol before the number with a no-break space (de-AT `€ 1,00`), or glued (en-US `$1.00`).
    prefix_space: bool,
    /// Plain-number grouping (de-AT uses NBSP for plain numbers, `.` inside currency).
    plain_group: char,
    /// Compact notation: suffixes for 1e6, 1e9 and whether thousands compact (`$71K`).
    compact_thousands: bool,
}

fn loc(cur: &str) -> Loc {
    match cur {
        "USD" => Loc {
            group: ',',
            decimal: '.',
            prefix_space: false,
            plain_group: ',',
            compact_thousands: true,
        },
        "GBP" => Loc {
            group: ',',
            decimal: '.',
            prefix_space: false,
            plain_group: ',',
            compact_thousands: true,
        },
        "CHF" => Loc {
            group: '’',
            decimal: '.',
            prefix_space: true,
            plain_group: '’',
            compact_thousands: false,
        },
        _ => Loc {
            group: '.',
            decimal: ',',
            prefix_space: true,
            plain_group: NBSP,
            compact_thousands: false,
        },
    }
}

pub fn symbol(cur: &str) -> String {
    match cur {
        "EUR" => "€".into(),
        "USD" => "$".into(),
        "GBP" => "£".into(),
        "CHF" => "CHF".into(),
        other => other.into(),
    }
}

/// Groups the integer digits of `digits` with `sep`, starting at `min_group` digits
/// (de-AT leaves four-digit numbers ungrouped in compact notation only).
fn group(digits: &str, sep: char, min_len: usize) -> String {
    if digits.len() < min_len {
        return digits.to_owned();
    }
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

/// `value` rounded half away from zero to `frac` digits, as integer and fraction digit strings.
fn split(value: f64, frac: usize) -> (String, String) {
    // `Intl` rounds half away from zero; `{:.N}` rounds exact halves to even.
    let p = 10f64.powi(frac as i32);
    let s = format!("{:.*}", frac, (value.abs() * p).round() / p);
    match s.split_once('.') {
        Some((i, f)) => (i.to_owned(), f.to_owned()),
        None => (s, String::new()),
    }
}

fn currency_with(value: f64, cur: &str, frac: usize, min_group: usize) -> String {
    let l = loc(cur);
    let (int, f) = split(value, frac);
    let mut num = group(&int, l.group, min_group);
    if !f.is_empty() {
        num.push(l.decimal);
        num.push_str(&f);
    }
    let sign = if value < 0.0 && num.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    };
    let sym = symbol(cur);
    if l.prefix_space {
        format!("{sign}{sym}{NBSP}{num}")
    } else {
        format!("{sign}{sym}{num}")
    }
}

/// `formatFiatAmount`: currency style with the currency's two fraction digits.
pub fn fiat(value: f64, cur: &str) -> String {
    if !value.is_finite() {
        return "Unpriced".into();
    }
    currency_with(value, cur, 2, 0)
}

/// `formatWholeFiatAmount`: no fraction digits.
pub fn fiat_whole(value: f64, cur: &str) -> String {
    if !value.is_finite() {
        return "Unpriced".into();
    }
    currency_with(value.round(), cur, 0, 0)
}

/// `formatCompactFiatAmount` / compact currency notation with no fraction digits.
pub fn fiat_compact(value: f64, cur: &str) -> String {
    if !value.is_finite() {
        return "Unpriced".into();
    }
    let l = loc(cur);
    let abs = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    let sym = symbol(cur);
    let (scaled, suffix) = if l.compact_thousands {
        match abs {
            a if a >= 1e12 => (a / 1e12, "T"),
            a if a >= 1e9 => (a / 1e9, "B"),
            a if a >= 1e6 => (a / 1e6, "M"),
            a if a >= 1e3 => (a / 1e3, "K"),
            a => (a, ""),
        }
    } else {
        match abs {
            a if a >= 1e9 => (a / 1e9, "Mrd."),
            a if a >= 1e6 => (a / 1e6, "Mio."),
            a => (a, ""),
        }
    };
    let digits = format!("{:.0}", scaled.round());
    if suffix.is_empty() {
        // de-AT compact keeps grouping off below five digits (`€ 1235`, `€ 71.420`).
        let num = group(&digits, l.group, 5);
        return if l.prefix_space {
            format!("{sign}{sym}{NBSP}{num}")
        } else {
            format!("{sign}{sym}{num}")
        };
    }
    if l.compact_thousands {
        format!("{sign}{sym}{digits}{suffix}")
    } else {
        format!("{sign}{digits}{NBSP}{suffix}{NBSP}{sym}")
    }
}

/// `fiatNumberFormatter(CUR).format(v)`: a plain number in the fiat's locale.
pub fn plain_number(value: f64, cur: &str) -> String {
    let (int, _) = split(value.round(), 0);
    let sign = if value.round() < 0.0 { "-" } else { "" };
    format!("{sign}{}", group(&int, loc(cur).plain_group, 0))
}

/// `formatFiatPrice`: `71 420 EUR`.
pub fn fiat_price(value: f64, cur: &str) -> String {
    format!("{} {cur}", plain_number(value, cur))
}

/// `formatBtc(btc, {precision, sign})`: `₿ 1.23400000`, signed `+ ₿ …` / `− ₿ …`.
pub fn btc(value: f64, precision: usize, sign: bool) -> String {
    let lead = if sign {
        if value >= 0.0 { "+ " } else { "\u{2212} " }
    } else {
        ""
    };
    format!("{lead}₿ {:.*}", precision, value.abs())
}

/// Signed fiat with a spaced sign: `+ € 1.234,56`.
pub fn signed_fiat(value: f64, cur: &str) -> String {
    let lead = if value >= 0.0 { "+ " } else { "\u{2212} " };
    format!("{lead}{}", fiat(value.abs(), cur))
}

/// `formatSignedMsatBtc`: 8 decimals, 11 when the change is below a satoshi.
pub fn signed_msat_btc(msat: i128) -> String {
    let lead = if msat >= 0 { "+ " } else { "\u{2212} " };
    let abs = msat.unsigned_abs();
    if !abs.is_multiple_of(1000) {
        format!(
            "{lead}₿ {}.{:011}",
            abs / 100_000_000_000,
            abs % 100_000_000_000
        )
    } else {
        let sat = abs / 1000;
        format!("{lead}₿ {}.{:08}", sat / 100_000_000, sat % 100_000_000)
    }
}

/// `formatSignedMsatSats`: `+ 1.234 sats` (en grouping) or `+ N sats R msat`.
pub fn signed_msat_sats(msat: i128) -> String {
    let lead = if msat >= 0 { "+ " } else { "\u{2212} " };
    let abs = msat.unsigned_abs();
    let sats = group(&(abs / 1000).to_string(), ',', 0);
    if abs.is_multiple_of(1000) {
        format!("{lead}{sats} sats")
    } else {
        format!("{lead}{sats} sats {} msat", abs % 1000)
    }
}

/// Plain count in the UI locale (`en`): `1,234`.
pub fn count(n: usize) -> String {
    count_in(n, crate::i18n::lang())
}

/// A count in a UI language's grouping (`formatCount`): en `1,234`, de-AT `1\u{a0}234`.
pub fn count_in(n: usize, lang: crate::i18n::Lang) -> String {
    match lang {
        crate::i18n::Lang::En => group(&n.to_string(), ',', 0),
        crate::i18n::Lang::De => group(&n.to_string(), NBSP, 0),
    }
}

/// `formatBtcAxisFitted` with the tick's `₿ ` shortened to `₿`. Like `formatBtc` without
/// `sign`, it prints the absolute value.
pub fn btc_axis(value: f64, domain: Option<(f64, f64)>) -> String {
    let precision = match domain {
        Some((lo, hi)) if hi > lo && value.abs() > 0.0 => {
            let step = (hi - lo) / 4.0;
            let a = if step > 0.0 {
                (-step.log10()).ceil()
            } else {
                0.0
            };
            let b = 2.0 - value.abs().log10().floor();
            a.max(b).clamp(0.0, 6.0) as usize
        }
        _ => {
            if value.abs() >= 10.0 {
                0
            } else if value.abs() >= 1.0 {
                2
            } else {
                3
            }
        }
    };
    format!("₿{:.*}", precision, value.abs())
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// de-AT short month names as `Intl` writes them (Austrian `Jän.`; short names for March to July).
const MONTHS_DE: [&str; 12] = [
    "Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez.",
];

/// The UI language's short month name (1-based): en `Jan`, de `Jän.`.
pub fn short_month_ui(m: u32) -> &'static str {
    let i = m.clamp(1, 12) as usize - 1;
    match crate::i18n::lang() {
        crate::i18n::Lang::En => MONTHS[i],
        crate::i18n::Lang::De => MONTHS_DE[i],
    }
}

/// A civil UTC date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Day {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

impl Day {
    /// `YYYY-MM-DD…`
    pub fn parse(s: &str) -> Option<Day> {
        let b = s.get(0..10)?;
        let y = b.get(0..4)?.parse().ok()?;
        let m = b.get(5..7)?.parse().ok()?;
        let d = b.get(8..10)?.parse().ok()?;
        (1..=12).contains(&m).then_some(Day { y, m, d })
    }
    /// Days since 1970-01-01 (Howard Hinnant's algorithm).
    pub fn epoch_days(self) -> i64 {
        let y = if self.m <= 2 {
            self.y as i64 - 1
        } else {
            self.y as i64
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (self.m as i64 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + self.d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
    pub fn from_epoch_days(z: i64) -> Day {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
        Day { y, m, d }
    }
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
    /// `Mar 24` (`month: short, year: 2-digit`).
    pub fn month_year(self) -> String {
        format!(
            "{} {:02}",
            MONTHS[self.m as usize - 1],
            self.y.rem_euclid(100)
        )
    }
    /// `Mar 5`.
    pub fn month_day(self) -> String {
        format!("{} {}", MONTHS[self.m as usize - 1], self.d)
    }
    /// `Mar 5, 2024`.
    pub fn long(self) -> String {
        format!("{} {}, {}", MONTHS[self.m as usize - 1], self.d, self.y)
    }

    // UI-language variants (`toLocaleDateString(currentUiLocale(), …)` in the web): English is
    // the en-US output above; German is de-AT's short months (`Jän.`, `März`, `Juni`).

    /// `{month: "short", year: "2-digit"}`: en `Mar 24`, de `März 24`.
    pub fn month_year_ui(self) -> String {
        match crate::i18n::lang() {
            crate::i18n::Lang::En => self.month_year(),
            crate::i18n::Lang::De => format!(
                "{} {:02}",
                MONTHS_DE[self.m as usize - 1],
                self.y.rem_euclid(100)
            ),
        }
    }
    /// `{month: "short", day: "numeric"}`: en `Mar 5`, de `5. März`.
    pub fn month_day_ui(self) -> String {
        match crate::i18n::lang() {
            crate::i18n::Lang::En => self.month_day(),
            crate::i18n::Lang::De => format!("{}. {}", self.d, MONTHS_DE[self.m as usize - 1]),
        }
    }
    /// `{month: "short", day: "numeric", year: "numeric"}`: en `Mar 5, 2024`, de `5. März 2024`.
    pub fn long_ui(self) -> String {
        match crate::i18n::lang() {
            crate::i18n::Lang::En => self.long(),
            crate::i18n::Lang::De => {
                format!("{}. {} {}", self.d, MONTHS_DE[self.m as usize - 1], self.y)
            }
        }
    }
}

/// Seconds since the epoch for an RFC 3339 / `YYYY-MM-DD[ T]HH:MM[:SS][.fff][Z]` timestamp (UTC).
pub fn parse_ts(s: &str) -> Option<f64> {
    let day = Day::parse(s)?;
    let rest = s.get(10..).unwrap_or("");
    let rest = rest.trim_start_matches(['T', ' ']);
    let mut secs = 0.0;
    if rest.len() >= 5 {
        let hh: f64 = rest.get(0..2)?.parse().ok()?;
        let mm: f64 = rest.get(3..5)?.parse().ok()?;
        let ss: f64 = rest.get(6..8).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let frac: f64 = rest
            .get(8..)
            .filter(|r| r.starts_with('.'))
            .map(|r| r.trim_end_matches(|c: char| !c.is_ascii_digit()))
            .and_then(|r| format!("0{r}").parse().ok())
            .unwrap_or(0.0);
        secs = hh * 3600.0 + mm * 60.0 + ss + frac;
    }
    Some(day.epoch_days() as f64 * 86_400.0 + secs)
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `just now` / `5m ago` / `3h ago` / `2d ago`.
pub fn relative(ts: &str, now: f64) -> Option<String> {
    let dt = (now - parse_ts(ts)?).max(0.0);
    Some(match dt {
        d if d < 60.0 => "just now".into(),
        d if d < 3600.0 => format!("{}m ago", (d / 60.0).floor()),
        d if d < 86_400.0 => format!("{}h ago", (d / 3600.0).floor()),
        d => format!("{}d ago", (d / 86_400.0).floor()),
    })
}

/// `formatShortDate`: `YYYY-MM-DD HH:MM`.
pub fn short_date(s: Option<&str>) -> String {
    match s {
        Some(s) if !s.is_empty() => s
            .replace('T', " ")
            .trim_end_matches('Z')
            .chars()
            .take(16)
            .collect(),
        _ => "—".into(),
    }
}

/// `first8…last6` for ids longer than 18 characters.
pub fn compact_id(id: &str) -> String {
    if id.chars().count() > 18 {
        let head: String = id.chars().take(8).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(6)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}…{tail}")
    } else {
        id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_intl_de_at() {
        assert_eq!(fiat(71_420.18, "EUR"), "€\u{a0}71.420,18");
        assert_eq!(fiat(-4321.5, "EUR"), "-€\u{a0}4.321,50");
        assert_eq!(fiat_compact(1_131_296.07, "EUR"), "1\u{a0}Mio.\u{a0}€");
        assert_eq!(fiat_compact(2_500_000_000.0, "EUR"), "3\u{a0}Mrd.\u{a0}€");
        assert_eq!(fiat_compact(1234.567, "EUR"), "€\u{a0}1235");
        assert_eq!(fiat_compact(71_420.18, "EUR"), "€\u{a0}71.420");
        assert_eq!(fiat_compact(71_420.18, "USD"), "$71K");
        assert_eq!(fiat_whole(1_131_296.07, "EUR"), "€\u{a0}1.131.296");
        assert_eq!(plain_number(71_420.18, "EUR"), "71\u{a0}420");
        assert_eq!(fiat_price(71_420.18, "EUR"), "71\u{a0}420 EUR");
    }

    #[test]
    fn ui_language_dates_match_intl() {
        let d = Day {
            y: 2024,
            m: 1,
            d: 5,
        };
        crate::i18n::set_lang(crate::i18n::Lang::De);
        assert_eq!(
            (d.month_year_ui(), d.month_day_ui(), d.long_ui()),
            ("Jän. 24".into(), "5. Jän.".into(), "5. Jän. 2024".into())
        );
        assert_eq!(
            Day {
                y: 2024,
                m: 3,
                d: 5
            }
            .long_ui(),
            "5. März 2024"
        );
        crate::i18n::set_lang(crate::i18n::Lang::En);
        assert_eq!(d.long_ui(), "Jan 5, 2024");
    }

    #[test]
    fn btc_and_dates() {
        assert_eq!(btc(1.2345, 3, false), "₿ 1.234");
        assert_eq!(btc(-0.5, 4, true), "\u{2212} ₿ 0.5000");
        assert_eq!(signed_msat_btc(123_456_789_000), "+ ₿ 1.23456789");
        assert_eq!(signed_msat_btc(-1), "\u{2212} ₿ 0.00000000001");
        let d = Day {
            y: 2024,
            m: 3,
            d: 5,
        };
        assert_eq!(Day::from_epoch_days(d.epoch_days()), d);
        assert_eq!(d.month_year(), "Mar 24");
        assert_eq!(d.long(), "Mar 5, 2024");
        assert_eq!(parse_ts("1970-01-02T00:00:01Z"), Some(86_401.0));
        assert_eq!(
            compact_id("373b15e0a627488e80c3ad301d0fe61efdcfd3a2d8cf387b5577bb6faf3ea6cb"),
            "373b15e0…f3ea6cb".replace("f3ea6cb", "3ea6cb")
        );
    }
}
