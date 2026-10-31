use super::*;
use std::io::Read;

const MAX_ENTRIES: usize = 4096;
const MAX_MEMBER: usize = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 128 * 1024 * 1024;
const MAX_RATIO: usize = 200;
const MAX_CELLS: usize = 1_000_000;
const MAX_MATERIALIZED: usize = 512 * 1024 * 1024;

struct Budget {
    remaining: usize,
    cells: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            remaining: MAX_MATERIALIZED,
            cells: 0,
        }
    }
}
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<(), AppError> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(bounded)?;
        Ok(())
    }
    fn rectangle(&mut self, rows: usize, columns: usize) -> Result<(), AppError> {
        let cells = rows.checked_mul(columns).ok_or_else(bounded)?;
        self.cells = self.cells.checked_add(cells).ok_or_else(bounded)?;
        if self.cells > MAX_CELLS {
            return Err(bounded());
        }
        self.charge(
            cells
                .checked_mul(2 * std::mem::size_of::<Json>())
                .ok_or_else(bounded)?,
        )?;
        self.charge(
            rows.checked_mul(2 * std::mem::size_of::<Vec<Json>>())
                .ok_or_else(bounded)?,
        )
    }
}

fn error(message: impl std::fmt::Display) -> AppError {
    fail(
        format!("Could not read .xlsx ledger file: {message}"),
        "Re-export the file as .xlsx or CSV and try again.",
    )
}
fn bounded() -> AppError {
    error("Workbook exceeds bounded XLSX reader limits")
}
fn crc_state(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let mut n = i as u32;
            for _ in 0..8 {
                n = (n >> 1) ^ (0xedb8_8320u32.wrapping_mul(n & 1));
            }
            n
        })
    });
    bytes.iter().fold(!0u32, |n, b| {
        table[((n ^ u32::from(*b)) & 255) as usize] ^ (n >> 8)
    })
}
fn u16le(bytes: &[u8], at: usize) -> Result<usize, AppError> {
    bytes
        .get(at..at + 2)
        .map(|s| u16::from_le_bytes(s.try_into().unwrap()) as usize)
        .ok_or_else(|| error("Truncated central directory"))
}
fn u32le(bytes: &[u8], at: usize) -> Result<usize, AppError> {
    bytes
        .get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()) as usize)
        .ok_or_else(|| error("Truncated central directory"))
}
fn archive(file: &str) -> Result<BTreeMap<String, Vec<u8>>, AppError> {
    let input = std::fs::File::open(file).map_err(error)?;
    if input.metadata().map_err(error)?.len() > MAX_TOTAL as u64 {
        return Err(bounded());
    }
    let mut bytes = vec![];
    input
        .take(MAX_TOTAL as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > MAX_TOTAL {
        return Err(bounded());
    }
    let end = (bytes.len().saturating_sub(65557)..bytes.len().saturating_sub(21))
        .rev()
        .find(|i| {
            bytes.get(*i..*i + 4) == Some(b"PK\x05\x06")
                && u16le(&bytes, *i + 20).is_ok_and(|n| *i + 22 + n <= bytes.len())
        })
        .ok_or_else(|| error("File is not a zip file"))?;
    if u16le(&bytes, end + 4)? != 0 || u16le(&bytes, end + 6)? != 0 {
        return Err(error(
            "zip files that span multiple disks are not supported",
        ));
    }
    let count = u16le(&bytes, end + 10)?;
    if count > MAX_ENTRIES {
        return Err(bounded());
    }
    let size = u32le(&bytes, end + 12)?;
    let offset = u32le(&bytes, end + 16)?;
    if offset.checked_add(size).is_none_or(|n| n > end) {
        return Err(error("Bad offset for central directory"));
    }
    let mut scan = offset;
    let mut headers = vec![];
    for _ in 0..count {
        if bytes.get(scan..scan + 4) != Some(b"PK\x01\x02") {
            return Err(error("Bad magic number for central directory"));
        }
        headers.push(u32le(&bytes, scan + 42)?);
        scan = scan
            .checked_add(
                46 + u16le(&bytes, scan + 28)?
                    + u16le(&bytes, scan + 30)?
                    + u16le(&bytes, scan + 32)?,
            )
            .ok_or_else(bounded)?;
        if scan > offset + size {
            return Err(error("Truncated central directory"));
        }
    }
    headers.sort_unstable();
    let mut header_ends = BTreeMap::new();
    let mut next = offset;
    for header in headers.into_iter().rev() {
        header_ends.insert(header, next);
        next = header;
    }
    let mut cursor = offset;
    let mut total = 0usize;
    let mut result = BTreeMap::new();
    for _ in 0..count {
        if bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            return Err(error("Bad magic number for central directory"));
        }
        let method = u16le(&bytes, cursor + 10)?;
        let compressed = u32le(&bytes, cursor + 20)?;
        let decompressed = u32le(&bytes, cursor + 24)?;
        total = total.checked_add(decompressed).ok_or_else(bounded)?;
        if decompressed > MAX_MEMBER
            || total > MAX_TOTAL
            || decompressed > compressed.max(1).saturating_mul(MAX_RATIO)
        {
            return Err(bounded());
        }
        let name_len = u16le(&bytes, cursor + 28)?;
        let extra_len = u16le(&bytes, cursor + 30)?;
        let comment_len = u16le(&bytes, cursor + 32)?;
        let name = std::str::from_utf8(
            bytes
                .get(cursor + 46..cursor + 46 + name_len)
                .ok_or_else(|| error("Truncated central directory"))?,
        )
        .map_err(error)?
        .to_owned();
        let local = u32le(&bytes, cursor + 42)?;
        let crc = u32le(&bytes, cursor + 16)? as u32;
        let flags = u16le(&bytes, cursor + 8)?;
        cursor = cursor
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(bounded)?;
        if cursor > offset + size {
            return Err(error("Truncated central directory"));
        }
        if flags & 1 != 0 {
            return Err(error(format!(
                "File {} is encrypted, password required for extraction",
                kassiber_pytext::repr_str(&name)
            )));
        }
        if bytes.get(local..local + 4) != Some(b"PK\x03\x04") {
            return Err(error("Bad magic number for file header"));
        }
        let local_name_len = u16le(&bytes, local + 26)?;
        let local_extra_len = u16le(&bytes, local + 28)?;
        if bytes.get(local + 30..local + 30 + local_name_len) != Some(name.as_bytes()) {
            return Err(error("File name in directory and header differ"));
        }
        if u16le(&bytes, local + 6)? & 1 != 0 || u16le(&bytes, local + 8)? != method {
            return Err(error("Invalid ZIP file header"));
        }
        let start = local
            .checked_add(30 + local_name_len + local_extra_len)
            .ok_or_else(bounded)?;
        let stop = start.checked_add(compressed).ok_or_else(bounded)?;
        if stop > header_ends[&local].min(offset) {
            return Err(error(format!(
                "Overlapped entries: {} (possible zip bomb)",
                kassiber_pytext::repr_str(&name)
            )));
        }
        let input = bytes
            .get(start..stop)
            .ok_or_else(|| error("Truncated file header"))?;
        let mut output = vec![];
        match method {
            0 => {
                if input.len() > MAX_MEMBER {
                    return Err(bounded());
                }
                output.extend_from_slice(input);
            }
            8 => {
                flate2::read::DeflateDecoder::new(input)
                    .take(decompressed as u64 + 1)
                    .read_to_end(&mut output)
                    .map_err(error)?;
            }
            _ => return Err(error("That compression method is not supported")),
        }
        if output.len() > MAX_MEMBER || output.len() > compressed.max(1).saturating_mul(MAX_RATIO) {
            return Err(bounded());
        }
        let observed = crc_state(&output);
        if output.len() != decompressed || !observed != crc {
            return Err(error(format!(
                "Bad CRC-32 for file {}",
                kassiber_pytext::repr_str(&name)
            )));
        }
        result.insert(name, output);
    }
    Ok(result)
}
#[derive(Default)]
struct Node {
    name: String,
    qualified: String,
    namespace: String,
    namespaces: BTreeMap<String, String>,
    attrs: BTreeMap<String, String>,
    text: String,
    children: Vec<Node>,
}
impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children
            .iter()
            .find(|n| n.name == name && n.namespace == self.namespace)
    }
    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children
            .iter()
            .filter(move |n| n.name == name && n.namespace == self.namespace)
    }
    fn attr(&self, name: &str) -> &str {
        self.attrs.get(name).map(String::as_str).unwrap_or_default()
    }
    fn integer(&self, name: &str, default: usize) -> Result<usize, AppError> {
        let s = self.attr(name);
        if s.is_empty() {
            Ok(default)
        } else {
            s.parse().map_err(|_| {
                error(format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_pytext::repr_str(s)
                ))
            })
        }
    }
}
fn entities(text: &str) -> Result<String, AppError> {
    let mut out = String::new();
    let mut rest = text;
    while let Some((prefix, tail)) = rest.split_once('&') {
        out.push_str(prefix);
        let (name, after) = tail
            .split_once(';')
            .ok_or_else(|| error("Invalid XML entity"))?;
        match name {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            n if n.starts_with('#') => {
                let value = if let Some(hex) = n.strip_prefix("#x") {
                    u32::from_str_radix(hex, 16)
                } else {
                    n[1..].parse()
                }
                .map_err(|_| error("Invalid XML character reference"))?;
                out.push(
                    char::from_u32(value)
                        .filter(|c| {
                            matches!(*c, '\t' | '\n' | '\r')
                                || *c >= ' ' && !matches!(value, 0xfffe | 0xffff)
                        })
                        .ok_or_else(|| error("Invalid XML character reference"))?,
                );
            }
            _ => return Err(error("Undefined XML entity")),
        }
        rest = after;
    }
    out.push_str(rest);
    Ok(out)
}
fn xml_error(text: &str, offset: usize, message: &str) -> AppError {
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count();
    error(format!("{message}: line {line}, column {column}"))
}
fn xml_name(name: &str) -> bool {
    let start = |c: char| matches!(c, ':' | '_' | 'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}');
    let mut chars = name.chars();
    chars.next().is_some_and(start)
        && chars.all(|c| start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}'))
}
fn xml(bytes: &[u8], budget: &mut Budget) -> Result<Node, AppError> {
    parse_xml(bytes, false, budget)
}
fn metadata_xml(bytes: &[u8], budget: &mut Budget) -> Result<Node, AppError> {
    parse_xml(bytes, true, budget)
}
fn parse_xml(bytes: &[u8], metadata: bool, budget: &mut Budget) -> Result<Node, AppError> {
    budget.charge(bytes.len().checked_mul(8).ok_or_else(bounded)?)?;
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        if !bytes.len().is_multiple_of(2) {
            return Err(error("Invalid XML encoding"));
        }
        let units: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| {
                if little {
                    u16::from_le_bytes(*s)
                } else {
                    u16::from_be_bytes(*s)
                }
            })
            .collect();
        String::from_utf16(&units).map_err(error)?
    } else {
        std::str::from_utf8(bytes).map_err(error)?.to_owned()
    };
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if text.chars().any(|c| c < ' ' && !matches!(c, '\t' | '\n')) {
        return Err(error("Invalid XML character"));
    }
    let mut rest = text.as_str();
    let mut stack = vec![Node::default()];
    let mut count = 0usize;
    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            let (comment, after) = rest[4..]
                .split_once("-->")
                .ok_or_else(|| error("Invalid XML comment"))?;
            if comment.contains("--") || comment.ends_with('-') {
                return Err(error("Invalid XML comment"));
            }
            rest = after;
            continue;
        }
        if rest.starts_with("<?") {
            rest = rest[2..]
                .split_once("?>")
                .ok_or_else(|| error("Invalid XML declaration"))?
                .1;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let (s, r) = rest[9..]
                .split_once("]]>")
                .ok_or_else(|| error("Invalid XML CDATA"))?;
            stack.last_mut().unwrap().text.push_str(s);
            rest = r;
            continue;
        }
        if rest.starts_with("<!") {
            return Err(error("XML declarations and entities are not supported"));
        }
        if !rest.starts_with('<') {
            let (s, r) = rest.split_once('<').map_or((rest, ""), |(s, r)| {
                (s, &rest[s.len()..s.len() + r.len() + 1])
            });
            let decoded = entities(s).map_err(|e| {
                if e.message.contains("Undefined XML entity") {
                    xml_error(
                        &text,
                        text.len() - rest.len() + s.find('&').unwrap_or(0),
                        "undefined entity",
                    )
                } else {
                    e
                }
            })?;
            stack.last_mut().unwrap().text.push_str(&decoded);
            rest = r;
            continue;
        }
        let mut quoted = None;
        let end = rest
            .char_indices()
            .skip(1)
            .find_map(|(i, c)| {
                if quoted == Some(c) {
                    quoted = None;
                } else if quoted.is_none() && matches!(c, '"' | '\'') {
                    quoted = Some(c);
                } else if quoted.is_none() && c == '>' {
                    return Some(i);
                }
                None
            })
            .ok_or_else(|| {
                if metadata {
                    let line = text.bytes().filter(|b| *b == b'\n').count() + 1;
                    let column = text.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
                    let message = if rest == "<" {
                        "StartTag: invalid element name"
                    } else if rest.starts_with("</") {
                        "expected '>'"
                    } else {
                        "attributes construct error"
                    };
                    error(format!(
                        "{message}, line {line}, column {column} (<string>, line {line})"
                    ))
                } else {
                    xml_error(&text, text.len() - rest.len(), "unclosed token")
                }
            })?;
        let start = text.len() - rest.len();
        let inside = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(name) = inside.strip_prefix('/') {
            if stack.len() < 2 || stack.last().unwrap().qualified != name.trim() {
                return Err(xml_error(&text, start + 2, "mismatched tag"));
            }
            let node = stack.pop().unwrap();
            stack.last_mut().unwrap().children.push(node);
            continue;
        }
        count += 1;
        if count > MAX_CELLS * 4 || stack.len() > 64 {
            return Err(bounded());
        }
        budget.charge(2 * std::mem::size_of::<Node>())?;
        let empty = inside.ends_with('/');
        let inside = if empty {
            &inside[..inside.len() - 1]
        } else {
            inside
        };
        let end_name = inside.find(char::is_whitespace).unwrap_or(inside.len());
        let name = &inside[..end_name];
        if !xml_name(name) {
            return Err(xml_error(
                &text,
                start + 1,
                "not well-formed (invalid token)",
            ));
        }
        budget.charge(name.len() * 2)?;
        let inherited: usize = stack
            .last()
            .unwrap()
            .namespaces
            .iter()
            .map(|(key, value)| 128 + key.len() + value.len())
            .sum();
        budget.charge(inherited)?;
        let mut node = Node {
            name: name.rsplit(':').next().unwrap().into(),
            qualified: name.into(),
            namespaces: stack.last().unwrap().namespaces.clone(),
            ..Default::default()
        };
        let mut raw_attrs = BTreeMap::new();
        let mut attrs = inside[end_name..].trim();
        while !attrs.is_empty() {
            let (name, after) = attrs
                .split_once('=')
                .ok_or_else(|| error("Invalid XML attribute"))?;
            let after = after.trim_start();
            let quote = after
                .chars()
                .next()
                .filter(|c| matches!(c, '"' | '\''))
                .ok_or_else(|| error("Invalid XML attribute"))?;
            let (value, after) = after[1..]
                .split_once(quote)
                .ok_or_else(|| error("Invalid XML attribute"))?;
            if value.contains('<') {
                return Err(error("Invalid XML attribute"));
            }
            let name = name.trim();
            if !xml_name(name) {
                return Err(xml_error(
                    &text,
                    start + 1 + end_name + inside[end_name..].len() - attrs.len(),
                    "not well-formed (invalid token)",
                ));
            }
            budget.charge(128 + name.len() + value.len() * 4)?;
            if raw_attrs
                .insert(
                    name.to_owned(),
                    entities(&value.replace(['\t', '\n'], " "))?,
                )
                .is_some()
            {
                return Err(error("Duplicate XML attribute"));
            }
            attrs = after.trim_start();
        }
        for (name, value) in &raw_attrs {
            if name == "xmlns" {
                budget.charge(128 + value.len())?;
                node.namespaces.insert(String::new(), value.clone());
            } else if let Some(prefix) = name.strip_prefix("xmlns:") {
                budget.charge(128 + prefix.len() + value.len())?;
                node.namespaces.insert(prefix.into(), value.clone());
            }
        }
        let prefix = name.split_once(':').map(|(p, _)| p).unwrap_or_default();
        budget.charge(node.namespaces.get(prefix).map_or(0, String::len))?;
        node.namespace = node.namespaces.get(prefix).cloned().unwrap_or_default();
        if !prefix.is_empty() && node.namespace.is_empty() {
            return Err(error("Undefined XML namespace"));
        }
        for (name, value) in raw_attrs {
            if name == "xmlns" || name.starts_with("xmlns:") {
                continue;
            }
            let expanded = if let Some((prefix, local)) = name.split_once(':') {
                let ns = if prefix == "xml" {
                    "http://www.w3.org/XML/1998/namespace"
                } else {
                    node.namespaces
                        .get(prefix)
                        .map(String::as_str)
                        .ok_or_else(|| error("Undefined XML namespace"))?
                };
                budget.charge(ns.len() + local.len() + 2)?;
                format!("{{{ns}}}{local}")
            } else {
                name
            };
            if node.attrs.insert(expanded, value).is_some() {
                return Err(error("Duplicate XML attribute"));
            }
        }
        if empty {
            stack.last_mut().unwrap().children.push(node);
        } else {
            stack.push(node);
        }
    }
    if stack.len() != 1 || stack[0].children.is_empty() {
        return Err(xml_error(&text, text.len(), "no element found"));
    }
    if stack[0].children.len() != 1 || !stack[0].text.trim().is_empty() {
        return Err(error("Invalid XML document"));
    }
    Ok(stack.pop().unwrap().children.remove(0))
}
fn member<'a>(archive: &'a BTreeMap<String, Vec<u8>>, name: &str) -> Result<&'a [u8], AppError> {
    archive.get(name).map(Vec::as_slice).ok_or_else(|| {
        error(format!(
            "\"There is no item named '{}' in the archive\"",
            name
        ))
    })
}
fn path(base: &str, target: &str) -> String {
    let full = if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("{base}/{target}")
    };
    let mut parts = vec![];
    for part in full.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}
