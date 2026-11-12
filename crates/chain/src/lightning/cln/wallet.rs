//! `sync_core_lightning_wallet`'s read side: `fetch_core_lightning_snapshot`
//! (every allowed method through the curated allowlists) and
//! `snapshot_records`, the narrow rows Kassiber persists. Raw RPC payloads
//! never leave this module; preimages, bolt11 strings, routes and onion
//! failures are dropped by the per-resource allowlists at the boundary.
use super::*;

const PEER_CHANNEL: &[&str] = &[
    "peer_id",
    "peer_alias",
    "alias",
    "peer_connected",
    "private",
    "is_private",
    "channel_id",
    "short_channel_id",
    "state",
    "status",
    "total_msat",
    "amount_msat",
    "funding_msat",
    "to_us_msat",
    "our_amount_msat",
    "their_amount_msat",
    "opener",
    "funder",
    "fee_base_msat",
    "fee_proportional_millionths",
    "opened_at",
    "closed_at",
    "funding",
    "funding_txid",
    "funding_outnum",
    "funding_outpoint",
];
const FORWARD: &[&str] = &[
    "in_channel",
    "out_channel",
    "fee_msat",
    "in_msat",
    "out_msat",
    "status",
    "received_time",
    "resolved_time",
];
const PAY: &[&str] = &[
    "payment_hash",
    "amount_msat",
    "amount_sent_msat",
    "status",
    "created_at",
    "completed_at",
    "destination",
    "rebalance",
];
const INVOICE: &[&str] = &[
    "label",
    "payment_hash",
    "amount_msat",
    "amount_received_msat",
    "status",
    "paid_at",
    "created_at",
    "description",
];
const INCOME: &[&str] = &[
    "account",
    "tag",
    "credit_msat",
    "debit_msat",
    "currency",
    "timestamp",
    "payment_id",
    "description",
    "txid",
];
const BALANCE_ACCOUNT: &[&str] = &["account", "peer_id", "balances", "account_closed"];
const ACCOUNT_EVENT: &[&str] = &[
    "account",
    "type",
    "tag",
    "txid",
    "outpoint",
    "timestamp",
    "credit_msat",
    "debit_msat",
];

/// `CoreLightningSnapshot` as the wallet sync reads it.
pub struct WalletSnapshot {
    pub node_id: PyStr,
    pub node_alias: PyStr,
    pub network: PyStr,
    channels: Vec<Value>,
    forwards: Vec<Value>,
    pays: Vec<Value>,
    invoices: Vec<Value>,
    income: Vec<Value>,
    balances: Vec<Value>,
    account_events: Vec<Value>,
    /// `errors`: each optional method that failed, with its message.
    pub errors: Vec<(String, String)>,
}

/// `_pick`: the allowlisted keys present in the row, in allowlist order.
fn pick(row: &Value, keys: &[&str]) -> Value {
    dict(
        keys.iter()
            .filter_map(|key| get(row, key).map(|value| (*key, value.clone()))),
    )
}

/// `_sanitize_peer_channel`.
fn peer_channel(row: &Value) -> Value {
    let mut curated = pick(row, PEER_CHANNEL);
    if let Some(funding @ Value::Dict(_)) = get(&curated, "funding").cloned() {
        kassiber_envelope::set(
            &mut curated,
            "funding",
            pick(&funding, &["txid", "funding_txid", "output", "outnum"]),
        );
    }
    curated
}

/// `_sanitize_balance_account`.
fn balance_account(row: &Value) -> Value {
    let mut curated = pick(row, BALANCE_ACCOUNT);
    if let Some(Value::List(entries)) = get(&curated, "balances").cloned() {
        let entries = entries
            .iter()
            .filter(|entry| matches!(entry, Value::Dict(_)))
            .map(|entry| pick(entry, &["balance_msat", "coin_type"]))
            .collect();
        kassiber_envelope::set(&mut curated, "balances", Value::List(entries));
    }
    curated
}

/// `_sanitize_list`.
fn sanitized(payload: Option<&Value>, key: &str, sanitize: impl Fn(&Value) -> Value) -> Vec<Value> {
    payload
        .map(|payload| rows(payload, key).into_iter().map(sanitize).collect())
        .unwrap_or_default()
}

