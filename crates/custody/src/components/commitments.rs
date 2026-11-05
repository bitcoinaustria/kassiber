//! Author commitments use canonical evidence without joined observer authority,
//! exactly as the historical SQL reads do. Lifecycle changes are not verdicts.
use super::*;
use crate::evidence;
use kassiber_pyjson::{Key, Value as Py};
use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

struct WithoutProvenance<'a, R>(&'a R);
impl<R: evidence::RowRecord> evidence::RowRecord for WithoutProvenance<'_, R> {
    fn field(&self, key: &str) -> Option<&Value> {
        if key.starts_with("observation_") {
            None
        } else {
            self.0.field(key)
        }
    }

    fn fields(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0
            .fields()
            .filter(|(key, _)| !key.starts_with("observation_"))
    }
}
fn order(row: &impl evidence::RowRecord) -> (String, String, String) {
    (
        pystr(row.field("occurred_at").expect("no entry found for key")),
        pystr(row.field("created_at").expect("no entry found for key")),
        pystr(row.field("id").expect("no entry found for key")),
    )
}
/// `order`, borrowing the row's strings.
fn order_ref(
    row: &crate::snapshot::Record,
) -> (
    std::borrow::Cow<'_, str>,
    std::borrow::Cow<'_, str>,
    std::borrow::Cow<'_, str>,
) {
    let field = |key: &str| match row.get(key).expect("no entry found for key") {
        Value::String(text) => std::borrow::Cow::Borrowed(text.as_str()),
        value => std::borrow::Cow::Owned(pystr(value)),
    };
    (field("occurred_at"), field("created_at"), field("id"))
}
fn anchors(legs: &[Value]) -> BTreeSet<String> {
    legs.iter()
        .filter(|l| !absent(&l["anchor_transaction_id"]) || !absent(&l["transaction_id"]))
        .map(|l| pystr(or(&l["anchor_transaction_id"], &l["transaction_id"])))
        .collect()
}
pub(super) fn current(shared: &Shared, h: &Value, legs: &[Value]) -> Result<Vec<Value>, Error> {
    let ids = anchors(legs);
    // Anchor rows with these ids, in anchor order.
    let mut candidates: Vec<_> = ids
        .iter()
        .filter_map(|id| shared.anchors.get(id))
        .flatten()
        .collect();
    candidates.sort_unstable_by_key(|(at, _)| *at);
    let mut rows: Vec<_> = candidates
        .into_iter()
        .map(|(_, r)| *r)
        .filter(|r| r.get("profile_id") == h.get("profile_id") && ids.contains(&pystr(&r["id"])))
        .map(WithoutProvenance)
        .collect();
    rows.sort_by_cached_key(|row| order(row.0));
    let canonical = evidence::canonical_of_rows(&rows)?;
    let rejected: BTreeSet<_> = canonical
        .rejected()
        .iter()
        .flat_map(|i| array(&i["transaction_ids"]).iter().map(pystr))
        .collect();
    if !rejected.is_empty() {
        return Err(format!(
            "component evidence contains a rejected canonical event: {}",
            rejected.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    let aliases: BTreeMap<_, _> = canonical
        .events()
        .iter()
        .flat_map(|e| {
            evidence::canonical::EventRef::Typed(e)
                .aliases()
                .iter()
                .map(|a| (pystr(&a[0]), pystr(&a[1])))
        })
        .collect();
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !aliases.contains_key(*id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "component evidence is unavailable for: {}",
            missing.join(", ")
        )
        .into());
    }
    let relevant: BTreeSet<_> = ids.iter().map(|id| aliases[id].clone()).collect();
    let mut snapshots = BTreeMap::new();
    for e in canonical.events() {
        for s in evidence::canonical::EventRef::Typed(e).snapshots() {
            if relevant.contains(&pystr(&s["quantity_hash"])) {
                snapshots.insert(pystr(&s["detail_hash"]), s.clone());
            }
        }
    }
    Ok(snapshots.into_values().collect())
}
/// The top-level entries of a compact JSON object without the volatile ones,
/// or `None` when the text is not a compact object this walk can split.
fn stable_payload(text: &str) -> Option<String> {
    const VOLATILE: [&[u8]; 4] = [b"fingerprint", b"occurred_at", b"confirmed_at", b"raw_json"];
    let bytes = text.as_bytes();
    // The end of the JSON string starting at `at`.
    let string_end = |mut at: usize| -> Option<usize> {
        at += 1;
        loop {
            match *bytes.get(at)? {
                b'\\' => at += 2,
                b'"' => return Some(at + 1),
                _ => at += 1,
            }
        }
    };
    // The end of the compact JSON value starting at `at`: the separator or
    // closing bracket after it.
    let value_end = |mut at: usize| -> Option<usize> {
        let mut depth = 0usize;
        loop {
            match *bytes.get(at)? {
                b'"' => at = string_end(at)?,
                b'{' | b'[' => {
                    depth += 1;
                    at += 1;
                }
                b'}' | b']' => {
                    if depth == 0 {
                        return Some(at);
                    }
                    depth -= 1;
                    at += 1;
                }
                b',' if depth == 0 => return Some(at),
                b' ' | b'\n' | b'\r' | b'\t' => return None,
                _ => at += 1,
            }
        }
    };
    if bytes.first() != Some(&b'{') || bytes.last() != Some(&b'}') {
        return None;
    }
    let mut kept = String::with_capacity(text.len());
    kept.push('{');
    let mut at = 1;
    while at < bytes.len() - 1 {
        if bytes[at] != b'"' {
            return None;
        }
        let key_end = string_end(at)?;
        if bytes.get(key_end) != Some(&b':') {
            return None;
        }
        let end = value_end(key_end + 1)?;
        if !VOLATILE.contains(&&bytes[at + 1..key_end - 1]) {
            if kept.len() > 1 {
                kept.push(',');
            }
            kept.push_str(&text[at..end]);
        }
        match bytes.get(end) {
            // A separator is followed by another entry.
            Some(b',') if end + 1 < bytes.len() - 1 => at = end + 1,
            Some(b'}') if end == bytes.len() - 1 => at = end,
            _ => return None,
        }
    }
    kept.push('}');
    Some(kept)
}

/// `commitment_pairs` of snapshots this crate's commitment builder wrote:
/// their payloads are compact sorted JSON that loading and dumping leaves
/// unchanged, so the volatile entries are removed from the text.
fn built_commitment_pairs(snapshots: &[Value]) -> Result<Vec<(String, String)>, Error> {
    let mut result = Vec::new();
    for s in snapshots {
        let stable = s["payload_json"].as_str().and_then(stable_payload);
        let Some(stable) = stable else {
            return commitment_pairs(snapshots);
        };
        result.push((pystr(&s["quantity_hash"]), codec::sha256(&stable)));
    }
    result.sort();
    Ok(result)
}

// Python computes detail hashes after the canonical-evidence exception guard.
pub(super) fn commitment_pairs(snapshots: &[Value]) -> Result<Vec<(String, String)>, Error> {
    let mut result = Vec::new();
    for s in snapshots {
        let mut payload = codec::loads(
            s["payload_json"]
                .as_str()
                .ok_or("custody evidence payload is invalid")?,
        )?;
        let Py::Dict(ref mut fields) = payload else {
            return Err("custody evidence payload is invalid".into());
        };
        fields.retain(|(k,_)|!matches!(k,Key::Str(k) if k.as_str().is_some_and(|k|["fingerprint","occurred_at","confirmed_at","raw_json"].contains(&k))));
        let detail_hash = codec::hash_py(&payload, false).map_err(Error::encoding)?.0;
        result.push((pystr(&s["quantity_hash"]), detail_hash));
    }
    result.sort();
    Ok(result)
}
pub(super) fn evidence_status(
    shared: &Shared,
    h: &Value,
    legs: &[Value],
    commitments: &[Value],
) -> Result<Value, Error> {
    let expected = &h["expected_evidence_count"];
    let count = commitments.len();
    let result = |status: &str, valid: bool| json!({"expected_count":expected,"commitment_count":count,"valid":valid,"usable":valid,"status":status,"reason":status});
    if expected.is_null() {
        return Ok(result("commitment_header_missing", false));
    }
    let n = wide(expected);
    if count as i128 != n
        || commitments
            .iter()
            .enumerate()
            .any(|(i, c)| wide(&c["ordinal"]) != i as i128)
    {
        return Ok(result("commitments_incomplete", false));
    }
    let ids = anchors(legs);
    if n == 0 {
        let mut v = result(
            if ids.is_empty() {
                "matched"
            } else {
                "unexpected_transaction_anchors"
            },
            ids.is_empty(),
        );
        v["current_count"] = json!(0);
        return Ok(v);
    }
    if ids.is_empty() {
        return Ok(result("transaction_anchors_missing", false));
    }
    let current = match current(shared, h, legs) {
        Ok(c) => c,
        Err(e) if e.is_evidence_unavailable() => {
            let mut v = result("current_evidence_unavailable", false);
            v["detail"] = json!(e);
            return Ok(v);
        }
        Err(e) => return Err(e),
    };
    let current = built_commitment_pairs(&current)?;
    let committed: Vec<_> = commitments
        .iter()
        .map(|c| (pystr(&c["quantity_hash"]), pystr(&c["detail_hash"])))
        .collect();
    let matched = current == committed;
    let mut v = result(
        if matched {
            "matched"
        } else {
            "evidence_mismatch"
        },
        matched,
    );
    v["current_count"] = json!(current.len());
    Ok(v)
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Direction {
    Inbound,
    Outbound,
}
impl Direction {
    fn text(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }
    fn opposite(self) -> Self {
        match self {
            Self::Inbound => Self::Outbound,
            Self::Outbound => Self::Inbound,
        }
    }
}

#[derive(Default)]
struct Pool {
    strings: HashSet<Rc<str>>,
    events: BTreeSet<Rc<evidence::EventKey>>,
}
impl Pool {
    fn string(&mut self, value: &str) -> Rc<str> {
        if let Some(value) = self.strings.get(value) {
            return value.clone();
        }
        let value: Rc<str> = value.into();
        self.strings.insert(value.clone());
        value
    }
    fn event(&mut self, key: evidence::EventKey) -> Rc<evidence::EventKey> {
        if let Some(value) = self.events.get(&key) {
            return value.clone();
        }
        let key = Rc::new(key);
        self.events.insert(key.clone());
        key
    }
}

pub(super) struct Observation {
    transaction: Rc<str>,
    event: Rc<evidence::EventKey>,
    wallet: Rc<str>,
    asset: Rc<str>,
    direction: Direction,
    amount: BigInt,
    fee: BigInt,
    includes_fee: bool,
    principal: BigInt,
    quantity_hash: OnceCell<String>,
}
impl Observation {
    fn hash(&self) -> &str {
        self.quantity_hash.get_or_init(|| {
            let options = kassiber_pyjson::Options {
                ensure_ascii: false,
                ..Default::default()
            };
            let mut payload = String::with_capacity(400);
            payload.push_str("{\"amount_includes_fee\":");
            payload.push_str(if self.includes_fee { "true" } else { "false" });
            payload.push_str(",\"amount_msat\":");
            payload.push_str(&self.amount.to_string());
            payload.push_str(",\"asset\":");
            kassiber_pyjson::dumps_str_into(&self.asset, &options, &mut payload);
            payload.push_str(",\"direction\":");
            kassiber_pyjson::dumps_str_into(self.direction.text(), &options, &mut payload);
            payload.push_str(",\"event\":{");
            for (at, (name, value)) in [
                ("chain", &self.event.chain),
                ("native_event_id", &self.event.native_event_id),
                ("native_namespace", &self.event.native_namespace),
                ("network", &self.event.network),
            ]
            .into_iter()
            .enumerate()
            {
                if at > 0 {
                    payload.push(',');
                }
                kassiber_pyjson::dumps_str_into(name, &options, &mut payload);
                payload.push(':');
                kassiber_pyjson::dumps_str_into(value, &options, &mut payload);
            }
            payload.push_str("},\"fee_msat\":");
            payload.push_str(&self.fee.to_string());
            payload.push_str(",\"schema_version\":1,\"wallet_id\":");
            kassiber_pyjson::dumps_str_into(&self.wallet, &options, &mut payload);
            payload.push('}');
            codec::sha256(&payload)
        })
    }
}

fn simple_detail<F: crate::fields::Fields + ?Sized>(row: &F) -> bool {
    let raw = row.at("raw_json");
    let safe_raw = raw.is_null()
        || raw.as_str().is_some_and(|s| {
            s.len() <= 4096
                && !s.contains('\\')
                && s.bytes()
                    .filter(|c| matches!(c, b'{' | b'['))
                    .take(64)
                    .count()
                    < 64
        });
    safe_raw
        && (row.at("swap_refund_funding_vout").is_null()
            || row.at("swap_refund_funding_vout").as_i64().is_some())
}

fn build<F: crate::fields::Fields + ?Sized>(
    row: &F,
    key: evidence::EventKey,
    pool: &mut Pool,
) -> Result<Observation, Error> {
    // Borrowed where the row spells them; the pool keeps one copy each.
    let transaction = codec::text_ref(row.at("id"));
    let anchor = if truth(row.at("journal_transaction_id")) {
        codec::text_ref(row.at("journal_transaction_id"))
    } else {
        transaction.clone()
    };
    let wallet = codec::text_ref(row.at("wallet_id"));
    let asset = codec::text_ref(row.at("asset"));
    let asset = crate::python_text::upper(&asset);
    let direction = codec::text_ref(row.at("direction"));
    let direction = crate::python_text::lower(&direction);
    let includes_fee = truth(row.at("amount_includes_fee"));
    let small = |v: &Value| if v.is_null() { Some(0) } else { v.as_i64() };
    let fast = small(row.at("amount"))
        .zip(small(row.at("fee")))
        .filter(|(amount, fee)| {
            simple_detail(row)
                && !transaction.is_empty()
                && !anchor.is_empty()
                && !wallet.is_empty()
                && !asset.is_empty()
                && matches!(&*direction, "inbound" | "outbound")
                && *amount >= 0
                && *fee >= 0
                && !(direction == "inbound" && includes_fee)
                && (*amount != 0 || (direction == "outbound" && *fee > 0))
                && (!includes_fee || *fee <= *amount)
        });
    let (amount, fee, quantity_hash) = if let Some((amount, fee)) = fast {
        (BigInt::from(amount), BigInt::from(fee), OnceCell::new())
    } else {
        let observed = evidence::observation(row, &key)?;
        (
            big(&observed["amount_msat"]),
            big(&observed["fee_msat"]),
            OnceCell::from(text(&observed["quantity_hash"])),
        )
    };
    let direction = if direction == "outbound" {
        Direction::Outbound
    } else {
        Direction::Inbound
    };
    let principal = if direction == Direction::Outbound && includes_fee {
        &amount - &fee
    } else {
        amount.clone()
    };
    Ok(Observation {
        transaction: pool.string(&transaction),
        event: pool.event(key),
        wallet: pool.string(&wallet),
        asset: pool.string(&asset),
        direction,
        amount,
        fee,
        includes_fee,
        principal,
        quantity_hash,
    })
}

type Capacities = BTreeMap<(Rc<evidence::EventKey>, Rc<str>, Direction), BigInt>;

pub(super) struct Native {
    observations: Vec<Observation>,
    by_transaction: HashMap<Rc<str>, usize>,
    by_event: BTreeMap<Rc<evidence::EventKey>, Vec<usize>>,
    capacities: RefCell<Capacities>,
}
impl Native {
    pub(super) fn new(observations: Vec<Observation>) -> Self {
        let mut by_transaction = HashMap::default();
        let mut by_event: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (at, o) in observations.iter().enumerate() {
            by_transaction.entry(o.transaction.clone()).or_insert(at);
            by_event.entry(o.event.clone()).or_default().push(at);
        }
        Self {
            observations,
            by_transaction,
            by_event,
            capacities: RefCell::new(BTreeMap::new()),
        }
    }
    fn event(&self, key: &evidence::EventKey) -> impl Iterator<Item = &Observation> {
        self.by_event
            .get(key)
            .into_iter()
            .flatten()
            .map(|&at| &self.observations[at])
    }
    fn capacity(&self, o: &Observation, direction: Direction) -> BigInt {
        let key = (o.event.clone(), o.asset.clone(), direction);
        if let Some(value) = self.capacities.borrow().get(&key) {
            return value.clone();
        }
        let unique: BTreeMap<_, _> = self
            .event(&o.event)
            .filter(|v| v.direction == direction && v.asset == o.asset)
            .map(|v| (v.hash(), &v.principal))
            .collect();
        let value: BigInt = unique.into_values().cloned().sum();
        self.capacities.borrow_mut().insert(key, value.clone());
        value
    }
}

/// The rows the authoring view enriches: active rows of the profile.
fn authored(r: &crate::snapshot::Record, profile: &Value) -> bool {
    r.get("profile_id") == Some(profile) && !truth(&r["excluded"])
}

/// Evidence's enrichment is this authoring view's enrichment when every row
/// is authored and has no closed provenance: both enrich the same rows, and
/// only provenance distinguishes them.
fn shared_enrichment<'a>(
    snapshot: &Snapshot,
    profile: &Value,
    prepared: Option<&'a evidence::Prepared>,
) -> Option<&'a evidence::Prepared> {
    prepared.filter(|_| {
        snapshot.transactions.iter().all(|r| {
            authored(r, profile)
                && r.get("observation_authority_version")
                    .is_none_or(Value::is_null)
        })
    })
}

