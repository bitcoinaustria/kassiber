//! The platform reader's CPython Sniffer and non-strict csv.reader dialect.
use super::*;

fn mode<T: Eq + Clone>(values: &[T]) -> Option<(T, usize)> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for v in values {
        if let Some((_, n)) = counts.iter_mut().find(|(k, _)| k == v) {
            *n += 1
        } else {
            counts.push((v.clone(), 1))
        }
    }
    counts
        .into_iter()
        .reduce(|best, next| if next.1 > best.1 { next } else { best })
}
fn delimiter(c: char) -> bool {
    !kassiber_pytext::is_alpha(c as u32)
        && !c.is_numeric()
        && c != '_'
        && !['\n', '"', '\''].contains(&c)
}
fn sniff(text: &str) -> (char, char, bool, bool) {
    let sample = text
        .chars()
        .take(8192)
        .collect::<String>()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let chars = sample.chars().collect::<Vec<_>>();
    let mut matches = Vec::new();
    for pattern in 0..4 {
        let mut at = 0;
        while at < chars.len() {
            let begin = at;
            let mut delim = None;
            let mut space = false;
            if pattern == 0 || pattern == 2 {
                if !delimiter(chars[at]) {
                    at += 1;
                    continue;
                }
                delim = Some(chars[at]);
                at += 1;
                if chars.get(at) == Some(&' ') {
                    space = true;
                    at += 1;
                }
            } else if at > 0 && chars[at - 1] != '\n' {
                at += 1;
                continue;
            }
            let Some(&quote) = chars.get(at).filter(|c| ['"', '\''].contains(c)) else {
                at = begin + 1;
                continue;
            };
            at += 1;
            let body = at;
            let mut doubled = false;
            while at < chars.len() {
                if chars[at] == quote {
                    if chars.get(at + 1) == Some(&quote) {
                        doubled = true;
                        at += 2;
                        continue;
                    }
                    break;
                }
                at += 1;
            }
            if at == chars.len() {
                at = begin + 1;
                continue;
            }
            at += 1;
            let valid = match pattern {
                0 => chars.get(at) == delim.as_ref(),
                1 => {
                    if let Some(&c) = chars.get(at).filter(|c| delimiter(**c)) {
                        delim = Some(c);
                        space = chars.get(at + 1) == Some(&' ');
                        true
                    } else {
                        false
                    }
                }
                _ => at == chars.len() || chars.get(at) == Some(&'\n'),
            };
            if valid {
                if pattern < 2 {
                    at += 1;
                    if pattern == 1 && space {
                        at += 1;
                    }
                }
                matches.push((quote, delim, space, doubled, body));
            } else {
                at = begin + 1;
            }
        }
        if !matches.is_empty() {
            break;
        }
    }
    let quote = mode(&matches.iter().map(|m| m.0).collect::<Vec<_>>())
        .map(|m| m.0)
        .unwrap_or('"');
    let guessed = mode(
        &matches
            .iter()
            .filter_map(|m| m.1.filter(|c| [',', ';', '\t', '|'].contains(c)))
            .collect::<Vec<_>>(),
    );
    let double = matches.iter().any(|m| m.0 == quote && m.3);
    if let Some((d, count)) = guessed {
        return (
            d,
            quote,
            double,
            count == matches.iter().filter(|m| m.2).count(),
        );
    }
    let lines = sample
        .split('\n')
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return (',', '"', true, false);
    }
    let mut candidates = Vec::new();
    let chunk = 10.min(lines.len());
    for end in (chunk..lines.len() + chunk).step_by(chunk) {
        let end = end.min(lines.len());
        let mut consistency = 1.0;
        while candidates.is_empty() && consistency >= 0.9 {
            for d in ['\t', ',', ';', '|'] {
                let counts = lines[..end]
                    .iter()
                    .map(|l| l.matches(d).count())
                    .collect::<Vec<_>>();
                if let Some((freq, count)) = mode(&counts) {
                    let adjusted = count as i64 - (end - count) as i64;
                    if freq > 0 && adjusted > 0 && adjusted as f64 / end as f64 >= consistency {
                        candidates.push(d);
                    }
                }
            }
            consistency -= 0.01;
        }
        if candidates.len() == 1 {
            break;
        }
    }
    if candidates.is_empty() {
        return (',', '"', true, false);
    }
    let d = [',', '\t', ';', '|']
        .into_iter()
        .find(|d| candidates.contains(d))
        .unwrap();
    (
        d,
        quote,
        double,
        lines[0].matches(d).count() == lines[0].matches(&format!("{d} ")).count(),
    )
}
pub(super) fn read(text: &str) -> Result<Vec<Vec<String>>> {
    let (delimiter, quote, double, skip) = sniff(text);
    let mut chars = text.chars().peekable();
    let mut records = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut start = true;
    let mut touched = false;
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == quote {
                if double && chars.peek() == Some(&quote) {
                    chars.next();
                } else {
                    quoted = false;
                    continue;
                }
            }
        } else {
            if start && skip && ch == ' ' {
                continue;
            }
            if ch == quote && start {
                quoted = true;
                start = false;
                touched = true;
                continue;
            }
            // `io.StringIO(text, newline="")`: a bare carriage return ends
            // a record like any other line break.
            if ch == delimiter || ch == '\r' || ch == '\n' {
                if ch == delimiter || touched || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                }
                if ch != delimiter {
                    if ch == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    records.push(std::mem::take(&mut row));
                    touched = false;
                } else {
                    touched = true;
                }
                start = true;
                count = 0;
                continue;
            }
        }
        field.push(ch);
        count += 1;
        if count > 131_072 {
            return Err(AppError::new(
                "internal_error",
                "field larger than field limit (131072)",
            ));
        }
        start = false;
        touched = true;
    }
    if touched || !row.is_empty() {
        row.push(field);
        records.push(row);
    }
    Ok(records)
}
