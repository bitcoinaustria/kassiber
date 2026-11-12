use super::*;
use kassiber_custody::{codec, components::review};
use provider::Evidence;

#[derive(Clone)]
pub struct MatchOptions {
    pub time_window_seconds: i64,
    /// Python converts the integer bound to a float only at a timed comparison.
    pub time_window_seconds_overflow: bool,
    pub fee_pct_max: f64,
    pub fee_sats_min: i64,
    pub now: String,
    pub include_heuristics: bool,
}
impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            time_window_seconds: 86400,
            time_window_seconds_overflow: false,
            fee_pct_max: 0.01,
            fee_sats_min: 2500,
            now: String::new(),
            include_heuristics: true,
        }
    }
}
/// Parsed observation facts can be reused across all four passes. Pairing never
/// edits observations; each pass still reloads occupancy and projection guards.
pub struct Population {
    pub(super) rows: Vec<Row>,
    cache: std::cell::RefCell<Option<(CacheKey, Vec<Candidate>)>>,
}
pub(super) struct Row {
    pub data: Value,
    pub raw: Value,
    pub id: String,
    pub wallet: String,
    pub direction: String,
    pub asset: String,
    pub wallet_kind: String,
    pub amount: i128,
    pub scope: Option<(String, String, String, String)>,
    pub domain: Option<String>,
    pub seconds: Option<f64>,
    pub hash: Option<String>,
    pub hash_role: &'static str,
    pub provider: Result<Option<Evidence>>,
}
impl Population {
    pub fn new(rows: &[Value]) -> Result<Self> {
        Self::from_rows(rows.to_vec())
    }
    pub(super) fn from_rows(rows: Vec<Value>) -> Result<Self> {
        let mut out = Vec::new();
        let mut config_domains: HashMap<(String, String), Option<String>> = HashMap::new();
        let mut configs: HashMap<String, serde_json::Value> = HashMap::new();
        for data in rows {
            if truthy(&value(&data, "excluded"))
                || !["outbound", "inbound"].contains(&text(&data, "direction").as_str())
            {
                continue;
            }
            let raw = payload(&data);
            // A malformed surfaced txid is an unconditional refusal in the
            // shared interpreter. Rows without any chain identity cannot have
            // a scope either. Avoid cloning their entire payload into serde.
            let surfaced = st(&raw, "txid");
            let malformed =
                !kassiber_pytext::strip(&surfaced).is_empty() && hex_id(&surfaced).is_none();
            let possible = !malformed
                && (hex_id(&st(&data, "external_id")).is_some()
                    || !surfaced.is_empty()
                    || ["tx", "ownership_graph"]
                        .iter()
                        .any(|k| get(&raw, k).and_then(|v| get(v, "txid")).is_some())
                    || matches!(raw, Value::List(_)));
            let config = get(&data, "config_json")
                .filter(|v| truthy(v))
                .or_else(|| get(&data, "wallet_config_json"))
                .unwrap_or(&Value::Null);
            let config_key = match config {
                Value::Str(v) if v.as_str().is_some() => format!("str:{}", v.as_str().unwrap()),
                v => format!(
                    "other:{}",
                    kassiber_pyjson::dumps(v, &kassiber_pyjson::Options::default())
                        .map_err(|e| AppError::new("internal_error", e.to_string()))?
                ),
            };
            let config = configs.entry(config_key.clone()).or_insert_with(|| {
                let encoded = identity_payload(config);
                encoded
                    .as_str()
                    .and_then(|v| serde_json::from_str(v).ok())
                    .unwrap_or(encoded)
            });
            let raw_metadata = ["chain", "network", "bitcoin_network", "chain_network"]
                .iter()
                .any(|k| get(&raw, k).is_some_and(|v| !matches!(v, Value::Null) && *v != s("")));
            let mut identity = serde_json::Map::new();
            if possible || raw_metadata {
                for k in [
                    "id",
                    "external_id",
                    "external_id_kind",
                    "wallet_kind",
                    "asset",
                ] {
                    if let Some(v) = get(&data, k) {
                        identity.insert(k.into(), codec::to_json(v));
                    }
                }
                identity.insert("config_json".into(), config.clone());
                identity.insert("raw_json".into(), identity_payload(&raw));
            }
            let identity = serde_json::Value::Object(identity);
            let scope = possible
                .then(|| review::matching_scope(&identity))
                .flatten();
            let domain = if let Some((chain, network, _, _)) = &scope {
                match (chain.as_str(), network.as_str()) {
                    ("bitcoin", _) => Some(network.clone()),
                    (_, "liquidv1") => Some("main".into()),
                    (_, "liquidtestnet") => Some("test".into()),
                    (_, "elementsregtest") => Some("regtest".into()),
                    _ => None,
                }
            } else if raw_metadata {
                review::matching_domain_fallback(&identity).0
            } else {
                config_domains
                    .entry((st(&data, "asset"), config_key))
                    .or_insert_with(|| {
                        review::matching_domain_fallback(&serde_json::json!({
                            "asset": codec::to_json(get(&data, "asset").unwrap_or(&Value::Null)),
                            "config_json": config,
                        }))
                        .0
                    })
                    .clone()
            };
            let when = st(&data, "occurred_at");
            let mut row = Row {
                data: Value::Null,
                raw,
                id: text(&data, "id"),
                wallet: st(&data, "wallet_id"),
                direction: text(&data, "direction"),
                asset: st(&data, "asset"),
                wallet_kind: st(&data, "wallet_kind"),
                amount: n(&get(&data, "amount")
                    .filter(|v| truthy(v))
                    .cloned()
                    .unwrap_or(num(0)))?,
                scope,
                domain,
                seconds: if kassiber_pytext::strip(&when) == "1970-01-01T00:00:00Z" {
                    None
                } else {
                    instant(&s(&when))
                },
                hash: hex_id(&st(&data, "payment_hash")),
                hash_role: "unknown",
                provider: Ok(None),
            };
            row.data = data;
            row.hash_role = htlc::hash_role(&row);
            row.provider = provider::extract(&row.raw);
            out.push(row);
        }
        Ok(Self {
            rows: out,
            cache: std::cell::RefCell::new(None),
        })
    }
}
/// Dataclass-shaped candidate, preserving Python field order at the oracle seam.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub data: std::sync::Arc<Value>,
}
impl Candidate {
    pub fn into_value(self) -> Value {
        std::sync::Arc::unwrap_or_clone(self.data)
    }
    pub(super) fn set(&mut self, k: &str, v: Value) {
        if get(&self.data, k) != Some(&v) {
            set(std::sync::Arc::make_mut(&mut self.data), k, v);
        }
    }
    pub fn str(&self, k: &str) -> &str {
        if let Some(Value::Str(v)) = get(&self.data, k) {
            v.as_str().unwrap_or("")
        } else {
            ""
        }
    }
    pub fn text(&self, k: &str) -> String {
        text(&self.data, k)
    }
    pub fn exact(&self) -> bool {
        self.text("confidence") == "exact"
    }
    pub fn conflict_size(&self) -> usize {
        n(&value(&self.data, "conflict_size")).unwrap_or(1) as usize
    }
    pub fn fee(&self) -> i128 {
        n(&value(&self.data, "swap_fee_msat")).unwrap_or(0)
    }
    pub fn key(&self) -> String {
        format!("{}->{}", self.text("out_id"), self.text("in_id"))
    }
    pub fn edge(&self) -> (String, String) {
        (self.text("out_id"), self.text("in_id"))
    }
}
fn threshold(amount: i128, pct: f64, floor: i64) -> Result<i128> {
    let pct = (amount.abs() as f64) * pct;
    if !pct.is_finite() {
        return Err(AppError::new(
            "internal_error",
            if pct.is_nan() {
                "cannot convert float NaN to integer"
            } else {
                "cannot convert float infinity to integer"
            },
        ));
    }
    // All persisted amounts are signed-64; the signed-128 calculation retains
    // additions and the truncated binary64 percentage without losing msat.
    Ok((pct.trunc() as i128).max(i128::from(floor) * 1000))
}
pub(super) fn conserves(o: &Row, i: &Row) -> bool {
    o.amount > 0
        && i.amount > 0
        && i.amount <= o.amount
        && o.amount - i.amount <= threshold(o.amount, 0.01, 2500).unwrap_or(0)
}
fn build(
    o: &Row,
    i: &Row,
    exact: bool,
    method: &str,
    kind: Option<&str>,
    e: Option<&Evidence>,
) -> Candidate {
    let fee = if truthy(&value(&o.data, "amount_includes_fee")) {
        0
    } else {
        n(&get(&o.data, "fee")
            .filter(|v| truthy(v))
            .cloned()
            .unwrap_or(num(0)))
        .unwrap_or(0)
        .max(0)
    };
    let oa = asset(&o.asset)
        .map(str::to_owned)
        .unwrap_or_else(|| kassiber_pytext::upper(&o.asset));
    let ia = asset(&i.asset)
        .map(str::to_owned)
        .unwrap_or_else(|| kassiber_pytext::upper(&i.asset));
    let policy = if oa == ia
        || [oa.as_str(), ia.as_str()]
            .iter()
            .all(|v| ["BTC", "LBTC"].contains(v))
    {
        "carrying-value"
    } else {
        "taxable"
    };
    let e = e.cloned().unwrap_or_default();
    let mut data = dict([
        ("out_id", s(&o.id)),
        ("in_id", s(&i.id)),
        ("out_asset", s(&o.asset)),
        ("in_asset", s(&i.asset)),
        ("out_amount_msat", num(o.amount)),
        ("in_amount_msat", num(i.amount)),
        ("out_wallet_id", s(&o.wallet)),
        ("in_wallet_id", s(&i.wallet)),
        ("out_wallet_label", s(st(&o.data, "wallet_label"))),
        ("in_wallet_label", s(st(&i.data, "wallet_label"))),
        ("out_wallet_kind", s(&o.wallet_kind)),
        ("in_wallet_kind", s(&i.wallet_kind)),
        ("out_occurred_at", s(st(&o.data, "occurred_at"))),
        ("in_occurred_at", s(st(&i.data, "occurred_at"))),
        ("confidence", s(if exact { "exact" } else { "strong" })),
        ("method", s(method)),
        ("swap_fee_msat", num(o.amount - i.amount + fee)),
        ("swap_fee_kind", s("combined")),
        (
            "default_kind",
            s(kind.filter(|v| !v.is_empty()).unwrap_or_else(|| {
                default_kind(&o.asset, &i.asset, &o.wallet_kind, &i.wallet_kind)
            })),
        ),
        ("default_policy", s(policy)),
        ("conflict_set_id", s("")),
        ("conflict_size", count(1)),
    ]);
    for (k, v) in [
        ("evidence_provider", e.provider),
        ("evidence_id", e.id),
        ("evidence_kind", e.kind),
        ("evidence_status", e.status),
        ("evidence_version", e.version),
        ("evidence_taproot", e.taproot),
        ("evidence_cooperative", e.cooperative),
        ("evidence_spend_path", e.spend_path),
    ] {
        set(&mut data, k, s(v));
    }
    set(
        &mut data,
        "evidence_conflicts",
        Value::List(
            [
                ("amount", e.amount_conflict),
                ("route", e.route_conflict),
                ("identity", e.identity_conflict),
                ("semantic", e.semantic_conflict),
            ]
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(k, _)| s(k))
            .collect(),
        ),
    );
    set(
        &mut data,
        "evidence_send_amount_msat",
        e.send_amount.map(num).unwrap_or(Value::Null),
    );
    set(
        &mut data,
        "evidence_receive_amount_msat",
        e.receive_amount.map(num).unwrap_or(Value::Null),
    );
    Candidate {
        data: std::sync::Arc::new(data),
    }
}
fn hash_pairs(rows: &[Row], outs: &[usize], ins: &[usize]) -> Vec<(usize, usize, bool)> {
    let mut groups: BTreeMap<&str, BTreeMap<Option<&str>, [Vec<usize>; 2]>> = BTreeMap::new();
    for (side, indices) in [outs, ins].iter().enumerate() {
        for &idx in *indices {
            let r = &rows[idx];
            if let Some(hash) = &r.hash {
                groups
                    .entry(hash)
                    .or_default()
                    .entry(r.domain.as_deref())
                    .or_default()[side]
                    .push(idx);
            }
        }
    }
    let mut pairs = Vec::new();
    for domains in groups.values() {
        let unknown = domains.get(&None).cloned().unwrap_or_default();
        let known: Vec<_> = domains.keys().filter(|v| v.is_some()).copied().collect();
        for domain in if known.is_empty() { vec![None] } else { known } {
            let mut pop = domains.get(&domain).cloned().unwrap_or_default();
            if domain.is_some() {
                for side in 0..2 {
                    pop[side].extend(&unknown[side]);
                }
            }
            if pop[0].len() != 1 || pop[1].len() != 1 {
                continue;
            }
            let (oi, ii) = (pop[0][0], pop[1][0]);
            let (o, i) = (&rows[oi], &rows[ii]);
            if o.wallet == i.wallet {
                continue;
            }
            let roles = BTreeSet::from([o.hash_role, i.hash_role]);
            if roles.contains("unknown") || roles.contains("chain_htlc_unscoped") {
                continue;
            }
            let exact = roles == BTreeSet::from(["lightning_node", "chain_htlc"]);
            let strong = roles.contains("provider_or_import")
                && (roles.len() == 1
                    || roles.contains("lightning_node")
                    || roles.contains("chain_htlc"));
            if (!exact && !strong)
                || !compatible(&o.domain, &i.domain)
                || (exact && (o.domain.is_none() || i.domain.is_none()))
                || !conserves(o, i)
            {
                continue;
            }
            pairs.push((oi, ii, exact));
        }
    }
    pairs
}
#[derive(PartialEq, Eq)]
struct CacheKey {
    paired: HashSet<String>,
    dismissed: HashSet<(String, String)>,
    booked: HashSet<String>,
    window: i64,
    window_overflow: bool,
    pct: u64,
    floor: i64,
    heuristics: bool,
}
/// `_active_paired_ids`: both legs of every pair that is not deleted.
fn active_paired_ids(pairs: &[Value]) -> HashSet<String> {
    pairs
        .iter()
        .filter(|r| !truthy(&value(r, "deleted_at")))
        .flat_map(|r| [st(r, "out_transaction_id"), st(r, "in_transaction_id")])
        .filter(|v| !v.is_empty())
        .collect()
}
/// `_seconds_or_now(now_iso)`.
fn now_seconds(now: &str) -> Option<f64> {
    if now.is_empty() {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs_f64())
    } else {
        instant(&s(now))
    }
}
/// `dismissal_is_active`: an expiry at or before `now` lifts the veto; a
/// missing or unreadable expiry never does.
pub fn dismissal_is_active(expires_at: &Value, now: &str) -> bool {
    !instant(expires_at)
        .zip(now_seconds(now))
        .is_some_and(|(expiry, now)| expiry <= now)
}
/// `_active_dismissals`: the (out, in) pairs still vetoed at `now`.
fn active_dismissals(dismissals: &[Value], now: &str) -> HashSet<(String, String)> {
    let now = now_seconds(now);
    dismissals
        .iter()
        .filter(|r| {
            !instant(&value(r, "expires_at"))
                .zip(now)
                .is_some_and(|(expiry, now)| expiry <= now)
        })
        .map(|r| (st(r, "out_transaction_id"), st(r, "in_transaction_id")))
        .collect()
}
/// `candidate_exclusion_counts`: why an expected candidate can be absent,
/// counted over the whole profile before any filter. Legs held by an active
/// pair, other legs a current stored move already books, and unexpired
/// dismissals whose two legs are otherwise still open (pairs, not legs).
pub fn exclusion_counts(
    pop: &Population,
    pairs: &[Value],
    dismissals: &[Value],
    booked: &HashSet<String>,
    now: &str,
) -> Value {
    let paired = active_paired_ids(pairs);
    let legs: HashSet<&str> = pop.rows.iter().map(|r| r.id.as_str()).collect();
    let unpaired: HashSet<&str> = legs
        .iter()
        .copied()
        .filter(|id| !paired.contains(*id))
        .collect();
    let open: HashSet<&str> = unpaired
        .iter()
        .copied()
        .filter(|id| !booked.contains(*id))
        .collect();
    let count = |n: usize| Value::Int(n.into());
    kassiber_envelope::dict([
        (
            "excluded_by_active_pair",
            count(legs.iter().filter(|id| paired.contains(**id)).count()),
        ),
        (
            "excluded_already_journaled",
            count(unpaired.iter().filter(|id| booked.contains(**id)).count()),
        ),
        (
            "excluded_by_dismissal",
            count(
                active_dismissals(dismissals, now)
                    .iter()
                    .filter(|(out, inbound)| {
                        open.contains(out.as_str()) && open.contains(inbound.as_str())
                    })
                    .count(),
            ),
        ),
    ])
}
/// Match against full evidence populations; occupancy only removes emitted edges.
pub fn suggest(
    pop: &Population,
    pairs: &[Value],
    dismissals: &[Value],
    booked: &HashSet<String>,
    options: &MatchOptions,
) -> Result<Vec<Candidate>> {
    let rows = &pop.rows;
    let paired = active_paired_ids(pairs);
    let dismissed = active_dismissals(dismissals, &options.now);
    let key = CacheKey {
        paired: paired.clone(),
        dismissed: dismissed.clone(),
        booked: booked.clone(),
        window: options.time_window_seconds,
        window_overflow: options.time_window_seconds_overflow,
        pct: options.fee_pct_max.to_bits(),
        floor: options.fee_sats_min,
        heuristics: options.include_heuristics,
    };
    if let Some((previous, candidates)) = pop.cache.borrow().as_ref() {
        if *previous == key {
            return Ok(candidates.clone());
        }
    }
    let mut outs = Vec::new();
    let mut ins = Vec::new();
    for (idx, r) in rows.iter().enumerate() {
        if !booked.contains(&r.id) {
            if r.direction == "outbound" {
                outs.push(idx)
            } else {
                ins.push(idx)
            }
        }
    }
    let eligible = |o: usize, i: usize| {
        !paired.contains(&rows[o].id)
            && !paired.contains(&rows[i].id)
            && (dismissed.is_empty()
                || !dismissed.contains(&(rows[o].id.clone(), rows[i].id.clone())))
    };
    let mut raw = Vec::new();
    for (o, i, exact) in hash_pairs(rows, &outs, &ins) {
        if eligible(o, i) {
            raw.push(build(&rows[o], &rows[i], exact, "payment_hash", None, None));
        }
    }
    for (o, i, e, exact) in provider::pairs(rows, &outs, &ins)? {
        if eligible(o, i) {
            raw.push(build(
                &rows[o],
                &rows[i],
                exact,
                "provider_swap_id",
                Some(&e.kind),
                Some(&e),
            ));
        }
    }
    for (o, i, e) in htlc::refund_pairs(rows, &outs, &ins) {
        if eligible(o, i) {
            raw.push(build(
                &rows[o],
                &rows[i],
                e.version == "outpoint_exact",
                "htlc_refund",
                Some("swap-refund"),
                Some(&e),
            ));
        }
    }
    let exact_outs: HashSet<_> = raw
        .iter()
        .filter(|c| c.exact())
        .map(|c| c.text("out_id"))
        .collect();
    let exact_ins: HashSet<_> = raw
        .iter()
        .filter(|c| c.exact())
        .map(|c| c.text("in_id"))
        .collect();
    if options.include_heuristics {
        let mut entries: Vec<_> = ins
            .iter()
            .copied()
            .filter(|&idx| {
                !paired.contains(&rows[idx].id)
                    && !exact_ins.contains(&rows[idx].id)
                    && rows[idx].amount > 0
                    && rows[idx].seconds.is_some()
            })
            .collect();
        entries.sort_by(|&a, &b| rows[a].seconds.partial_cmp(&rows[b].seconds).unwrap());
        for &oi in &outs {
            let o = &rows[oi];
            if paired.contains(&o.id) || exact_outs.contains(&o.id) || o.amount <= 0 {
                continue;
            }
            let Some(t) = o.seconds else { continue };
            let threshold = threshold(o.amount, options.fee_pct_max, options.fee_sats_min)?;
            if options.time_window_seconds_overflow {
                return Err(AppError::new(
                    "internal_error",
                    "int too large to convert to float",
                ));
            }
            let lo = entries.partition_point(|&i| {
                rows[i].seconds.unwrap() < t - options.time_window_seconds as f64
            });
            let hi = entries.partition_point(|&i| {
                rows[i].seconds.unwrap() <= t + options.time_window_seconds as f64
            });
            for &ii in entries.get(lo..hi).unwrap_or_default() {
                let i = &rows[ii];
                if o.wallet == i.wallet || !compatible(&o.domain, &i.domain) {
                    continue;
                }
                let delta = o.amount - i.amount;
                if delta < 0 || delta > threshold {
                    continue;
                }
                let same = match (asset(&o.asset), asset(&i.asset)) {
                    (Some(o), Some(i)) => o == i,
                    _ => kassiber_pytext::upper(&o.asset) == kassiber_pytext::upper(&i.asset),
                };
                if same
                    && o.scope
                        .as_ref()
                        .zip(i.scope.as_ref())
                        .is_some_and(|(o, i)| o.2 != i.2)
                {
                    continue;
                }
                if !same
                    && default_kind(&o.asset, &i.asset, &o.wallet_kind, &i.wallet_kind) == "manual"
                {
                    continue;
                }
                if eligible(oi, ii) {
                    raw.push(build(o, i, false, "heuristic", None, None));
                }
            }
        }
    }
    let mut result = finalize(raw);
    sort(&mut result, false);
    *pop.cache.borrow_mut() = Some((key, result.clone()));
    Ok(result)
}
pub(super) fn sort(candidates: &mut [Candidate], ownership: bool) {
    candidates.sort_by_cached_key(|c| {
        (
            !c.exact(),
            ownership && c.text("method") != "ownership_graph",
            c.fee().abs(),
            c.text("out_occurred_at"),
            c.text("out_id"),
            c.text("in_id"),
        )
    });
}
fn rank(c: &Candidate) -> (bool, u8) {
    (
        !c.exact(),
        match c.text("method").as_str() {
            "htlc_refund" => 0,
            "payment_hash" => 1,
            "provider_swap_id" => 2,
            "ownership_graph" => 3,
            "heuristic" => 4,
            _ => 99,
        },
    )
}
/// Conflict cardinality is computed before any view or mutation filter.
pub fn finalize(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut edges = Vec::<Candidate>::new();
    let mut indices = HashMap::new();
    for c in candidates {
        let key = c.edge();
        if let Some(&idx) = indices.get(&key) {
            if rank(&c) < rank(&edges[idx]) {
                edges[idx] = c;
            }
        } else {
            indices.insert(key, edges.len());
            edges.push(c);
        }
    }
    let exact_legs: HashSet<_> = edges
        .iter()
        .filter(|c| c.exact())
        .flat_map(|c| [c.text("out_id"), c.text("in_id")])
        .collect();
    let mut surviving: Vec<_> = edges.iter().filter(|c| c.exact()).cloned().collect();
    surviving.extend(edges.into_iter().filter(|c| {
        !c.exact()
            && (c.text("method") != "heuristic"
                || (!exact_legs.contains(&c.text("out_id"))
                    && !exact_legs.contains(&c.text("in_id"))))
    }));
    let mut parent: Vec<usize> = (0..surviving.len()).collect();
    fn root(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut legs = HashMap::new();
    for (idx, c) in surviving.iter().enumerate() {
        for leg in [c.text("out_id"), c.text("in_id")] {
            if let Some(&other) = legs.get(&leg) {
                let a = root(&mut parent, idx);
                let b = root(&mut parent, other);
                parent[a] = b;
            } else {
                legs.insert(leg, idx);
            }
        }
    }
    let mut groups: HashMap<usize, (String, usize)> = HashMap::new();
    for (idx, c) in surviving.iter().enumerate() {
        let key = c.key();
        let group = groups
            .entry(root(&mut parent, idx))
            .or_insert((key.clone(), 0));
        group.0 = group.0.clone().min(key);
        group.1 += 1;
    }
    for (idx, c) in surviving.iter_mut().enumerate() {
        let (key, size) = &groups[&root(&mut parent, idx)];
        c.set("conflict_set_id", s(key));
        c.set("conflict_size", count(*size));
    }
    surviving
}