impl ClnReportClient {
    /// `fetch_core_lightning_snapshot`: `getinfo` must answer; every other
    /// method that fails is recorded in `errors` and read as empty. A refused
    /// network action (egress, consent, the overall deadline) stops the read.
    pub fn wallet_snapshot(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<WalletSnapshot> {
        let mut read = |method| {
            check()?;
            self.read(method, consent, deadline)
        };
        let info = read(ClnMethod::GetInfo)?;
        let text_of = |key: &str| stripped(field(&info, key)).unwrap_or_default();
        let (node_id, node_alias, network) = (text_of("id"), text_of("alias"), text_of("network"));
        let mut errors = Vec::new();
        let mut payloads = Vec::new();
        for method in [
            ClnMethod::ListFunds,
            ClnMethod::ListPeerChannels,
            ClnMethod::ListForwards,
            ClnMethod::ListPays,
            ClnMethod::ListInvoices,
            ClnMethod::ListIncome,
            ClnMethod::ListBalances,
            ClnMethod::ListAccountEvents,
        ] {
            payloads.push(match read(method) {
                Ok(payload) => Some(payload),
                Err(error)
                    if ![
                        "interaction_required",
                        "network_egress_disabled",
                        "backend_timeout",
                    ]
                    .contains(&error.code.as_str()) =>
                {
                    errors.push((method.name().to_owned(), error.message));
                    None
                }
                Err(error) => return Err(error),
            });
        }
        check()?;
        self.gate
            .authorize(self.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        crate::remaining(deadline)?;
        // `listfunds` is read for the node dashboard; the wallet sync keeps
        // none of its rows.
        let [_funds, channels, forwards, pays, invoices, income, balances, events] =
            <[Option<Value>; 8]>::try_from(payloads).expect("eight methods");
        Ok(WalletSnapshot {
            node_id,
            node_alias,
            network,
            channels: sanitized(channels.as_ref(), "channels", peer_channel),
            forwards: sanitized(forwards.as_ref(), "forwards", |r| pick(r, FORWARD)),
            pays: sanitized(pays.as_ref(), "pays", |r| pick(r, PAY)),
            invoices: sanitized(invoices.as_ref(), "invoices", |r| pick(r, INVOICE)),
            income: sanitized(income.as_ref(), "income_events", |r| pick(r, INCOME)),
            balances: sanitized(balances.as_ref(), "accounts", balance_account),
            account_events: sanitized(events.as_ref(), "events", |r| pick(r, ACCOUNT_EVENT)),
            errors,
        })
    }
}

/// `f"{prefix}{value}"`, keeping any lone surrogate of `value`.
fn concat(prefix: &str, value: &PyStr) -> PyStr {
    let points = prefix.chars().map(u32::from).chain(value.code_points());
    PyStr::from_code_points(points.collect::<Vec<u32>>()).expect("valid points")
}

fn s(text: impl Into<PyStr>) -> Value {
    Value::Str(text.into())
}

fn int(value: BigInt) -> Value {
    Value::Int(value)
}

/// One persistence row, in `snapshot_records`' key order.
#[allow(clippy::too_many_arguments)]
fn record(
    record_type: &str,
    external_id: Value,
    occurred_at: Value,
    account: Value,
    peer_id: Value,
    channel_id: Value,
    direction: &str,
    amount_msat: BigInt,
    fee_msat: BigInt,
    tag: &str,
    status: Value,
    currency: Value,
    payment_hash: Value,
) -> Value {
    dict([
        ("record_type", s(record_type)),
        ("external_id", external_id),
        ("occurred_at", occurred_at),
        ("account", account),
        ("peer_id", peer_id),
        ("channel_id", channel_id),
        ("direction", s(direction)),
        ("amount_msat", int(amount_msat)),
        ("fee_msat", int(fee_msat)),
        ("tag", s(tag)),
        ("status", status),
        ("currency", currency),
        ("payment_hash", payment_hash),
        ("txid", Value::Null),
        ("outpoint", Value::Null),
    ])
}

/// `_normalize_payment_hash`: 32-byte lowercase hex, else `None`.
fn payment_hash(value: &Value) -> Value {
    let Some(text) = stripped(value) else {
        return Value::Null;
    };
    let lowered = lower(&text);
    if lowered.len() == 64 && lowered.bytes().all(|b| b.is_ascii_hexdigit()) {
        s(lowered.as_str())
    } else {
        Value::Null
    }
}

/// `_aggregate_forwards`: settled forwards as one row per (day, out channel).
fn forward_days(snapshot: &WalletSnapshot) -> Result<Vec<Value>> {
    // ((day, out channel), (occurred_at, fee, amount, count)).
    type Bucket = ((PyStr, PyStr), (String, BigInt, BigInt, BigInt));
    let mut buckets: Vec<Bucket> = Vec::new();
    for row in &snapshot.forwards {
        if forward_status(field(row, "status")) != "settled" {
            continue;
        }
        let occurred = timestamp(first(row, &["resolved_time", "received_time"]))?;
        let day: PyStr = if occurred != UNKNOWN_OCCURRED_AT {
            occurred[..10].into()
        } else {
            occurred.as_str().into()
        };
        let channel = stripped(field(row, "out_channel")).unwrap_or_else(|| "unknown".into());
        let key = (day.clone(), channel);
        let index = match buckets.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                let occurred_at = if occurred != UNKNOWN_OCCURRED_AT {
                    format!("{}T00:00:00Z", &occurred[..10])
                } else {
                    occurred
                };
                buckets.push((key, (occurred_at, 0.into(), 0.into(), 0.into())));
                buckets.len() - 1
            }
        };
        let bucket = &mut buckets[index].1;
        bucket.1 += msat(field(row, "fee_msat"))?;
        bucket.2 += msat(field(row, "out_msat"))?;
        bucket.3 += 1;
    }
    buckets.sort_by(|a, b| a.0.cmp(&b.0));
    buckets
        .into_iter()
        .map(|((day, channel), (occurred_at, fee, amount, _count))| {
            Ok(record(
                "forward_day",
                Value::Str(stable_hash(vec![
                    s("fw_day"),
                    Value::Str(day),
                    Value::Str(channel.clone()),
                ])?),
                s(occurred_at.as_str()),
                Value::Str(channel.clone()),
                Value::Null,
                Value::Str(channel),
                "inbound",
                amount,
                fee,
                "routing_fee",
                s("settled"),
                s("bc"),
                Value::Null,
            ))
        })
        .collect()
}

