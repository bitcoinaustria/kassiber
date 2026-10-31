# R4-12 journal persistence checkpoint

R4-12 has an unregistered native persistence service and complete logical-dump
oracles. **Acceptance remains open.** The performance budget, exhaustive
persistence-branch mutation coverage and inherited full R4 campaigns must close
before this becomes production authority. Python remains the reference. No CLI
or daemon registration changes are included.

## Implementation and ownership

`crates/journals/src/persistence.rs::store_ledger_state` consumes the R4-11
`JournalState`, the profile, an injected clock and a caller-supplied UUID source.
It binds SQLite scalars through the existing store driver and prepares each
table's write statement once. The transitional stage-output JSON seam remains;
the ledger response and rendered entries retain native types. No dependency
version changes are required.

The service mirrors compact v2 calculation bytes, UUIDv8 entry ids, column-tuple
comparison excluding `created_at`, timestamp-group rewrites, signed-zero REAL
comparison, unchanged rowid retention and quantity-posting deltas. Small derived
tables are replaced. An absent quantity stage preserves its old quantity tables
and still clears reconciliation proofs, following Python. Quarantine
deduplication retains Python's first-seen order,
root precedence, carried reasons, nonfinite legacy JSON and escaped surrogates;
storage filters synthetic transaction ids and enriches live ownership candidates.
Holdings, exact prices, freshness counts and input versions follow Python.

Processing markers are read from the database and advance by one microsecond
when the injected clock repeats or moves backwards. Row timestamps and filed
resolution timestamps use the original clock. This preserves explanation expiry
without changing unchanged entry timestamps. Filed-impact summaries and chain
authority checks reuse the existing reports and wallets services. Quarantines,
component blockers and quantity issues retain the report-readiness barrier.

The caller owns `BEGIN IMMEDIATE`, the encompassing `journals_process`
savepoint, all pre-build repairs and COMMIT. The writer additionally holds a
rollback-on-error/unwind savepoint. It cannot prove which BEGIN mode a caller
used; R4-13 must acquire the write lock before reading the snapshot and unwind
the outer savepoint on any build or persistence failure.

`storage::RebuildCache` scopes `cache_size=-131072` and restores its exact signed
previous value on success, refusal or unwind. It leaves spill settings intact.
`storage::DeferredCheckpoint` disables automatic WAL checkpoints before BEGIN,
runs PASSIVE after COMMIT and the response, then restores the prior setting.
SQLCipher remains DELETE/FULL and receives no WAL checkpoint. These guards are
caller-owned so they can cover the complete rebuild transaction.

## Dump oracle and corpora

Captures preserve every table, column, rowid, id and timestamp. Both runtimes
start from the same logical snapshot; benchmark trials copy the same physical
starting database. Nothing is masked. SQL scalar types are preserved and REAL
values are compared by all 64 bits; JSON exponent spelling is transport syntax.
The current schema has 112 tables.

The twelve generator families are sampled once per scenario. Raw-stage
generators can clone an import fingerprint without creating a database. The
harness gives those clones distinct fingerprints **before** stage computation
and database insertion in either runtime. Schema-invalid or pre-persistence
refusals remain separately recorded; they do not count as matched persistence.
The sampled harness materializes storage-relevant scope, observations and
component headers; it supplies the already-computed stage at the boundary.
It does not recreate every raw custody leg or evidence row and therefore is
not a whole-pipeline custody oracle.

| Corpus | Complete dumps matched |
| --- | ---: |
| Twelve sampled generator families | 502 / 502 |
| Separate payout books | 10 / 10 |
| Focused census captures | 69 / 69 |
| Focused custody-shadow captures | 75 / 75 |
| Frozen persistence cases | 26 / 26 |
| Total (frozen cases can overlap captures) | 682 / 682 |

Thirteen sampled generator cases refused at schema validation or upstream of
persistence. Three payout inputs (`legacy_NaN`, `legacy_inf`, `legacy_junk`)
refused in Python before persistence. Neither group is counted as parity.
The 26 frozen cases also pass on physical SQLCipher DELETE/FULL books, including
reopen verification. Fourteen base fixtures are replayed through Python in
`test_rust_journal_persistence.py`; the other twelve retain captured stage
operands. The benchmark adds physical-copy parity for fresh, unchanged,
1% appended and 5% changed journals.

The focused census run passed 99 tests. The focused shadow run passed 128 tests
and two subtests, with one pre-existing read-budget failure:

