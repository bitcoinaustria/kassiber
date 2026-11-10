//! Diagnostic comparison of observations with a selected platform export.
pub(super) use super::cross_check_number::Number;
pub(super) use super::*;
pub(super) use kassiber_pyjson::BigInt;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
pub(super) fn s(raw: impl AsRef<str>) -> Value {
    super::s(raw.as_ref())
}

pub(super) const STATUSES: [&str; 6] = [
    "different",
    "only_tool",
    "only_kassiber",
    "excluded_in_kassiber",
    "matched",
    "not_comparable",
];
#[derive(Clone)]
pub(super) struct Tool {
    pub index: usize,
    pub source: String,
    pub label: String,
    pub txid: String,
    pub at: Option<i64>,
    pub direction: String,
    pub asset: String,
    pub amount: BigInt,
    pub fee: BigInt,
    pub kind: Option<String>,
    pub cash: Option<Number>,
    pub error: String,
}
struct BookRow {
    id: String,
    wallet: String,
    txid: String,
    at: i64,
    direction: String,
    asset: String,
    amount: BigInt,
    fee: BigInt,
    classification: String,
    cash: Option<Number>,
    excluded: bool,
    non_taxable: bool,
    matched: bool,
}
impl BookRow {
    fn movement(&self) -> BigInt {
        &self.amount
            + if self.direction == "outbound" {
                self.fee.clone()
            } else {
                0.into()
            }
    }
}
pub(super) fn utc(raw: &str) -> Result<Option<i64>> {
    let Some(parsed) =
        kassiber_pytext::iso_datetime(&kassiber_pytext::strip(raw).replace('Z', "+00:00"))
    else {
        return Ok(None);
    };
    let at = parsed.utc_us();
    if !(0..315_537_897_600_000_000).contains(&at) {
        return Err(AppError::new("internal_error", "date value out of range"));
    }
    Ok(Some(at))
}
fn iso(at: Option<i64>) -> String {
    at.and_then(kassiber_rates::time::format)
        .map(|s| format!("{}Z", s[..19].trim_start_matches('0')))
        .unwrap_or_default()
}
fn year(at: i64, zone: Option<&str>) -> Result<i64> {
    let raw = kassiber_rates::time::format(at)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
    if let Some(zone) = zone {
        calendar::local(&raw, zone).map(|v| v.0)
    } else {
        raw[..4]
            .parse()
            .map_err(|_| AppError::new("internal_error", "date value out of range"))
    }
}
pub(super) fn msat(d: &Decimal) -> Result<BigInt> {
    let raw = d
        .mul(&Decimal::from(100_000_000_000_i64), &DECIMAL)
        .map_err(arithmetic)?
        .to_fixed_places(0, Rounding::HalfUp)
        .map_err(arithmetic)?;
    kassiber_pytext::parse_int(&raw)
        .map_err(|_| AppError::new("internal_error", "invalid report quantity"))
}
fn abs(n: BigInt) -> BigInt {
    if n < 0.into() {
        -n
    } else {
        n
    }
}
fn same(a: &BigInt, b: &BigInt) -> bool {
    abs(a - b) < 1000.into()
}
fn exact(n: &BigInt) -> Result<Value> {
    let d: Decimal = n
        .to_string()
        .parse()
        .map_err(|_| AppError::new("internal_error", "invalid quantity"))?;
    let text = d
        .div(&Decimal::from(100_000_000_000_i64), &DECIMAL)
        .map_err(arithmetic)?
        .to_fixed_places(11, Rounding::HalfEven)
        .map_err(arithmetic)?;
    let mut text = text.trim_end_matches('0').to_owned();
    let point = text.find('.').unwrap();
    while text.len() - point - 1 < 8 {
        text.push('0');
    }
    Ok(s(text))
}
fn money(n: Option<&Number>) -> Result<Value> {
    n.map(Number::money)
        .transpose()
        .map(|s| s.unwrap_or_else(|| super::s("")))
}
fn display(asset: &str) -> &str {
    if asset == "liquid:lbtc" {
        "L-BTC"
    } else {
        "BTC"
    }
}
fn asset(raw: &str) -> (String, String) {
    let upper = kassiber_pytext::upper(kassiber_pytext::strip(raw));
    match upper.as_str() {
        "BTC" => ("bitcoin:btc".into(), "BTC".into()),
        "LBTC" | "L-BTC" => ("liquid:lbtc".into(), "L-BTC".into()),
        _ => {
            let code = kassiber_pytext::strip(raw);
            if code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit()) {
                (
                    format!("liquid:{}", code.to_lowercase()),
                    format!("{}…", code[..8].to_lowercase()),
                )
            } else {
                let id = if code.contains(':') {
                    code.into()
                } else {
                    upper
                };
                let label = id.rsplit(':').next().unwrap_or("").to_uppercase();
                (id, label)
            }
        }
    }
}
fn tool_payload(t: &Tool) -> Result<Value> {
    Ok(dict([
        ("row", Value::Int(t.index.into())),
        ("source", s(&t.source)),
        ("type", s(&t.label)),
        ("occurred_at", s(iso(t.at))),
        ("direction", s(&t.direction)),
        (
            "asset",
            s(if t.direction.is_empty() {
                ""
            } else {
                display(&t.asset)
            }),
        ),
        (
            "amount",
            if t.direction.is_empty() {
                s("")
            } else {
                exact(&t.amount)?
            },
        ),
        (
            "fee",
            if t.fee == 0.into() {
                s("")
            } else {
                exact(&t.fee)?
            },
        ),
        ("value", money(t.cash.as_ref())?),
        ("txid", s(&t.txid)),
    ]))
}
fn book_payload(b: &BookRow) -> Result<Value> {
    Ok(dict([
        ("ref", s(&b.id)),
        ("wallet", s(&b.wallet)),
        ("classification", s(&b.classification)),
        ("occurred_at", s(iso(Some(b.at)))),
        ("direction", s(&b.direction)),
        ("asset", s(display(&b.asset))),
        ("amount", exact(&b.amount)?),
        (
            "fee",
            if b.fee == 0.into() {
                s("")
            } else {
                exact(&b.fee)?
            },
        ),
        ("value", money(b.cash.as_ref())?),
        ("txid", s(&b.txid)),
        ("excluded", Value::Bool(b.excluded)),
    ]))
}
fn class(kind: Option<&str>) -> Option<&'static str> {
    Some(match kind {
        None | Some("buy") => "acquire",
        Some("deposit" | "withdrawal") => "transfer",
        Some("sell" | "spend" | "gift" | "donation" | "lost" | "stolen") => "dispose",
        Some("expense_non_taxable") => "dispose_free",
        Some(
            "income" | "wages" | "mining" | "staking" | "interest" | "lending_interest" | "airdrop"
            | "hardfork",
        ) => "income",
        _ => return None,
    })
}
fn book_rows(
    conn: &Connection,
    profile: &Value,
    wallets: Option<&BTreeSet<String>>,
    currency: &str,
) -> Result<(Vec<BookRow>, BTreeMap<String, usize>)> {
    let id = text(profile, "id");
    let tx=rows(conn,"SELECT t.id,t.wallet_id,w.label AS wallet,t.occurred_at,t.direction,t.asset,t.amount,t.fee,t.amount_includes_fee,t.excluded,COALESCE(t.external_id,'') AS external_id,COALESCE(t.fiat_value_exact,t.fiat_value) AS fiat_value,COALESCE(t.fiat_currency,'') AS fiat_currency,t.taxability_override FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.occurred_at,t.id",[&id])?;
    let entries=rows(conn,"SELECT transaction_id,entry_type,at_category,COALESCE(NULLIF(fiat_value_exact,''),fiat_value) AS fiat_value,COALESCE(NULLIF(proceeds_exact,''),proceeds) AS proceeds FROM journal_entries WHERE profile_id=?",[&id])?;
    let neutral = entries
        .iter()
        .filter(|r| text(r, "at_category") == "neu_swap")
        .map(|r| text(r, "transaction_id"))
        .collect::<BTreeSet<_>>();
    let relations=rows(conn,"SELECT relation_kind,out_transaction_id,in_transaction_id FROM journal_custody_projection_relations WHERE profile_id=? AND relation_kind IN ('move','conversion','direct_payout')",[&id])?;
    let mut swaps = BTreeSet::new();
    for r in relations {
        let out = text(&r, "out_transaction_id");
        let incoming = text(&r, "in_transaction_id");
        if text(&r, "relation_kind") != "move"
            && (neutral.contains(&out) || neutral.contains(&incoming))
        {
            swaps.insert(out);
            swaps.insert(incoming);
        }
    }
    let mut facts: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut values: BTreeMap<String, BTreeMap<&str, Number>> = BTreeMap::new();
    for r in entries {
        let kind = text(&r, "entry_type");
        let category = text(&r, "at_category");
        let tx = text(&r, "transaction_id");
        let fact = facts.entry(tx.clone()).or_default();
        fact.0
            .insert(if kind == "disposal" && category == "neu_swap" {
                "neutral_disposal".into()
            } else {
                kind.clone()
            });
        if truthy(value(&r, "at_category")) {
            fact.1.insert(category);
        }
        let key = match kind.as_str() {
            "income" => "income",
            "disposal" => "proceeds",
            "acquisition" => "cost",
            _ => continue,
        };
        let amount = Number::from_value(value(
            &r,
            if key == "proceeds" {
                "proceeds"
            } else {
                "fiat_value"
            },
        ))?;
        if !matches!(amount, Number::Finite(_)) {
            return Err(AppError::new(
                "validation",
                "Journal cash values must be finite",
            ));
        }
        let bucket = values.entry(tx).or_default();
        let total = bucket
            .get(key)
            .cloned()
            .unwrap_or_else(Number::zero)
            .add(&amount)?;
        bucket.insert(key, total);
    }
    let mut result = Vec::new();
    let mut other = BTreeMap::new();
    for r in tx {
        if wallets.is_some_and(|w| !w.contains(&text(&r, "wallet_id"))) {
            continue;
        }
        let (asset, label) = asset(&text(&r, "asset"));
        if !["bitcoin:btc", "liquid:lbtc"].contains(&asset.as_str()) {
            if !truthy(value(&r, "excluded")) {
                *other.entry(label).or_default() += 1;
            }
            continue;
        }
        let id = text(&r, "id");
        let empty = (BTreeSet::new(), BTreeSet::new());
        let (types, categories) = facts.get(&id).unwrap_or(&empty);
        let swap = swaps.contains(&id);
        let mut labels = Vec::new();
        if ["transfer_in", "transfer_out", "transfer_fee"]
            .iter()
            .any(|t| types.contains(*t))
        {
            labels.push("internal_transfer");
        }
        if swap || categories.contains("neu_swap") {
            labels.push("swap");
        }
        if types.contains("income") {
            labels.push("income");
        } else if types.contains("acquisition") && !swap {
            labels.push("acquisition");
        }
        if types.contains("disposal") {
            labels.push("disposal");
        }
        if labels.is_empty() && types.contains("fee") {
            labels.push("fee");
        }
        let classification = if labels.is_empty() {
            "no_journal_entry".into()
        } else {
            labels.join("+")
        };
        let cash = values
            .get(&id)
            .and_then(|v| {
                v.get("income").or_else(|| v.get("proceeds")).or_else(|| {
                    (!classification.contains("swap"))
                        .then(|| v.get("cost"))
                        .flatten()
                })
            })
            .cloned();
        let cash = if cash.is_some() {
            cash
        } else if !matches!(value(&r, "fiat_value"), Value::Null)
            && value(&r, "fiat_value") != &s("")
            && ["", currency].contains(&text(&r, "fiat_currency").to_uppercase().as_str())
        {
            Some(text(&r, "fiat_value").parse::<Number>().map_err(|_| {
                AppError::new(
                    "app_error",
                    format!("Invalid decimal value: {}", text(&r, "fiat_value")),
                )
            })?)
        } else {
            None
        };
        if cash
            .as_ref()
            .is_some_and(|cash| !matches!(cash, Number::Finite(_)))
        {
            return Err(AppError::new(
                "validation",
                "Stored comparison values must be finite",
            ));
        }
        // A malformed stored instant refuses the comparison rather than
        // becoming a year-one date that a year filter silently drops.
        let Some(at) = utc(&text(&r, "occurred_at"))? else {
            return Err(AppError::new(
                "validation",
                "Stored comparison dates must be valid timestamps",
            ));
        };
        let raw_direction = text(&r, "direction");
        let direction = kassiber_pytext::lower(kassiber_pytext::strip(&raw_direction));
        let fee = BigInt::from(int(&r, "fee"));
        let mut amount = BigInt::from(int(&r, "amount"));
        if direction == "outbound" && truthy(value(&r, "amount_includes_fee")) {
            amount -= &fee;
        }
        result.push(BookRow {
            id,
            wallet: text(&r, "wallet"),
            txid: text(&r, "external_id"),
            at,
            direction,
            asset,
            amount,
            fee,
            classification,
            cash,
            excluded: truthy(value(&r, "excluded")),
            non_taxable: value(&r, "taxability_override") == &Value::Int(0.into()),
            matched: false,
        });
    }
    Ok((result, other))
}
fn fits(t: &Tool, b: &BookRow) -> bool {
    same(&t.amount, &b.amount)
        || (b.direction == "outbound"
            && (same(&(&t.amount + &t.fee), &b.movement()) || same(&t.amount, &b.movement())))
}
fn chain_id(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit())
}
/// `_shift_utc`: a shift past the calendar's first or last instant clamps
/// to it instead of overflowing.
fn shift(at: i64, delta: i64) -> i64 {
    (at + delta).clamp(0, 315_537_897_599_999_999)
}
fn shifted(at: Option<i64>, offset: i64) -> Result<Option<i64>> {
    Ok(at.map(|at| shift(at, -offset)))
}
fn probable(t: &Tool, book: &[BookRow], excluded: bool, offset: i64) -> Result<Option<usize>> {
    if t.at.is_none() {
        return Ok(None);
    }
    let mut candidates = book
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            !b.matched && b.excluded == excluded && b.direction == t.direction && b.asset == t.asset
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(None);
    }
    let at = shifted(t.at, offset)?.unwrap();
    // The window's bounds clamp at the calendar's endpoints.
    let window = 26 * 3_600_000_000;
    let (start, end) = (shift(at, -window), shift(at, window));
    candidates.sort_by_key(|(_, b)| b.at);
    Ok(candidates
        .into_iter()
        .filter(|(_, b)| {
            (start..=end).contains(&b.at)
                && !(chain_id(&t.txid)
                    && chain_id(&b.txid)
                    && t.txid.to_lowercase() != b.txid.to_lowercase())
                && fits(t, b)
        })
        .min_by_key(|(_, b)| (b.at - at).abs())
        .map(|(i, _)| i))
}
fn offset(pairs: &[(usize, usize, &str)], tool: &[Tool], book: &[BookRow]) -> i64 {
    let mut votes: Vec<(i64, usize)> = Vec::new();
    let mut total = 0;
    for (t, b, _) in pairs {
        if let Some(at) = tool[*t].at {
            total += 1;
            let seconds = (at - book[*b].at) as f64 / 1_000_000.0;
            let hour = (seconds / 3600.0).round_ties_even() as i64;
            if (seconds - hour as f64 * 3600.0).abs() <= 600.0 {
                if let Some((_, n)) = votes.iter_mut().find(|(h, _)| *h == hour) {
                    *n += 1
                } else {
                    votes.push((hour, 1));
                }
            }
        }
    }
    let best = votes
        .into_iter()
        .reduce(|a, b| if b.1 > a.1 { b } else { a });
    match best {
        Some((h, n)) if total >= 3 && h != 0 && h.abs() <= 14 && n * 5 >= total * 4 => {
            h * 3_600_000_000
        }
        _ => 0,
    }
}
fn diff(
    field: &str,
    severity: &str,
    tool: Value,
    book: Value,
    message: String,
    delta: Option<Value>,
) -> Value {
    let mut result = dict([
        ("field", s(field)),
        ("severity", s(severity)),
        ("tool", tool),
        ("kassiber", book),
    ]);
    if let Some(v) = delta {
        replace(&mut result, "delta", v);
    }
    replace(&mut result, "message", s(message));
    result
}
fn differences(
    t: &Tool,
    b: &BookRow,
    offset: i64,
    zone: Option<&str>,
    types: bool,
) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    if types {
        let tc = class(t.kind.as_deref()).unwrap_or(if t.direction == "inbound" {
            "acquire"
        } else {
            "dispose"
        });
        let parts = b
            .classification
            .split('+')
            .filter(|p| {
                [
                    "internal_transfer",
                    "swap",
                    "income",
                    "acquisition",
                    "disposal",
                    "fee",
                ]
                .contains(p)
            })
            .collect::<Vec<_>>();
        let classes = parts
            .iter()
            .map(|p| match *p {
                "internal_transfer" | "swap" => "transfer",
                "income" => {
                    if b.non_taxable {
                        "acquire"
                    } else {
                        "income"
                    }
                }
                "acquisition" => "acquire",
                _ => {
                    if b.non_taxable {
                        "dispose_free"
                    } else {
                        "dispose"
                    }
                }
            })
            .collect::<BTreeSet<_>>();
        let mixed = classes.contains("transfer")
            && ["dispose", "dispose_free", "income"]
                .iter()
                .any(|c| classes.contains(c));
        let compatible = classes.contains(tc) && !mixed
            || tc == "transfer" && (classes == BTreeSet::from(["acquire"]) || parts == ["fee"]);
        if !parts.is_empty() && !compatible {
            let label = if !t.label.is_empty() {
                t.label.clone()
            } else {
                t.kind.clone().unwrap_or_else(|| "None".into())
            };
            result.push(diff(
                "type",
                "tax",
                s(if t.label.is_empty() {
                    t.kind.as_deref().unwrap_or("")
                } else {
                    &t.label
                }),
                s(&b.classification),
                format!("{label} in the tool, {} in Kassiber", b.classification),
                None,
            ));
        }
    }
    let out = b.direction == "outbound";
    let movement = &t.amount + if out { t.fee.clone() } else { 0.into() };
    if out && !same(&movement, &b.movement()) {
        if same(&t.amount, &b.movement()) && t.fee == 0.into() {
            result.push(diff(
                "fee",
                "info",
                exact(&t.amount)?,
                exact(&b.fee)?,
                "The tool folds the network fee into the amount".into(),
                None,
            ));
        } else if same(&t.amount, &b.amount) {
            result.push(diff(
                "fee",
                if b.classification.contains("disposal") {
                    "tax"
                } else {
                    "info"
                },
                exact(&t.fee)?,
                exact(&b.fee)?,
                "Fees differ".into(),
                Some(exact(&(&t.fee - &b.fee))?),
            ));
        } else {
            result.push(diff(
                "amount",
                "tax",
                exact(&t.amount)?,
                exact(&b.amount)?,
                "Amounts differ".into(),
                Some(exact(&(movement - b.movement()))?),
            ));
        }
    } else if !out && !same(&t.amount, &b.amount) {
        result.push(diff(
            "amount",
            "tax",
            exact(&t.amount)?,
            exact(&b.amount)?,
            "Amounts differ".into(),
            Some(exact(&(&t.amount - &b.amount))?),
        ));
    } else if out && !same(&t.amount, &b.amount) {
        result.push(diff(
            "fee",
            "info",
            exact(&t.fee)?,
            exact(&b.fee)?,
            "Same total, but the fee is split differently".into(),
            None,
        ));
    }
    if let (Some(t), Some(b)) = (&t.cash, &b.cash) {
        let tolerance = b.tolerance()?;
        let delta = t.sub(b)?;
        if delta.abs()?.greater(&tolerance)? {
            result.push(diff(
                "value",
                "tax",
                money(Some(t))?,
                money(Some(b))?,
                "Values differ".into(),
                Some(money(Some(&delta))?),
            ));
        }
    }
    if let Some(at) = t.at {
        let shifted = shifted(Some(at), offset)?.unwrap();
        let drift = shifted - b.at;
        let crosses = year(shifted, zone)? != year(b.at, zone)?;
        if crosses || drift.abs() > 600_000_000 {
            let minutes = drift.div_euclid(60_000_000);
            result.push(diff(
                "time",
                if crosses { "tax" } else { "info" },
                s(iso(t.at)),
                s(iso(Some(b.at))),
                if crosses {
                    "Dates fall in different tax years".into()
                } else {
                    format!("Times differ by {minutes} min")
                },
                Some(s(minutes.to_string())),
            ));
        }
    }
    Ok(result)
}
struct Total {
    incoming: BigInt,
    outgoing: BigInt,
    fees: BigInt,
    income: Number,
    unvalued: usize,
}
impl Default for Total {
    fn default() -> Self {
        Self {
            incoming: 0.into(),
            outgoing: 0.into(),
            fees: 0.into(),
            income: Number::zero(),
            unvalued: 0,
        }
    }
}
fn add_tool(t: &Tool, totals: &mut BTreeMap<String, Total>) -> Result<()> {
    let bucket = totals.entry(t.asset.clone()).or_default();
    if t.direction == "inbound" {
        bucket.incoming += &t.amount;
        if class(t.kind.as_deref()) == Some("income") {
            if let Some(v) = &t.cash {
                bucket.income = bucket.income.add(v)?
            } else {
                bucket.unvalued += 1;
            }
        }
    } else {
        bucket.outgoing += &t.amount;
        bucket.fees += &t.fee;
    }
    Ok(())
}
fn add_book(b: &BookRow, totals: &mut BTreeMap<String, Total>) -> Result<()> {
    let bucket = totals.entry(b.asset.clone()).or_default();
    if b.direction == "inbound" {
        bucket.incoming += &b.amount;
        if b.classification.split('+').any(|c| c == "income") && !b.non_taxable {
            if let Some(v) = &b.cash {
                bucket.income = bucket.income.add(v)?;
            }
        }
    } else {
        bucket.outgoing += &b.amount;
        bucket.fees += &b.fee;
    }
    Ok(())
}
fn counts(rows: &[Value]) -> Value {
    dict(STATUSES.map(|status| {
        (
            status,
            Value::Int(
                rows.iter()
                    .filter(|r| text(r, "status") == status)
                    .count()
                    .into(),
            ),
        )
    }))
}
fn row(
    status: &str,
    how: Option<&str>,
    t: Option<&Tool>,
    b: Option<&BookRow>,
    diffs: Vec<Value>,
) -> Result<Value> {
    Ok(dict([
        ("status", s(status)),
        ("match", how.map(s).unwrap_or(Value::Null)),
        (
            "tool",
            t.map(tool_payload).transpose()?.unwrap_or(Value::Null),
        ),
        (
            "kassiber",
            b.map(book_payload).transpose()?.unwrap_or(Value::Null),
        ),
        ("differences", Value::List(diffs)),
    ]))
}
fn pairs(rows: &mut [Value], votes: &BTreeMap<String, Vec<(String, usize)>>) -> Vec<Value> {
    let counterparts = votes
        .iter()
        .filter_map(|(source, ws)| {
            ws.iter()
                .reduce(|a, b| if b.1 > a.1 { b } else { a })
                .map(|(w, _)| (source.clone(), w.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut groups: Vec<Value> = Vec::new();
    for row in rows {
        let b = value(row, "kassiber");
        let t = value(row, "tool");
        let wallet = if !matches!(b, Value::Null) {
            Some(text(b, "wallet"))
        } else {
            counterparts.get(&text(t, "source")).cloned()
        };
        let key = wallet
            .as_ref()
            .filter(|w| !w.is_empty())
            .map(|w| format!("wallet:{w}"))
            .unwrap_or_else(|| {
                format!(
                    "tool:{}",
                    if matches!(t, Value::Null) {
                        String::new()
                    } else {
                        text(t, "source")
                    }
                )
            });
        let index = groups
            .iter()
            .position(|g| text(g, "key") == key)
            .unwrap_or_else(|| {
                groups.push(dict([
                    ("key", s(&key)),
                    ("wallet", wallet.map(s).unwrap_or(Value::Null)),
                    ("tool_names", Value::List(vec![])),
                    ("counts", counts(&[])),
                    ("rows", Value::Int(0.into())),
                ]));
                groups.len() - 1
            });
        let group = &mut groups[index];
        let mut names = artifact_csv::list(value(group, "tool_names")).to_vec();
        if !matches!(t, Value::Null) {
            names.push(value(t, "source").clone());
        }
        names.sort_by_key(py_str);
        names.dedup();
        replace(group, "tool_names", Value::List(names));
        let mut cnt = value(group, "counts").clone();
        let status = text(row, "status");
        let n = int(&cnt, &status) + 1;
        replace(&mut cnt, &status, Value::Int(n.into()));
        replace(group, "counts", cnt);
        let n = int(group, "rows") + 1;
        replace(group, "rows", Value::Int(n.into()));
        replace(row, "pair", s(key));
    }
    for group in &mut groups {
        let mut names = artifact_csv::list(value(group, "tool_names")).to_vec();
        for (source, wallet) in &counterparts {
            if text(group, "key") == format!("wallet:{wallet}") {
                names.push(s(source));
            }
        }
        names.sort_by_key(py_str);
        names.dedup();
        replace(group, "tool_names", Value::List(names));
        let cnt = value(group, "counts");
        let both = int(cnt, "matched") + int(cnt, "different") + int(cnt, "excluded_in_kassiber");
        let state =
            if both > 0 || truthy(value(group, "wallet")) && truthy(value(group, "tool_names")) {
                "paired"
            } else if truthy(value(group, "wallet")) {
                "kassiber_only"
            } else {
                "tool_only"
            };
        let issues = if state == "paired" {
            int(group, "rows") - int(cnt, "matched")
        } else {
            0
        };
        replace(group, "state", s(state));
        replace(group, "issues", Value::Int(issues.into()));
        // Preserve Python's public key order independently from grouping state.
        *group = dict(
            [
                "key",
                "wallet",
                "tool_names",
                "state",
                "rows",
                "issues",
                "counts",
            ]
            .map(|k| (k, value(group, k).clone())),
        );
    }
    groups.sort_by_key(|g| {
        (
            ["paired", "kassiber_only", "tool_only"]
                .iter()
                .position(|v| *v == text(g, "state"))
                .unwrap(),
            -int(g, "issues"),
            if truthy(value(g, "wallet")) {
                text(g, "wallet")
            } else {
                py_str(value(g, "tool_names"))
            },
        )
    });
    groups
}
fn rows_label(n: usize) -> String {
    format!("{n} {}", if n == 1 { "row" } else { "rows" })
}
fn insight(code: &str, params: Value, message: String) -> Value {
    dict([
        ("code", s(code)),
        ("params", params),
        ("message", s(message)),
    ])
}

pub(super) fn report(
    conn: &Connection,
    args: &Value,
    bytes: &[u8],
    filename: &str,
) -> Result<Value> {
    let raw = text(args, "tool");
    let tool = kassiber_pytext::lower(kassiber_pytext::strip(&raw));
    let label = match tool.as_str() {
        "cointracking" => "CoinTracking",
        "blockpit" => "Blockpit",
        _ => {
            return Err(AppError::new("validation", format!("Unknown tool '{raw}'"))
                .with_hint("Use one of: blockpit, cointracking."))
        }
    };
    let (workspace, profile) = kassiber_setup::references::scope(conn, args)?;
    let currency = text(&profile, "fiat_currency").to_uppercase();
    let selected_year = if matches!(value(args, "year"), Value::Null) {
        None
    } else {
        Some(
            super::austrian::normalize_tax_year(value(args, "year")).map_err(|e| {
                if e.code == "internal_error" {
                    e
                } else {
                    AppError::new("validation", "Year must be a calendar year such as 2025")
                }
            })?,
        )
    };
    let zone = crate::country::zone(&text(&profile, "tax_country").to_lowercase());
    let selected = match value(args, "wallets") {
        Value::List(v) => Some(v.clone()),
        Value::Null => {
            if truthy(value(args, "wallet")) {
                Some(vec![value(args, "wallet").clone()])
            } else {
                None
            }
        }
        _ => {
            return Err(AppError::new(
                "internal_error",
                "wallet references must be iterable",
            ))
        }
    };
    let explicit = selected.is_some();
    let mut wallet_labels = Vec::new();
    let mut selected_ids = BTreeSet::new();
    if let Some(refs) = selected {
        if refs.is_empty() {
            return Err(AppError::new(
                "validation",
                "Select at least one wallet to export",
            ));
        }
        for r in refs {
            let w = kassiber_wallets::resolve(conn, &text(&profile, "id"), &py_str(&r))?;
            if selected_ids.insert(w.id) {
                wallet_labels.push(w.label);
            }
        }
    }
    let mut skipped = Vec::new();
    let wallet_ids = if explicit {
        Some(selected_ids)
    } else {
        let all=rows(conn,"SELECT id,label,kind FROM wallets WHERE workspace_id=? AND profile_id=? ORDER BY label",[text(&workspace,"id"),text(&profile,"id")])?;
        if !truthy(value(args, "include_tool_wallets")) {
            skipped = all
                .iter()
                .filter(|w| text(w, "kind").to_lowercase() == tool)
                .map(|w| text(w, "label"))
                .collect();
        }
        if skipped.is_empty() {
            None
        } else {
            Some(
                all.iter()
                    .filter(|w| !skipped.contains(&text(w, "label")))
                    .map(|w| text(w, "id"))
                    .collect(),
            )
        }
    };
    let (mut tool_rows, tool_other) =
        super::cross_check_csv::tool_rows(bytes, &tool, label, &currency)?;
    let (mut book, book_other) = book_rows(conn, &profile, wallet_ids.as_ref(), &currency)?;
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [text(&profile, "id")],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    let types = semantics_current(&profile)
        && truthy(value(&profile, "last_processed_at"))
        && int(&profile, "last_processed_tx_count") == active
        && int(&profile, "journal_input_version") == int(&profile, "last_processed_input_version");
    let in_period = |at: Option<i64>| -> Result<bool> {
        Ok(selected_year.is_none()
            || at.is_none()
            || Some(year(at.unwrap(), zone)?) == selected_year)
    };
    let mut source_counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in &tool_rows {
        if in_period(t.at)? {
            *source_counts.entry(t.source.clone()).or_default() += 1;
        }
    }
    if let Value::List(wanted) = value(args, "tool_sources") {
        tool_rows.retain(|t| wanted.contains(&s(&t.source)));
    }
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for (i, t) in tool_rows
        .iter()
        .enumerate()
        .filter(|(_, t)| t.error.is_empty() && !t.direction.is_empty())
    {
        let candidate = book
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                !b.excluded
                    && !b.matched
                    && b.direction == t.direction
                    && b.asset == t.asset
                    && !t.txid.is_empty()
                    && (t.txid.to_lowercase() == b.id.to_lowercase()
                        || t.txid.to_lowercase() == b.txid.to_lowercase())
            })
            .min_by_key(|(_, b)| abs(&b.amount - &t.amount))
            .map(|(i, _)| i);
        if let Some(b) = candidate {
            book[b].matched = true;
            matched.push((i, b, "txid"));
        } else {
            unmatched.push(i);
        }
    }
    let offset = offset(&matched, &tool_rows, &book);
    let mut excluded = Vec::new();
    let mut without_id = Vec::new();
    for t in unmatched {
        let row = &tool_rows[t];
        let candidate = book
            .iter()
            .enumerate()
            .find(|(_, b)| {
                // An excluded row only claims its ID within its own asset.
                b.excluded
                    && b.direction == row.direction
                    && b.asset == row.asset
                    && !row.txid.is_empty()
                    && (row.txid.to_lowercase() == b.id.to_lowercase()
                        || row.txid.to_lowercase() == b.txid.to_lowercase())
            })
            .map(|(i, _)| i);
        if let Some(b) = candidate.filter(|b| !book[*b].matched && book[*b].asset == row.asset) {
            book[b].matched = true;
            excluded.push((t, b, "txid"));
        } else {
            without_id.push(t);
        }
    }
    without_id.sort_by_key(|t| tool_rows[*t].at.unwrap_or(0));
    let mut only_tool = Vec::new();
    for t in without_id {
        if let Some(b) = probable(&tool_rows[t], &book, false, offset)? {
            book[b].matched = true;
            matched.push((t, b, "probable"));
        } else if let Some(b) = probable(&tool_rows[t], &book, true, offset)? {
            book[b].matched = true;
            excluded.push((t, b, "probable"));
        } else {
            only_tool.push(t);
        }
    }
    let mut result = Vec::new();
    let mut tt = BTreeMap::new();
    let mut bt = BTreeMap::new();
    let mut votes: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    let mut source_matched: BTreeMap<String, usize> = BTreeMap::new();
    for (t, b, how) in matched {
        let t = &tool_rows[t];
        let b = &book[b];
        let shifted = shifted(t.at, offset)?;
        if !in_period(shifted)? && !in_period(Some(b.at))? {
            continue;
        }
        let diffs = differences(t, b, offset, zone, types)?;
        let status = if diffs.iter().any(|d| text(d, "severity") == "tax") {
            "different"
        } else {
            "matched"
        };
        result.push(row(status, Some(how), Some(t), Some(b), diffs)?);
        if in_period(shifted)? {
            add_tool(t, &mut tt)?;
        }
        if in_period(Some(b.at))? {
            add_book(b, &mut bt)?;
        }
        let ws = votes.entry(t.source.clone()).or_default();
        if let Some((_, n)) = ws.iter_mut().find(|(w, _)| w == &b.wallet) {
            *n += 1
        } else {
            ws.push((b.wallet.clone(), 1));
        }
        *source_matched.entry(t.source.clone()).or_default() += 1;
    }
    for (t, b, how) in excluded {
        let t = &tool_rows[t];
        let b = &book[b];
        let shifted = shifted(t.at, offset)?;
        if !in_period(shifted)? && !in_period(Some(b.at))? {
            continue;
        }
        result.push(row(
            "excluded_in_kassiber",
            Some(how),
            Some(t),
            Some(b),
            vec![],
        )?);
        if in_period(shifted)? {
            add_tool(t, &mut tt)?;
        }
    }
    for t in only_tool {
        let t = &tool_rows[t];
        if !in_period(shifted(t.at, offset)?)? {
            continue;
        }
        result.push(row("only_tool", None, Some(t), None, vec![])?);
        add_tool(t, &mut tt)?;
    }
    for b in &book {
        if b.excluded || b.matched || !in_period(Some(b.at))? {
            continue;
        }
        result.push(row("only_kassiber", None, None, Some(b), vec![])?);
        add_book(b, &mut bt)?;
    }
    for t in &tool_rows {
        if t.error.is_empty() || !in_period(t.at)? {
            continue;
        }
        let mut r = row("not_comparable", None, Some(t), None, vec![])?;
        replace(&mut r, "reason", s(&t.error));
        result.push(r);
    }
    result.sort_by_key(|r| {
        let side = if truthy(value(r, "kassiber")) {
            value(r, "kassiber")
        } else {
            value(r, "tool")
        };
        (
            text(side, "occurred_at"),
            STATUSES
                .iter()
                .position(|s| *s == text(r, "status"))
                .unwrap(),
        )
    });
    for (i, r) in result.iter_mut().enumerate() {
        replace(r, "id", s(format!("r{}", i + 1)));
    }
    let count = counts(&result);
    let tax = result
        .iter()
        .flat_map(|r| artifact_csv::list(value(r, "differences")))
        .filter(|d| text(d, "severity") == "tax")
        .count();
    let mut totals = Vec::new();
    for asset in tt.keys().chain(bt.keys()).cloned().collect::<BTreeSet<_>>() {
        let empty = Total::default();
        let t = tt.get(&asset).unwrap_or(&empty);
        let b = bt.get(&asset).unwrap_or(&empty);
        for (metric, t, b) in [
            ("inbound", &t.incoming, &b.incoming),
            ("outbound", &t.outgoing, &b.outgoing),
            ("fees", &t.fees, &b.fees),
        ] {
            totals.push(dict([
                ("asset", s(display(&asset))),
                ("metric", s(metric)),
                ("tool", exact(t)?),
                ("kassiber", exact(b)?),
                ("delta", exact(&(t - b))?),
                ("agrees", Value::Bool(same(t, b))),
            ]));
        }
        if !t.income.is_zero() || !b.income.is_zero() {
            let delta = t.income.sub(&b.income)?;
            let known = t.unvalued == 0;
            totals.push(dict([
                ("asset", s(display(&asset))),
                ("metric", s("income_value")),
                (
                    "tool",
                    if known {
                        money(Some(&t.income))?
                    } else {
                        s("")
                    },
                ),
                ("kassiber", money(Some(&b.income))?),
                ("delta", if known { money(Some(&delta))? } else { s("") }),
                (
                    "agrees",
                    if known {
                        Value::Bool(!delta.abs()?.greater(&Number::from(Decimal::from(1)))?)
                    } else {
                        Value::Null
                    },
                ),
            ]));
        }
    }
    let mut insights = Vec::new();
    if offset != 0 {
        let hours = offset / 3_600_000_000;
        insights.push(insight(
            "clock_offset",
            dict([("hours", Value::Int(hours.into()))]),
            format!(
                "{label} times are {hours:+} h from Kassiber's (UTC); matching allowed for it."
            ),
        ));
    }
    if !types {
        insights.push(insight("journals_stale",dict::<&str>([]),"Journals are not current, so transaction types were not compared. Process journals and check again.".into()));
    }
    if !skipped.is_empty() {
        insights.push(insight("skipped_tool_wallets",dict([("count",Value::Int(skipped.len().into())),("wallets",Value::List(skipped.iter().map(s).collect()))]),format!("Skipped {} {} imported from {label} ({}); comparing them would check {label} against itself.",skipped.len(),if skipped.len()==1{"wallet"}else{"wallets"},skipped.join(", "))));
    }
    if tool_other > 0 {
        insights.push(insight(
            "tool_other_assets",
            dict([("count", Value::Int(tool_other.into()))]),
            format!(
                "{label}: {} without a Bitcoin leg not compared.",
                rows_label(tool_other)
            ),
        ));
    }
    if !book_other.is_empty() {
        let count: usize = book_other.values().sum();
        let assets = book_other.keys().cloned().collect::<Vec<_>>().join(", ");
        insights.push(insight(
            "kassiber_other_assets",
            dict([("count", Value::Int(count.into())), ("assets", s(&assets))]),
            format!("Kassiber: {} in {assets} not compared.", rows_label(count)),
        ));
    }
    let probable = result
        .iter()
        .filter(|r| text(r, "match") == "probable")
        .count();
    if probable > 0 {
        insights.push(insight(
            "probable_matches",
            dict([("count", Value::Int(probable.into()))]),
            format!(
                "{} without a shared transaction id matched by amount and time.",
                rows_label(probable)
            ),
        ));
    }
    let mut sources = source_counts
        .into_iter()
        .map(|(name, n)| {
            dict([
                ("name", s(&name)),
                ("rows", Value::Int(n.into())),
                (
                    "matched",
                    Value::Int(source_matched.get(&name).copied().unwrap_or(0).into()),
                ),
            ])
        })
        .collect::<Vec<_>>();
    sources.sort_by_key(|r| (-int(r, "rows"), text(r, "name")));
    let pairs = pairs(&mut result, &votes);
    let bounds=rows(conn,"SELECT MIN(occurred_at) AS first,MAX(occurred_at) AS last FROM transactions WHERE profile_id=? AND occurred_at NOT IN ('','1970-01-01T00:00:00Z')",[text(&profile,"id")])?;
    let mut years = Vec::new();
    if let Some(b) = bounds
        .first()
        .filter(|r| truthy(value(r, "first")) && truthy(value(r, "last")))
    {
        let parse = |k| {
            let raw = text(b, k);
            let invalid = || {
                AppError::new(
                    "internal_error",
                    format!(
                        "Invalid isoformat string: {}",
                        kassiber_pytext::repr_str(&raw.replace('Z', "+00:00"))
                    ),
                )
            };
            if raw != kassiber_pytext::strip(&raw) {
                return Err(invalid());
            }
            utc(&raw)?.ok_or_else(invalid)
        };
        let first = year(parse("first")?, zone)?;
        let last = year(parse("last")?, zone)?;
        years = (first..=last).rev().map(|y| Value::Int(y.into())).collect();
    }
    Ok(dict([
        ("tool", s(tool)),
        ("tool_label", s(label)),
        ("file_name", s(filename)),
        ("book_currency", s(currency)),
        (
            "year",
            selected_year
                .map(|y| Value::Int(y.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "calendar",
            s(if selected_year.is_none() {
                ""
            } else {
                zone.unwrap_or("UTC")
            }),
        ),
        ("years", Value::List(years)),
        (
            "wallet",
            if wallet_labels.is_empty() {
                s("")
            } else if wallet_labels.len() > 3 {
                s(format!("{} wallets", wallet_labels.len()))
            } else {
                s(wallet_labels.join(", "))
            },
        ),
        (
            "agrees",
            Value::Bool(int(&count, "matched") == result.len() as i64 && !result.is_empty()),
        ),
        ("counts", count),
        ("tax_differences", Value::Int(tax.into())),
        ("totals", Value::List(totals)),
        ("sources", Value::List(sources)),
        ("pairs", Value::List(pairs)),
        ("insights", Value::List(insights)),
        ("rows", Value::List(result)),
    ]))
}

pub(super) fn read_cli(conn: &Connection, args: &Value) -> Result<Value> {
    let raw = text(args, "tool");
    let tool = kassiber_pytext::lower(kassiber_pytext::strip(&raw));
    let label = match tool.as_str() {
        "cointracking" => "CoinTracking",
        "blockpit" => "Blockpit",
        _ => {
            return Err(AppError::new("validation", format!("Unknown tool '{raw}'"))
                .with_hint("Use one of: blockpit, cointracking."))
        }
    };
    let path = text(args, "file");
    let path = Path::new(&path);
    if path.as_os_str().is_empty() || !path.is_file() {
        return Err(
            AppError::new("validation", format!("{label} export not found")).with_hint(format!(
                "Choose the transaction CSV you exported from {label}."
            )),
        );
    }
    // Scope errors precede decoding or opening an otherwise existing export.
    kassiber_setup::references::scope(conn, args)?;
    let bytes = std::fs::read(path).map_err(|e| export::io_error(e, Some(path)))?;
    report(
        conn,
        args,
        &bytes,
        &path.file_name().unwrap_or_default().to_string_lossy(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn canonical(v: &Value) -> String {
        kassiber_pyjson::dumps(
            v,
            &kassiber_pyjson::Options {
                sort_keys: true,
                separators: Some((",".into(), ":".into())),
                ..Default::default()
            },
        )
        .unwrap()
    }
    /// `_shift_utc(datetime.min, -26h)` is `datetime.min` and
    /// `_shift_utc(datetime.max, 26h)` is `datetime.max`; other shifts are
    /// plain arithmetic (a6fbe0ffd).
    #[test]
    fn shifts_clamp_at_the_calendar_ends_like_python() {
        let last = 315_537_897_599_999_999;
        let day = 26 * 3_600_000_000;
        assert_eq!(shift(0, -day), 0);
        assert_eq!(shift(last, day), last);
        assert_eq!(shift(0, 3_600_000_000), 3_600_000_000);
        assert_eq!(shifted(Some(last - 5), -10).unwrap(), Some(last));
        assert_eq!(shifted(Some(5), 10).unwrap(), Some(0));
    }
    /// The probable-match window holds exactly 26 hours on either side, as
    /// Python's `bisect_left`/`bisect_right` bounds do.
    #[test]
    fn probable_matches_include_both_window_ends() {
        let day = 26 * 3_600_000_000;
        let at = 1_000 * day;
        let tool = Tool {
            index: 0,
            source: String::new(),
            label: String::new(),
            txid: String::new(),
            at: Some(at),
            direction: "inbound".into(),
            asset: "BTC".into(),
            amount: BigInt::from(1000),
            fee: BigInt::from(0),
            kind: None,
            cash: None,
            error: String::new(),
        };
        let row = |delta: i64| BookRow {
            id: "b".into(),
            wallet: "w".into(),
            txid: String::new(),
            at: at + delta,
            direction: "inbound".into(),
            asset: "BTC".into(),
            amount: BigInt::from(1000),
            fee: BigInt::from(0),
            classification: String::new(),
            cash: None,
            excluded: false,
            non_taxable: false,
            matched: false,
        };
        for (delta, found) in [
            (day, true),
            (-day, true),
            (day + 1, false),
            (-day - 1, false),
        ] {
            let matched = probable(&tool, &[row(delta)], false, 0).unwrap();
            assert_eq!(matched, found.then_some(0), "{delta}");
        }
    }
    #[test]
    fn cross_check_vectors_pin_parsing_matching_totals_and_workbooks() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/data/cross_check.json")).unwrap();
        assert!(cases.len() > 100);
        let root = tempfile::tempdir().unwrap();
        let book = kassiber_store::Book::create(&root.path().join("data"), None).unwrap();
        let conn = book.connection();
        // Every case rolls back; durable syncs of its spilled pages only slow
        // the run on a busy disk.
        conn.execute_batch("PRAGMA synchronous=OFF").unwrap();
        let mut differing = Vec::new();
        for case in cases {
            conn.execute_batch("SAVEPOINT cross_fixture").unwrap();
            conn.execute_batch(include_str!("../tests/data/book.sql"))
                .unwrap();
            conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
            let before = conn.total_changes();
            let mut workbook_differs = false;
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let actual = match report(
                conn,
                &args,
                case["csv"].as_str().unwrap().as_bytes(),
                "export.csv",
            ) {
                Ok(report) => {
                    let lang = crate::report_design::Lang::En;
                    if let Some(error) = case.get("workbook_error") {
                        let actual = cross_check_xlsx::document(
                            &report,
                            "2026-01-02T03:04:05Z".into(),
                            lang,
                        )
                        .err();
                        workbook_differs |= actual.is_none_or(|actual| {
                            actual.message != error["message"].as_str().unwrap()
                        });
                    }
                    if !case["document"].is_null() {
                        // The workbook is the shared design over this report:
                        // a summary, then one sheet per outcome.
                        let expected = case["document"]["sheets"].as_array().unwrap().len();
                        // A pending case's workbook may still refuse (non-finite
                        // values); it counts as differing, as its report does.
                        match cross_check_xlsx::document(
                            &report,
                            "2026-01-02T03:04:05Z".into(),
                            lang,
                        ) {
                            Err(_) => workbook_differs = true,
                            Ok(actual) => {
                                // The summary, the design's totals and wallet tables,
                                // then a sheet per outcome that has rows.
                                let names: Vec<&str> =
                                    actual.sheets.iter().map(|s| s.name.as_str()).collect();
                                let outcomes = [
                                    "differences",
                                    "only_in_tool",
                                    "only_in_kassiber",
                                    "excluded_in_kassiber",
                                    "matched",
                                    "not_comparable",
                                ];
                                assert_eq!(expected, outcomes.len() + 1, "{}", case["name"]);
                                assert!(
                                    names[1..].iter().all(|n| outcomes.contains(n)
                                        || ["totals", "wallet_pairs"].contains(n)),
                                    "{}: {names:?}",
                                    case["name"]
                                );
                                assert_eq!(actual.sheets[0].name, "summary", "{}", case["name"]);
                                assert!(kassiber_xlsx::render(&actual).is_ok(), "{}", case["name"]);
                            }
                        }
                    }
                    dict([("ok", report)])
                }
                Err(e) => dict([(
                    "error",
                    dict([
                        ("code", s(e.code)),
                        ("message", s(e.message)),
                        ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                        ("details", e.details.unwrap_or(Value::Null)),
                        ("retryable", Value::Bool(e.retryable)),
                    ]),
                )]),
            };
            if workbook_differs || canonical(&actual) != case["expected_json"].as_str().unwrap() {
                differing.push(case["name"].as_str().unwrap().to_owned());
            }
            assert_eq!(
                before,
                conn.total_changes(),
                "{} wrote the book",
                case["name"]
            );
            conn.execute_batch("ROLLBACK TO cross_fixture; RELEASE cross_fixture")
                .unwrap();
        }
        assert_eq!(differing, Vec::<String>::new());
    }
}
