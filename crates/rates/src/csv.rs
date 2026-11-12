//! Local Kraken OHLCVT ingestion. Archive members are read without extraction.
use super::*;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

fn io_error(_: impl std::fmt::Display) -> AppError {
    AppError::new("validation", "Could not read Kraken rate source")
}
fn archive_error() -> AppError {
    validation("Invalid or unsupported Kraken ZIP archive")
}
fn u16le(b: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2)
            .ok_or_else(archive_error)?
            .try_into()
            .unwrap(),
    ))
}
fn u32le(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4)
            .ok_or_else(archive_error)?
            .try_into()
            .unwrap(),
    ))
}
fn u64le(b: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        b.get(at..at + 8)
            .ok_or_else(archive_error)?
            .try_into()
            .unwrap(),
    ))
}
struct Member {
    name: String,
    offset: u64,
    compressed: u64,
    size: u64,
    method: u16,
    crc: u32,
}
// ZIP's central directory carries sizes even for streamed archives. ZIP64 is
// needed for Kraken's full-history archives. No path is ever used as a write path.
fn zip_members(file: &mut File) -> Result<Vec<Member>> {
    let size = file.metadata().map_err(io_error)?.len();
    let tail_len = size.min(65557);
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(io_error)?;
    let mut tail = vec![0; tail_len as usize];
    file.read_exact(&mut tail).map_err(io_error)?;
    let at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|i| {
            tail.get(*i..*i + 4) == Some(b"PK\x05\x06")
                && u16le(&tail, *i + 20)
                    .ok()
                    .is_some_and(|n| *i + 22 + n as usize == tail.len())
        })
        .ok_or_else(archive_error)?;
    if u16le(&tail, at + 4)? != 0 || u16le(&tail, at + 6)? != 0 {
        return Err(archive_error());
    }
    let mut count = u16le(&tail, at + 10)? as u64;
    let mut offset = u32le(&tail, at + 16)? as u64;
    if count == 65535 || offset == u32::MAX as u64 {
        let locator = at.checked_sub(20).ok_or_else(archive_error)?;
        if tail.get(locator..locator + 4) != Some(b"PK\x06\x07")
            || u32le(&tail, locator + 4)? != 0
            || u32le(&tail, locator + 16)? != 1
        {
            return Err(archive_error());
        }
        file.seek(SeekFrom::Start(u64le(&tail, locator + 8)?))
            .map_err(io_error)?;
        let mut record = [0; 56];
        file.read_exact(&mut record).map_err(io_error)?;
        if &record[..4] != b"PK\x06\x06" || u32le(&record, 16)? != 0 || u32le(&record, 20)? != 0 {
            return Err(archive_error());
        }
        count = u64le(&record, 32)?;
        offset = u64le(&record, 48)?;
    }
    if count > 1_000_000 || offset > size {
        return Err(archive_error());
    }
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    let mut out = Vec::new();
    for _ in 0..count {
        let mut header = [0; 46];
        file.read_exact(&mut header).map_err(io_error)?;
        if &header[..4] != b"PK\x01\x02" || u16le(&header, 8)? & 1 != 0 {
            return Err(archive_error());
        }
        let mut name = vec![0; u16le(&header, 28)? as usize];
        file.read_exact(&mut name).map_err(io_error)?;
        // Kraken filenames and directory components are ASCII/UTF-8.
        let name = String::from_utf8(name).map_err(io_error)?;
        let mut extra = vec![0; u16le(&header, 30)? as usize];
        file.read_exact(&mut extra).map_err(io_error)?;
        file.seek(SeekFrom::Current(u16le(&header, 32)? as i64))
            .map_err(io_error)?;
        let mut member = Member {
            name,
            offset: u32le(&header, 42)? as u64,
            compressed: u32le(&header, 20)? as u64,
            size: u32le(&header, 24)? as u64,
            method: u16le(&header, 10)?,
            crc: u32le(&header, 16)?,
        };
        let mut at = 0;
        while at + 4 <= extra.len() {
            let tag = u16le(&extra, at)?;
            let len = u16le(&extra, at + 2)? as usize;
            let end = at + 4 + len;
            if end > extra.len() {
                return Err(archive_error());
            }
            if tag == 1 {
                let mut cursor = at + 4;
                for field in [&mut member.size, &mut member.compressed, &mut member.offset] {
                    if *field == u32::MAX as u64 {
                        *field = u64le(&extra, cursor)?;
                        cursor += 8;
                    }
                }
            }
            at = end;
        }
        if !member.name.ends_with('/') {
            out.push(member)
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
fn read_zip_member(file: &mut File, member: &Member) -> Result<String> {
    file.seek(SeekFrom::Start(member.offset))
        .map_err(io_error)?;
    let mut h = [0; 30];
    file.read_exact(&mut h).map_err(io_error)?;
    if &h[..4] != b"PK\x03\x04" || u16le(&h, 6)? & 1 != 0 || u16le(&h, 8)? != member.method {
        return Err(archive_error());
    }
    file.seek(SeekFrom::Current(
        i64::from(u16le(&h, 26)?) + i64::from(u16le(&h, 28)?),
    ))
    .map_err(io_error)?;
    // Bound a single member, including malformed compressed streams. Full
    // history uses multiple CSVs and never requires archive extraction on disk.
    if member.size > 1024 * 1024 * 1024 {
        return Err(validation("Kraken CSV member exceeds 1 GiB"));
    }
    let input = file.take(member.compressed);
    let mut reader: Box<dyn Read + '_> = match member.method {
        0 => Box::new(input),
        8 => Box::new(flate2::read::DeflateDecoder::new(input)),
        _ => return Err(archive_error()),
    };
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(member.size + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let mut crc = 0xffff_ffffu32;
    for b in &bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32.wrapping_mul(crc & 1));
        }
    }
    if bytes.len() as u64 != member.size || !crc != member.crc {
        return Err(archive_error());
    }
    String::from_utf8(bytes).map_err(io_error)
}
fn member_pair(name: &str) -> Option<(String, i64)> {
    let base = name.rsplit('/').next()?;
    if !base.to_lowercase().ends_with(".csv") {
        return None;
    }
    let (code, interval) = base[..base.len() - 4].rsplit_once('_')?;
    let interval = match interval {
        "1" => 1,
        "60" => 60,
        "1440" => 1440,
        _ => return None,
    };
    let code = code.trim().to_uppercase();
    let quote = code
        .strip_prefix("XXBT")
        .or_else(|| code.strip_prefix("XBT"))?;
    let quote = quote
        .strip_prefix('Z')
        .filter(|v| ["EUR", "USD", "DAI", "USDC", "USDT"].contains(v))
        .unwrap_or(quote);
    ["EUR", "USD"]
        .contains(&quote)
        .then(|| (format!("BTC-{quote}"), interval))
}
/// CPython csv.reader default dialect: comma, quote doubling, embedded lines,
/// no whitespace stripping outside a quoted field (numeric coercion strips).
fn records(input: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = input.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"')
                } else {
                    quoted = false
                }
            } else {
                field.push(c)
            };
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                started = true
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                started = true
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                if started || !field.is_empty() || !row.is_empty() {
                    row.push(std::mem::take(&mut field));
                }
                out.push(std::mem::take(&mut row));
                started = false;
            }
            _ => {
                field.push(c);
                started = true
            }
        }
    }
    if started || !field.is_empty() || !row.is_empty() {
        row.push(field);
        out.push(row)
    }
    out
}
fn expand_path(raw: &str) -> PathBuf {
    if let Some(tail) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(tail);
        }
    }
    raw.into()
}
pub fn sync(conn: &Connection, pair: Option<&str>, path: &str) -> Result<Value> {
    let filter = pair.map(require_supported_pair).transpose()?;
    let root = expand_path(path);
    if !root.exists() {
        return Err(AppError::new(
            "not_found",
            format!("Rate source path does not exist: {}", root.display()),
        )
        .with_hint(
            "Pass --path to a Kraken OHLCVT .csv file, .zip archive, or extracted directory",
        ));
    }
    let zipped = root
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    let mut archive = if zipped {
        Some(File::open(&root).map_err(io_error)?)
    } else {
        None
    };
    let members = if let Some(file) = &mut archive {
        zip_members(file)?
    } else {
        Vec::new()
    };
    let names = if zipped {
        members.iter().map(|m| m.name.clone()).collect::<Vec<_>>()
    } else if root.is_dir() {
        fs::read_dir(&root)
            .map_err(io_error)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file() && e.file_name().to_string_lossy().ends_with(".csv"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    } else {
        vec![root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()]
    };
    ingest(conn, filter, names, |name| {
        if let Some(file) = &mut archive {
            read_zip_member(file, members.iter().find(|m| m.name == name).unwrap())
        } else {
            fs::read_to_string(if root.is_dir() {
                root.join(name)
            } else {
                root.clone()
            })
            .map_err(io_error)
        }
    })
}
fn ingest(
    conn: &Connection,
    filter: Option<String>,
    mut names: Vec<String>,
    mut read: impl FnMut(&str) -> Result<String>,
) -> Result<Value> {
    names.sort();
    let mut unique = BTreeMap::new();
    let mut skipped_files = 0;
    for name in names {
        let Some((pair, interval)) = member_pair(&name) else {
            skipped_files += 1;
            continue;
        };
        if filter.as_ref().is_some_and(|p| p != &pair)
            || unique.contains_key(&(pair.clone(), interval))
        {
            skipped_files += 1;
            continue;
        }
        unique.insert((pair, interval), name);
    }
    let mut selected: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for ((pair, interval), name) in unique {
        if let Some((current, _)) = selected.get(&pair) {
            skipped_files += 1;
            if interval >= *current {
                continue;
            }
        }
        selected.insert(pair, (interval, name));
    }
    let fetched_at = time::iso_now();
    let mut result = Vec::new();
    // Python reads selected members in lexical member order, then returns pairs
    // in lexical pair order. This matters for partially failed ingestion.
    let mut selected: Vec<_> = selected.into_iter().collect();
    selected.sort_by(|a, b| a.1 .1.cmp(&b.1 .1));
    for (pair, (interval, name)) in selected {
        let input = read(&name)?;
        let granularity = match interval {
            1 => "minute",
            60 => "hourly",
            _ => "daily",
        };
        let mut samples = 0;
        let mut skipped = 0;
        let mut first: Option<String> = None;
        let mut last: Option<String> = None;
        for row in records(&input) {
            let row: Vec<_> = row.iter().map(|s| kassiber_pytext::strip(s)).collect();
            let parsed = (|| -> Option<(String, i64)> {
                if row.len() != 7 {
                    return None;
                }
                let seconds = i64::try_from(kassiber_pytext::parse_int(row[0]).ok()?).ok()?;
                let close = time::epoch(seconds.checked_add(interval * 60)?)?;
                time::epoch(seconds)?;
                for price in &row[1..5] {
                    if kassiber_pydecimal::finite_price(price).ok()? <= Decimal::zero() {
                        return None;
                    }
                }
                // Empty volume/prices use Python dec's default zero.
                if row[5].is_empty() {
                    return None;
                }
                let vol = kassiber_pydecimal::finite_price(row[5]).ok()?;
                if vol < Decimal::zero() {
                    return None;
                }
                let trades = i64::try_from(kassiber_pytext::parse_int(row[6]).ok()?).ok()?;
                if trades < 0 {
                    return None;
                }
                Some((close, trades))
            })();
            let Some((ts, trades)) = parsed else {
                skipped += 1;
                continue;
            };
            upsert(
                conn,
                &Sample {
                    pair: &pair,
                    timestamp: &ts,
                    rate: row[4],
                    source: "kraken-csv",
                    fetched_at: &fetched_at,
                    granularity: Some(granularity),
                    method: Some("ohlcvt_csv"),
                    open: Some(row[1]),
                    high: Some(row[2]),
                    low: Some(row[3]),
                    close: Some(row[4]),
                    volume: Some(row[5]),
                    trades: Some(trades),
                },
            )?;
            samples += 1;
            if first.as_ref().is_none_or(|s| ts < *s) {
                first = Some(ts.clone())
            }
            if last.as_ref().is_none_or(|s| ts > *s) {
                last = Some(ts)
            }
        }
        if samples > 0 {
            invalidate(conn, &pair)?
        }
        result.push(dict([
            ("pair", s(pair)),
            ("source", s("kraken-csv")),
            ("samples", Value::from(samples)),
            ("rows", Value::from(samples)),
            ("files", Value::from(1)),
            ("skipped_rows", Value::from(skipped)),
            ("skipped_files", Value::from(skipped_files)),
            ("first_timestamp", optional(first.as_deref())),
            ("last_timestamp", optional(last.as_deref())),
            ("granularity", s(granularity)),
            ("method", s("ohlcvt_csv")),
            ("fetched_at", s(fetched_at.as_str())),
        ]));
    }
    result.sort_by(|a, b| text(a, "pair").cmp(&text(b, "pair")));
    Ok(Value::List(result))
}
pub fn bundled_path(hourly: bool) -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/data/kraken")).join(if hourly {
        "btc_hourly"
    } else {
        "btc_daily"
    })
}

pub fn sync_bundled(conn: &Connection, pair: Option<&str>, hourly: bool) -> Result<Value> {
    let files = if hourly {
        [
            (
                "XBTEUR_60.csv",
                include_str!("../data/kraken/btc_hourly/XBTEUR_60.csv"),
            ),
            (
                "XBTUSD_60.csv",
                include_str!("../data/kraken/btc_hourly/XBTUSD_60.csv"),
            ),
        ]
    } else {
        [
            (
                "XBTEUR_1440.csv",
                include_str!("../data/kraken/btc_daily/XBTEUR_1440.csv"),
            ),
            (
                "XBTUSD_1440.csv",
                include_str!("../data/kraken/btc_daily/XBTUSD_1440.csv"),
            ),
        ]
    };
    ingest(
        conn,
        pair.map(require_supported_pair).transpose()?,
        files.iter().map(|(n, _)| n.to_string()).collect(),
        |name| Ok(files.iter().find(|(n, _)| *n == name).unwrap().1.to_owned()),
    )
}