```sh
KASSIBER_TEST_CUSTODY_ENGINE=shadow uv run --locked python -m pytest \
  'tests/test_custody_native_reconciliation.py::test_disjoint_reviewed_routes_limit_observation_reads[True]' -q
```

This also fails without the persistence capture plugin: shadow executes the
reference work again, giving 5,254 observation reads against the 3,840 budget.
The failure is preserved as a harness/performance repro; Python product code is
unchanged. The broader census/shadow campaigns have not been run here.

## Verification and measurements

`every_write_failure_and_panic_rolls_back_the_journal` injects both a returned
error and an unwind after each of 604 successful DML statements across the
26 frozen stores: 1,208 rollback cases. The physical cipher case adds 38
injections over its 19 writes, for **1,246 injections**. Complete dumps match
the stale starting database both after the writer's inner rollback and after
the caller rolls back its journal savepoint and transaction. Cache and
checkpoint tests separately pin signed setting restoration, DELETE/FULL,
deferred PASSIVE behavior and a retained WAL reader.

The complete journal mutation catalog kills **112 / 112** recipes: 61 inherited
render/join recipes and 51 persistence recipes, with zero survivors or build
errors. All 20 named baseline filters pass before mutation. Every recipe names
its failing test in `crates/journals/scripts/mutations.json`; a build failure
does not count as a kill. The disposable build tree was removed automatically.
The broader branch audit and inherited custody campaign remain open.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Offline workspace clippy, all targets, `-D warnings` | Pass |
| Offline workspace tests, one test thread | 1,510 passed; zero failed or ignored |
| Journal package tests | 24 passed |
| Python render and storage oracles | 8 passed |
| Required inventory update and preflight `--ci` | Updated; 57 passed |
| Capture target guard, empty-corpus refusal and Python syntax | Pass |

The first sandboxed workspace run could not bind local mock sockets in twelve
chain tests. The same complete workspace command passed after sandbox escalation;
no test remains blocked by the sandbox. Preflight refreshed eight generated
error-index source references for the prerequisite storage changes. The focused
shadow read-budget repro above remains separate from these passing gates.

### Persistence timing

The books were built by `scripts/journals-bench-books.py` at `4f6ca7d06`,
with 20,000 transactions, generic seed 1 and multi-wallet seed 7. Generic
has 20,000 entries and no quarantines. Multi-wallet has 19,983 entries,
17 quarantines and 300 paired pegs. The SQLCipher matrix encrypts a copy of
the generic book using a synthetic test key. No real books are opened.

The timers cover storage and COMMIT. Rendering, request decoding, complete
dump comparison and the deferred checkpoint are outside the timer. The floor
replays Python's exact recorded DML using prepared statements and scalar
values converted before timing; binding, execution and COMMIT remain timed.
It therefore measures the new compact format and changed-row writes rather
than reusing the old-format floor. Python and Rust alternate back to back;
all modes use five pairs because the one-minute load exceeds 8. The minima
for Python, Rust and floor are selected independently. Every measured store
also passes the unmasked dump oracle.

| Book / rebuild | Python (s) | Rust (s) | Floor (s) | Rust − floor (s) | Load 1m range |
| --- | ---: | ---: | ---: | ---: | --- |
| generic / fresh | 1.325212 | 1.389373 | 0.665849 | 0.723524 | 18.099–21.243 |
| generic / unchanged | 1.054679 | 0.988387 | 0.027396 | 0.960991 | 15.845–17.524 |
| generic / appended | 1.725385 | 0.986117 | 0.034397 | 0.951720 | 14.970–27.080 |
| generic / changed | 1.208219 | 1.142653 | 0.187284 | 0.955369 | 19.834–21.690 |
| multi-wallet / fresh | 1.636201 | 1.539973 | 0.837604 | 0.702369 | 17.289–19.870 |
| multi-wallet / unchanged | 1.185058 | 1.038579 | 0.039320 | 0.999259 | 21.156–25.740 |
| multi-wallet / appended | 1.212678 | 1.039325 | 0.032752 | 1.006573 | 20.763–22.720 |
| multi-wallet / changed | 1.336596 | 1.191396 | 0.191914 | 0.999481 | 17.587–18.867 |
| cipher / fresh | 1.613862 | 1.582221 | 0.784623 | 0.797598 | 17.134–17.706 |
| cipher / unchanged | 1.266166 | 1.127561 | 0.070325 | 1.057236 | 15.237–15.624 |
| cipher / appended | 1.244402 | 1.095607 | 0.066551 | 1.029056 | 14.887–16.046 |
| cipher / changed | 1.709245 | 1.551336 | 0.465619 | 1.085717 | 13.916–19.496 |

