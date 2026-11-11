//! No replay normalizer may rewrite a figure: amounts, msat values, prices,
//! fees, gains, bases, tax figures, or the dates of tax events (plan 22,
//! "Comparison rules"). The tax, journal and report vectors are run through
//! the same canonical rewrite a replay applies, with a clock window at the
//! time the test runs (as a replay's is), and every figure field must come
//! out byte for byte.

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use kassiber_session_replay::compare::{Canon, Window};
use regex::Regex;

/// Locked test data holding tax, journal and report envelopes.
const VECTORS: &[&str] = &[
    "docs/reference/tax-country-ch-vectors.json",
    "docs/reference/tax-country-de-vectors.json",
    "docs/reference/tax-country-es-vectors.json",
    "docs/reference/tax-country-gb-vectors.json",
    "docs/reference/tax-country-ie-vectors.json",
    "docs/reference/tax-country-jp-vectors.json",
    "docs/reference/tax-country-us-vectors.json",
    "tax-engine/core/tests/data/at_fixtures.json",
    "tax-engine/core/tests/data/generic_fixtures.json",
    "tax-engine/core/tests/data/ledger_fixtures.json",
    "tax-engine/core/tests/data/us_fixtures.json",
    "crates/journals/tests/data/msat-machine.json",
    "crates/journals/tests/data/storage.json",
    "crates/journals/tests/data/persistence.json",
    "crates/reports/tests/data/exit_tax.json",
    "crates/reports/tests/data/gain_reads.json",
    "crates/reports/tests/data/artifact_csv.json",
    "crates/reports/tests/data/commercial.json",
    "crates/reports/tests/data/export_requests.json",
];

fn figures(text: &str) -> Vec<String> {
    // A figure field and its scalar value, as JSON (also JSON in a string)
    // or as a dumped book row.
    let pattern = Regex::new(concat!(
        r#"(?i)\\?"[A-Za-z0-9_]*(?:amount|msat|sats?\b|price|fee|gain|loss|basis|proceeds|tax|cost|value|quantity|balance|total|fiat|eur|usd|income|rate|date|occurred|_at\b|timestamp)[A-Za-z0-9_]*\\?"\s*:\s*"#,
        r#"(?:-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|\\?"(?:[^"\\]|\\.)*?\\?")"#,
        r#"|\b[A-Za-z0-9_]*(?:amount|msat|price|fee|gain|basis|proceeds|tax|cost|value|quantity|balance|total|date|occurred_at)[A-Za-z0-9_]*=(?:int|real) -?[\d.]+"#,
    ))
    .unwrap();
    let mut found: Vec<String> = pattern
        .find_iter(text)
        .map(|m| m.as_str().to_owned())
        // Placeholders a rewrite produces are not input figures.
        .filter(|f| !f.contains("<gen") && !f.contains("<time>"))
        .collect();
    found.sort();
    found
}

fn now_window() -> Window {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    Window {
        start: now - 10_000_000,
        end: now,
        local_offset_us: 0,
        also: vec![(now - 610_000_000, now - 600_000_000)],
    }
}

fn assert_figures_survive(name: &str, text: &str, window: Window) {
    let pinned = HashSet::new();
    let mut canon = Canon::new(window, &pinned);
    let rewritten = canon.rewrite(text, true);
    let (before, after) = (figures(text), figures(&rewritten));
    let lost: Vec<&String> = before
        .iter()
        .filter(|f| after.binary_search(f).is_err())
        .take(5)
        .collect();
    assert!(
        lost.is_empty() && before.len() == after.len(),
        "{name}: the replay normalizers rewrote figures ({} before, {} after), e.g. {lost:?}",
        before.len(),
        after.len()
    );
}

#[test]
fn normalizers_leave_tax_journal_and_report_figures_alone() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut checked = 0;
    for vector in VECTORS {
        let path = repo.join(vector);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{vector}: {e} (run scripts/fetch-test-data.sh)"));
        if figures(&text).is_empty() {
            // Unkeyed vectors (`[[btc, msat], ...]`): nothing may change.
            let pinned = HashSet::new();
            let rewritten = Canon::new(now_window(), &pinned).rewrite(&text, true);
            assert!(
                rewritten == text,
                "{vector}: the replay normalizers rewrote it"
            );
        } else {
            assert_figures_survive(vector, &text, now_window());
        }
        checked += 1;
    }
    assert_eq!(checked, VECTORS.len());
}

#[test]
fn figures_that_look_like_clock_readings_stay() {
    // An msat amount equal to the current millisecond epoch, a price equal
    // to the current second, a fee of fractional seconds, a dumped row, and
    // a tax event dated inside the window's 90-day expiry offset all stay.
    let window = now_window();
    let ms = window.end / 1000;
    let secs = window.end / 1_000_000;
    let envelope = format!(
        concat!(
            r#"{{"kind":"tax.summary","data":{{"amount_msat":{ms},"price_eur":"{secs}","#,
            r#""fee_msat":{secs},"cost_basis":{secs}.25,"proceeds":"{ms}","#,
            r#""inner":"{{\"gain_msat\":{ms}}}"}}}}"#,
            "\namount_msat=int {ms} | fee_msat=int {secs}\n"
        ),
        ms = ms,
        secs = secs
    );
    assert_figures_survive("synthetic", &envelope, window.clone());
    // The same numbers under a clock field are clock readings.
    let pinned = HashSet::new();
    let mut canon = Canon::new(window, &pinned);
    assert_eq!(
        canon.rewrite(&format!(r#"{{"updated":{ms}}}"#), true),
        r#"{"updated":<time>}"#
    );
}

#[test]
fn derived_ids_number_as_a_bijection() {
    // Equal values share a number and different values get different ones,
    // so a row pointing at the wrong derived id still differs.
    let pinned = HashSet::new();
    let window = now_window();
    let a = "11111111-2222-5333-8444-555555555555";
    let b = "21111111-2222-5333-8444-555555555555";
    let c = "31111111-2222-5333-8444-555555555555";
    let token = |n: u8| format!("chat-{:032x}", n);
    let recorded = format!("{a} {b} {a} {} {}", token(1), token(1));
    let right = format!("{c} {b} {c} {} {}", token(2), token(2));
    let swapped = format!("{c} {b} {b} {} {}", token(2), token(2));
    let split = format!("{c} {b} {c} {} {}", token(2), token(3));
    let rewrite = |text: &str| Canon::new(window.clone(), &pinned).rewrite(text, true);
    assert_eq!(rewrite(&recorded), rewrite(&right));
    assert_ne!(rewrite(&recorded), rewrite(&swapped));
    assert_ne!(rewrite(&recorded), rewrite(&split));
}
