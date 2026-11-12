# Transaction page index implementation evidence

> **Historical.** This records evidence gathered while Python and Rust both
> opened books. Python was removed under [plan 22](../plan/22-python-removal.md);
> the Python columns and steps below exist only at the `archive/python-backend`
> tag.

Local branch `t3code/page-index`, based on `f261b9b2`. Only the approved
`idx_transactions_profile_page` index is added. The flow and metrics proposals
remain unimplemented. See [database compatibility](database-compatibility.md#transaction-page-index)
for the definition, ordering, old-Rust refusal and Python cookie limitation.

## Migration matrix

| Starting state / failure | Plaintext | SQLCipher | Assertions |
| --- | --- | --- | --- |
| Fresh Python and Rust books | Pass | Pass | Generated creation/current-schema parity and integrity |
| Populated current book without page index, Python first | Pass | Pass | Exact typed logical rows, first-open index, subsequent Python/Rust schema-object parity |
| Populated current book without page index, Rust first | Pass | Pass | Same; includes BTC integer above float exact-integer range |
| Both original historical fixtures, Python first | Pass | Pass | Complete first-open dump unchanged by subsequent Rust/Python opens |
| Both pair-free historical variants, Rust first | Pass | Pass | Full first-pass Python/Rust dump parity, integrity/FK checks and subsequent opens |
| Both original historical fixtures, Rust first | Existing R2 refusal | Existing R2 refusal | Exact Python-prefix parity; authored consolidation still requires Python |
| Injected failure before/after page creation | Pass | Pass | No page index survives; retry succeeds |
| Process killed after CREATE before commit, Python/Rust recovery | Pass | Pass | Complete pre-index logical dump retained; retry completes |
| Busy/full SQLite storage | Python + Rust pass | Not separately injected | Schema cookie/index absence retained; Rust retries successfully |
| Interrupted SQLite CREATE | Python pass | Not separately injected | Trace arms progress interruption at CREATE, rollback leaves no index |
| Unchanged isolated page helper | Python + Rust pass | Reopens covered above | No extra schema-cookie change; Python rootpage preserved |
| Conflicting same-name definition | Python pass | Not separately injected | Existing definition retained and error returned |

The current-book test compares all tables including SQLite storage classes,
not only transactions or counts. The complete migration/fresh-schema selection
passed **280 tests**; a subsequent populated-book strengthening passed all
**4** parameter combinations. The page-specific selection passed **20 tests**.

## Measurements

Milliseconds, before → after, best of three complete subprocess runs. Python
and Rust run back to back for each trial. Both supplied books are copied; no
original book is opened. Baseline Python sources and an ordinary release Rust
executable come from `f261b9b2`; candidate Rust is an ordinary release build,
without test-harness instrumentation. The locked Python environment and Rust
1.99.1 use mise, offline Cargo and `MATURIN_NO_INSTALL_RUST=1`. `HOME` is never
changed by benchmark commands. `KASSIBER_NO_EGRESS=1` is set.

A preliminary Python open finishes upgrades outside the read timings. Desktop
measurement includes process startup, daemon-ready, one `ui.transactions.list`
request (page and exact count), and shutdown. It is not a count-only SQL span.
CSV trials each restore an independent copy and import 100 new transaction IDs;
row counts must increase by exactly 100. CLI stdout/stderr/status and desktop
response bytes match across both implementations and both index states.

| Operation | Existing rows | Python ms | Rust ms |
| --- | ---: | ---: | ---: |
| CLI list, limit 100 | 2,000 | 637.3 → 5456.4 | 25.9 → 42.1 |
| CLI list, limit 100 | 20,000 | 888.2 → 1967.5 | 99.3 → 97.2 |
| Desktop list + exact count, limit 100 | 2,000 | 681.3 → 2460.0 | 61.8 → 117.4 |
| Desktop list + exact count, limit 100 | 20,000 | 731.1 → 1186.5 | 315.2 → 311.0 |
| Import CSV, 100 new transactions | 2,000 | 1132.0 → 1060.5 | 223.2 → 261.5 |
| Import CSV, 100 new transactions | 20,000 | 1105.6 → 735.5 | 507.4 → 265.4 |

The host was heavily loaded (load averages reached **121 / 71 / 39**), and
these wall-clock samples are noisy. An earlier complete run measured the
20,000-row Rust CLI at **83.6 → 41.6 ms**, desktop page/count at
**284.6 → 162.4 ms**, and import at **211.5 → 270.0 ms**. The final run above
does not establish a stable write-cost delta or a precise causal speedup;
Python startup in particular varied by seconds. Neither run is a latency SLA.

## Verification

- Store suite: **61 passed**, no failures; includes Rust busy/full/rollback retry
  and isolated schema-cookie idempotence.
- Workspace/all-feature offline tests: **1,197 passed**, zero failed/ignored,
  across **92** test/doctest groups. The restricted initial run could not bind
  loopback fixtures. An intermediate run had a rustdoc missing-crate error
  during competing builds; the final complete run passed after those finished.
- Workspace/all-target/all-feature clippy with `-D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- All three normal schema generators regenerated and their `--check` modes
  passed. The error-code drift snapshot changes only db.py source locations.
- `classify_tests update`: passed. Preflight: **57 passed, 57 subtests passed**.
- Hybrid transaction listing/desktop parity: **8 passed, 57 subtests passed**.
  Route logs recorded **159** CLI listing calls and **241** desktop page calls,
  all Rust, **zero fallbacks** for these targets. Setup routes are outside that
  assertion.
- Baseline Rust accepts a control book, then returns
  `requires_python_migration` after adding only the page index. Admission was
  not weakened. Compatible Rust must precede Python's upgrade.
- Manual second pass and whitespace review completed. No push or PR.

Whole-open Python schema-cookie stability remains blocked by its pre-existing
unrelated DDL churn; the new index itself is stable. Full Rust migration of
original authored historical fixtures remains blocked by the existing R2
boundary. Reliable index-only end-to-end performance attribution is limited by
host load. None of these limitations is silently waived.
