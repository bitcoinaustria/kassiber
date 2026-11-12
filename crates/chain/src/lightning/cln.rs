use super::*;
use kassiber_net::{ClnConfig, ClnMethod, ClnProcessError};
use kassiber_pyjson::Decimal;
use std::time::Duration;

pub struct ClnReportClient {
    gate: Gate,
    request: Request,
    timeout: Duration,
    window_days: u16,
    /// A fixed `now_iso()` in UNIX seconds; only harness builds pin it.
    clock: Option<u64>,
}

impl ClnReportClient {
    pub fn new(config: &Json, gate: Gate, action: ReportAction) -> Result<Self> {
        let window_days = action.window_days;
        Self::build(config, gate, |config| action.binding(config), window_days)
    }

    /// A client for one wallet sync (`sync_core_lightning_wallet`), its grant
    /// bound to the book, profile, wallet and backend configuration.
    pub fn for_wallet_sync(config: &Json, gate: Gate, action: &super::SyncAction) -> Result<Self> {
        Self::build(config, gate, |config| action.binding(config), 1)
    }

    fn build(
        config: &Json,
        gate: Gate,
        binding: impl FnOnce(&Json) -> Result<[u8; 32]>,
        window_days: u16,
    ) -> Result<Self> {
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
        // lightning-cli takes a commando rune only on its command line
        // (`--commando=peerid:rune`), where any local user can read it from
        // the process list, and documents no config-file, environment or
        // stdin form. With no secret-free channel, commando is refused.
        let wants_commando = value(&["commando_peer_id"]).is_some()
            || value(&["token"]).is_some()
            || value(&["url"])
                .is_some_and(|s| kassiber_pytext::lower(&s).starts_with("cln://commando"));
        if wants_commando {
            return Err(AppError::new(
                "validation",
                "Core Lightning commando connections are not supported",
            )
            .with_hint("lightning-cli accepts a commando rune only on its command line, where other local users can read it. Clear the rune and commando peer and use a local rpc_file, or set lightning_cli to a wrapper you control."));
        }
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
            },
            binding(&config)?,
        );
        Ok(Self {
            gate,
            request,
            timeout: Duration::from_secs(timeout),
            window_days,
            clock: None,
        })
    }

    /// Ends the routing window at `now` instead of the system clock, as
    /// reference vectors frozen at one instant need. Harness builds only.
    #[cfg(any(test, feature = "test-harness"))]
    pub fn pin_clock(mut self, now: u64) -> Self {
        self.clock = Some(now);
        self
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
        check: impl FnMut() -> Result<()>,
    ) -> Result<ProfitabilitySnapshot> {
        self.snapshot_checked(consent, deadline, check)
            .map(NodeSnapshot::profitability)
    }

    /// `CoreLightningAdapter.fetch_node_snapshot` over the curated reads.
    pub fn snapshot_checked(
        &self,
        consent: Option<&Consent>,
        deadline: Instant,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<NodeSnapshot> {
        // `now_iso()`: whole UNIX seconds.
        let now = self.clock.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs())
        });
        let result = acquire(
            |method| {
                check()?;
                self.read(method, consent, deadline)
            },
            self.window_days,
            now,
        )?;
        check()?;
        self.gate
            .authorize(self.request.clone(), consent.cloned())
            .map_err(|e| e.app_error())?;
        crate::remaining(deadline)?;
        Ok(result)
    }
}

