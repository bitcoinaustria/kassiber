use kassiber_envelope::{get, py_str};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_transfers::pairing::{suggest, MatchOptions, Population};
use std::collections::HashSet;
fn list(v: &Value) -> &[Value] {
    if let Value::List(v) = v {
        v
    } else {
        &[]
    }
}
fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
    get(v, k).unwrap_or(&Value::Null)
}
fn bytes(v: &Value) -> String {
    dumps(v, &Options::default()).unwrap()
}
#[test]
fn pairing_matcher_matches_python_transfer_fixtures_byte_for_byte() {
    let cases = kassiber_pytext::loads(include_str!("pairing_matcher.json")).unwrap();
    assert_eq!(list(&cases).len(), 149);
    for (index, case) in list(&cases).iter().enumerate() {
        let opts = field(case, "options");
        let mut o = MatchOptions::default();
        for (k, target) in [
            ("time_window_seconds", &mut o.time_window_seconds),
            ("fee_sats_min", &mut o.fee_sats_min),
        ] {
            if let Some(v) = get(opts, k) {
                *target = py_str(v).parse().unwrap();
            }
        }
        if let Some(v) = get(opts, "fee_pct_max") {
            o.fee_pct_max = py_str(v).parse().unwrap();
        }
        if let Some(v) = get(opts, "now_iso") {
            o.now = py_str(v);
        }
        if let Some(Value::Bool(v)) = get(opts, "include_heuristics") {
            o.include_heuristics = *v;
        }
        let pop = Population::new(list(field(case, "rows"))).unwrap();
        let booked: HashSet<_> = list(field(opts, "booked_move_transaction_ids"))
            .iter()
            .map(py_str)
            .collect();
        let actual = suggest(
            &pop,
            list(field(opts, "pair_records")),
            list(field(opts, "dismissals")),
            &booked,
            &o,
        );
        if let Some(expected) = get(case, "error") {
            assert_eq!(
                actual.unwrap_err().message,
                py_str(expected),
                "matcher case {index}: refusal"
            );
            continue;
        }
        let actual = actual.unwrap();
        let actual = Value::List(actual.into_iter().map(|c| c.into_value()).collect());
        assert_eq!(
            bytes(&actual),
            bytes(field(case, "result")),
            "matcher case {index}"
        );
    }
}

#[test]
fn pairing_cache_rechecks_occupancy_dismissal_expiry_guards_and_options() {
    let rows = kassiber_pytext::loads(r#"[
        {"id":"o","wallet_id":"source","direction":"outbound","asset":"BTC","wallet_kind":"custom","amount":10000000,"occurred_at":"2026-10-30T11:00:00Z","raw_json":{}},
        {"id":"i","wallet_id":"target","direction":"inbound","asset":"BTC","wallet_kind":"custom","amount":9000000,"occurred_at":"2026-10-30T11:00:10Z","raw_json":{}}
    ]"#).unwrap();
    let pop = Population::new(list(&rows)).unwrap();
    let mut options = MatchOptions {
        now: "2026-10-30T12:00:00Z".into(),
        ..MatchOptions::default()
    };
    let empty = HashSet::new();
    let baseline = suggest(&pop, &[], &[], &empty, &options).unwrap();
    assert_eq!(baseline.len(), 1);
    options.time_window_seconds_overflow = true;
    assert_eq!(
        suggest(&pop, &[], &[], &empty, &options)
            .unwrap_err()
            .message,
        "int too large to convert to float"
    );
    options.time_window_seconds_overflow = false;
    assert_eq!(
        bytes(&baseline[0].data),
        bytes(&suggest(&pop, &[], &[], &empty, &options).unwrap()[0].data)
    );
    let pairs =
        kassiber_pytext::loads(r#"[{"out_transaction_id":"o","in_transaction_id":"i"}]"#).unwrap();
    assert!(suggest(&pop, list(&pairs), &[], &empty, &options)
        .unwrap()
        .is_empty());
    assert_eq!(suggest(&pop, &[], &[], &empty, &options).unwrap().len(), 1);
    let dismissed = kassiber_pytext::loads(r#"[{"out_transaction_id":"o","in_transaction_id":"i","expires_at":"2026-10-30T12:00:01Z"}]"#).unwrap();
    assert!(suggest(&pop, &[], list(&dismissed), &empty, &options)
        .unwrap()
        .is_empty());
    options.now = "2026-10-30T12:00:01Z".into();
    assert_eq!(
        suggest(&pop, &[], list(&dismissed), &empty, &options)
            .unwrap()
            .len(),
        1
    );
    assert!(
        suggest(&pop, &[], &[], &HashSet::from(["o".into()]), &options)
            .unwrap()
            .is_empty()
    );
    assert_eq!(suggest(&pop, &[], &[], &empty, &options).unwrap().len(), 1);
    options.time_window_seconds = 1;
    assert!(suggest(&pop, &[], &[], &empty, &options)
        .unwrap()
        .is_empty());
    options.time_window_seconds = 86400;
    options.fee_pct_max = 0.0;
    options.fee_sats_min = 0;
    assert!(suggest(&pop, &[], &[], &empty, &options)
        .unwrap()
        .is_empty());
    options.fee_sats_min = 2500;
    assert_eq!(suggest(&pop, &[], &[], &empty, &options).unwrap().len(), 1);
    options.fee_sats_min = 0;
    assert!(suggest(&pop, &[], &[], &empty, &options)
        .unwrap()
        .is_empty());
    options.fee_pct_max = 0.1;
    assert_eq!(suggest(&pop, &[], &[], &empty, &options).unwrap().len(), 1);
    options.fee_pct_max = 0.0;
    options.fee_sats_min = 2500;
    assert_eq!(suggest(&pop, &[], &[], &empty, &options).unwrap().len(), 1);
    options.include_heuristics = false;
    assert!(suggest(&pop, &[], &[], &empty, &options)
        .unwrap()
        .is_empty());
    options.include_heuristics = true;
    let mut returned = suggest(&pop, &[], &[], &empty, &options).unwrap();
    kassiber_envelope::set(
        std::sync::Arc::make_mut(&mut returned[0].data),
        "confidence",
        kassiber_envelope::s("modified"),
    );
    assert_eq!(
        bytes(&baseline[0].data),
        bytes(&suggest(&pop, &[], &[], &empty, &options).unwrap()[0].data)
    );
}