**The ≤ floor + 0.15 s target fails in every measured mode.** The fresh
plaintext overhead is 0.724 s generic and 0.702 s multi-wallet. Unchanged and
partial rebuilds still prepare and compare complete entry projections. Further
profiling and native calculation/scalar preparation work remain; these results
do not establish which part dominates.

[The measurement record](r4-journal-persistence-measurements.json) retains every
trial, load, source hash, CSV hash and corpus/write summary. Raw captures,
benchmark copies and disposable mutation build trees are removed after their
results are recorded.

## R4-13 handoff and remaining acceptance

- Keep the 18 borrowed fields and their owners alive through persistence. Supply
  Python-equivalent ledger response and completed stage outputs at the existing
  boundary until the native pipeline replaces them.
- Acquire BEGIN IMMEDIATE before reads and repairs; own the outer journal
  savepoint, cache scope, rollback/unwind handling and COMMIT. Do not allow a
  caught storage error to commit pre-build repairs or fresh markers.
- Open books with the shared connection policy: plaintext WAL/NORMAL and
  SQLCipher DELETE/FULL during coexistence. The writer consumes that connection;
  it does not change journal mode inside a pending transaction.
- Use one injected clock for row/impact timestamps and a UUID provider for small
  replacement tables. Entry construction consumes no UUID4 values.
- Publish CLI/daemon success only after COMMIT; invoke the passive checkpoint
  after the reply/response record. Restore connection settings on every exit.
- Preserve report barriers and explanation binding. Do not expose this probe,
  SQL floor replay or arbitrary filesystem/SQL dispatch to AI tools.
- Close the 20k floor-plus-0.15 s budget, exhaustive persistence mutations and
  the inherited 24,000-book/permutation, recording/replay, full census/shadow and
  2,285-recipe custody campaign gates. This checkpoint does not claim them.

The remaining mutation audit must add named witnesses for every compact-shape
validation/refusal, binding-width and invalid-shape refusal, missing/inactive or
revision-mismatched reconciliation component, economic-relation missing/fallback
and nonpositive-amount path, filed-chain availability/authority path, and cache
or checkpoint refusal/restoration path. The checked catalog samples these services;
its kill count alone is not proof of exhaustive branch coverage. The complete
inherited campaign also includes 1,030 generator permutations, 159 recorded
requests and their permutations plus fresh recordings, and the 2k benchmark
comparisons. None of those full campaigns is replaced by the focused captures.

## Reproduction

Use `MATURIN_NO_INSTALL_RUST=1 mise exec rust@1.99.1 uv@latest --` for commands;
the environment was synchronized with `uv sync --locked --python 3.11`.
Cargo runs offline. Point `TMPDIR` at an existing directory under worktree
`target/`; a read-only shared uv cache can use `UV_NO_SYNC=1` and
`UV_CACHE_DIR=target/uv-cache` after synchronization. Never use a real book.

```sh
cargo test --offline -p kassiber-journals -- --test-threads=1
uv run --locked python -m pytest tests/test_rust_journal_persistence.py -q
cargo build --offline --release -p kassiber-journals --example journal-persist
uv run --locked python scripts/capture-journal-persistence-samples.py target/r4-persist/samples
uv run --locked python scripts/capture-journal-persistence-samples.py target/r4-persist/payouts --payouts
uv run --locked python scripts/verify-journal-persistence.py target/r4-persist/samples/*/ target/r4-persist/payouts/payouts --probe target/release/examples/journal-persist --summary target/r4-persist/parity.json
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/generic-20000 --out target/r4-persist/timings/generic
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/multi-wallet-20000 --out target/r4-persist/timings/multi-wallet
uv run --locked python scripts/benchmark-journal-persistence.py --book target/r4-persist/bench/generic-20000 --out target/r4-persist/timings/cipher --cipher
uv run --locked python crates/journals/scripts/mutation_check.py
cargo test --offline --workspace -- --test-threads=1
uv run --locked python -m tests.rust_port.classify_tests update
uv run --locked python scripts/python_test_shards.py run preflight --ci
```

Build bench books first with `scripts/journals-bench-books.py`, kinds `generic`
and `multi-wallet`, 20,000 transactions. Pairs alternate Python/Rust order and
record one-minute load for each runtime and floor run. At load above 8 use the
best of five. Rendering and dump comparison are outside the timer; persistence
and COMMIT are inside, and the deferred checkpoint is outside. The floor replays
the exact reference DML with statements prepared and scalars converted before timing,
including the new compact format and changed-row behavior.
