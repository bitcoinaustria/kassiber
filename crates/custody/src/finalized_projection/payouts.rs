//! Reviewed payout admission, legacy time and settlement/pricing primitives.
use super::*;
use crate::evidence::canonical::LegRef;
use crate::python_text::{decimal_digit, PythonText};
use money::{empty, Money};

pub(super) fn valid(records: &Value, rows: &dyn Rows) -> Result<Value, String> {
    let mut result = json!({});
    for record in records.as_array().unwrap() {
        let id = text(&record["out_transaction_id"]);
        let row = rows.row(&id);
        if row.is_null() {
            continue;
        }
        // Normalize amount then fee even on an inbound or fee-exclusive row.
        let amount = integer_big(&row["amount"])?;
        let fee = integer_big(&row["fee"])?;
        let principal = if text(&row["direction"]).py_strip().py_lower() == "outbound"
            && codec::truth(&row["amount_includes_fee"])
        {
            amount - fee
        } else {
            amount
        };
        let reviewed = if empty(&record["out_amount"]) {
            principal.clone()
        } else {
            integer(&record["out_amount"])?
        };
        if reviewed > BigInt::from(0) && reviewed <= principal {
            result[&id] = record.clone();
        }
    }
    Ok(result)
}
fn integer(v: &Value) -> Result<BigInt, String> {
    let n = codec::integer(v).map_err(|_| {
        if v.is_array() || v.is_object() || v.is_null() {
            "TypeError"
        } else {
            "ValueError"
        }
        .to_owned()
    })?;
    BigInt::from_str(&n.to_string()).map_err(|_| "ValueError".into())
}

