//! Checks `ledger::text` against RP2's own `str(transaction)`.
//!
//! The v1 fixtures store RP2's rendering as an entry's `text` for every
//! entry of the hand-written generic histories and wherever an expected
//! message embeds it (`gen_engine_fixtures.py`). Each such entry is
//! constructed again and rendered by the engine; every mismatch is reported
//! at once. The edge cases below were rendered by RP2 on CPython 3.11
//! (Linux), which `v1_request.entry_text` also matches, except the last:
//! RP2's `str()` raises for it, so its text is `v1_request.entry_text`'s.

use std::collections::BTreeMap;

use kassiber_tax_core::entry::Entry;
use kassiber_tax_core::ledger::text::entry_text;
use kassiber_tax_core::model::EntryInput;
use serde_json::Value;

const FIXTURES: &str = include_str!("data/generic_fixtures.json");
const AT_FIXTURES: &str = include_str!("data/at_fixtures.json");

/// Fewest stored texts of each entry kind, so a truncated file cannot pass.
const MIN_TEXTS: [(&str, usize); 3] = [("in", 190), ("out", 140), ("intra", 55)];

/// Most mismatches printed; all are counted.
const MAX_REPORTED: usize = 50;

/// Every fixture entry that carries a `text`, labelled by where it is.
fn stored_texts() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let generic: Value = serde_json::from_str(FIXTURES).expect("generic fixtures are JSON");
    for case in generic["cases"].as_array().expect("generic cases") {
        if case["operation"] != "compute" {
            continue;
        }
        let name = case["name"].as_str().expect("case name");
        collect(name, &case["request"]["assets"], &mut out);
    }
    let at: Value = serde_json::from_str(AT_FIXTURES).expect("Austrian fixtures are JSON");
    for scenario in at["scenarios"].as_array().expect("Austrian scenarios") {
        let name = scenario["name"].as_str().expect("scenario name");
        collect(name, &scenario["assets"], &mut out);
    }
    out
}

fn collect(name: &str, assets: &Value, out: &mut Vec<(String, Value)>) {
    for asset in assets.as_array().expect("assets") {
        for (index, entry) in asset["entries"]
            .as_array()
            .expect("entries")
            .iter()
            .enumerate()
        {
            if entry.get("text").is_some() {
                let label = format!("{name}/{}[{index}]", asset["asset"]);
                out.push((label, entry.clone()));
            }
        }
    }
}

/// Constructs the entry and renders its text, or says why it could not.
fn render(entry: &Value) -> Result<String, String> {
    let input: EntryInput =
        serde_json::from_value(entry.clone()).map_err(|e| format!("bad entry: {e}"))?;
    let entry = Entry::from_input(&input, None)
        .map_err(|e| format!("constructor failed: {:?}: {}", e.class, e.message))?;
    entry_text(&entry).map_err(|e| format!("rendering failed: {:?}: {}", e.class, e.message))
}

#[test]
fn entry_text_matches_rp2_in_the_fixtures() {
    let stored = stored_texts();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut mismatches = Vec::new();
    for (label, entry) in &stored {
        let kind = entry["kind"].as_str().expect("entry kind");
        *counts.entry(kind.to_owned()).or_default() += 1;
        let expected = entry["text"].as_str().expect("text is a string");
        match render(entry) {
            Ok(text) if text == expected => {}
            Ok(text) => mismatches.push(format!("{label}:\n{text}\n--- RP2:\n{expected}")),
            Err(problem) => mismatches.push(format!("{label}: {problem}")),
        }
    }
    for (kind, minimum) in MIN_TEXTS {
        let count = counts.get(kind).copied().unwrap_or(0);
        assert!(
            count >= minimum,
            "only {count} stored {kind} texts (want {minimum}); counts {counts:?}"
        );
    }
    let shown: Vec<&str> = mismatches
        .iter()
        .take(MAX_REPORTED)
        .map(String::as_str)
        .collect();
    assert!(
        mismatches.is_empty(),
        "{} of {} texts differ from RP2:\n{}",
        mismatches.len(),
        stored.len(),
        shown.join("\n")
    );
}

/// One entry and RP2's text for it, line by line.
struct EdgeCase {
    entry: &'static str,
    text: &'static [&'static str],
}