fn rich(node: &Node) -> String {
    let mut out = node.child("t").map(|n| n.text.clone()).unwrap_or_default();
    for run in node.children("r") {
        if let Some(t) = run.child("t") {
            out.push_str(&t.text);
        }
    }
    out
}
fn rich_size(node: &Node) -> usize {
    node.child("t").map_or(0, |n| n.text.len())
        + node
            .children("r")
            .filter_map(|n| n.child("t"))
            .map(|n| n.text.len())
            .sum::<usize>()
}
fn date_style(format: &str) -> (bool, bool) {
    let first = format.split(';').next().unwrap_or_default();
    let chars: Vec<_> = first.chars().collect();
    let mut clean = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '"' {
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                at += 1;
            }
        } else if chars[at] == '[' {
            let start = at;
            while at < chars.len() && chars[at] != ']' {
                at += 1;
            }
            let part = chars[start..at.min(chars.len())].iter().collect::<String>();
            if part
                .to_lowercase()
                .chars()
                .any(|c| matches!(c, 'h' | 'm' | 's'))
                && part[1..]
                    .chars()
                    .all(|c| matches!(c, 'h' | 'H' | 'm' | 'M' | 's' | 'S' | ':'))
            {
                clean.push_str(&part);
                clean.push(']');
            }
        } else {
            clean.push(chars[at]);
        }
        at += 1;
    }
    let duration = ["[h]", "[hh]", "[m]", "[mm]", "[s]", "[ss]"]
        .iter()
        .any(|s| clean.to_lowercase().contains(s));
    let date = clean.char_indices().any(|(i, c)| {
        matches!(c, 'd' | 'D' | 'm' | 'M' | 'h' | 'H' | 'y' | 'Y' | 's' | 'S')
            && (i == 0 || !clean[..i].ends_with(['_', '\\']))
    });
    (date, duration)
}
fn serial(value: f64, mac: bool, duration: bool) -> Json {
    if !value.is_finite() || value.abs() > 4_000_000.0 {
        return json!("#VALUE!");
    }
    let day = value.floor();
    let fraction = value - day;
    let fractional_millis = if duration {
        ((fraction * 86_400.0 * 1_000_000.0).round_ties_even() / 1000.0).round_ties_even()
    } else {
        (fraction * 86_400.0 * 1000.0).round_ties_even()
    } as i64;
    let days = day as i64 + fractional_millis.div_euclid(86_400_000);
    let rest = fractional_millis.rem_euclid(86_400_000);
    let clock = format!(
        "{}:{:02}:{:02}",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60
    );
    let fraction = if rest % 1000 != 0 {
        format!(".{:06}", rest % 1000 * 1000)
    } else {
        String::new()
    };
    if duration {
        let prefix = if days == 0 {
            String::new()
        } else {
            format!("{days} day{}, ", if days.abs() == 1 { "" } else { "s" })
        };
        return json!(format!("{prefix}{clock}{fraction}"));
    }
    if (0.0..1.0).contains(&value) && fractional_millis < 86_400_000 {
        return json!(format!(
            "{:02}:{:02}:{:02}{fraction}",
            rest / 3_600_000,
            rest / 60_000 % 60,
            rest / 1000 % 60
        ));
    }
    let days = if !mac && value > 0.0 && value < 60.0 {
        days + 1
    } else {
        days
    };
    let epoch = if mac { "1904-01-01" } else { "1899-12-30" };
    let stamp = crate::time::shifted(epoch, days * 86400).unwrap_or_else(|_| "#VALUE!".into());
    if stamp == "#VALUE!" {
        return json!(stamp);
    }
    json!(format!(
        "{}T{:02}:{:02}:{:02}{fraction}",
        &stamp[..10],
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60
    ))
}
fn worksheet(
    node: &Node,
    shared: &[String],
    styles: &[(bool, bool)],
    mac: bool,
    budget: &mut Budget,
) -> Result<Vec<Vec<Json>>, AppError> {
    if node.namespace != "http://schemas.openxmlformats.org/spreadsheetml/2006/main" {
        return Ok(vec![]);
    }
    let Some(data) = node.child("sheetData") else {
        return Ok(vec![]);
    };
    let mut values = BTreeMap::new();
    let (mut max_row, mut max_col, mut current) = (0, 0, 0);
    for row in data.children("row") {
        current = row.integer("r", current + 1)?;
        let mut column = 0usize;
        for c in row.children("c") {
            let mut r = current;
            if !c.attr("r").is_empty() {
                let coord = c.attr("r");
                let split = coord
                    .find(|c: char| c.is_ascii_digit())
                    .ok_or_else(|| error(format!("Invalid cell coordinates ({coord})")))?;
                column = 0;
                for b in coord[..split].bytes() {
                    if !b.is_ascii_alphabetic() {
                        return Err(error("Invalid cell coordinates"));
                    }
                    column = column
                        .checked_mul(26)
                        .and_then(|n| n.checked_add((b.to_ascii_uppercase() - b'A' + 1) as usize))
                        .ok_or_else(bounded)?;
                }
                r = coord[split..]
                    .parse()
                    .map_err(|_| error("Invalid cell coordinates"))?;
            } else {
                column += 1;
            }
            if r == 0 || column == 0 || r > 1_048_576 || column > 16_384 {
                return Err(error("Invalid cell coordinates"));
            }
            max_row = max_row.max(r);
            max_col = max_col.max(column);
            if max_row.checked_mul(max_col).is_none_or(|n| n > MAX_CELLS) {
                return Err(bounded());
            }
            let style = c.integer("s", 0)?;
            let raw = c
                .child("v")
                .map(|v| v.text.as_str())
                .filter(|s| !s.is_empty());
            budget.charge(128 + 2 * std::mem::size_of::<Json>())?;
            let string_bytes = if c.attr("t") == "inlineStr" {
                c.child("is").map_or(0, rich_size)
            } else if c.attr("t") == "s" {
                raw.and_then(|s| s.parse::<usize>().ok())
                    .and_then(|i| shared.get(i))
                    .map_or(0, String::len)
            } else {
                raw.map_or(0, str::len).max(64)
            };
            budget.charge(string_bytes.checked_mul(2).ok_or_else(bounded)?)?;
            let value = if c.attr("t") == "inlineStr" {
                c.child("is").map(|n| json!(rich(n))).unwrap_or(json!(""))
            } else if let Some(raw) = raw {
                match c.attr("t") {
                    "s" => json!(shared
                        .get(
                            raw.parse::<usize>()
                                .map_err(|_| error("invalid shared string index"))?
                        )
                        .ok_or_else(|| error("list index out of range"))?),
                    "b" => json!(
                        raw.parse::<i64>()
                            .map_err(|_| error("invalid boolean cell"))?
                            != 0
                    ),
                    "str" | "e" => json!(raw),
                    "d" => json!(if raw.len() == 10 {
                        raw.to_owned()
                    } else {
                        raw.replace('Z', "")
                    }),
                    "" | "n" => {
                        let number = raw
                            .parse::<f64>()
                            .map_err(|_| error("Invalid numeric cell"))?;
                        let (date, duration) = styles.get(style).copied().unwrap_or_default();
                        if date {
                            serial(number, mac, duration)
                        } else if raw.contains(['.', 'e', 'E']) {
                            serde_json::Number::from_f64(number)
                                .map(Json::Number)
                                .unwrap_or(Json::Null)
                        } else {
                            let integer = kassiber_pytext::parse_int(raw)
                                .map_err(|_| error("Invalid numeric cell"))?;
                            serde_json::from_str::<Json>(&integer.to_string()).map_err(error)?
                        }
                    }
                    _ => json!(raw),
                }
            } else {
                json!("")
            };
            values.insert((r - 1, column - 1), value);
            if style >= styles.len().max(1) {
                return Err(error("list index out of range"));
            }
        }
    }
    budget.rectangle(max_row, max_col)?;
    let mut out = vec![vec![json!(""); max_col]; max_row];
    for ((r, c), value) in values {
        out[r][c] = value;
    }
    Ok(out)
}
pub(super) fn read(file: &str) -> Result<Vec<Vec<Json>>, AppError> {
    let archive = archive(file)?;
    let mut budget = Budget::default();
    let manifest = metadata_xml(member(&archive, "[Content_Types].xml")?, &mut budget)?;
    let workbook_name = manifest
        .children("Override")
        .find(|n| {
            [
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            ]
            .contains(&n.attr("ContentType"))
        })
        .map(|n| n.attr("PartName").trim_start_matches('/'))
        .ok_or_else(|| error("File contains no valid workbook part"))?;
    let workbook = metadata_xml(member(&archive, workbook_name)?, &mut budget)?;
    let base = workbook_name
        .rsplit_once('/')
        .map(|(p, _)| p)
        .unwrap_or_default();
    let filename = workbook_name.rsplit('/').next().unwrap();
    let relname = format!("{base}/_rels/{filename}.rels");
    let rels = metadata_xml(member(&archive, &relname)?, &mut budget)?;
    let mut targets = BTreeMap::new();
    for rel in rels.children("Relationship") {
        if rel.attr("TargetMode") != "External" {
            targets.insert(
                rel.attr("Id").to_owned(),
                (path(base, rel.attr("Target")), rel.attr("Type").to_owned()),
            );
        }
    }
    let mut shared = vec![];
    if let Some(part) = manifest.children("Override").find(|n| {
        n.attr("ContentType")
            == "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"
    }) {
        let node = metadata_xml(
            member(&archive, part.attr("PartName").trim_start_matches('/'))?,
            &mut budget,
        )?;
        for si in node.children("si") {
            budget.charge(2 * rich_size(si) + 2 * std::mem::size_of::<String>())?;
            shared.push(rich(si).replace("x005F_", ""));
        }
    }
    let mut styles = vec![];
    if let Some(part) = manifest.children("Override").find(|n| {
        n.attr("ContentType")
            == "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"
    }) {
        let node = metadata_xml(
            member(&archive, part.attr("PartName").trim_start_matches('/'))?,
            &mut budget,
        )?;
        let mut custom = BTreeMap::new();
        if let Some(formats) = node.child("numFmts") {
            for f in formats.children("numFmt") {
                custom.insert(f.integer("numFmtId", 0)?, date_style(f.attr("formatCode")));
            }
        }
        if let Some(formats) = node.child("cellXfs") {
            for f in formats.children("xf") {
                let id = f.integer("numFmtId", 0)?;
                styles.push(
                    custom
                        .get(&id)
                        .copied()
                        .unwrap_or(((14..=22).contains(&id) || (45..=47).contains(&id), id == 46)),
                );
            }
        }
    }
    let mac = workbook
        .child("workbookPr")
        .is_some_and(|n| ["1", "true"].contains(&n.attr("date1904")));
    let active = workbook
        .child("bookViews")
        .and_then(|n| n.child("workbookView"))
        .map(|n| n.integer("activeTab", 0))
        .transpose()?
        .unwrap_or(0);
    let mut sheets = vec![];
    if let Some(list) = workbook.child("sheets") {
        for sheet in list.children("sheet") {
            let Some((target, kind)) =
                targets.get(sheet.attr(
                    "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id",
                ))
            else {
                continue;
            };
            if kind.ends_with("/chartsheet") {
                sheets.push(None);
                continue;
            }
            let Some(content) = archive.get(target) else {
                continue;
            };
            let node = xml(content, &mut budget)?;
            let rows = worksheet(&node, &shared, &styles, mac, &mut budget).map_err(|e| {
                if ["Invalid numeric cell", "invalid boolean cell", "Invalid cell coordinates", "invalid literal for int()"].iter().any(|s| e.message.contains(s)) {
                    error(format!("Unable to read workbook: could not read worksheets from {file}.\nThis is most probably because the workbook source files contain some invalid XML.\nPlease see the exception for more details."))
                } else {
                    e
                }
            })?;
            sheets.push(Some(rows));
        }
    }
    let selected = sheets
        .iter()
        .position(|sheet| {
            sheet
                .as_ref()
                .is_some_and(|rows| rows.iter().take(10).any(|r| has_header(r)))
        })
        .unwrap_or(active);
    match sheets.into_iter().nth(selected) {
        Some(Some(rows)) => Ok(rows),
        other => Err(AppError::new(
            "internal_error",
            if other.is_some() {
                "'Chartsheet' object has no attribute 'iter_rows'"
            } else {
                "'NoneType' object has no attribute 'iter_rows'"
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn refusal<T>(result: Result<T, AppError>) -> AppError {
        match result {
            Err(e) => e,
            Ok(_) => panic!("bounded reader must refuse"),
        }
    }

    fn bytes() -> Vec<u8> {
        let cases: Json = serde_json::from_str(include_str!(
            "../../../tests/vectors/python_ledger_imports.json"
        ))
        .unwrap();
        cases
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "ledger:xlsx-native")
            .unwrap()["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as u8)
            .collect()
    }
    fn check(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, AppError> {
        let path = std::env::temp_dir().join(format!(
            "bounded-ledger-{}-{:?}.xlsx",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, bytes).unwrap();
        let result = archive(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        result
    }
    fn set32(bytes: &mut [u8], at: usize, value: usize) {
        bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
    }

    #[test]
    fn ledger_zip_limits_precede_decompression_and_check_integrity() {
        let original = bytes();
        assert!(check(&original).is_ok());
        let end = original.len() - 22;
        let central = u32le(&original, end + 16).unwrap();
        let mut count = original.clone();
        count[end + 10..end + 12].copy_from_slice(&((MAX_ENTRIES + 1) as u16).to_le_bytes());
        assert!(check(&count).unwrap_err().message.contains("bounded XLSX"));
        let mut member = original.clone();
        set32(&mut member, central + 20, MAX_MEMBER + 1);
        set32(&mut member, central + 24, MAX_MEMBER + 1);
        assert!(check(&member).unwrap_err().message.contains("bounded XLSX"));
        let mut ratio = original.clone();
        let compressed = u32le(&ratio, central + 20).unwrap();
        set32(&mut ratio, central + 24, compressed * (MAX_RATIO + 1));
        assert!(check(&ratio).unwrap_err().message.contains("bounded XLSX"));
        let mut crc = original.clone();
        crc[central + 16] ^= 1;
        assert!(check(&crc).unwrap_err().message.contains("Bad CRC-32"));
        let mut name = original.clone();
        let offset = u32le(&name, central + 42).unwrap();
        name[offset + 30] ^= 1;
        assert!(check(&name).unwrap_err().message.contains("header differ"));
    }

    #[test]
    fn ledger_zip_total_expansion_is_bounded_across_duplicate_members() {
        let mut seed = 21u32;
        let block: Vec<u8> = (0..16384)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        let payload: Vec<u8> = block.iter().copied().cycle().take(MAX_MEMBER).collect();
        let mut encoder = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::fast());
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();
        assert!(payload.len() <= compressed.len() * MAX_RATIO);
        let crc = crc_state(&payload);
        let mut data = vec![];
        let mut directory = vec![];
        for _ in 0..5 {
            let offset = data.len();
            let mut local = vec![0; 30];
            local[..4].copy_from_slice(b"PK\x03\x04");
            local[8..10].copy_from_slice(&8u16.to_le_bytes());
            set32(&mut local, 14, !crc as usize);
            set32(&mut local, 18, compressed.len());
            set32(&mut local, 22, payload.len());
            local[26..28].copy_from_slice(&1u16.to_le_bytes());
            data.extend(local);
            data.push(b'x');
            data.extend_from_slice(&compressed);
            let mut central = vec![0; 46];
            central[..4].copy_from_slice(b"PK\x01\x02");
            central[10..12].copy_from_slice(&8u16.to_le_bytes());
            set32(&mut central, 16, !crc as usize);
            set32(&mut central, 20, compressed.len());
            set32(&mut central, 24, payload.len());
            central[28..30].copy_from_slice(&1u16.to_le_bytes());
            set32(&mut central, 42, offset);
            directory.extend(central);
            directory.push(b'x');
        }
        let offset = data.len();
        let size = directory.len();
        data.extend(directory);
        let mut end = vec![0; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8..10].copy_from_slice(&5u16.to_le_bytes());
        end[10..12].copy_from_slice(&5u16.to_le_bytes());
        set32(&mut end, 12, size);
        set32(&mut end, 16, offset);
        data.extend(end);
        assert!(refusal(check(&data)).message.contains("bounded XLSX"));
    }

    #[test]
    fn ledger_xml_limits_and_namespaces_prevent_ambiguous_cells() {
        let nested = format!("{}{}", "<a>".repeat(65), "</a>".repeat(65));
        assert!(xml(nested.as_bytes(), &mut Budget::default())
            .err()
            .unwrap()
            .message
            .contains("bounded XLSX"));
        for text in [
            "<a x='<'/>",
            "<a><!-- invalid -- comment --></a>",
            "<a><b></a>",
            "<!DOCTYPE a SYSTEM 'http://127.0.0.1/no-contact'><a/>",
        ] {
            assert!(xml(text.as_bytes(), &mut Budget::default()).is_err());
        }
        let foreign=xml(br#"<worksheet xmlns="foreign"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Amount</t></is></c></row></sheetData></worksheet>"#, &mut Budget::default()).unwrap();
        assert!(worksheet(&foreign, &[], &[], false, &mut Budget::default())
            .unwrap()
            .is_empty());
        let sparse=xml(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row><c r="ALL1001"><v>1</v></c></row></sheetData></worksheet>"#, &mut Budget::default()).unwrap();
        assert!(
            refusal(worksheet(&sparse, &[], &[], false, &mut Budget::default()))
                .message
                .contains("bounded XLSX")
        );
        let oversized_invalid=xml(br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row><c r="ALL1001"><v>invalid</v></c></row></sheetData></worksheet>"#, &mut Budget::default()).unwrap();
        assert!(refusal(worksheet(
            &oversized_invalid,
            &[],
            &[],
            false,
            &mut Budget::default()
        ))
        .message
        .contains("bounded XLSX"));
    }

    #[test]
    fn ledger_materialization_limits_cover_namespace_attributes_strings_and_workbook_rectangles() {
        let small = |remaining| Budget {
            remaining,
            cells: 0,
        };
        let namespace = "n".repeat(4096);
        let inherited = format!("<a xmlns:p='{namespace}'>{}</a>", "<b/>".repeat(20));
        assert!(refusal(xml(inherited.as_bytes(), &mut small(90_000)))
            .message
            .contains("bounded XLSX"));
        let attrs = (0..20).map(|n| format!("p:a{n}='1' ")).collect::<String>();
        let expanded = format!("<a xmlns:p='{namespace}' {attrs}/>");
        assert!(refusal(xml(expanded.as_bytes(), &mut small(90_000)))
            .message
            .contains("bounded XLSX"));
        let sheet = |cells: &str| {
            xml(format!("<worksheet xmlns='http://schemas.openxmlformats.org/spreadsheetml/2006/main'><sheetData><row>{cells}</row></sheetData></worksheet>").as_bytes(), &mut Budget::default()).unwrap()
        };
        let repeated = sheet(&"<c t='s'><v>0</v></c>".repeat(20));
        assert!(refusal(worksheet(
            &repeated,
            &[namespace],
            &[],
            false,
            &mut small(32_000)
        ))
        .message
        .contains("bounded XLSX"));
        let sparse = sheet("<c r='CV100'><v>1</v></c>");
        assert!(
            refusal(worksheet(&sparse, &[], &[], false, &mut small(8192)))
                .message
                .contains("bounded XLSX")
        );
        let pair = sheet("<c><v>1</v></c><c><v>2</v></c>");
        let mut aggregate = Budget {
            remaining: MAX_MATERIALIZED,
            cells: MAX_CELLS - 3,
        };
        assert_eq!(
            worksheet(&pair, &[], &[], false, &mut aggregate).unwrap(),
            vec![vec![json!(1), json!(2)]]
        );
        assert!(refusal(worksheet(&pair, &[], &[], false, &mut aggregate))
            .message
            .contains("bounded XLSX"));
    }
}