// The legacy regex is deliberately separate from datetime.fromisoformat.
// Rebuilt ISO text retains Unicode digits (which fromisoformat rejects).
// RFC2822 numeric groups instead use Python int semantics and normalize below.
struct Cursor {
    chars: Vec<char>,
    at: usize,
}
impl Cursor {
    fn take(&mut self, c: char) -> bool {
        if self.chars.get(self.at) == Some(&c) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn digits(&mut self, n: usize) -> Option<String> {
        let end = self.at + n;
        let s: Option<String> = self
            .chars
            .get(self.at..end)?
            .iter()
            .map(|&c| decimal_digit(c).map(|_| c))
            .collect();
        if s.is_some() {
            self.at = end;
        }
        s
    }
    fn done(&self) -> bool {
        self.at == self.chars.len()
    }
}
fn legacy(raw: &str) -> Option<String> {
    let mut c = Cursor {
        chars: raw.py_upper().chars().collect(),
        at: 0,
    };
    let y = c.digits(4)?;
    c.take('-');
    let m = c.digits(2)?;
    c.take('-');
    let d = c.digits(2)?;
    let mut result = format!("{y}-{m}-{d}");
    if c.done() {
        return Some(result);
    }
    if !c.take('T') && !c.take(' ') {
        return None;
    }
    let h = c.digits(2)?;
    let mut minute = "00".to_owned();
    let mut second = "00".to_owned();
    let mut fraction = String::new();
    let saved = c.at;
    c.take(':');
    if let Some(value) = c.digits(2) {
        minute = value;
        let saved = c.at;
        c.take(':');
        if let Some(value) = c.digits(2) {
            second = value;
            if c.take('.') || c.take(',') {
                let begin = c.at;
                while let Some(value) = c.digits(1) {
                    fraction.push_str(&value);
                }
                if c.at == begin {
                    return None;
                }
            }
        } else {
            c.at = saved;
        }
    } else {
        c.at = saved;
    }
    result.push_str(&format!("T{h}:{minute}:{second}"));
    if !fraction.is_empty() {
        result.push('.');
        result.push_str(&format!(
            "{:0<6}",
            fraction.chars().take(6).collect::<String>()
        ));
    }
    while c
        .chars
        .get(c.at)
        .is_some_and(|ch| ch.to_string().py_strip().is_empty())
    {
        c.at += 1;
    }
    let rest: String = c.chars[c.at..].iter().collect();
    if ["Z", "UTC", "GMT"].contains(&rest.as_str()) {
        result.push_str("+00:00");
        return Some(result);
    }
    if c.done() {
        return Some(result);
    }
    let sign = if c.take('+') {
        '+'
    } else if c.take('-') {
        '-'
    } else {
        return None;
    };
    let hour = c.digits(2)?;
    let minute = if c.done() {
        "00".into()
    } else {
        c.take(':');
        c.digits(2)?
    };
    if !c.done() {
        return None;
    }
    result.push_str(&format!("{sign}{hour}:{minute}"));
    Some(result)
}
fn iso(raw: &str) -> Result<Option<i64>, String> {
    let rewritten = if raw.chars().count() == 10 {
        format!("{raw}T00:00:00+00:00")
    } else {
        raw.to_owned()
    };
    crate::components::time::instant(&json!(rewritten)).map_err(|e| e.code.to_owned())
}
fn rfc(raw: &str) -> Result<Option<i64>, String> {
    let upper = raw.py_upper();
    let raw = if upper
        .get(..4)
        .is_some_and(|s| ["MON,", "TUE,", "WED,", "THU,", "FRI,", "SAT,", "SUN,"].contains(&s))
    {
        &upper[4..]
    } else {
        &upper
    };
    let fields: Vec<&str> = raw
        .split(|c: char| c.to_string().py_strip().is_empty())
        .filter(|s| !s.is_empty())
        .collect();
    if fields.len() != 5 {
        return Ok(None);
    }
    let [day, month, year, clock, zone] = fields[..] else {
        unreachable!()
    };
    if !(1..=2).contains(&day.chars().count()) || year.chars().count() != 4 {
        return Ok(None);
    }
    let Some(month) = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ]
    .iter()
    .position(|m| *m == month) else {
        return Ok(None);
    };
    let numeric = |s: &str| {
        s.chars()
            .map(|c| decimal_digit(c).map(|d| char::from(b'0' + d)))
            .collect::<Option<String>>()
    };
    let (Some(day), Some(year)) = (numeric(day), numeric(year)) else {
        return Ok(None);
    };
    let parts: Vec<_> = clock.split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.chars().count() != 2) {
        return Ok(None);
    }
    let parts: Option<Vec<_>> = parts.iter().map(|p| numeric(p)).collect();
    let Some(parts) = parts else {
        return Ok(None);
    };
    let zone = if ["GMT", "UTC", "UT", "Z"].contains(&zone) {
        "+00:00".into()
    } else {
        let mut c = Cursor {
            chars: zone.chars().collect(),
            at: 0,
        };
        let sign = if c.take('+') {
            '+'
        } else if c.take('-') {
            '-'
        } else {
            return Ok(None);
        };
        let Some(h) = c.digits(2) else {
            return Ok(None);
        };
        c.take(':');
        let Some(m) = c.digits(2) else {
            return Ok(None);
        };
        if !c.done() {
            return Ok(None);
        };
        // timezone(timedelta(...)) allows oversized minutes, matching Python.
        format!("{sign}{h}:{m}")
    };
    iso(&format!(
        "{year}-{:02}-{:0>2}T{}:{}:{}{zone}",
        month + 1,
        day,
        parts[0],
        parts[1],
        parts.get(2).map(String::as_str).unwrap_or("00")
    ))
}
fn render(us: i64) -> String {
    let day = us / 86_400_000_000;
    let year_days = |y: i64| {
        let n = y - 1;
        n * 365 + n / 4 - n / 100 + n / 400
    };
    let mut lo = 1;
    let mut hi = 10000;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if year_days(mid) <= day {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let y = lo;
    let mut rem = day - year_days(y);
    let months = [
        31,
        if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            29
        } else {
            28
        },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    while rem >= months[m] {
        rem -= months[m];
        m += 1;
    }
    let seconds = (us % 86_400_000_000) / 1_000_000;
    format!(
        "{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        m + 1,
        rem + 1,
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}
pub(super) fn time(payout: &Value) -> Result<Value, String> {
    let value = &payout["payout_occurred_at"];
    if empty(value) {
        return Ok(Value::Null);
    }
    let raw = text(value);
    let raw = raw.py_strip();
    let instant = if let Some(rebuilt) = legacy(raw) {
        iso(&rebuilt)?
    } else {
        match iso(raw)? {
            Some(v) => Some(v),
            None => rfc(raw)?,
        }
    };
    instant
        .map(|v| json!(render(v)))
        .ok_or_else(|| "AppError".into())
}

pub(super) fn settlement(obs: LegRef<'_>, payout: &Value, rows: &dyn Rows) -> Result<Value, Error> {
    let amount = integer_big(&payout["payout_amount"])?;
    let value = &payout["payout_fiat_value"];
    if amount <= BigInt::from(0) || empty(value) {
        return Ok(Value::Null);
    }
    let value = Money::finite(value)?;
    if !value.positive()? {
        return Ok(Value::Null);
    }
    let rate = value.rate(&amount)?;
    rate.require_finite()?;
    let id = if codec::truth(&payout["id"]) {
        string(&payout["id"])
    } else {
        string(&obs["anchor_transaction_id"])
    };
    let at = time(payout)?;
    let mut base = base_row_with_rows(obs, rows)?;
    base["asset"] = json!(text(&payout["payout_asset"]).py_upper());
    base["amount"] = number(&amount);
    base["fee"] = json!(0);
    base["fiat_rate"] = rate.float()?;
    base["fiat_rate_exact"] = json!(rate.fixed()?);
    base["fiat_value"] = value.float()?;
    base["fiat_value_exact"] = json!(value.fixed()?);
    base["occurred_at"] = if at.is_null() {
        obs["occurred_at"].clone()
    } else {
        at
    };
    base["external_id"] = if codec::truth(&payout["payout_external_id"]) {
        payout["payout_external_id"].clone()
    } else {
        json!(format!("direct-payout:{id}"))
    };
    base["journal_transaction_id"] = obs["anchor_transaction_id"].clone();
    base["custody_finalized_tax_projection"] = json!(true);
    base["custody_quantity_hash"] = obs["quantity_hash"].clone();
    base["custody_slice_start_msat"] = json!(0);
    base["custody_slice_end_msat"] = number(&amount);
    let mut inbound = base.clone();
    inbound["id"] = json!(payout_row_id(&id, "in"));
    inbound["direction"] = json!("inbound");
    inbound["kind"] = json!("direct_swap_payout_in");
    base["id"] = json!(payout_row_id(&id, "out"));
    base["direction"] = json!("outbound");
    base["kind"] = json!("sell");
    Ok(json!([inbound, base]))
}
pub(super) fn price(row: Value, payout: &Value, amount: &BigInt) -> Result<Value, String> {
    price_row(row::FinalizedRow::owned(row), payout, amount).map(row::FinalizedRow::into_json)
}

pub(super) fn price_row<'a>(
    mut row: row::FinalizedRow<'a>,
    payout: &Value,
    amount: &BigInt,
) -> Result<row::FinalizedRow<'a>, String> {
    if !empty(&payout["payout_fiat_value"]) && amount > &BigInt::from(0) {
        let value = Money::finite(&payout["payout_fiat_value"])?;
        let rate = value.rate(amount)?;
        rate.require_finite()?;
        row["fiat_rate"] = rate.float()?;
        row["fiat_rate_exact"] = json!(rate.fixed()?);
        row["fiat_value"] = value.float()?;
        row["fiat_value_exact"] = json!(value.fixed()?);
        row["pricing_method"] = json!("direct_payout_review");
    }
    Ok(row)
}