/// Keep one built observation: a later valid observation of a transaction
/// replaces it in place; recoverable failures skip the row.
fn keep(
    observations: &mut Vec<Observation>,
    positions: &mut HashMap<Rc<str>, usize>,
    built: Result<Observation, Error>,
) -> Result<(), Error> {
    match built {
        Ok(o) => {
            if let Some(&at) = positions.get(&o.transaction) {
                observations[at] = o;
            } else {
                positions.insert(o.transaction.clone(), observations.len());
                observations.push(o);
            }
        }
        Err(e) if e.is_value_or_type() => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

pub(super) fn native_context(
    snapshot: &Snapshot,
    prepared: Option<&evidence::Prepared>,
) -> Result<Vec<Observation>, Error> {
    let profile = json!(pystr(&snapshot.profile["id"]));
    let mut observations = Vec::new();
    let mut positions: HashMap<Rc<str>, usize> = HashMap::default();
    let mut pool = Pool::default();
    if let Some(prepared) = shared_enrichment(snapshot, &profile, prepared) {
        let records = &snapshot.transactions;
        let mut rows: Vec<usize> = (0..records.len()).collect();
        rows.sort_by_cached_key(|at| order_ref(&records[*at]));
        let _span = crate::timing::span("components.native.observations");
        for at in rows {
            let row = prepared.row(at, records);
            let built = prepared
                .key(at, records)
                .and_then(|key| build(&row, key, &mut pool));
            keep(&mut observations, &mut positions, built)?;
        }
        return Ok(observations);
    }
    let mut rows: Vec<_> = snapshot
        .transactions
        .iter()
        .filter(|r| authored(r, &profile))
        .map(WithoutProvenance)
        .collect();
    rows.sort_by_cached_key(|row| order(row.0));
    let span = crate::timing::span("components.native.enrich");
    let rows = evidence::enriched_rows(&rows)?;
    drop(span);
    let _span = crate::timing::span("components.native.observations");
    for row in &rows {
        let built = evidence::event_key(row).and_then(|key| build(row, key, &mut pool));
        keep(&mut observations, &mut positions, built)?;
    }
    Ok(observations)
}

pub(super) fn native_status(
    legs: &[Value],
    allocs: &[Value],
    native: &Native,
) -> Result<Value, Error> {
    let boundaries: Vec<_> = legs
        .iter()
        .filter(|l| {
            (l["role"] == "source" || owned(l))
                && truth(or(&l["anchor_transaction_id"], &l["transaction_id"]))
                && big(&l["amount_msat"]) > BigInt::from(0)
        })
        .collect();
    let mut exact = BTreeSet::new();
    let anchor = |l: &Value| text(or(&l["anchor_transaction_id"], &l["transaction_id"]));
    let observation = |id: String| {
        native
            .by_transaction
            .get(id.as_str())
            .map(|&at| &native.observations[at])
    };
    for a in allocs {
        let Some((s, t)) = by_id(legs, &a["source_leg_id"]).zip(by_id(legs, &a["sink_leg_id"]))
        else {
            continue;
        };
        let Some((so, to)) = observation(anchor(s)).zip(observation(anchor(t))) else {
            continue;
        };
        if so.event == to.event
            && so.direction == Direction::Outbound
            && to.direction == Direction::Inbound
            && s["asset"] == t["asset"]
            && big(&a["source_amount_msat"]) == big(&a["sink_amount_msat"])
        {
            exact.insert(text(&t["id"]));
        }
    }
    let (mut supported, mut partial, mut contradicted) = (0, 0, 0);
    for l in &boundaries {
        if exact.contains(&text(&l["id"])) {
            supported += 1;
            continue;
        }
        let id = anchor(l);
        let Some(o) = observation(id.clone()) else {
            continue;
        };
        if o.event.native_namespace != "chain"
            || !matches!(o.event.chain.as_str(), "bitcoin" | "liquid")
        {
            continue;
        }
        let opposite = o.direction.opposite();
        let compatible = native
            .event(&o.event)
            .any(|v| v.transaction.as_ref() != id && v.direction == opposite && v.asset == o.asset);
        if !compatible {
            continue;
        }
        let recovered = native.capacity(o, opposite);
        let capacity = native.capacity(o, o.direction);
        if recovered > capacity {
            contradicted += 1;
        } else if recovered == capacity {
            supported += 1;
        } else {
            partial += 1;
        }
    }
    let status = if contradicted > 0 {
        "contradicted"
    } else if supported == boundaries.len() && !boundaries.is_empty() {
        "corroborated"
    } else if supported > 0 || partial > 0 {
        "partial"
    } else {
        "unverified"
    };
    Ok(
        json!({"boundary_count":boundaries.len(),"supported_boundary_count":supported,"partially_supported_boundary_count":partial,"contradicted_boundary_count":contradicted,"status":status,"usable":status!="contradicted"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Value {
        json!({"id":"tx","profile_id":"profile","wallet_id":"wallet","external_id":"event","wallet_kind":"address","excluded":false,"raw_json":"{}","asset":"BTC","direction":"outbound","amount":1000,"fee":10,"amount_includes_fee":false,"occurred_at":"2026-01-01","created_at":"2026-01-01"})
    }

    fn compare(row: &Value) {
        let key = evidence::event_key(row).unwrap_or_else(|_| evidence::EventKey {
            chain: "bitcoin".into(),
            network: "main".into(),
            native_namespace: "fallback".into(),
            native_event_id: "event".into(),
        });
        let expected = evidence::observation(row, &key);
        let actual = build(row, key.clone(), &mut Pool::default());
        match (expected, actual) {
            (Ok(expected), Ok(actual)) => {
                assert_eq!(
                    actual.transaction.as_ref(),
                    text(&expected["transaction_id"])
                );
                assert_eq!(actual.wallet.as_ref(), text(&expected["wallet_id"]));
                assert_eq!(actual.asset.as_ref(), text(&expected["asset"]));
                assert_eq!(actual.direction.text(), text(&expected["direction"]));
                assert_eq!(*actual.event, key);
                assert_eq!(actual.amount, big(&expected["amount_msat"]));
                assert_eq!(actual.fee, big(&expected["fee_msat"]));
                assert_eq!(actual.includes_fee, truth(&expected["amount_includes_fee"]));
                assert_eq!(actual.hash(), text(&expected["quantity_hash"]));
                assert_eq!(actual.principal, anchors::principal(row).unwrap());
            }
            (Err(expected), Err(actual)) => {
                assert_eq!(actual.code, expected.code);
                assert_eq!(actual.message, expected.message);
            }
            _ => panic!("native support admission differs from canonical observation"),
        }
    }

    #[test]
    fn native_support_preserves_observation_admission_and_hashes() {
        compare(&row());
        for direction in ["inbound", "outbound"] {
            let mut candidate = row();
            candidate["direction"] = json!(direction);
            candidate["amount"] = json!(0);
            candidate["fee"] = json!(0);
            compare(&candidate);
        }
        for (field, values) in [
            (
                "id",
                vec![json!(""), json!("雪\n\"\\"), json!(false), Value::Null],
            ),
            (
                "wallet_id",
                vec![json!(""), json!("Österreich 🐍"), json!(17), Value::Null],
            ),
            (
                "asset",
                vec![json!(""), json!("lbtc"), json!("ü"), Value::Null],
            ),
            (
                "direction",
                vec![
                    json!("inbound"),
                    json!("OUTBOUND"),
                    json!(" outbound "),
                    json!("invalid"),
                ],
            ),
            (
                "amount",
                vec![
                    json!(0),
                    json!(-1),
                    json!(i64::MAX),
                    json!("1000"),
                    json!(1.5),
                    json!(true),
                    Value::Null,
                    json!([]),
                    serde_json::from_str("1e999").unwrap(),
                ],
            ),
            (
                "fee",
                vec![
                    json!(-1),
                    json!(1001),
                    json!("10"),
                    json!(false),
                    Value::Null,
                ],
            ),
            (
                "raw_json",
                vec![
                    json!("{\"memo\":\"\\ud800\"}"),
                    json!("{\"memo\":\"\\ud83d\\udc0d\"}"),
                    json!("{\"memo\":\"雪\"}"),
                    json!("{\"a\":NaN,\"b\":Infinity}"),
                    json!("unparsed"),
                    json!("[1,2]"),
                    json!({"a":[1,2]}),
                    Value::Null,
                ],
            ),
            (
                "swap_refund_funding_vout",
                vec![json!(true), json!({"unexpected":[]})],
            ),
        ] {
            for value in values {
                let mut candidate = row();
                candidate[field] = value;
                compare(&candidate);
                candidate["amount_includes_fee"] = json!(true);
                compare(&candidate);
            }
        }
        for raw in [
            format!("{{\"n\":{}}}", "7".repeat(4400)),
            format!("{}0{}", "[".repeat(70), "]".repeat(70)),
        ] {
            let mut candidate = row();
            candidate["raw_json"] = json!(raw);
            compare(&candidate);
        }
    }

    #[test]
    fn built_payloads_hash_as_loaded_payloads() {
        let mut compared = 0;
        for fixture in [
            include_str!("../../../../tests/fixtures/custody_evidence_cases.json"),
            include_str!("../../../../tests/fixtures/custody_graph_cases.json"),
            include_str!("../../../../tests/fixtures/custody_component_cases.json"),
        ] {
            let cases: Vec<Value> = serde_json::from_str(fixture).unwrap();
            for case in cases {
                let Ok(snapshot) = serde_json::from_value::<Snapshot>(case["input"].clone()) else {
                    continue;
                };
                let rows: Vec<_> = snapshot
                    .transactions
                    .iter()
                    .map(WithoutProvenance)
                    .collect();
                let Ok(canonical) = evidence::canonical_of_rows(&rows) else {
                    continue;
                };
                for event in canonical.events() {
                    let snapshots = evidence::canonical::EventRef::Typed(event).snapshots();
                    for snapshot in snapshots {
                        let text = snapshot["payload_json"].as_str().unwrap();
                        assert!(stable_payload(text).is_some(), "{text}");
                    }
                    assert_eq!(
                        built_commitment_pairs(snapshots).unwrap(),
                        commitment_pairs(snapshots).unwrap()
                    );
                    compared += snapshots.len();
                }
            }
        }
        assert!(compared > 100, "{compared}");
        for text in [
            "{}",
            "{\"a\":1}",
            "{\"raw_json\":{\"x\":[1,{\"y\":\"}\"}]},\"z\":\"a,b\"}",
        ] {
            let mut loaded = codec::loads(text).unwrap();
            let Py::Dict(ref mut fields) = loaded else {
                unreachable!()
            };
            fields.retain(|(k, _)| !matches!(k, Key::Str(k) if k.is("raw_json")));
            assert_eq!(
                stable_payload(text).unwrap(),
                codec::dumps(&loaded, false).unwrap()
            );
        }
        for text in ["", "{", "[]", "{\"a\": 1}", "{\"a\":1,}", "{\"a\"}"] {
            assert_eq!(stable_payload(text), None, "{text}");
        }
    }

    #[test]
    fn native_support_shares_evidence_enrichment() {
        let mut shared_books = 0;
        for fixture in [
            include_str!("../../../../tests/fixtures/custody_evidence_cases.json"),
            include_str!("../../../../tests/fixtures/custody_graph_cases.json"),
            include_str!("../../../../tests/fixtures/custody_component_cases.json"),
            include_str!("../../../../tests/fixtures/custody_interpretation_cases.json"),
        ] {
            let cases: Vec<Value> = serde_json::from_str(fixture).unwrap();
            for case in cases {
                let Ok(snapshot) = serde_json::from_value::<Snapshot>(case["input"].clone()) else {
                    continue;
                };
                if snapshot.profile.get("id").is_none()
                    || snapshot.transactions.iter().any(|r| {
                        ["occurred_at", "created_at", "id", "excluded"]
                            .iter()
                            .any(|k| r.get(k).is_none())
                    })
                {
                    continue;
                }
                let Ok(prepared) = crate::evidence::prepare(&snapshot) else {
                    continue;
                };
                let facts = |o: &Observation| {
                    (
                        o.transaction.to_string(),
                        (*o.event).clone(),
                        o.amount.clone(),
                        o.fee.clone(),
                        o.hash().to_owned(),
                    )
                };
                let outcome = |result: Result<Vec<Observation>, Error>| {
                    result
                        .map(|o| o.iter().map(facts).collect::<Vec<_>>())
                        .map_err(|e| (e.code, e.message))
                };
                let profile = json!(pystr(&snapshot.profile["id"]));
                shared_books +=
                    usize::from(shared_enrichment(&snapshot, &profile, Some(&prepared)).is_some());
                assert_eq!(
                    outcome(native_context(&snapshot, Some(&prepared))),
                    outcome(native_context(&snapshot, None)),
                    "{}",
                    case["case"]
                );
            }
        }
        assert!(shared_books > 20, "{shared_books}");
    }

    #[test]
    fn native_support_keeps_replacement_slots_and_refusal_order() {
        let mut snapshot = Snapshot {
            profile: serde_json::from_value(json!({"id":"profile"})).unwrap(),
            ..Default::default()
        };
        for (id, event, created, raw) in [
            ("a", "old", "1", "{}"),
            ("b", "other", "2", "{}"),
            ("a", "new", "3", "{}"),
            ("a", "ignored", "4", "{\"memo\":\"\\ud800\"}"),
        ] {
            let mut r = row();
            r["id"] = json!(id);
            r["external_id"] = json!(event);
            r["created_at"] = json!(created);
            r["raw_json"] = json!(raw);
            snapshot
                .transactions
                .push(serde_json::from_value(r).unwrap());
        }
        let observations = native_context(&snapshot, None).unwrap();
        // Evidence's enrichment yields the same authoring observations.
        let prepared = crate::evidence::prepare(&snapshot).unwrap();
        let shared = native_context(&snapshot, Some(&prepared)).unwrap();
        let facts = |o: &Observation| {
            (
                o.transaction.to_string(),
                (*o.event).clone(),
                o.amount.clone(),
                o.fee.clone(),
                o.hash().to_owned(),
            )
        };
        assert_eq!(
            observations.iter().map(facts).collect::<Vec<_>>(),
            shared.iter().map(facts).collect::<Vec<_>>()
        );
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].transaction.as_ref(), "a");
        assert_eq!(observations[0].event.native_event_id, "new");
        assert_eq!(observations[1].transaction.as_ref(), "b");
        assert!(Rc::ptr_eq(&observations[0].wallet, &observations[1].wallet));
        let mut r = row();
        r["amount"] = serde_json::from_str("1e999").unwrap();
        snapshot
            .transactions
            .push(serde_json::from_value(r).unwrap());
        assert_eq!(
            native_context(&snapshot, None).err().unwrap().code,
            "OverflowError"
        );
    }
}
