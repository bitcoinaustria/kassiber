use super::*;

#[derive(Clone, Copy)]
struct Dialect {
    delimiter: char,
    quote: char,
    double: bool,
    spaces: bool,
}
fn word(c: char) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(&format!("^{}$", kassiber_pytext::REGEX_WORD)).unwrap())
        .is_match(&c.to_string())
}
fn candidate(c: char) -> bool {
    c != '\n' && c != '\"' && c != '\'' && !word(c)
}
fn doubled(chars: &[char], quote: char, delimiter: char) -> bool {
    for at in 0..chars.len() {
        if at > 0 && chars[at - 1] != delimiter && chars[at - 1] != '\n' {
            continue;
        }
        let mut q = at;
        while chars.get(q) == Some(&' ') {
            q += 1;
        }
        if chars.get(q) != Some(&quote) {
            continue;
        }
        let mut end = q + 1;
        let mut pair = false;
        while end < chars.len() {
            if chars[end] == quote {
                if chars.get(end + 1) == Some(&quote) {
                    pair = true;
                    end += 2;
                } else {
                    break;
                }
            } else {
                end += 1;
            }
        }
        if pair
            && end < chars.len()
            && (end + 1 == chars.len()
                || chars
                    .get(end + 1)
                    .is_some_and(|c| *c == delimiter || *c == '\n'))
        {
            return true;
        }
    }
    false
}
fn sniff(text: &str) -> Dialect {
    sniff_decided(text).0
}
/// `csv.Sniffer().sniff(text[:8192], delimiters=",;\t|").delimiter`, or
/// `None` where the Sniffer raises "Could not determine delimiter".
pub(crate) fn sniffed_delimiter(text: &str) -> Option<char> {
    let (dialect, decided) = sniff_decided(text);
    decided.then_some(dialect.delimiter)
}
/// The sniffed dialect, falling back to excel's when undecided.
fn sniff_decided(text: &str) -> (Dialect, bool) {
    let sample = text
        .chars()
        .take(8192)
        .collect::<String>()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let chars: Vec<char> = sample.chars().collect();
    for pattern in 0..4 {
        let mut quotes: Vec<(char, usize)> = vec![];
        let mut delims: Vec<(char, usize)> = vec![];
        let mut spaces = 0;
        let mut at = 0;
        while at < chars.len() {
            let mut q = at;
            let mut delim = None;
            let mut space = false;
            if pattern == 0 || pattern == 2 {
                if !candidate(chars[at]) {
                    at += 1;
                    continue;
                }
                delim = Some(chars[at]);
                q += 1;
                if chars.get(q) == Some(&' ') {
                    space = true;
                    q += 1;
                }
            } else if at > 0 && chars[at - 1] != '\n' {
                at += 1;
                continue;
            }
            let Some(&quote @ ('\"' | '\'')) = chars.get(q) else {
                at += 1;
                continue;
            };
            let mut end = q + 1;
            while end < chars.len() {
                if chars[end] == quote {
                    if chars.get(end + 1) == Some(&quote) {
                        end += 2;
                    } else {
                        break;
                    }
                } else {
                    end += 1;
                }
            }
            if end == chars.len() {
                at += 1;
                continue;
            }
            let valid = match pattern {
                0 => chars.get(end + 1) == delim.as_ref(),
                1 => {
                    delim = chars.get(end + 1).copied().filter(|c| candidate(*c));
                    space = chars.get(end + 2) == Some(&' ');
                    delim.is_some()
                }
                _ => end + 1 == chars.len() || chars.get(end + 1) == Some(&'\n'),
            };
            if !valid {
                at += 1;
                continue;
            }
            let count = |items: &mut Vec<(char, usize)>, key| {
                if let Some((_, count)) = items.iter_mut().find(|(k, _)| *k == key) {
                    *count += 1;
                } else {
                    items.push((key, 1));
                }
            };
            count(&mut quotes, quote);
            if let Some(d) = delim.filter(|d| ",;\t|".contains(*d)) {
                count(&mut delims, d);
            }
            spaces += usize::from(space);
            at = end
                + if pattern < 2 {
                    2 + usize::from(pattern == 1 && space)
                } else {
                    1
                };
        }
        if !quotes.is_empty() {
            let quote = quotes.iter().rev().max_by_key(|(_, n)| *n).unwrap().0;
            if let Some((delimiter, n)) = delims.iter().rev().max_by_key(|(_, n)| *n) {
                let double = doubled(&chars, quote, *delimiter);
                return (
                    Dialect {
                        delimiter: *delimiter,
                        quote,
                        double,
                        spaces: *n == spaces,
                    },
                    true,
                );
            }
            break;
        }
    }
    let lines: Vec<&str> = sample.split('\n').filter(|s| !s.is_empty()).collect();
    let chunk = lines.len().min(10);
    let mut frequency: BTreeMap<char, Vec<(usize, i64)>> = BTreeMap::new();
    let mut delims = BTreeMap::new();
    if chunk > 0 {
        for end in (chunk..lines.len() + chunk).step_by(chunk) {
            for line in &lines[end - chunk..end.min(lines.len())] {
                for d in [',', ';', '\t', '|'] {
                    let n = line.chars().filter(|c| *c == d).count();
                    let entries = frequency.entry(d).or_default();
                    if let Some((_, count)) = entries.iter_mut().find(|(key, _)| *key == n) {
                        *count += 1;
                    } else {
                        entries.push((n, 1));
                    }
                }
            }
            let total = end.min(lines.len()) as f64;
            let mut consistency = 1.0;
            while delims.is_empty() && consistency >= 0.9 {
                for (&d, entries) in &frequency {
                    let (n, count) = entries.iter().rev().max_by_key(|(_, n)| *n).unwrap();
                    let adjusted = *count
                        - entries
                            .iter()
                            .filter(|(key, _)| key != n)
                            .map(|(_, n)| n)
                            .sum::<i64>();
                    if *n > 0 && adjusted > 0 && adjusted as f64 / total >= consistency {
                        delims.insert(d, (*n, adjusted));
                    }
                }
                consistency -= 0.01;
            }
            if delims.len() == 1 {
                break;
            }
        }
    }
    let delimiter = [',', '\t', ';', '|']
        .into_iter()
        .find(|d| delims.contains_key(d))
        .unwrap_or(',');
    (
        Dialect {
            delimiter,
            quote: '"',
            double: delims.is_empty(),
            spaces: lines.first().is_some_and(|line| {
                line.matches(delimiter).count() == line.matches(&format!("{delimiter} ")).count()
            }) && !delims.is_empty(),
        },
        !delims.is_empty(),
    )
}
pub(super) fn raw_rows(file: &str) -> Result<Vec<Vec<String>>, AppError> {
    let text = csv::read_sig(file)?;
    if kassiber_pytext::strip(&text).is_empty() {
        return Ok(vec![]);
    }
    // `io.StringIO(text, newline="")`: a bare carriage return ends a record.
    match parse(&text, sniff(&text), true) {
        (records, None) => Ok(records),
        (_, Some(error)) => Err(error),
    }
}
/// `csv.Sniffer().sniff(text[:8192], delimiters=",;\t|")`, falling back to
/// the excel dialect when it cannot decide, then `csv.reader` over
/// `io.StringIO(text)`. A csv.Error keeps the records completed before it.
pub(crate) fn sniffed_rows(text: &str) -> (Vec<Vec<String>>, Option<AppError>) {
    parse(text, sniff(text), false)
}
/// `bare_cr` reads lines as `io.StringIO(text, newline="")` does (any of
/// `\r`, `\n`, `\r\n` ends one); otherwise only `\n` ends a line, so a
/// carriage return must be followed by a line feed.
fn parse(text: &str, d: Dialect, bare_cr: bool) -> (Vec<Vec<String>>, Option<AppError>) {
    let mut records = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let (mut quoted, mut start, mut touched, mut count) = (false, true, false, 0);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == d.quote {
                if d.double && chars.peek() == Some(&d.quote) {
                    chars.next();
                } else {
                    quoted = false;
                    continue;
                }
            }
        } else {
            if start && d.spaces && ch == ' ' {
                touched = true;
                continue;
            }
            if start && ch == d.quote {
                quoted = true;
                start = false;
                touched = true;
                continue;
            }
            if ch == d.delimiter || ch == '\r' || ch == '\n' {
                if ch == d.delimiter || touched || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                }
                if ch != d.delimiter {
                    if ch == '\r' && !bare_cr {
                        let mut ahead = chars.clone();
                        while ahead.peek() == Some(&'\r') {
                            ahead.next();
                        }
                        if ahead.next().is_some_and(|c| c != '\n') {
                            return (records, Some(AppError::new("internal_error","new-line character seen in unquoted field - do you need to open the file with newline=''?")));
                        }
                    }
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
        start = false;
        touched = true;
        if count > 131072 {
            return (
                records,
                Some(AppError::new(
                    "internal_error",
                    "field larger than field limit (131072)",
                )),
            );
        }
    }
    if touched || !row.is_empty() {
        row.push(field);
        records.push(row);
    }
    (records, None)
}
pub(super) fn load(file: &str) -> Result<(Vec<String>, Vec<csv::Row>), AppError> {
    let records = raw_rows(file)?;
    let mut records = records
        .into_iter()
        .filter(|r| r.iter().any(|s| !kassiber_pytext::strip(s).is_empty()));
    let header: Vec<String> = records
        .next()
        .unwrap_or_default()
        .iter()
        .map(|s| kassiber_pytext::strip(s).to_owned())
        .collect();
    let rows = records
        .map(|r| {
            let mut data = serde_json::Map::new();
            for (i, key) in header.iter().enumerate().filter(|(_, k)| !k.is_empty()) {
                data.insert(
                    key.clone(),
                    json!(r.get(i).map(String::as_str).unwrap_or_default()),
                );
            }
            csv::Row {
                data: Json::Object(data),
                headers: header.iter().filter(|k| !k.is_empty()).cloned().collect(),
                rest: None,
                original: None,
            }
        })
        .collect();
    Ok((header, rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_records_end_at_any_line_break_like_python() {
        // csv.reader(io.StringIO(text, newline="")) on the same text.
        let text = "Type,Date\rtrade,2025\r\r\nx,\"a\rb\"\r";
        let rows = |bare_cr| parse(text, sniff(text), bare_cr);
        assert_eq!(
            rows(true),
            (
                vec![
                    vec!["Type".to_owned(), "Date".to_owned()],
                    vec!["trade".to_owned(), "2025".to_owned()],
                    vec![],
                    vec!["x".to_owned(), "a\rb".to_owned()],
                ],
                None
            )
        );
        // io.StringIO(text) splits lines only at "\n": a bare return is an
        // unquoted newline the csv module refuses within the first record.
        let (records, error) = rows(false);
        assert!(records.is_empty() && error.is_some());
    }
}