/// An optional resource that failed is recorded in `failed`, so its absence
/// is never mistaken for a complete empty response.
fn read_rows(
    read: &mut impl FnMut(ClnMethod) -> Result<Value>,
    failed: &mut Vec<ClnMethod>,
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
        Err(error) if !["interaction_required", "network_egress_disabled", "backend_timeout"].contains(&error.code.as_str()) => {
            failed.push(method);
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

fn acquire(
    mut read: impl FnMut(ClnMethod) -> Result<Value>,
    window: u16,
    now: u64,
) -> Result<NodeSnapshot> {
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
    let node = Info {
        alias: text("alias"),
        pubkey: text("id"),
        network: if network.code_points().next().is_none() {
            "mainnet".into()
        } else {
            network
        },
        version: stripped(field(&info, "version")),
        block_height,
        peer_count: peers.unwrap_or_else(|| 0.into()),
    };
    drop(info);
    let mut failed = Vec::new();
    let funds = read_rows(
        &mut read,
        &mut failed,
        ClnMethod::ListFunds,
        "outputs",
        &["status", "reserved_to_block", "amount_msat"],
    )?;
    let channels = read_rows(
        &mut read,
        &mut failed,
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
        ],
    )?;
    let forwards = read_rows(
        &mut read,
        &mut failed,
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
        &mut failed,
        ClnMethod::ListPays,
        "pays",
        &[
            "amount_msat",
            "amount_sent_msat",
            "status",
            "rebalance",
            "completed_at",
            "created_at",
        ],
    )?;
    let invoices = read_rows(
        &mut read,
        &mut failed,
        ClnMethod::ListInvoices,
        "invoices",
        &["status"],
    )?;
    let income = read_rows(
        &mut read,
        &mut failed,
        ClnMethod::ListIncome,
        "income_events",
        &["tag", "debit_msat", "credit_msat", "timestamp"],
    )?;
    read_rows(
        &mut read,
        &mut failed,
        ClnMethod::ListBalances,
        "accounts",
        &[],
    )?;
    read_rows(
        &mut read,
        &mut failed,
        ClnMethod::ListAccountEvents,
        "events",
        &[],
    )?;
    // A failed routing or channel resource leaves the window unknown; a
    // complete empty response remains a known zero window.
    let routing_known = !failed.iter().any(|method| {
        matches!(
            method,
            ClnMethod::ListPeerChannels
                | ClnMethod::ListForwards
                | ClnMethod::ListPays
                | ClnMethod::ListIncome
        )
    });
    project(
        node,
        Rows {
            channels,
            funds,
            forwards,
            pays,
            invoices,
            income,
        },
        Window::new(window, now),
        routing_known,
    )
}

/// The node fields `fetch_core_lightning_snapshot` keeps from `getinfo`.
struct Info {
    alias: PyStr,
    pubkey: PyStr,
    network: PyStr,
    version: Option<PyStr>,
    block_height: Option<BigInt>,
    peer_count: BigInt,
}

/// `CoreLightningSnapshot`'s curated collections the snapshot reads.
struct Rows {
    channels: Vec<Value>,
    funds: Vec<Value>,
    forwards: Vec<Value>,
    pays: Vec<Value>,
    invoices: Vec<Value>,
    income: Vec<Value>,
}

fn stripped(value: &Value) -> Option<PyStr> {
    if matches!(value, Value::Null) {
        None
    } else {
        let text = kassiber_pytext::strip_py(&text(value));
        (!text.is("")).then_some(text)
    }
}
/// Python's `row.get(a) or row.get(b) ...`: the last operand when none is truthy.
fn first<'a>(row: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter()
        .map(|key| field(row, key))
        .find(|v| truthy(v))
        .unwrap_or_else(|| field(row, keys[keys.len() - 1]))
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
fn funding_point(row: &Value) -> Option<PyStr> {
    let funding = field(row, "funding");
    if matches!(funding, Value::Dict(_)) {
        let txid = stripped(first(funding, &["txid", "funding_txid"]));
        // Output zero is a valid funding outpoint; only an absent value falls back.
        let output = if matches!(field(funding, "output"), Value::Null) {
            field(funding, "outnum")
        } else {
            field(funding, "output")
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
/// CLN `_timestamp`: unknown for falsy, unparsable or out-of-range years;
/// an error where CPython's conversion raises outside its handler.
fn timestamp(value: &Value) -> Result<String> {
    let unknown = || Ok(UNKNOWN_OCCURRED_AT.to_owned());
    let seconds = match value {
        Value::Null | Value::Bool(false) => return unknown(),
        Value::Int(n) if *n == BigInt::from(0) => return unknown(),
        Value::Float(n) if *n == 0.0 => return unknown(),
        Value::Str(s) if s.is("") || s.is("0") => return unknown(),
        Value::Bool(true) => 1.0,
        Value::Int(n) => n.to_string().parse::<f64>().unwrap_or(f64::INFINITY),
        Value::Float(n) => *n,
        Value::Str(s) => match s.as_str().map(kassiber_pytext::parse_float) {
            Some(Ok(n)) => n,
            _ => return unknown(),
        },
        _ => return unknown(),
    };
    if seconds.is_nan() {
        return unknown();
    }
    let seconds = seconds.trunc();
    // `time_t` is 64-bit: CPython's conversion overflows outside [-2^63, 2^63).
    let limit = 2f64.powi(63);
    if !(-limit..limit).contains(&seconds) {
        return Err(AppError::new(
            "internal_error",
            "Lightning timestamp is out of range",
        ));
    }
    match utc_iso(seconds as i64)? {
        Some(iso) => Ok(iso),
        None => unknown(),
    }
}
/// `str.lower()` for state matching; a lone surrogate can never match.
fn lower(value: &PyStr) -> String {
    kassiber_pytext::lower(
        &value
            .code_points()
            .map(|p| char::from_u32(p).unwrap_or('\u{fffd}'))
            .collect::<String>(),
    )
}
/// `_coerce_channel_state`.
fn channel_state(raw: &Value, connected: &Value) -> &'static str {
    let state = stripped(raw).map(|s| lower(&s)).unwrap_or_default();
    if state.is_empty() {
        return "inactive";
    }
    if state.contains("breach") {
        return "force_closed";
    }
    match state.as_str() {
        "channeld_normal" => "active",
        "channeld_awaiting_lockin"
        | "fundingd"
        | "openingd"
        | "dualopend_open_init"
        | "dualopend_awaiting_lockin" => "pending_open",
        "channeld_shutting_down" | "closingd_sigexchange" => "pending_close",
        "closingd_complete" | "closed" => "closed",
        "onchaind_their_unilateral" | "onchaind_our_unilateral" => "force_closed",
        _ if state.contains("onchain") => "force_closed",
        _ if state.contains("shutdown") || state.contains("closing") => "pending_close",
        _ if state.contains("await") || state.contains("open") => "pending_open",
        _ if state.contains("normal") && *connected == Value::Bool(false) => "inactive",
        _ if state.contains("normal") => "active",
        _ if state.contains("close") => "closed",
        _ => "inactive",
    }
}
fn settled(row: &Value) -> bool {
    forward_status(field(row, "status")) == "settled"
}
/// `_forward_status`.
fn forward_status(value: &Value) -> &'static str {
    match stripped(value).map(|s| lower(&s)).as_deref() {
        Some("settled" | "complete" | "completed") => "settled",
        Some("offered" | "pending") => "offered",
        _ => "failed",
    }
}
/// `_stable_hash`: SHA-256 over the compact, key-sorted JSON of the parts.
fn stable_hash(parts: Vec<Value>) -> Result<PyStr> {
    let options = kassiber_pyjson::Options {
        sort_keys: true,
        ..Default::default()
    }
    .separators(",", ":");
    let encoded = kassiber_pyjson::dumps(&Value::List(parts), &options)
        .map_err(|_| AppError::new("internal_error", "Invalid Core Lightning forward"))?;
    Ok(Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .into())
}
fn optional_value(value: Option<PyStr>) -> Value {
    value.map(Value::Str).unwrap_or(Value::Null)
}
/// The requested routing window, `[now - days, now]` in whole UNIX seconds,
/// as Python's `_routing_window` bounds it from `now_iso()`.
struct Window {
    days: u16,
    start: Decimal,
    end: Decimal,
    undated: bool,
}
impl Window {
    fn new(days: u16, now: u64) -> Self {
        let end = now.to_string().parse().expect("integer seconds");
        let start = (i128::from(now) - i128::from(days) * 86_400)
            .to_string()
            .parse()
            .expect("integer seconds");
        Self {
            days,
            start,
            end,
            undated: false,
        }
    }
    /// The first present timestamp field must be a finite decimal number of
    /// seconds; otherwise the row is excluded and the window is unknown.
    fn inside(&mut self, row: &Value, fields: &[&str]) -> bool {
        let stamp = fields
            .iter()
            .map(|key| field(row, key))
            .find(|value| !matches!(value, Value::Null))
            .unwrap_or(&Value::Null);
        // Decimal(str(stamp)) strips whitespace; the parser, like Python's,
        // ignores underscores. A surrogate can never be a number.
        let text = kassiber_pytext::strip_py(&py_str(stamp).into());
        match text.as_str().unwrap_or("\u{0}").parse::<Decimal>() {
            Ok(value) => self.start <= value && value <= self.end,
            Err(_) => {
                self.undated = true;
                false
            }
        }
    }
}

/// `build_node_snapshot`, in the reference's evaluation order. Routing totals
/// and per-channel earnings read only the facts inside `window`
/// (`_routing_window`); the forward list keeps every forward.
fn project(
    info: Info,
    rows: Rows,
    mut window: Window,
    routing_known: bool,
) -> Result<NodeSnapshot> {
    let mut aliases = BTreeMap::new();
    for row in &rows.channels {
        if let (Some(peer), Some(alias)) = (
            stripped(field(row, "peer_id")),
            stripped(first(row, &["peer_alias", "alias"])),
        ) {
            aliases.insert(peer, alias);
        }
    }
    let mut channels = Vec::new();
    for row in &rows.channels {
        let private = ["private", "is_private"]
            .iter()
            .find_map(|key| match field(row, key) {
                Value::Bool(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(false);
        let peer = stripped(field(row, "peer_id"));
        let alias = peer
            .as_ref()
            .and_then(|p| aliases.get(p))
            .cloned()
            .unwrap_or_else(|| {
                if private {
                    "private peer".into()
                } else {
                    peer.clone().unwrap_or_default()
                }
            });
        let mut capacity = msat(first(row, &["total_msat", "amount_msat", "funding_msat"]))?;
        let local = msat(first(row, &["to_us_msat", "our_amount_msat"]))?;
        if capacity <= BigInt::from(0) {
            capacity = &local + msat(field(row, "their_amount_msat"))?;
        }
        let remote = (&capacity - &local).max(0.into());
        let state_raw = first(row, &["state", "status"]);
        let state = channel_state(state_raw, field(row, "peer_connected"));
        let point = funding_point(row);
        let id = stripped(field(row, "channel_id"))
            .or_else(|| stripped(field(row, "short_channel_id")))
            .or_else(|| point.clone())
            .unwrap_or_default();
        let short = ["short_channel_id", "alias", "channel_id"]
            .iter()
            .find_map(|key| stripped(field(row, key)));
        let base_fee = msat(field(row, "fee_base_msat"))?;
        let ppm = strict_int(if truthy(field(row, "fee_proportional_millionths")) {
            field(row, "fee_proportional_millionths")
        } else {
            &Value::Bool(false)
        })?;
        let mut stamps = Vec::new();
        for key in ["opened_at", "closed_at"] {
            stamps.push(if truthy(field(row, key)) {
                Some(timestamp(field(row, key))?)
            } else {
                None
            });
        }
        let close_kind = if stripped(state_raw).is_some_and(|s| lower(&s).contains("breach")) {
            Some("breach")
        } else if state == "force_closed" {
            Some("force")
        } else if state == "closed" {
            Some("cooperative")
        } else {
            None
        };
        validate_short_id(short.as_ref())?;
        if let Some(point) = &point {
            validate_outpoint(&Value::Str(point.clone()))?;
        }
        let nonzero = |n: BigInt| (n != BigInt::from(0)).then_some(n);
        let closed_at = stamps.pop().flatten();
        let opened_at = stamps.pop().flatten();
        channels.push(NodeChannel {
            id,
            peer_alias: alias,
            peer_pubkey: if private {
                Value::Null
            } else {
                optional_value(peer)
            },
            capacity_sat: sat(capacity),
            local_balance_sat: sat(local),
            remote_balance_sat: sat(remote),
            state,
            is_private: private,
            is_initiator: field(row, "opener") == &Value::from("local")
                || field(row, "funder") == &Value::from("LOCAL"),
            short_channel_id: short,
            funding_outpoint: point,
            base_fee_msat: nonzero(base_fee),
            fee_rate_ppm: nonzero(ppm),
            opened_at,
            closed_at,
            close_kind,
            forward_count: None,
            earned_routing_sat: None,
            htlc_count: None,
        });
    }
    // Routing facts outside the requested window never reach the totals.
    let windowed: Vec<&Value> = rows
        .forwards
        .iter()
        .filter(|r| settled(r) && window.inside(r, &["resolved_time", "received_time"]))
        .collect();
    let windowed_pays: Vec<&Value> = rows
        .pays
        .iter()
        .filter(|r| {
            stripped(field(r, "status"))
                .is_some_and(|s| s.is("complete") || s.is("completed") || s.is("paid"))
                && window.inside(r, &["completed_at", "created_at"])
        })
        .collect();
    let windowed_income: Vec<&Value> = rows
        .income
        .iter()
        .filter(|r| {
            let tag = stripped(field(r, "tag"))
                .map(|s| lower(&s))
                .unwrap_or_default();
            ["rebalance_fee", "onchain_fee", "onchain_fees"].contains(&tag.as_str())
                && window.inside(r, &["timestamp"])
        })
        .collect();
    let mut earned = BTreeMap::<PyStr, BigInt>::new();
    for row in &windowed {
        if let Some(out) = stripped(field(row, "out_channel")) {
            *earned.entry(out).or_insert_with(|| 0.into()) += msat(field(row, "fee_msat"))?;
        }
    }
    for channel in &mut channels {
        if let Some(fees) = channel
            .short_channel_id
            .as_ref()
            .and_then(|id| earned.get(id))
        {
            channel.earned_routing_sat = Some(sat(fees.clone()));
        }
    }
    let (closed, open): (Vec<_>, Vec<_>) = channels
        .into_iter()
        .partition(|c| CLOSED_STATES.contains(&c.state));
    let mut onchain: BigInt = 0.into();
    for row in &rows.funds {
        if !stripped(field(row, "status")).is_none_or(|s| s.is("confirmed") || s.is("unconfirmed"))
            || stripped(field(row, "reserved_to_block")).is_some()
        {
            continue;
        }
        onchain += msat(field(row, "amount_msat"))?;
    }
    let mut revenue: BigInt = 0.into();
    let mut payment_cost: BigInt = 0.into();
    let mut rebalance_cost: BigInt = 0.into();
    let mut onchain_cost: BigInt = 0.into();
    let mut forward_count = 0;
    let mut payment_count = 0;
    let mut rebalance_count = 0;
    // `_routing_summary`: no summary over an unavailable resource or an
    // undated relevant fact.
    let routing_known = routing_known && !window.undated;
    for row in windowed.iter().filter(|_| routing_known) {
        revenue += msat(field(row, "fee_msat"))?;
        forward_count += 1;
    }
    for row in windowed_pays.iter().filter(|_| routing_known) {
        if truthy(field(row, "rebalance")) {
            rebalance_count += 1;
            continue;
        }
        payment_cost += (msat(field(row, "amount_sent_msat"))? - msat(field(row, "amount_msat"))?)
            .max(0.into());
        payment_count += 1;
    }
    for row in windowed_income.iter().filter(|_| routing_known) {
        let tag = stripped(field(row, "tag"))
            .map(|s| lower(&s))
            .unwrap_or_default();
        if tag == "rebalance_fee" {
            rebalance_cost += msat(first(row, &["debit_msat", "credit_msat"]))?;
        } else if ["onchain_fee", "onchain_fees"].contains(&tag.as_str()) {
            onchain_cost += msat(first(row, &["debit_msat", "credit_msat"]))?;
        }
    }
    let routing = routing_known.then(|| Routing {
        window_label: format!("Last {} days", window.days),
        net_profit_sat: sat(&revenue - &payment_cost - &rebalance_cost - &onchain_cost),
        routing_revenue_sat: sat(revenue),
        payment_cost_sat: sat(payment_cost),
        rebalance_cost_sat: sat(rebalance_cost),
        onchain_cost_sat: sat(onchain_cost),
        forward_count,
        payment_count,
        rebalance_count,
    });
    let mut forwards = Vec::new();
    for row in &rows.forwards {
        let in_channel = stripped(field(row, "in_channel"));
        let out_channel = stripped(field(row, "out_channel"));
        let id = stable_hash(vec![
            Value::from("fw"),
            optional_value(in_channel.clone()),
            optional_value(out_channel.clone()),
            field(row, "received_time").clone(),
            field(row, "resolved_time").clone(),
            field(row, "in_msat").clone(),
            field(row, "out_msat").clone(),
            field(row, "status").clone(),
        ])?;
        let occurred_at = timestamp(first(row, &["resolved_time", "received_time"]))?;
        let alias = |channel: &Option<PyStr>| {
            let channel = channel.clone().unwrap_or_default();
            aliases.get(&channel).cloned().unwrap_or(channel)
        };
        let forward = NodeForward {
            id,
            occurred_at,
            in_peer_alias: alias(&in_channel),
            out_peer_alias: alias(&out_channel),
            amount_in_msat: msat(field(row, "in_msat"))?,
            amount_out_msat: msat(field(row, "out_msat"))?,
            fee_msat: msat(field(row, "fee_msat"))?,
            status: forward_status(field(row, "status")),
            in_short_channel_id: in_channel,
            out_short_channel_id: out_channel,
        };
        validate_short_id(forward.in_short_channel_id.as_ref())?;
        validate_short_id(forward.out_short_channel_id.as_ref())?;
        forwards.push(forward);
    }
    let statuses = |rows: &[Value]| -> Vec<String> {
        rows.iter()
            .map(|row| {
                stripped(field(row, "status"))
                    .map(|s| lower(&s))
                    .unwrap_or_default()
            })
            .collect()
    };
    let invoices = statuses(&rows.invoices);
    let pays = statuses(&rows.pays);
    let tally = |values: &[String], keys: &[&str]| {
        values.iter().filter(|v| keys.contains(&v.as_str())).count()
    };
    Ok(NodeSnapshot {
        alias: info.alias,
        pubkey: info.pubkey,
        network: info.network,
        implementation_version: info.version,
        peer_count: info.peer_count,
        block_height: info.block_height,
        invoice_count: invoices.len(),
        paid_invoice_count: tally(&invoices, &["paid"]),
        expired_invoice_count: tally(&invoices, &["expired"]),
        payment_count: pays.len(),
        completed_payment_count: tally(&pays, &["complete", "completed", "paid"]),
        failed_payment_count: tally(&pays, &["failed", "error"]),
        onchain_balance_sat: sat(onchain),
        total_local_balance_sat: open.iter().map(|c| &c.local_balance_sat).sum(),
        total_remote_balance_sat: open.iter().map(|c| &c.remote_balance_sat).sum(),
        total_capacity_sat: open.iter().map(|c| &c.capacity_sat).sum(),
        channels: open,
        closed_channels: closed,
        routing,
        forwards,
    })
}
mod wallet;
pub use wallet::{wallet_records, WalletSnapshot};

#[cfg(test)]
mod tests;