/// `_invoice_record`: a paid invoice with a positive amount.
fn invoice(row: &Value) -> Result<Option<Value>> {
    if stripped(field(row, "status"))
        .as_ref()
        .map(|s| s.is("paid"))
        != Some(true)
    {
        return Ok(None);
    }
    let hash = payment_hash(field(row, "payment_hash"));
    let amount = msat(first(row, &["amount_received_msat", "amount_msat"]))?;
    if amount <= BigInt::from(0) {
        return Ok(None);
    }
    let label = stripped(field(row, "label"));
    let description = stripped(field(row, "description"));
    let external_id = match (&hash, &label) {
        (Value::Str(_), _) => hash.clone(),
        (_, Some(label)) => Value::Str(label.clone()),
        _ => Value::Str(stable_hash(vec![
            s("invoice"),
            field(row, "paid_at").clone(),
        ])?),
    };
    let status = description
        .or(label)
        .map(Value::Str)
        .unwrap_or_else(|| s("paid"));
    Ok(Some(record(
        "invoice",
        external_id,
        s(timestamp(first(row, &["paid_at", "created_at"]))?.as_str()),
        Value::Null,
        Value::Null,
        Value::Null,
        "inbound",
        amount,
        0.into(),
        "invoice",
        status,
        s("bc"),
        hash,
    )))
}

/// `_pay_record`: a completed payment, its routing fee split out.
fn pay(row: &Value) -> Result<Option<Value>> {
    let hash = payment_hash(field(row, "payment_hash"));
    let sent = msat(field(row, "amount_sent_msat"))?;
    let amount = msat(field(row, "amount_msat"))?;
    let fee = (&sent - &amount).max(0.into());
    let status = stripped(field(row, "status"))
        .map(|status| lower(&status))
        .unwrap_or_default();
    if !["complete", "completed", "paid"].contains(&status.as_str()) {
        return Ok(None);
    }
    let external_id = match &hash {
        Value::Str(_) => hash.clone(),
        _ => Value::Str(stable_hash(vec![
            s("pay"),
            field(row, "created_at").clone(),
            int(sent.clone()),
        ])?),
    };
    let tag = if truthy(field(row, "rebalance")) {
        "rebalance"
    } else {
        "payment"
    };
    let principal = if amount != BigInt::from(0) {
        amount
    } else {
        sent
    };
    Ok(Some(record(
        "pay",
        external_id,
        s(timestamp(first(row, &["completed_at", "created_at"]))?.as_str()),
        Value::Null,
        optional_value(stripped(field(row, "destination"))),
        Value::Null,
        "outbound",
        principal,
        fee,
        tag,
        s(status.as_str()),
        s("bc"),
        hash,
    )))
}

