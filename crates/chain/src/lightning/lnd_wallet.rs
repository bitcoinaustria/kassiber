//! `sync_lnd_wallet`'s reads: the node's network, its complete payment,
//! invoice and forwarding histories, then its open and closed channels. Rows
//! stay in memory; the wallet sync keeps only the fields its ledger rows and
//! lifecycle evidence need.
use super::*;

/// What `sync_lnd_wallet` reads before its import.
pub struct LndWalletHistory {
    pub network: PyStr,
    pub payments: Vec<Value>,
    pub invoices: Vec<Value>,
    pub forwards: Vec<Value>,
}

impl LndReportClient {
    /// `getinfo`, `_fetch_lnd_payments` (completed only), `_fetch_lnd_invoices`
    /// and `_fetch_lnd_forwarding_history` over the whole history, in
    /// Python's order; any failed read fails the sync.
    pub fn wallet_history(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<LndWalletHistory> {
        let mut read = |path: &str, body: Option<Json>| self.read(path, body, consent, deadline);
        let info = read("/v1/getinfo", None)?;
        let network = lnd_network(&info);
        let payments = indexed_history(
            &mut read,
            "/v1/payments?include_incomplete=false",
            "payments",
            "max_payments",
            &["payment_index", "payment_hash"],
        )?;
        let invoices = indexed_history(
            &mut read,
            "/v1/invoices?",
            "invoices",
            "num_max_invoices",
            &["add_index", "r_hash"],
        )?;
        let mut forwards = Vec::new();
        let mut seen = BTreeSet::new();
        let mut offset = BigInt::from(0);
        loop {
            let mut body = serde_json::Map::new();
            body.insert("start_time".into(), "0".into());
            body.insert("index_offset".into(), offset.to_string().into());
            body.insert("num_max_events".into(), LND_MAX_PAGE_SIZE.into());
            let payload = read("/v1/switch", Some(Json::Object(body)))?;
            let page = lnd_rows(&payload, "forwarding_events")?;
            for row in &page {
                let key = [
                    first_text(row, &["timestamp_ns", "timestamp"]),
                    first_text(row, &["chan_id_in"]),
                    first_text(row, &["chan_id_out"]),
                    first_text(row, &["amt_in_msat", "amt_in"]),
                    first_text(row, &["amt_out_msat", "amt_out"]),
                    first_text(row, &["fee_msat", "fee"]),
                ];
                if seen.insert(key) {
                    forwards.push((*row).clone());
                }
            }
            let next = cursor(&payload, "last_offset_index", "last_index_offset")?;
            if page.is_empty() {
                break;
            }
            if next <= offset {
                return Err(history_incomplete(
                    "LND forwarding history cursor did not advance",
                ));
            }
            offset = next;
        }
        Ok(LndWalletHistory {
            network,
            payments,
            invoices,
            forwards,
        })
    }

    /// `/v1/channels` and `/v1/channels/closed`: the rows behind the
    /// wallet's channel-lifecycle evidence.
    pub fn wallet_channels(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let open = self.read("/v1/channels", None, consent, deadline)?;
        let closed = self.read("/v1/channels/closed", None, consent, deadline)?;
        let rows = |payload: &Value| match field(payload, "channels") {
            Value::List(rows) => rows
                .iter()
                .filter(|row| matches!(row, Value::Dict(_)))
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let channels = (rows(&open), rows(&closed));
        self.http
            .gate
            .authorize(self.request().clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        Ok(channels)
    }
}

const CHANNEL_ID_PREFIX: &str = "lnd:";
/// `_LIFECYCLE_AMOUNT_INCOMPLETE`.
const AMOUNT_INCOMPLETE: i64 = -1;

fn s(text: impl Into<PyStr>) -> Value {
    Value::Str(text.into())
}

/// CPython's non-strict `binascii.a2b_base64`, after `urlsafe_b64decode`'s
/// `-_` to `+/` translation when `urlsafe`: characters outside the alphabet
/// are skipped, a complete pad sequence ends the input, and a partial final
/// quad fails.
fn a2b_base64(text: &str, urlsafe: bool) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let value = |byte: u8| -> Option<u32> {
        Some(match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'-' if urlsafe => 62,
            b'_' if urlsafe => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::new();
    let (mut quad, mut left, mut pads) = (0u8, 0u32, 0u8);
    for byte in text.bytes() {
        if byte == b'=' {
            if quad >= 2 {
                pads += 1;
                if quad + pads >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let Some(bits) = value(byte) else {
            continue;
        };
        pads = 0;
        match quad {
            0 => {
                quad = 1;
                left = bits;
            }
            1 => {
                quad = 2;
                out.push(((left << 2) | (bits >> 4)) as u8);
                left = bits & 0x0f;
            }
            2 => {
                quad = 3;
                out.push(((left << 4) | (bits >> 2)) as u8);
                left = bits & 0x03;
            }
            _ => {
                quad = 0;
                out.push(((left << 6) | bits) as u8);
                left = 0;
            }
        }
    }
    (quad == 0).then_some(out)
}

/// `_normalize_lnd_hash`: hex as is, else 32 bytes of URL-safe or standard
/// base64 (the REST gateway's `r_hash`), else `None`.
fn payment_hash(value: &Value) -> Value {
    if !truthy(value) {
        return Value::Null;
    }
    let text = kassiber_pytext::strip(&py_str(value)).to_owned();
    if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return s(text.to_lowercase().as_str());
    }
    let padded = format!("{text}{}", "=".repeat((4 - text.len() % 4) % 4));
    for urlsafe in [true, false] {
        if let Some(raw) = a2b_base64(&padded, urlsafe).filter(|raw| raw.len() == 32) {
            return s(raw
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
                .as_str());
        }
    }
    Value::Null
}

/// `_msat(value, sats=...)`.
fn lnd_msat(value: &Value, sats: bool) -> Result<BigInt> {
    let amount = integer(value)?;
    Ok(if sats { amount * 1000u32 } else { amount })
}

fn first_of<'a>(row: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter()
        .map(|key| field(row, key))
        .find(|v| truthy(v))
        .unwrap_or_else(|| field(row, keys[keys.len() - 1]))
}

/// `_timestamp(...) or UNKNOWN_OCCURRED_AT`.
fn occurred(value: &Value) -> Result<Value> {
    Ok(s(lnd_iso(value)?
        .unwrap_or_else(|| UNKNOWN_OCCURRED_AT.to_owned())
        .as_str()))
}

/// One ledger row as `lnd_import_records` builds it; `amount` and `fee`
/// carry integer msat for the caller to render as BTC.
#[allow(clippy::too_many_arguments)]
fn ledger_row(
    id: String,
    occurred_at: Value,
    direction: &str,
    amount: BigInt,
    fee: BigInt,
    kind: &str,
    description: Value,
    hash: Value,
) -> Value {
    let source = if truthy(&hash) { s("lnd") } else { Value::Null };
    dict([
        ("id", s(id.as_str())),
        ("occurred_at", occurred_at.clone()),
        ("confirmed_at", occurred_at),
        ("direction", s(direction)),
        ("asset", s("BTC")),
        ("amount", Value::Int(amount)),
        ("fee", Value::Int(fee)),
        ("kind", s(kind)),
        ("description", description),
        ("counterparty", Value::Null),
        ("payment_hash", hash),
        ("payment_hash_source", source),
        ("raw_json", s("{}")),
    ])
}

/// `_lnd_invoice_import`: a settled invoice as inbound income.
fn invoice(row: &Value) -> Result<Option<Value>> {
    let state = kassiber_pytext::upper(&py_str(field(row, "state")));
    let state = if truthy(field(row, "state")) {
        state
    } else {
        String::new()
    };
    if !(lnd_truthy(field(row, "settled")) || state == "SETTLED") {
        return Ok(None);
    }
    let mut amount = lnd_msat(first_of(row, &["amt_paid_msat", "value_msat"]), false)?;
    if amount <= BigInt::from(0) {
        amount = lnd_msat(first_of(row, &["amt_paid_sat", "value"]), true)?;
    }
    if amount <= BigInt::from(0) {
        return Ok(None);
    }
    let hash = payment_hash(field(row, "r_hash"));
    let external = match &hash {
        Value::Str(_) => py_str(&hash),
        _ => {
            let fallback = first_of(row, &["settle_index", "add_index", "creation_date"]);
            if truthy(fallback) {
                py_str(fallback)
            } else {
                String::new()
            }
        }
    };
    if external.is_empty() {
        return Ok(None);
    }
    let memo = field(row, "memo");
    Ok(Some(ledger_row(
        format!("lnd:invoice:{external}"),
        occurred(first_of(row, &["settle_date", "creation_date"]))?,
        "inbound",
        amount,
        0.into(),
        "lnd_invoice",
        if truthy(memo) {
            s(py_str(memo).as_str())
        } else {
            s("LND invoice")
        },
        hash,
    )))
}

/// `_lnd_payment_settled_at`: the latest successful HTLC resolution, only
/// when every successful part carries valid timing.
fn settled_at(row: &Value) -> Result<Option<String>> {
    let Value::List(attempts) = field(row, "htlcs") else {
        return Ok(None);
    };
    let successful: Vec<&Value> = attempts
        .iter()
        .filter(|a| matches!(a, Value::Dict(_)) && *field(a, "status") == s("SUCCEEDED"))
        .collect();
    if successful.is_empty() {
        return Ok(None);
    }
    let mut latest: Option<u64> = None;
    for attempt in successful {
        let text = match field(attempt, "resolve_time_ns") {
            Value::Int(n) => n.to_string(),
            Value::Str(t) => match t.as_str() {
                Some(t) => t.to_owned(),
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        if text.len() > 19 || !text.bytes().all(|b| b.is_ascii_digit()) || text.is_empty() {
            return Ok(None);
        }
        let Ok(nanos) = text.parse::<u64>() else {
            return Ok(None);
        };
        if !(1_000_000_000..=i64::MAX as u64).contains(&nanos) {
            return Ok(None);
        }
        latest = Some(latest.map_or(nanos, |l| l.max(nanos)));
    }
    let seconds = latest.expect("successful parts") / 1_000_000_000;
    lnd_iso(&Value::Int(seconds.into()))
}

/// `_lnd_payment_import`: a succeeded payment as an outbound row.
fn payment(row: &Value) -> Result<Option<Value>> {
    let status = kassiber_pytext::upper(&if truthy(field(row, "status")) {
        py_str(field(row, "status"))
    } else {
        String::new()
    });
    if !["SUCCEEDED", "COMPLETE", "COMPLETED"].contains(&status.as_str()) {
        return Ok(None);
    }
    let mut amount = lnd_msat(field(row, "value_msat"), false)?;
    if amount <= BigInt::from(0) {
        amount = lnd_msat(field(row, "value"), true)?;
    }
    if amount <= BigInt::from(0) {
        return Ok(None);
    }
    let mut fee = lnd_msat(field(row, "fee_msat"), false)?;
    if fee <= BigInt::from(0) {
        fee = lnd_msat(field(row, "fee"), true)?;
    }
    let hash = payment_hash(field(row, "payment_hash"));
    let external = match &hash {
        Value::Str(_) => py_str(&hash),
        _ => {
            let fallback = first_of(row, &["payment_index", "creation_date"]);
            if truthy(fallback) {
                py_str(fallback)
            } else {
                String::new()
            }
        }
    };
    if external.is_empty() {
        return Ok(None);
    }
    let mut created = field(row, "creation_date").clone();
    if !truthy(&created) && truthy(field(row, "creation_time_ns")) {
        created = Value::Int(floor_div(
            integer(field(row, "creation_time_ns"))?,
            1_000_000_000,
        ));
    }
    let occurred_at = match settled_at(row)? {
        Some(at) => s(at.as_str()),
        None => occurred(&created)?,
    };
    Ok(Some(ledger_row(
        format!("lnd:pay:{external}"),
        occurred_at,
        "outbound",
        amount,
        fee,
        "lnd_pay",
        s("LND payment"),
        hash,
    )))
}

/// `_lnd_routing_income_imports`: settled forwarding fees, one row per UTC day.
fn routing_income(forwards: &[Value]) -> Result<Vec<Value>> {
    let mut days: BTreeMap<String, BigInt> = BTreeMap::new();
    for row in forwards {
        let mut raw = integer(field(row, "timestamp"))?;
        if raw <= BigInt::from(0) {
            raw = floor_div(integer(field(row, "timestamp_ns"))?, 1_000_000_000);
        }
        let Some(at) = lnd_iso(&Value::Int(raw))? else {
            continue;
        };
        let mut fee = integer(field(row, "fee_msat"))?;
        if fee <= BigInt::from(0) {
            fee = lnd_msat(field(row, "fee"), true)?;
        }
        if fee <= BigInt::from(0) {
            continue;
        }
        *days.entry(at[..10].to_owned()).or_insert_with(|| 0.into()) += fee;
    }
    Ok(days
        .into_iter()
        .map(|(day, fee)| {
            ledger_row(
                format!("lnd:routing:{day}"),
                s(format!("{day}T23:59:59Z").as_str()),
                "inbound",
                fee,
                0.into(),
                "routing_income",
                s(format!("Lightning routing fees ({day})").as_str()),
                Value::Null,
            )
        })
        .collect())
}

/// `lnd_import_records` in Python's order: invoices, payments, routing days.
/// `amount` and `fee` carry integer msat.
pub fn lnd_import_records(history: &LndWalletHistory) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for row in &history.invoices {
        records.extend(invoice(row)?);
    }
    for row in &history.payments {
        records.extend(payment(row)?);
    }
    records.extend(routing_income(&history.forwards)?);
    Ok(records)
}

/// The ids of payments live HTLC timing settles: only these may correct a
/// previous import's date (`authoritative_settlement_ids`).
pub fn lnd_settlement_ids(history: &LndWalletHistory) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for row in &history.payments {
        if settled_at(row)?.is_some() {
            if let Some(record) = payment(row)? {
                ids.push(py_str(field(&record, "id")));
            }
        }
    }
    Ok(ids)
}

/// `canonical_txid`: a lowercase 64-hex txid, else `None`.
fn canonical_txid(value: &Value) -> Option<String> {
    let text = if truthy(value) {
        py_str(value)
    } else {
        String::new()
    };
    let text = kassiber_pytext::strip(&text);
    (text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_lowercase())
}

fn text_of(row: &Value, key: &str) -> String {
    let value = field(row, key);
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

/// `_lnd_channel_id`.
fn channel_id(row: &Value) -> String {
    let point = kassiber_pytext::strip(&text_of(row, "channel_point")).to_owned();
    let fallback =
        kassiber_pytext::strip(&py_str_or_empty(first_of(row, &["chan_id", "channel_id"])))
            .to_owned();
    format!(
        "{CHANNEL_ID_PREFIX}{}",
        if point.is_empty() { fallback } else { point }
    )
}

fn py_str_or_empty(value: &Value) -> String {
    if truthy(value) {
        py_str(value)
    } else {
        String::new()
    }
}

/// `_lnd_local_initiator`.
fn local_initiator(row: &Value) -> Option<bool> {
    if get(row, "initiator").is_some() {
        return Some(lnd_truthy(field(row, "initiator")));
    }
    match kassiber_pytext::upper(kassiber_pytext::strip(&text_of(row, "open_initiator"))).as_str() {
        "INITIATOR_LOCAL" | "LOCAL" => Some(true),
        "INITIATOR_REMOTE" | "REMOTE" => Some(false),
        _ => None,
    }
}

/// `_lnd_local_funding_sat`.
fn local_funding_sat(row: &Value, initiator: Option<bool>, infer: bool) -> Result<BigInt> {
    let explicit = integer(first_of(
        row,
        &["local_funding_amount_sat", "local_contribution_sat"],
    ))?;
    if explicit > BigInt::from(0) {
        return Ok(explicit);
    }
    if !infer || initiator != Some(true) {
        return Ok(0.into());
    }
    if integer(first_of(row, &["push_amount_sat", "push_sat"]))? > BigInt::from(0)
        || integer(first_of(
            row,
            &["remote_funding_amount_sat", "remote_contribution_sat"],
        ))? > BigInt::from(0)
        || lnd_truthy(first_of(row, &["is_dual_funded", "dual_funded"]))
    {
        return Ok(0.into());
    }
    integer(field(row, "capacity"))
}

/// `_lnd_local_close_sweeps`: claimed resolutions of the closing outputs.
fn close_sweeps(row: &Value, closing: &str) -> Vec<(String, String)> {
    let mut evidence = BTreeSet::new();
    if let Value::List(resolutions) = field(row, "resolutions") {
        for resolution in resolutions.iter().filter(|r| matches!(r, Value::Dict(_))) {
            let outcome =
                kassiber_pytext::upper(kassiber_pytext::strip(&text_of(resolution, "outcome")));
            if !["CLAIMED", "RESOLUTION_OUTCOME_CLAIMED"].contains(&outcome.as_str()) {
                continue;
            }
            let Some(sweep) = canonical_txid(field(resolution, "sweep_txid")) else {
                continue;
            };
            let raw = kassiber_pytext::strip(&text_of(resolution, "outpoint")).to_owned();
            let Some((txid, vout)) = raw.rsplit_once(':') else {
                continue;
            };
            if canonical_txid(&s(txid)).as_deref() != Some(closing) {
                continue;
            }
            let Ok(index) = kassiber_pytext::parse_int(vout) else {
                continue;
            };
            if index < BigInt::from(0) {
                continue;
            }
            evidence.insert((sweep, format!("{closing}:{index}")));
        }
    }
    evidence.into_iter().collect()
}

/// `_lnd_channel_record`.
fn channel_record(
    tag: &str,
    txid: &str,
    row: &Value,
    amount: BigInt,
    network: &PyStr,
    sweeps: &[(String, String)],
) -> Result<Value> {
    let network = kassiber_pytext::strip(&py_str(&Value::Str(network.clone()))).to_owned();
    let mut scope = if network.is_empty() {
        dict::<&str>([])
    } else {
        dict([("chain", s("bitcoin")), ("network", s(network.as_str()))])
    };
    if !sweeps.is_empty() {
        kassiber_envelope::set(
            &mut scope,
            "_kassiber_provenance",
            dict([("import_source", s("lnd_adapter"))]),
        );
        kassiber_envelope::set(
            &mut scope,
            "channel_close_local_sweeps",
            Value::List(
                sweeps
                    .iter()
                    .map(|(sweep, outpoint)| {
                        dict([
                            ("sweep_txid", s(sweep.as_str())),
                            ("outpoint", s(outpoint.as_str())),
                        ])
                    })
                    .collect(),
            ),
        );
    }
    let id = channel_id(row);
    let point = text_of(row, "channel_point");
    let raw = kassiber_pyjson::dumps(&scope, &kassiber_pyjson::Options::new().sort_keys(true))
        .map_err(|_| AppError::new("internal_error", "Invalid LND channel record"))?;
    Ok(dict([
        ("record_type", s("channel")),
        (
            "external_id",
            s(format!("channel:{tag}:{id}:{txid}").as_str()),
        ),
        ("tag", s(tag)),
        ("txid", s(txid)),
        (
            "outpoint",
            if point.is_empty() {
                Value::Null
            } else {
                s(point.as_str())
            },
        ),
        ("channel_id", s(id.as_str())),
        ("amount_msat", Value::Int(amount.clone())),
        (
            "status",
            s(if amount > BigInt::from(0) {
                "complete"
            } else {
                "incomplete"
            }),
        ),
        ("raw_json", s(raw.as_str())),
    ]))
}

/// `lnd_channel_records`: amount-bearing funding and closing evidence keyed
/// by external id, in key order.
pub fn lnd_channel_records(
    open: &[Value],
    closed: &[Value],
    network: &PyStr,
) -> Result<Vec<Value>> {
    let mut records: BTreeMap<String, Value> = BTreeMap::new();
    for (row, infer) in open
        .iter()
        .map(|r| (r, true))
        .chain(closed.iter().map(|r| (r, false)))
    {
        let point = text_of(row, "channel_point");
        let funding = point
            .split_once(':')
            .map_or(point.as_str(), |(t, _)| t)
            .to_owned();
        let initiator = local_initiator(row);
        if funding.is_empty() || initiator == Some(false) {
            continue;
        }
        let local = local_funding_sat(row, initiator, infer)?;
        let amount = if initiator == Some(true) && local > BigInt::from(0) {
            local * 1000u32
        } else {
            BigInt::from(AMOUNT_INCOMPLETE)
        };
        let record = channel_record("channel_open", &funding, row, amount, network, &[])?;
        records.insert(py_str(field(&record, "external_id")), record);
    }
    for row in closed {
        let Some(closing) = canonical_txid(field(row, "closing_tx_hash")) else {
            continue;
        };
        let balance =
            integer(field(row, "settled_balance"))? + integer(field(row, "time_locked_balance"))?;
        let amount = if balance > BigInt::from(0) {
            balance * 1000u32
        } else {
            BigInt::from(AMOUNT_INCOMPLETE)
        };
        let sweeps = close_sweeps(row, &closing);
        let record = channel_record("channel_close", &closing, row, amount, network, &sweeps)?;
        records.insert(py_str(field(&record, "external_id")), record);
    }
    Ok(records.into_values().collect())
}
