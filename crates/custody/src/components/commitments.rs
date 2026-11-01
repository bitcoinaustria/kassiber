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
    let enriched = evidence::enriched_rows(&rows)?;
    let canonical = evidence::canonical_input(&enriched, &json!({}), &BTreeSet::new())?;
    let rejected: BTreeSet<_> = array(&canonical["rejected_events"])
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
    let aliases: BTreeMap<_, _> = array(&canonical["events"])
        .iter()
        .flat_map(|e| {
            array(&e["observation_aliases"])
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
    for e in array(&canonical["events"]) {
        for s in array(&e["evidence_snapshots"]) {
            if relevant.contains(&pystr(&s["quantity_hash"])) {
                snapshots.insert(pystr(&s["detail_hash"]), s.clone());
            }
        }
    }
    Ok(snapshots.into_values().collect())
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
    let current = commitment_pairs(&current)?;
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
    fn string(&mut self, value: String) -> Rc<str> {
        if let Some(value) = self.strings.get(value.as_str()) {
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

fn simple_detail(row: &Value) -> bool {
    let raw = &row["raw_json"];
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
        && (row["swap_refund_funding_vout"].is_null()
            || row["swap_refund_funding_vout"].as_i64().is_some())
}

fn build(row: &Value, key: evidence::EventKey, pool: &mut Pool) -> Result<Observation, Error> {
    let transaction = text(&row["id"]);
    let anchor = if truth(&row["journal_transaction_id"]) {
        text(&row["journal_transaction_id"])
    } else {
        transaction.clone()
    };
    let wallet = text(&row["wallet_id"]);
    let asset = text(&row["asset"]).py_upper();
    let direction = text(&row["direction"]).py_lower();
    let includes_fee = truth(&row["amount_includes_fee"]);
    let small = |v: &Value| if v.is_null() { Some(0) } else { v.as_i64() };
    let fast = small(&row["amount"])
        .zip(small(&row["fee"]))
        .filter(|(amount, fee)| {
            simple_detail(row)
                && !transaction.is_empty()
                && !anchor.is_empty()
                && !wallet.is_empty()
                && !asset.is_empty()
                && matches!(direction.as_str(), "inbound" | "outbound")
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
        transaction: pool.string(transaction),
        event: pool.event(key),
        wallet: pool.string(wallet),
        asset: pool.string(asset),
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
        let mut by_transaction = HashMap::new();
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

pub(super) fn native_context(snapshot: &Snapshot) -> Result<Vec<Observation>, Error> {
    let profile = json!(pystr(&snapshot.profile["id"]));
    let mut rows: Vec<_> = snapshot
        .transactions
        .iter()
        .filter(|r| r.get("profile_id") == Some(&profile) && !truth(&r["excluded"]))
        .map(WithoutProvenance)
        .collect();
    rows.sort_by_cached_key(|row| order(row.0));
    let span = crate::timing::span("components.native.enrich");
    let rows = evidence::enriched_rows(&rows)?;
    drop(span);
    let _span = crate::timing::span("components.native.observations");
    let mut observations = Vec::new();
    let mut positions: HashMap<Rc<str>, usize> = HashMap::new();
    let mut pool = Pool::default();
    for row in &rows {
        match evidence::event_key(row).and_then(|key| build(row, key, &mut pool)) {
            Ok(o) => {
                if let Some(&at) = positions.get(&o.transaction) {
                    observations[at] = o;
                } else {
                    positions.insert(o.transaction.clone(), observations.len());
                    observations.push(o);
                }
            }
            Err(e) if e.is_value_or_type() => continue,
            Err(e) => return Err(e),
        }
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
        let observations = native_context(&snapshot).unwrap();
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
            native_context(&snapshot).err().unwrap().code,
            "OverflowError"
        );
    }
}