const EDGE_CASES: [EdgeCase; 7] = [
    // A year before 1000 (glibc's `%Y` does not pad it), microseconds, a
    // negative offset, and half-even ties at four and eight places.
    EdgeCase {
        entry: r#"{"kind": "in", "row": 1, "unique_id": "old", "asset": "BTC",
            "transaction_type": "BUY", "exchange": "A", "holder": "Profile",
            "spot_price": "1234565E-5", "crypto_in": "123456785E-9",
            "fiat_in_no_fee": "152E-2", "fiat_in_with_fee": "152E-2", "fiat_fee": "0E0",
            "ts": {"raw": "0999-06-01T12:34:56.789012-05:30", "us": -30628648503210988,
                   "offset_s": -19800, "date": "0999-06-01", "year": 999,
                   "vienna_date": "0999-06-01", "display": "0999-06-01 12:34:56.789012-05:30",
                   "iso": "0999-06-01T12:34:56.789012-05:30"}}"#,
        text: &[
            "InTransaction:",
            "id=1",
            "timestamp=999-06-01 12:34:56.789012 -0530",
            "asset=BTC",
            "exchange=A",
            "holder=Profile",
            "transaction_type=TransactionType.BUY",
            "spot_price=12.3456",
            "crypto_in=0.12345678",
            "fiat_fee=0.0000",
            "fiat_in_no_fee=1.5200",
            "fiat_in_with_fee=1.5200",
            "unique_id=old",
            "is_taxable=False",
            "fiat_taxable_amount=0.0000",
            "from_lot=",
            "to_lots=",
            "cost_basis_timestamp=999-06-01 12:34:56.789012 -0530",
        ],
    },
    // An earn type (taxable), no unique id, derived values with 20 integer
    // digits, and a +14:00 offset.
    EdgeCase {
        entry: r#"{"kind": "in", "row": 2, "asset": "BTC", "transaction_type": "INCOME",
            "exchange": "A", "holder": "Profile", "spot_price": "9876543210987654321E-2",
            "crypto_in": "123456789012345E-12",
            "ts": {"raw": "2024-02-29T23:59:59.999999+14:00", "us": 1709200799999999,
                   "offset_s": 50400, "date": "2024-02-29", "year": 2024,
                   "vienna_date": "2024-02-29", "display": "2024-02-29 23:59:59.999999+14:00",
                   "iso": "2024-02-29T23:59:59.999999+14:00"}}"#,
        text: &[
            "InTransaction:",
            "id=2",
            "timestamp=2024-02-29 23:59:59.999999 +1400",
            "asset=BTC",
            "exchange=A",
            "holder=Profile",
            "transaction_type=TransactionType.INCOME",
            "spot_price=98765432109876543.2100",
            "crypto_in=123.45678901",
            "fiat_fee=0.0000",
            "fiat_in_no_fee=12193263113702112470.5228",
            "fiat_in_with_fee=12193263113702112470.5228",
            "unique_id=",
            "is_taxable=True",
            "fiat_taxable_amount=12193263113702112470.5228",
            "from_lot=",
            "to_lots=",
            "cost_basis_timestamp=2024-02-29 23:59:59.999999 +1400",
        ],
    },
    // A negative-zero fee keeps its sign; a tie rounds to the even digit.
    EdgeCase {
        entry: r#"{"kind": "out", "row": 3, "unique_id": "s1", "asset": "BTC",
            "transaction_type": "SELL", "exchange": "B", "holder": "Profile",
            "spot_price": "1234575E-5", "crypto_out_no_fee": "5E-1", "crypto_fee": "-0E-8",
            "fiat_out_no_fee": "617E-2",
            "ts": {"raw": "2023-07-01T10:00:00+02:00", "us": 1688198400000000,
                   "offset_s": 7200, "date": "2023-07-01", "year": 2023,
                   "vienna_date": "2023-07-01", "display": "2023-07-01 10:00:00+02:00",
                   "iso": "2023-07-01T10:00:00+02:00"}}"#,
        text: &[
            "OutTransaction:",
            "id=3",
            "timestamp=2023-07-01 10:00:00.000000 +0200",
            "asset=BTC",
            "exchange=B",
            "holder=Profile",
            "transaction_type=TransactionType.SELL",
            "spot_price=12.3458",
            "crypto_out_no_fee=0.50000000",
            "crypto_fee=-0.00000000",
            "unique_id=s1",
            "is_taxable=True",
            "fiat_taxable_amount=6.1700",
        ],
    },
    // A fee-only spend before 1970 (a negative instant with a fraction).
    EdgeCase {
        entry: r#"{"kind": "out", "row": 4, "unique_id": "7", "asset": "BTC",
            "transaction_type": "FEE", "exchange": "A", "holder": "Profile",
            "spot_price": "100E0", "crypto_out_no_fee": "0E0", "crypto_fee": "1E-5",
            "ts": {"raw": "1969-12-31T23:59:59.5Z", "us": -500000, "offset_s": 0,
                   "date": "1969-12-31", "year": 1969, "vienna_date": "1970-01-01",
                   "display": "1969-12-31 23:59:59.500000+00:00",
                   "iso": "1969-12-31T23:59:59.500000+00:00"}}"#,
        text: &[
            "OutTransaction:",
            "id=4",
            "timestamp=1969-12-31 23:59:59.500000 +0000",
            "asset=BTC",
            "exchange=A",
            "holder=Profile",
            "transaction_type=TransactionType.FEE",
            "spot_price=100.0000",
            "crypto_out_no_fee=0.00000000",
            "crypto_fee=0.00001000",
            "unique_id=7",
            "is_taxable=True",
            "fiat_taxable_amount=0.0010",
        ],
    },
    // A move with a fee: the sent amount and the price both round at a tie.
    EdgeCase {
        entry: r#"{"kind": "intra", "row": 5, "unique_id": "m1", "asset": "BTC",
            "from_exchange": "A", "from_holder": "Profile", "to_exchange": "B",
            "to_holder": "Profile", "spot_price": "4321000005E-5",
            "crypto_sent": "1000000005E-9", "crypto_received": "99999999E-8",
            "ts": {"raw": "2021-02-28T23:00:00+00:00", "us": 1614553200000000,
                   "offset_s": 0, "date": "2021-02-28", "year": 2021,
                   "vienna_date": "2021-03-01", "display": "2021-02-28 23:00:00+00:00",
                   "iso": "2021-02-28T23:00:00+00:00"}}"#,
        text: &[
            "IntraTransaction:",
            "id=5",
            "timestamp=2021-02-28 23:00:00.000000 +0000",
            "asset=BTC",
            "from_exchange=A",
            "from_holder=Profile",
            "to_exchange=B",
            "to_holder=Profile",
            "transaction_type=TransactionType.MOVE",
            "spot_price=43210.0000",
            "crypto_sent=1.00000000",
            "crypto_received=0.99999999",
            "crypto_fee=0.00000002",
            "fiat_fee=0.0006",
            "unique_id=m1",
            "is_taxable=True",
            "fiat_taxable_amount=0.0006",
        ],
    },
    // A move without a fee or a price, at a one-minute negative offset.
    EdgeCase {
        entry: r#"{"kind": "intra", "row": 6, "unique_id": "m2", "asset": "BTC",
            "from_exchange": "B", "from_holder": "Profile", "to_exchange": "A",
            "to_holder": "Profile", "crypto_sent": "2E0", "crypto_received": "2E0",
            "ts": {"raw": "2021-03-01T00:00:00.000001-00:01", "us": 1614556860000001,
                   "offset_s": -60, "date": "2021-03-01", "year": 2021,
                   "vienna_date": "2021-03-01", "display": "2021-03-01 00:00:00.000001-00:01",
                   "iso": "2021-03-01T00:00:00.000001-00:01"}}"#,
        text: &[
            "IntraTransaction:",
            "id=6",
            "timestamp=2021-03-01 00:00:00.000001 -0001",
            "asset=BTC",
            "from_exchange=B",
            "from_holder=Profile",
            "to_exchange=A",
            "to_holder=Profile",
            "transaction_type=TransactionType.MOVE",
            "spot_price=0.0000",
            "crypto_sent=2.00000000",
            "crypto_received=2.00000000",
            "crypto_fee=0.00000000",
            "fiat_fee=0.0000",
            "unique_id=m2",
            "is_taxable=False",
            "fiat_taxable_amount=0.0000",
        ],
    },
    // A move whose fee reaches 1E19: comparing it with zero at 13 places
    // needs 33 digits, so `is_taxable` cannot be decided and the text says
    // `None`, as the derived value `v1_request` reads is (RP2 raises).
    EdgeCase {
        entry: r#"{"kind": "intra", "row": 7, "unique_id": "m3", "asset": "BTC",
            "from_exchange": "A", "from_holder": "Profile", "to_exchange": "B",
            "to_holder": "Profile", "spot_price": "1E18",
            "crypto_sent": "11E0", "crypto_received": "1E0",
            "ts": {"raw": "2024-01-01T00:00:00+00:00", "us": 1704067200000000,
                   "offset_s": 0, "date": "2024-01-01", "year": 2024,
                   "vienna_date": "2024-01-01", "display": "2024-01-01 00:00:00+00:00",
                   "iso": "2024-01-01T00:00:00+00:00"}}"#,
        text: &[
            "IntraTransaction:",
            "id=7",
            "timestamp=2024-01-01 00:00:00.000000 +0000",
            "asset=BTC",
            "from_exchange=A",
            "from_holder=Profile",
            "to_exchange=B",
            "to_holder=Profile",
            "transaction_type=TransactionType.MOVE",
            "spot_price=1000000000000000000.0000",
            "crypto_sent=11.00000000",
            "crypto_received=1.00000000",
            "crypto_fee=10.00000000",
            "fiat_fee=10000000000000000000.0000",
            "unique_id=m3",
            "is_taxable=None",
            "fiat_taxable_amount=10000000000000000000.0000",
        ],
    },
];

#[test]
fn entry_text_matches_rp2_on_edge_cases() {
    for (index, case) in EDGE_CASES.iter().enumerate() {
        let entry: Value = serde_json::from_str(case.entry).expect("edge case is JSON");
        assert_eq!(
            render(&entry).as_deref(),
            Ok(case.text.join("\n  ").as_str()),
            "edge case {index}"
        );
    }
}