/// `_income_invoice_record`: bookkeeper invoice income without an L1 txid.
fn income(row: &Value) -> Result<Option<Value>> {
    let tag = stripped(field(row, "tag"))
        .map(|tag| lower(&tag))
        .unwrap_or_default();
    if tag != "invoice" || stripped(field(row, "txid")).is_some() {
        return Ok(None);
    }
    let credit = msat(field(row, "credit_msat"))?;
    let debit = msat(field(row, "debit_msat"))?;
    let received = &credit - debit;
    if received <= BigInt::from(0) {
        return Ok(None);
    }
    let hash = payment_hash(field(row, "payment_id"));
    let external_id = stable_hash(vec![
        s("income_invoice"),
        field(row, "account").clone(),
        field(row, "timestamp").clone(),
        hash.clone(),
        int(credit),
    ])?;
    let account = optional_value(stripped(field(row, "account")));
    Ok(Some(record(
        "income",
        Value::Str(external_id),
        s(timestamp(field(row, "timestamp"))?.as_str()),
        account.clone(),
        Value::Null,
        account,
        "inbound",
        received,
        0.into(),
        "invoice",
        stripped(field(row, "description"))
            .map(Value::Str)
            .unwrap_or_else(|| s("invoice")),
        stripped(field(row, "currency"))
            .map(Value::Str)
            .unwrap_or_else(|| s("bc")),
        hash,
    )))
}

/// `_balance_snapshot_records`: one row per bookkeeper account and day.
fn balances(snapshot: &WalletSnapshot, synced_at: &str) -> Result<Vec<Value>> {
    let day = synced_at.get(..10).unwrap_or(synced_at);
    let mut records = Vec::new();
    for account in &snapshot.balances {
        let mut total = BigInt::from(0);
        if let Value::List(entries) = field(account, "balances") {
            for entry in entries.iter().filter(|e| matches!(e, Value::Dict(_))) {
                total += msat(field(entry, "balance_msat"))?;
            }
        }
        let name = optional_value(stripped(field(account, "account")));
        records.push(record(
            "balance_snapshot",
            Value::Str(stable_hash(vec![s("balance_day"), s(day), name.clone()])?),
            if day.is_empty() {
                s(synced_at)
            } else {
                s(format!("{day}T00:00:00Z").as_str())
            },
            name.clone(),
            Value::Null,
            if truthy(field(account, "peer_id")) {
                name
            } else {
                Value::Null
            },
            "",
            total,
            0.into(),
            "balance",
            s(if truthy(field(account, "account_closed")) {
                "closed"
            } else {
                "open"
            }),
            s(""),
            Value::Null,
        ));
    }
    Ok(records)
}

/// `_channel_short_id`.
fn short_id(row: &Value) -> Option<PyStr> {
    ["short_channel_id", "alias", "channel_id"]
        .iter()
        .find_map(|key| stripped(field(row, key)))
}

/// `_channel_record`: one channel-lifecycle txid, never a wallet transaction.
fn channel_record(
    tag: &str,
    txid: &PyStr,
    account: &PyStr,
    amount: BigInt,
    network: &PyStr,
) -> Result<Value> {
    let scope = if network.is("") {
        "{}".to_owned()
    } else {
        // `json.dumps({"chain": ..., "network": ...}, sort_keys=True)`.
        kassiber_pyjson::dumps(
            &dict([
                ("chain", s("bitcoin")),
                ("network", Value::Str(network.clone())),
            ]),
            &kassiber_pyjson::Options::new().sort_keys(true),
        )
        .map_err(|_| AppError::new("internal_error", "Invalid Core Lightning network"))?
    };
    let mut row = record(
        "channel",
        Value::Str(stable_hash(vec![
            s("channel"),
            s(tag),
            Value::Str(txid.clone()),
            Value::Str(account.clone()),
        ])?),
        s(UNKNOWN_OCCURRED_AT),
        Value::Str(account.clone()),
        Value::Null,
        Value::Str(concat("coreln:", account)),
        "",
        amount,
        0.into(),
        tag,
        s(""),
        s("bc"),
        Value::Null,
    );
    kassiber_envelope::set(&mut row, "txid", Value::Str(txid.clone()));
    kassiber_envelope::set(&mut row, "raw_json", s(scope.as_str()));
    Ok(row)
}

