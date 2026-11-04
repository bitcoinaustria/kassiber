use super::*;
use kassiber_net::{ClnConfig, ClnMethod, ClnProcessError};
use kassiber_pyjson::Decimal;
use std::time::Duration;

pub struct ClnReportClient {
    gate: Gate,
    request: Request,
    timeout: Duration,
    window_days: u16,
}

impl ClnReportClient {
    pub fn new(config: &Json, gate: Gate, action: ReportAction) -> Result<Self> {
        if config.is_null() {
            return Err(AppError::new("validation", "Core Lightning connection requires a backend configuration")
                .with_hint("Create a coreln backend (`kassiber backends create cln --kind coreln ...`) and reference it from the wallet."));
        }
        let mut config = config.clone();
        if !config.is_object() {
            return Err(AppError::new(
                "config_error",
                "Invalid backend configuration",
            ));
        }
        if let Some(raw) = config.get("config_json").and_then(Json::as_str) {
            config["config"] = serde_json::from_str(raw)
                .map_err(|_| AppError::new("config_error", "Invalid backend configuration JSON"))?;
        }
        let value = |keys: &[&str]| {
            config::field(&config, keys)
                .and_then(|v| stripped(&details(v.clone())))
                .and_then(|s| s.as_str().map(str::to_owned))
        };
        let peer = value(&["commando_peer_id"]);
        let rune = value(&["token"]);
        let wants_commando = peer.is_some()
            || rune.is_some()
            || value(&["url"])
                .is_some_and(|s| kassiber_pytext::lower(&s).starts_with("cln://commando"));
        let commando = if wants_commando {
            Some(peer.zip(rune).ok_or_else(|| AppError::new("validation", "Core Lightning commando sync requires both --commando-peer-id and a rune token")
                .with_hint("Pipe a restricted rune through --token-stdin or --token-fd FD."))?)
        } else {
            None
        };
        let timeout = value(&["timeout"])
            .map(|s| kassiber_pytext::parse_int(&s))
            .transpose()
            .map_err(|_| {
                AppError::new(
                    "app_error",
                    "Invalid integer value for Core Lightning timeout",
                )
            })?
            .unwrap_or_else(|| 30.into());
        let timeout = u64::try_from(timeout)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| AppError::new("validation", "Backend timeout must be positive"))?;
        let request = Request::cln(
            ClnConfig {
                program: value(&["lightning_cli"])
                    .unwrap_or_else(|| "lightning-cli".into())
                    .into(),
                lightning_dir: value(&["lightning_dir"]),
                rpc_file: value(&["rpc_file"]),
                network: value(&["network"]),
                commando,
            },
            action.binding(&config)?,
        );
        Ok(Self {
            gate,
            request,
            timeout: Duration::from_secs(timeout),
            window_days: action.window_days,
        })
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    fn read(
        &self,
        method: ClnMethod,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<Value> {
        let overall_deadline = deadline;
        let deadline = deadline.min(Instant::now().checked_add(self.timeout).unwrap_or(deadline));
        let result = self
            .gate
            .authorize(self.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?
            .cln(method, deadline, RESPONSE_LIMIT);
        let bytes = result.map_err(|error| match error {
            ClnProcessError::Gate(error) => error.app_error(),
            ClnProcessError::Missing => AppError::new("dependency_missing", "Core Lightning CLI executable was not found")
                .with_hint("Install Core Lightning or set backend config lightning_cli to the lightning-cli path.").retryable(true),
            ClnProcessError::Timeout if Instant::now() >= overall_deadline => AppError::new("backend_timeout", "Backend request budget exhausted").retryable(true),
            ClnProcessError::Timeout => AppError::new("timeout", format!("Core Lightning RPC method '{}' timed out", method.name())).retryable(true),
            ClnProcessError::OutputLimit => AppError::new("sync_error", "Core Lightning response is too large").retryable(true),
            _ => AppError::new("sync_error", format!("Core Lightning RPC method '{}' failed", method.name()))
                .with_hint("Check that the backend points at a running CLN node and that the rune allows this read method.").retryable(true),
        })?;
        let raw = std::str::from_utf8(&bytes).map_err(|_| {
            AppError::new("sync_error", "Core Lightning returned invalid UTF-8").retryable(true)
        })?;
        let payload =
            kassiber_pytext::loads(if raw.is_empty() { "{}" } else { raw }).map_err(|_| {
                AppError::new(
                    "sync_error",
                    format!(
                        "Core Lightning RPC method '{}' did not return JSON",
                        method.name()
                    ),
                )
                .retryable(true)
            })?;
        Ok(if matches!(payload, Value::Dict(_)) {
            payload
        } else {
            dict([("result", payload)])
        })
    }

    pub fn fetch(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
    ) -> Result<ProfitabilitySnapshot> {
        self.fetch_checked(consent, deadline, || Ok(()))
    }

    pub fn fetch_checked(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<ProfitabilitySnapshot> {
        let result = acquire(
            |method| {
                check()?;
                self.read(method, consent, deadline)
            },
            self.window_days,
        )?;
        check()?;
        self.gate
            .authorize(self.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        crate::remaining(deadline)?;
        Ok(result)
    }
}

fn read_rows(
    read: &mut impl FnMut(ClnMethod) -> Result<Value>,
    method: ClnMethod,
    key: &str,
    keep: &[&str],
) -> Result<Vec<Value>> {
    match read(method) {
        Ok(payload) => Ok(rows(&payload, key).into_iter().map(|row| {
            let mut curated = dict(keep.iter().filter_map(|key| get(row, key).map(|value| (*key, value.clone()))));
            if let Some(Value::Dict(funding)) = get(&curated, "funding") {
                let funding = dict(funding.iter().filter_map(|(key, value)| match key {
                    kassiber_pyjson::Key::Str(key) if ["txid", "funding_txid", "output", "outnum"].iter().any(|s| key.is(s)) => Some((key.as_str().unwrap(), value.clone())),
                    _ => None,
                }));
                if let Value::Dict(fields) = &mut curated {
                    fields.iter_mut().find(|(key, _)| matches!(key, kassiber_pyjson::Key::Str(k) if k.is("funding"))).unwrap().1 = funding;
                }
            }
            curated
        }).collect()),
        Err(error) if !["interaction_required", "network_egress_disabled", "backend_timeout"].contains(&error.code.as_str()) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn acquire(
    mut read: impl FnMut(ClnMethod) -> Result<Value>,
    window: u16,
) -> Result<ProfitabilitySnapshot> {
    let info = read(ClnMethod::GetInfo)?;
    let mut numbers = [None, None];
    for (slot, key) in numbers.iter_mut().zip(["blockheight", "num_peers"]) {
        if matches!(
            field(&info, key),
            Value::Int(_) | Value::Float(_) | Value::Bool(_)
        ) {
            *slot = Some(strict_int(field(&info, key))?);
        }
    }
    // `fetch_core_lightning_snapshot` then `build_node_snapshot`.
    let [block_height, peers] = numbers;
    let text = |key: &str| match field(&info, key) {
        Value::Null => PyStr::default(),
        value => kassiber_pytext::strip_py(&match value {
            Value::Str(text) => text.clone(),
            other => py_str(other).into(),
        }),
    };
    let network = text("network");
    let node = super::NodeInfo {
        alias: text("alias"),
        network: if network.code_points().next().is_none() {
            "mainnet".into()
        } else {
            network
        },
        block_height,
        peer_count: peers.unwrap_or_else(|| 0.into()),
    };
    drop(info);
    let funds = read_rows(
        &mut read,
        ClnMethod::ListFunds,
        "outputs",
        &["status", "reserved_to_block", "amount_msat"],
    )?;
    let channels = read_rows(
        &mut read,
        ClnMethod::ListPeerChannels,
        "channels",
        &[
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
            "fee_base_msat",
            "fee_proportional_millionths",
            "opened_at",
            "closed_at",
            "funding",
            "funding_txid",
            "funding_outnum",
            "funding_outpoint",
        ],
    )?;
    let forwards = read_rows(
        &mut read,
        ClnMethod::ListForwards,
        "forwards",
        &[
            "in_channel",
            "out_channel",
            "fee_msat",
            "in_msat",
            "out_msat",
            "status",
            "received_time",
            "resolved_time",
        ],
    )?;
    let pays = read_rows(
        &mut read,
        ClnMethod::ListPays,
        "pays",
        &["amount_msat", "amount_sent_msat", "status", "rebalance"],
    )?;
    read_rows(&mut read, ClnMethod::ListInvoices, "invoices", &[])?;
    let income = read_rows(
        &mut read,
        ClnMethod::ListIncome,
        "income_events",
        &["tag", "debit_msat", "credit_msat"],
    )?;
    read_rows(&mut read, ClnMethod::ListBalances, "accounts", &[])?;
    read_rows(&mut read, ClnMethod::ListAccountEvents, "events", &[])?;
    let mut snapshot = project(channels, funds, forwards, pays, income, window)?;
    snapshot.node = node;
    Ok(snapshot)
}

fn stripped(value: &Value) -> Option<PyStr> {
    if matches!(value, Value::Null) {
        None
    } else {
        let text = kassiber_pytext::strip_py(&text(value));
        (!text.is("")).then_some(text)
    }
}
fn first<'a>(row: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter()
        .map(|key| field(row, key))
        .find(|v| truthy(v))
        .unwrap_or(&Value::Null)
}
fn invalid_amount() -> AppError {
    AppError::new("app_error", "Invalid Core Lightning msat value")
}
fn strict_int(value: &Value) -> Result<BigInt> {
    match value {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(b) => Ok(u8::from(*b).into()),
        Value::Float(n) if n.is_finite() => integer(value),
        Value::Str(s) => s
            .as_str()
            .and_then(|s| kassiber_pytext::parse_int(s).ok())
            .ok_or_else(|| AppError::new("internal_error", "Invalid Lightning integer field")),
        _ => Err(AppError::new(
            "internal_error",
            "Invalid Lightning integer field",
        )),
    }
}
fn msat(value: &Value) -> Result<BigInt> {
    match value {
        Value::Null | Value::Bool(_) => return Ok(0.into()),
        Value::Int(n) => return Ok(n.clone()),
        Value::Float(n) if n.is_finite() && n.fract() == 0.0 => return integer(value),
        Value::Float(_) => return Err(invalid_amount()),
        Value::Dict(_) => {
            for key in ["msat", "millisatoshis", "millisatoshi"] {
                if let Some(value) = get(value, key) {
                    return msat(value);
                }
            }
            return Ok(0.into());
        }
        _ => {}
    }
    let raw = kassiber_pytext::lower(&py_str(value));
    let raw = kassiber_pytext::strip(&raw).replace(',', "");
    if raw.is_empty() {
        return Ok(0.into());
    }
    for (suffix, shift) in [("msat", 0), ("sat", 3), ("btc", 11)] {
        if let Some(number) = raw.strip_suffix(suffix) {
            let decimal: Decimal = kassiber_pytext::strip(number)
                .parse()
                .map_err(|_| invalid_amount())?;
            let mut coefficient = BigInt::from(decimal.coefficient().clone());
            if decimal.is_sign_negative() {
                coefficient = -coefficient;
            }
            let digits = decimal.coefficient().to_str_radix(10).len();
            let mut exponent = decimal.exponent().saturating_add(shift);
            let discarded =
                (digits.saturating_sub(32) as i64).max((-1_000_030i64).saturating_sub(exponent));
            if discarded > 0 {
                if discarded > digits as i64 + 1 {
                    return Ok(0.into());
                }
                let divisor = BigInt::from(10).pow(discarded as u32);
                let sign = if coefficient < BigInt::from(0) { -1 } else { 1 };
                let absolute = &coefficient * sign;
                let remainder = &absolute % &divisor;
                coefficient = &absolute / &divisor;
                if &remainder * 2 > divisor
                    || (&remainder * 2 == divisor && &coefficient % 2u32 == BigInt::from(1))
                {
                    coefficient += 1;
                }
                coefficient *= sign;
                exponent += discarded;
            }
            if coefficient == BigInt::from(0) {
                return Ok(coefficient);
            }
            if exponent
                .saturating_add(coefficient.to_string().trim_start_matches('-').len() as i64 - 1)
                > 999_999
            {
                return Err(AppError::new(
                    "internal_error",
                    "Core Lightning decimal overflow",
                ));
            }
            if exponent >= 0 {
                return Ok(coefficient * BigInt::from(10).pow(exponent as u32));
            }
            if -exponent > digits as i64 + 1 {
                return Err(invalid_amount());
            }
            let divisor = BigInt::from(10).pow((-exponent) as u32);
            if &coefficient % &divisor != BigInt::from(0) {
                return Err(invalid_amount());
            }
            return Ok(coefficient / divisor);
        }
    }
    kassiber_pytext::parse_int(&raw).map_err(|_| invalid_amount())
}
fn sat(value: BigInt) -> BigInt {
    value / 1000u32
}
fn timestamp(value: &Value) -> Result<()> {
    if matches!(value, Value::Null) || text(value).is("") {
        return Ok(());
    }
    let parsed = match value {
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        _ => kassiber_pytext::parse_float(&py_str(value)),
    };
    if let Ok(n) = parsed {
        if n.is_infinite() || (n.is_finite() && (n < i64::MIN as f64 || n >= i64::MAX as f64)) {
            return Err(AppError::new(
                "internal_error",
                "Lightning timestamp is out of range",
            ));
        }
    }
    Ok(())
}
fn funding_point(row: &Value) -> Option<PyStr> {
    let funding = field(row, "funding");
    if matches!(funding, Value::Dict(_)) {
        let txid = stripped(first(funding, &["txid", "funding_txid"]));
        let output = if truthy(field(funding, "output")) {
            field(funding, "output")
        } else {
            field(funding, "outnum")
        };
        if let Some(txid) = txid.filter(|_| !matches!(output, Value::Null)) {
            let mut points = txid.code_points().collect::<Vec<_>>();
            points.push(u32::from(':'));
            points.extend(text(output).code_points());
            return Some(PyStr::from_code_points(points).expect("valid points"));
        }
    }
    if let Some(txid) = stripped(field(row, "funding_txid"))
        .filter(|_| !matches!(field(row, "funding_outnum"), Value::Null))
    {
        let mut points = txid.code_points().collect::<Vec<_>>();
        points.push(u32::from(':'));
        points.extend(text(field(row, "funding_outnum")).code_points());
        return Some(PyStr::from_code_points(points).expect("valid points"));
    }
    stripped(field(row, "funding_outpoint"))
}
fn is_closed(row: &Value) -> bool {
    let state = stripped(first(row, &["state", "status"])).unwrap_or_default();
    let state = kassiber_pytext::lower(&py_str(&Value::Str(state)));
    if state.contains("breach") {
        return true;
    }
    match state.as_str() {
        "channeld_normal"
        | "channeld_awaiting_lockin"
        | "channeld_shutting_down"
        | "closingd_sigexchange"
        | "fundingd"
        | "openingd"
        | "dualopend_open_init"
        | "dualopend_awaiting_lockin" => false,
        "closingd_complete"
        | "onchaind_their_unilateral"
        | "onchaind_our_unilateral"
        | "closed" => true,
        _ if state.contains("onchain") => true,
        _ if state.contains("shutdown")
            || state.contains("closing")
            || state.contains("await")
            || state.contains("open")
            || state.contains("normal") =>
        {
            false
        }
        _ => state.contains("close"),
    }
}
fn settled(row: &Value) -> bool {
    stripped(field(row, "status")).is_some_and(|s| {
        ["settled", "complete", "completed"]
            .contains(&kassiber_pytext::lower(&py_str(&Value::Str(s))).as_str())
    })
}
fn project(
    channels: Vec<Value>,
    funds: Vec<Value>,
    forwards: Vec<Value>,
    pays: Vec<Value>,
    income: Vec<Value>,
    window: u16,
) -> Result<ProfitabilitySnapshot> {
    let mut aliases = BTreeMap::new();
    for row in &channels {
        if let (Some(peer), Some(alias)) = (
            stripped(field(row, "peer_id")),
            stripped(first(row, &["peer_alias", "alias"])),
        ) {
            aliases.insert(peer, alias);
        }
    }
    let mut output = Vec::new();
    let mut short_ids = Vec::new();
    for row in &channels {
        let peer = stripped(field(row, "peer_id"));
        let private = ["private", "is_private"]
            .iter()
            .find_map(|key| match field(row, key) {
                Value::Bool(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(false);
        let alias = peer
            .as_ref()
            .and_then(|p| aliases.get(p))
            .cloned()
            .unwrap_or_else(|| {
                if private {
                    "private peer".into()
                } else {
                    peer.unwrap_or_default()
                }
            });
        let mut capacity = msat(first(row, &["total_msat", "amount_msat", "funding_msat"]))?;
        let local = msat(first(row, &["to_us_msat", "our_amount_msat"]))?;
        if capacity <= BigInt::from(0) {
            capacity = local + msat(field(row, "their_amount_msat"))?;
        }
        msat(field(row, "fee_base_msat"))?;
        strict_int(if truthy(field(row, "fee_proportional_millionths")) {
            field(row, "fee_proportional_millionths")
        } else {
            &Value::Bool(false)
        })?;
        for key in ["opened_at", "closed_at"] {
            if truthy(field(row, key)) {
                timestamp(field(row, key))?;
            }
        }
        let short = ["short_channel_id", "alias", "channel_id"]
            .iter()
            .find_map(|key| stripped(field(row, key)));
        let point = funding_point(row);
        validate_short_id(short.as_ref())?;
        if let Some(point) = &point {
            validate_outpoint(&Value::Str(point.clone()))?;
        }
        if !is_closed(row) {
            output.push(Channel {
                id: stripped(field(row, "channel_id"))
                    .or_else(|| stripped(field(row, "short_channel_id")))
                    .or(point)
                    .unwrap_or_default(),
                peer_alias: alias,
                capacity_sat: sat(capacity),
                earned_routing_sat: None,
            });
            short_ids.push(short);
        }
    }
    let mut earned = BTreeMap::<PyStr, BigInt>::new();
    for row in forwards.iter().filter(|r| settled(r)) {
        if let Some(out) = stripped(field(row, "out_channel")) {
            *earned.entry(out).or_insert_with(|| 0.into()) += msat(field(row, "fee_msat"))?;
        }
    }
    for (channel, short) in output.iter_mut().zip(short_ids) {
        channel.earned_routing_sat = short.and_then(|id| earned.get(&id).cloned()).map(sat);
    }
    for row in &funds {
        if !stripped(field(row, "status")).is_none_or(|s| s.is("confirmed") || s.is("unconfirmed"))
            || stripped(field(row, "reserved_to_block")).is_some()
        {
            continue;
        }
        msat(field(row, "amount_msat"))?;
    }
    let mut revenue: BigInt = 0.into();
    let mut payment_cost: BigInt = 0.into();
    let mut rebalance_cost: BigInt = 0.into();
    let mut onchain_cost: BigInt = 0.into();
    let mut forward_count = 0;
    let mut payment_count = 0;
    let mut rebalance_count = 0;
    for row in forwards.iter().filter(|r| settled(r)) {
        revenue += msat(field(row, "fee_msat"))?;
        forward_count += 1;
    }
    for row in &pays {
        if !stripped(field(row, "status"))
            .is_some_and(|s| s.is("complete") || s.is("completed") || s.is("paid"))
        {
            continue;
        }
        if truthy(field(row, "rebalance")) {
            rebalance_count += 1;
            continue;
        }
        payment_cost += (msat(field(row, "amount_sent_msat"))? - msat(field(row, "amount_msat"))?)
            .max(0.into());
        payment_count += 1;
    }
    for row in &income {
        let tag = kassiber_pytext::lower(&py_str(&Value::Str(
            stripped(field(row, "tag")).unwrap_or_default(),
        )));
        if tag == "rebalance_fee" {
            rebalance_cost += msat(first(row, &["debit_msat", "credit_msat"]))?;
        } else if ["onchain_fee", "onchain_fees"].contains(&tag.as_str()) {
            onchain_cost += msat(first(row, &["debit_msat", "credit_msat"]))?;
        }
    }
    let present = forward_count != 0
        || payment_count != 0
        || rebalance_count != 0
        || [&revenue, &payment_cost, &rebalance_cost, &onchain_cost]
            .iter()
            .any(|n| **n != BigInt::from(0));
    let routing = present.then(|| Routing {
        window_label: format!("Last {window} days"),
        net_profit_sat: sat(&revenue - &payment_cost - &rebalance_cost - &onchain_cost),
        routing_revenue_sat: sat(revenue),
        payment_cost_sat: sat(payment_cost),
        rebalance_cost_sat: sat(rebalance_cost),
        onchain_cost_sat: sat(onchain_cost),
        forward_count,
        payment_count,
        rebalance_count,
    });
    for row in &forwards {
        timestamp(first(row, &["resolved_time", "received_time"]))?;
        for key in ["in_msat", "out_msat", "fee_msat"] {
            msat(field(row, key))?;
        }
        validate_short_id(stripped(field(row, "in_channel")).as_ref())?;
        validate_short_id(stripped(field(row, "out_channel")).as_ref())?;
    }
    Ok(ProfitabilitySnapshot {
        channels: output,
        routing,
        node: super::NodeInfo::default(),
    })
}

#[cfg(test)]
mod tests;
