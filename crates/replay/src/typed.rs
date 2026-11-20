//! Strict parsers for the replay bundle's typed classes. Each parses the
//! book's text into a value and serializes that value; nothing is copied.
//! `None` means the text does not parse.
use kassiber_pydecimal::Decimal;
use serde_json::{Number, Value};

/// Non-finite decimals Python's `Decimal` spells, as emitted constants.
const SPECIAL: &[(&str, &str)] = &[
    ("nan", "NaN"),
    ("-nan", "-NaN"),
    ("snan", "sNaN"),
    ("-snan", "-sNaN"),
    ("infinity", "Infinity"),
    ("-infinity", "-Infinity"),
    ("inf", "Infinity"),
    ("-inf", "-Infinity"),
];

/// A decimal amount: sign, digits, optional fraction, optional exponent.
/// Re-emitted fixed-point unless the book spelled it with an exponent.
pub fn amount(text: &str) -> Option<String> {
    if let Some((_, special)) = SPECIAL.iter().find(|(s, _)| text.eq_ignore_ascii_case(s)) {
        return Some((*special).to_owned());
    }
    let body = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (body, None),
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 80 && s.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = match mantissa.split_once('.') {
        Some((a, b)) => {
            (a.is_empty() || digits(a))
                && (b.is_empty() || digits(b))
                && !(a.is_empty() && b.is_empty())
        }
        None => digits(mantissa),
    };
    let exponent_ok =
        exponent.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)) && e.len() <= 6);
    if !mantissa_ok || !exponent_ok {
        return None;
    }
    let value: Decimal = text.parse().ok()?;
    if exponent.is_some() {
        Some(value.to_string())
    } else {
        value.to_fixed_string().ok()
    }
}

/// A JSON number as a typed integer or finite float.
pub fn number(value: &Number) -> Option<Value> {
    if let Some(n) = value.as_i64() {
        return Some(Value::from(n));
    }
    if let Some(n) = value.as_u64() {
        return Some(Value::from(n));
    }
    value
        .as_f64()
        .filter(|n| n.is_finite())
        .and_then(Number::from_f64)
        .map(Value::Number)
}

pub fn int(text: &str) -> Option<String> {
    text.parse::<i64>().ok().map(|n| n.to_string())
}

pub fn boolean(text: &str) -> Option<&'static str> {
    match text {
        "0" => Some("0"),
        "1" => Some("1"),
        "true" => Some("true"),
        "false" => Some("false"),
        _ => None,
    }
}

fn number_field(text: &str, len: usize, max: u32) -> Option<u32> {
    (text.len() == len && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse::<u32>().ok())
        .flatten()
        .filter(|n| *n <= max)
}