/// `_channel_lifecycle_records`: funding and closing txids keyed by
/// (transaction, channel account), from bookkeeper events and open channels.
fn channels(snapshot: &WalletSnapshot) -> Result<Vec<Value>> {
    let network = &snapshot.network;
    let mut opens: BTreeMap<(PyStr, PyStr), BigInt> = BTreeMap::new();
    let mut closes: BTreeMap<(PyStr, PyStr), BigInt> = BTreeMap::new();
    for event in &snapshot.account_events {
        let tag = stripped(field(event, "tag"))
            .map(|tag| lower(&tag))
            .unwrap_or_default();
        let outpoint = stripped(field(event, "outpoint"));
        let txid = stripped(field(event, "txid")).or_else(|| {
            outpoint.as_ref().map(|outpoint| {
                let points: Vec<u32> = outpoint.code_points().collect();
                let end = points
                    .iter()
                    .position(|p| *p == u32::from(':'))
                    .unwrap_or(points.len());
                PyStr::from_code_points(points[..end].to_vec()).expect("valid points")
            })
        });
        let Some(txid) = txid.filter(|t| !t.is("")) else {
            continue;
        };
        let account = stripped(field(event, "account")).unwrap_or_else(|| txid.clone());
        let key = (txid, account);
        if tag == "channel_open" {
            let funded = msat(field(event, "credit_msat"))?.max(0.into());
            *opens.entry(key).or_insert_with(|| 0.into()) += funded;
        } else if tag == "channel_close" {
            let settled = msat(field(event, "debit_msat"))?.max(0.into());
            *closes.entry(key).or_insert_with(|| 0.into()) += settled;
        }
    }
    let mut by_txid: BTreeMap<PyStr, Vec<&Value>> = BTreeMap::new();
    for channel in &snapshot.channels {
        if let Some(outpoint) = funding_point(channel) {
            let points: Vec<u32> = outpoint.code_points().collect();
            let end = points
                .iter()
                .position(|p| *p == u32::from(':'))
                .unwrap_or(points.len());
            let txid = PyStr::from_code_points(points[..end].to_vec()).expect("valid points");
            if !txid.is("") {
                by_txid.entry(txid).or_default().push(channel);
            }
        }
    }
    for (txid, rows) in &by_txid {
        let accounts: Vec<PyStr> = opens
            .keys()
            .filter(|(t, _)| t == txid)
            .map(|(_, account)| account.clone())
            .collect();
        for channel in rows {
            let aliases: Vec<PyStr> = [stripped(field(channel, "channel_id")), short_id(channel)]
                .into_iter()
                .flatten()
                .collect();
            if accounts.iter().any(|account| aliases.contains(account)) {
                continue;
            }
            if rows.len() == 1 && accounts.len() == 1 {
                continue;
            }
            let account = aliases.first().cloned().unwrap_or_else(|| txid.clone());
            opens
                .entry((txid.clone(), account))
                .or_insert_with(|| 0.into());
        }
    }
    let mut records = Vec::new();
    for ((txid, account), amount) in opens {
        records.push(channel_record(
            "channel_open",
            &txid,
            &account,
            amount,
            network,
        )?);
    }
    for ((txid, account), amount) in closes {
        records.push(channel_record(
            "channel_close",
            &txid,
            &account,
            amount,
            network,
        )?);
    }
    Ok(records)
}

/// `snapshot_records`: aggregated forwards, paid invoices, completed pays,
/// invoice-only bookkeeper income, daily balances and channel txids.
pub fn wallet_records(snapshot: &WalletSnapshot, synced_at: &str) -> Result<Vec<Value>> {
    let mut records = forward_days(snapshot)?;
    for row in &snapshot.invoices {
        records.extend(invoice(row)?);
    }
    for row in &snapshot.pays {
        records.extend(pay(row)?);
    }
    for row in &snapshot.income {
        records.extend(income(row)?);
    }
    records.extend(balances(snapshot, synced_at)?);
    records.extend(channels(snapshot)?);
    Ok(records)
}