/// A timestamp in the formats the book stores: `YYYY`, `YYYY-MM-DD`, and
/// `YYYY-MM-DD[T ]HH:MM[:SS[.fraction]]` with `Z`, `±HH:MM`, `±HHMM` or no
/// zone. The fields are parsed, range-checked and printed again in the same
/// layout.
pub fn time(text: &str) -> Option<String> {
    // Every accepted layout is ASCII; refusing anything else up front keeps
    // the byte offsets below on character boundaries.
    if !text.is_ascii() {
        return None;
    }
    let year = number_field(text.get(..4)?, 4, 9999)?;
    if text.len() == 4 {
        return Some(format!("{year:04}"));
    }
    let rest = text.get(4..)?;
    let rest = rest.strip_prefix('-')?;
    let month = number_field(rest.get(..2)?, 2, 12).filter(|m| *m >= 1)?;
    let rest = rest.get(2..)?.strip_prefix('-')?;
    let day = number_field(rest.get(..2)?, 2, 31).filter(|d| *d >= 1)?;
    let rest = rest.get(2..)?;
    let mut out = format!("{year:04}-{month:02}-{day:02}");
    if rest.is_empty() {
        return Some(out);
    }
    let separator = match rest.as_bytes().first()? {
        b'T' => 'T',
        b' ' => ' ',
        _ => return None,
    };
    let rest = rest.get(1..)?;
    let hour = number_field(rest.get(..2)?, 2, 23)?;
    let rest = rest.get(2..)?.strip_prefix(':')?;
    let minute = number_field(rest.get(..2)?, 2, 59)?;
    let mut rest = rest.get(2..)?;
    out.push_str(&format!("{separator}{hour:02}:{minute:02}"));
    if let Some(seconds) = rest.strip_prefix(':') {
        let second = number_field(seconds.get(..2)?, 2, 60)?;
        out.push_str(&format!(":{second:02}"));
        rest = seconds.get(2..)?;
        if let Some(fraction) = rest.strip_prefix('.') {
            let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 || digits > 9 {
                return None;
            }
            let value: u32 = fraction.get(..digits)?.parse().ok()?;
            out.push_str(&format!(".{value:0digits$}"));
            rest = fraction.get(digits..)?;
        }
    }
    match rest {
        "" => {}
        "Z" => out.push('Z'),
        zone => {
            let sign = match zone.as_bytes().first()? {
                b'+' => '+',
                b'-' => '-',
                _ => return None,
            };
            let zone = zone.get(1..)?;
            let (hours, minutes, colon) = match zone.len() {
                5 if zone.as_bytes().get(2) == Some(&b':') => {
                    (zone.get(..2)?, zone.get(3..)?, true)
                }
                4 => (zone.get(..2)?, zone.get(2..)?, false),
                _ => return None,
            };
            let hours = number_field(hours, 2, 23)?;
            let minutes = number_field(minutes, 2, 59)?;
            out.push(sign);
            out.push_str(&if colon {
                format!("{hours:02}:{minutes:02}")
            } else {
                format!("{hours:02}{minutes:02}")
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_parsed_and_printed_again() {
        for (input, output) in [
            ("40000", Some("40000")),
            ("100.00", Some("100.00")),
            ("-0.00000001", Some("-0.00000001")),
            ("1E+2", Some("1E+2")),
            ("nan", Some("NaN")),
            ("1.5x", None),
            ("AT611904300234573201", None),
            ("node.private.example:50002", None),
            ("", None),
        ] {
            assert_eq!(amount(input).as_deref(), output, "{input}");
        }
        for (input, output) in [
            ("2020", Some("2020")),
            ("2024-05-01", Some("2024-05-01")),
            ("2024-05-01T00:00:00Z", Some("2024-05-01T00:00:00Z")),
            (
                "2024-05-01 12:30:01.250+02:00",
                Some("2024-05-01 12:30:01.250+02:00"),
            ),
            ("2024-05-01T12:30+0100", Some("2024-05-01T12:30+0100")),
            ("2001:db8::1", None),
            ("2024-13-01", None),
            ("2024-05-01Tuesday", None),
            ("2024-05-01T00:00:00Z alice", None),
        ] {
            assert_eq!(time(input).as_deref(), output, "{input}");
        }
        assert_eq!(int("42").as_deref(), Some("42"));
        assert_eq!(int("4 2"), None);
        assert_eq!(boolean("1"), Some("1"));
        assert_eq!(boolean("yes"), None);
    }

    /// Fourth review, 2: a timestamp with a multi-byte character in it (in
    /// the zone, the fraction, anywhere) is refused, never a panic.
    #[test]
    fn non_ascii_text_never_panics_a_parser() {
        assert_eq!(time("2024-01-01T00:00+0\u{e9}0"), None);
        assert_eq!(time("2024-01-01T00:00+\u{e9}:00"), None);
        let pseudonymizer = crate::pseudonym::Pseudonymizer::new([7; 32]);
        let seeds = [
            "2024-01-01T00:00+01:00",
            "2024-01-01 12:30:01.250-0530",
            "2024-05-01",
            "1E+2",
            "-0.00000001",
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b:1",
        ];
        let inserts = [
            "\u{e9}",
            "\u{20ac}",
            "\u{1f600}",
            "\u{0}",
            "\u{ff}",
            "\u{300}",
        ];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut cases = Vec::new();
        for seed in seeds {
            for at in 0..=seed.len() {
                for insert in inserts {
                    cases.push(format!("{}{insert}{}", &seed[..at], &seed[at..]));
                    if at < seed.len() {
                        cases.push(format!("{}{insert}{}", &seed[..at], &seed[at + 1..]));
                    }
                }
            }
        }
        let alphabet: Vec<char> = "0123456789-+:.TZ eE\u{e9}\u{20ac}\u{1f600}\u{300}"
            .chars()
            .collect();
        for _ in 0..20_000 {
            let len = (next() % 32) as usize;
            cases.push(
                (0..len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect(),
            );
        }
        for case in &cases {
            let parsed = time(case);
            if !case.is_ascii() {
                assert_eq!(parsed, None, "{case:?}");
            }
            let _ = amount(case);
            let _ = int(case);
            let _ = boolean(case);
            let _ = pseudonymizer.id(case);
            let _ = pseudonymizer.label(case);
        }
    }
}
