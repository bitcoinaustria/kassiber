# R4-11 journal rendering checkpoint

This is a partial implementation on `1e6565c8`, dated 2026-10-30. R4-11
acceptance remains open. Python still builds and stores every production
journal. The Rust crate is available only as a library and a local test probe.
There is no CLI/daemon registration, persistence, SQL, network access, clock
read or entropy source in the crate.

A [local performance follow-up](r4-journal-render-performance-checkpoint.md)
reaches both 20k render budgets and records expanded verification. The
lossless boundary and full branch mutation requirements remain open.

## Implemented boundary

`kassiber_journals::boundary::build` accepts the profile, today's serialized
custody projection, wallet references in their original insertion order, the
typed `kassiber_tax_core::ledger::response::LedgerResponse`, and a caller-supplied
UUID function. `boundary::ledger_response` decodes the current wire response
for tests; native orchestration can pass the engine result directly.

Rendered entries, calculations, fragments, source provenance, holdings and
tax summaries have native structs. Quantities and basis stay `pydecimal`;
msat conversion preserves Python's 32-digit context and half-up rounding.
Fragment provenance shares immutable source facts. The adapter retains
dynamic row scalars and pass-through custody facts as `serde_json::Value`.
Typed custody/loader integration must replace those accesses, including the
row reads in `render.rs`; this is not yet the final typed pipeline.

The port covers acquisition, realized and transfer entries, carrying-price
provenance, Austrian category fields, fragment basis authority, quarantine
templates, two-level holdings, direct-payout compatibility metadata, balance
comparison reasons, and ownership review selection. It reuses the custody
graph parser, scope/ownership helpers and balance comparator. Review
candidates remain suggestions and grant no ownership or basis authority.

The new workspace member uses only dependencies already locked in this
checkout; no external package or runtime requirement was added.
The dependency review list admits the existing `kassiber-custody/timing`
feature, which only records static span names and durations in thread-local
RAM and is off by default. Serde's `rc` feature is not enabled: the source
provenance field serializes through an explicit shared-pointer dereference.

Python `build()` retains **raw quarantines**. Deduplication, live-transaction
filtering and ownership-candidate enrichment happen in `store_ledger_state`.
The crate therefore exposes `enrich_quarantine_detail` separately for R4-12.

## Oracle and sampled parity

`tests/rust_port/journal_render_oracle.py` runs the actual Python `build()`
with an already computed custody projection and captures every operand and
complete canonical output. The optional pytest/child hook is enabled only by
`KASSIBER_TEST_JOURNAL_RENDER_DIR`. It records an explicit bijection for every
UUID generated inside `read_ledger`; both sides use the same declared UUIDs.
Nothing is masked. The reproducible synthetic vectors use seeded UUIDv4s.

Canonical state JSON uses the existing custody oracle's Decimal, tuple-key
mapping and dataclass encodings. Refusals retain family, message, code, hint,
details, retryability, engine error class, invariant and explicit cause.
State and refusal comparisons compare canonical bytes. `created_at` does
not exist in pre-persistence `build()` state; the recorded `as_of` is fixed
by the supplied fixture. R4-12 must inject the persistence clock when
comparing complete logical database dumps.

The focused census and shadow runs each passed 108 tests and 12 subtests.
Both compared all seven custody/fusion stages on 65 builds; shadow also
performed 65 frozen Python comparisons. These are focused runs, not the
complete census or shadow lanes.

| Corpus | Compared | Byte matches | Rendering refusals |
| --- | ---: | ---: | ---: |
| Focused census | 65 | 65 | 0 |
| Focused custody shadow | 65 | 65 | 0 |
| Twelve generator families | 508 | 508 | 0 |
| Additional generated tax cases | 145 | 145 | 25 |
| Copied bench books (generic/multi-wallet, 2k/20k) | 4 | 4 | 0 |

The sampled generator run takes seeds `0..len(SCENARIOS)-1`, covering one
cycle of each family's scenarios. Activity is counted from the Python state,
not inferred from the scenario name:

| Family | Compared | Entries | Quarantines | Review candidates | Quantity differences |
| --- | ---: | ---: | ---: | ---: | ---: |
| Evidence snapshots | 28 | 17 | 37 | 0 | 22 |
| Components | 156 | 0 | 777 | 0 | 185 |
| Samourai | 34 | 1 | 89 | 0 | 71 |
| Recorded evidence | 46 | 18 | 84 | 0 | 68 |
| Fan-out | 43 | 0 | 141 | 5 | 117 |
| Native | 53 | 21 | 103 | 0 | 89 |
| Graph | 8 | 0 | 18 | 0 | 18 |
| Channel | 32 | 3 | 72 | 0 | 42 |
| Gaps | 32 | 3 | 978 | 0 | 60 |
| Native reconciliation | 25 | 18 | 94 | 0 | 56 |
| Safety | 39 | 3 | 88 | 2 | 76 |
| Pipeline | 12 | 0 | 43 | 0 | 32 |

Three sampled books refuse before rendering (one evidence snapshot and two
native books), and four fan-out scenarios refuse during upstream projection
construction. The additional tax corpus has 21 pre-render refusals. These
are recorded separately and **are not counted as rendering parity**. Of its
166 cases, 120 render states and 25 rendering refusals were byte-compared.

The checked-in corpus has 246 reproducible synthetic vectors, including
engine error stages/classes, graph proofs, literal/hex transaction identity,
loan/Lightning difference reasons, duplicate wallet labels, holding order,
quarantine templates, scalar stringification and nested reconciliation
dataclasses. Another 80 vectors pin msat rounding and integer width.

## Mutations

All **39/39** new recipes in `crates/journals/scripts/mutations.json` were
killed by their declared named test, with passing baselines and no survivors
or build failures. The checker removes its disposable build tree afterward.

| Named test | Responsibility |
| --- | --- |
| `msat_conversion_matches_python_at_rounding_and_width_boundaries` | Sign, half-up ties, sub-msat values and wide context rounding |
| `ledger_rendering_matches_python` | Entry fields, fragments, carry provenance, transfer signs, tax categories, holdings, summaries and quarantine output |
| `ownership_reviews_and_reconciliations_match_python` | Native dataclass conversion, loan/Lightning explanations, graph amounts, review candidates and literal transaction identity |
| `refusal_mapping_matches_python` | Error wrapping, codes, message order and explicit causes |
| `quarantine_candidate_enrichment_is_compact_sorted_and_ascii` | Candidate key and compact sorted JSON layout |

This is focused mutation coverage. It does **not** establish a mutation for
every rendering/join branch, and the inherited 2,246-recipe catalog was not
rerun in this worktree. Both remain acceptance work.

## R4-12 handoff

`JournalState<'a>` has exactly the eighteen keys returned by Python's current
`build()`. Its borrowed fields require the custody projection to remain alive
through persistence. The caller can consume owned rows directly and bind
prepared statements without serializing the entire state.

| Fields | Representation and persistence contract |
| --- | --- |
| `entries` | Owned `JournalEntryRow`s; UUIDs already supplied by the caller; exact Decimal fields and optional-key presence retained |
| `quarantines` | Owned raw `QuarantineRow`s; deduplicate and filter against live transaction IDs before enrichment |
| `cross_asset_pairs` | Borrowed finalized projection pairs |
| `direct_swap_payouts` | Owned compatibility metadata, never an independent tax-event source |
| `tax_summary` | Owned summary rows, including integer `quantity_msat` beside Python-compatible float projections |
| `account_holdings`, `wallet_holdings` | Ordered tuple-key/holding pairs with exact Decimal quantity and basis |
| `ownership_review_counts`, `ownership_review_candidates` | Owned advisory projections; group candidates by `out_id` for quarantine enrichment |
| `custody_component_blockers` | Borrowed component blockers |
| `custody_native_reconciliations` | Owned recursive `asdict` projection, without dataclass tags |
| `custody_quantity`, `custody_transfers` | Borrowed custody projections |
| `quantity_differences` | Owned balance comparison, including known non-event reasons |
| `latest_rates`, `warnings` | Borrowed rate rows and warnings |
| `custody_gap_ignored_transaction_ids`, `custody_gap_accounting_ignored_transaction_ids` | Borrowed ignored-ID projections; preserve both scopes separately |

R4-12 still owns all generated persistence-row UUIDs and the injected
`created_at`, quarantine deduplication/live-ID filtering, enrichment, integer
and float SQLite bindings, prepared statements, gap input-version markers,
freshness updates, filed-report impacts, rollback and transaction boundaries.
It must compare complete logical dumps from identical starting database
copies. This checkpoint's canonical JSON serializer is an oracle helper,
not the persistence API.

## Verification

- Workspace formatting check passed.
- Offline workspace/all-target clippy passed with `-D warnings`.
- Offline workspace tests passed: **1,299 passed, 0 failed, 0 ignored**
  across 105 test/doc-test targets, with one test thread and two build jobs.
- The journal crate's five native tests and two Python oracle tests passed.
- All nine network/dependency chokepoint tests passed, including the reviewed
  timing feature forwarding.
- The required Python inventory update completed; both new test entries are
  reviewed journal oracles. The required CI preflight passed: **57 tests**.

The sandbox initially denied loopback binds in twelve existing chain tests.
The successful complete workspace run used sandbox elevation for those
local test transports. No test remains blocked by the sandbox. Mutation
scratch trees were removed automatically; verification build trees are
removed after their summaries are saved.

## Remaining acceptance work

1. The [performance follow-up](r4-journal-render-performance-checkpoint.md)
   reaches the generic-20k and multi-wallet-20k render budgets. Recheck them
   when integrating the typed custody and loader outputs.
2. Close the lossless boundary gaps: actual lone-surrogate string scalars,
   legacy nonfinite float projections/details, and malformed transitional
   operands. Some malformed integer/row paths still default or panic rather
   than reproducing Python's precise refusal; the current corpus does not
   certify arbitrary legacy or malformed stage JSON. Keep native production
   registration disabled until these are closed.
3. Add a killed mutation for every rendering and join branch, including the
   full ownership-selection and compatibility fallback/refusal branches;
   rerun the inherited 2,246-recipe catalog and retain all named kill evidence.
4. Run the complete census/shadow lanes, full generated campaigns and
   recorded replay required by the R4 pipeline gate. The focused and sampled
   runs above do not replace them.
5. Replace the transitional row/stage accesses with the typed custody and
   loader outputs as those parallel steps land, preserving these byte gates.

No Python reference bug was identified in this run. Production Python files
were not changed.

## Timing record

The release probe and Python were run back to back on copied snapshots from
the supplied benchmark worktree, with no edits or concurrent verification
jobs in this worktree. Every book compared the entire canonical build state
before timing. Custody and tax computation, wire decoding, canonical output
serialization and persistence are outside the measured span on both sides.
Rust receives the captured UUID tape; Python retains its UUIDv4 generator.

The final matrix's one-minute load stayed below 8, so each book used three
alternating Python / Rust trials and the minimum for each implementation.
The script uses five trials when the load exceeds 8. Both implementations
release completed states outside the measured build span. These are shared
machine measurements, not an end-to-end journal benchmark:

| Book | Python best (s) | Rust best (s) | Speedup | Load range across runs |
| --- | ---: | ---: | ---: | --- |
| Generic 2k | 0.078926 | 0.022542 | 3.50× | 5.875–5.875 |
| Generic 20k | 0.873809 | 0.288475 | 3.03× | 4.783–7.283 |
| Multi-wallet 2k | 0.078482 | 0.026433 | 2.97× | 7.180–7.180 |
| Multi-wallet 20k | 1.113527 | 0.680659 | 1.64× | 7.039–7.773 |

**At this original checkpoint, the generic-20k 0.15 s target fails.** The measured Rust minimum is 1.92×
the budget. The multi-wallet case also needs substantial optimization.

| Book | Trial | Python (s) | Python load 1m | Rust (s) | Rust load 1m |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic-2000 | 1 | 0.118290823 | 5.875000 | 0.025706304 | 5.875000 |
| generic-2000 | 2 | 0.082271282 | 5.875000 | 0.023030484 | 5.875000 |
| generic-2000 | 3 | 0.078926003 | 5.875000 | 0.022542331 | 5.875000 |
| generic-20000 | 1 | 0.873809389 | 4.938477 | 0.299836925 | 4.783203 |
| generic-20000 | 2 | 0.995859675 | 4.783203 | 0.297076252 | 4.783203 |
| generic-20000 | 3 | 1.532015952 | 4.783203 | 0.288475212 | 7.283203 |
| rich-2000 | 1 | 0.078481893 | 7.180176 | 0.030892963 | 7.180176 |
| rich-2000 | 2 | 0.137229770 | 7.180176 | 0.028535461 | 7.180176 |
| rich-2000 | 3 | 0.083054956 | 7.180176 | 0.026433174 | 7.180176 |
| rich-20000 | 1 | 1.118204503 | 7.773438 | 0.734055172 | 7.391113 |
| rich-20000 | 2 | 1.605734289 | 7.391113 | 0.680658698 | 7.391113 |
| rich-20000 | 3 | 1.113527032 | 7.039062 | 0.682520098 | 7.039062 |

## Reproduction

Use `MATURIN_NO_INSTALL_RUST=1 mise exec rust@1.99.1 uv@latest --` for every
command below. The environment was synchronized with
`uv sync --locked --python 3.11`. All Cargo commands use `--offline`.
Set `TMPDIR` to an existing disposable directory under this worktree's
`target/`; do not use a real book or override `HOME`. A sandbox with a
read-only shared uv cache can use `UV_NO_SYNC=1 UV_CACHE_DIR=target/uv-cache`
after that locked synchronization.

```sh
cargo test --offline -p kassiber-journals
uv run --locked python -m pytest tests/test_rust_journal_render.py -q
uv run --locked python -m tests.rust_port.journal_render_vectors
uv run --locked python crates/journals/scripts/mutation_check.py
cargo build --offline --release -p kassiber-journals --example journal-render
uv run --locked python scripts/capture-journal-render-samples.py target/r4-render/samples --tax-corpus
uv run --locked python scripts/verify-journal-render.py target/r4-render/samples/*/ --summary target/r4-render/parity.json
uv run --locked python scripts/benchmark-journal-render.py --snapshots target/r4-render/bench --output target/r4-render/benchmark
```

Copy `snap-generic-{2000,20000}.json` and
`snap-rich-{2000,20000}.json` from the supplied benchmark directory into
`target/r4-render/bench/` before the benchmark. The scripts never open the
source benchmark books. Captured operands and timing JSON stay local in
`target/`; the tables above preserve the summarized evidence.

For each focused capture, set `KASSIBER_TEST_CUSTODY_ENGINE` to `census` or
`shadow` and `KASSIBER_TEST_JOURNAL_RENDER_DIR` to a separate empty directory
under `target/`, then run these modules together:

```sh
uv run --locked python -m pytest tests/test_rp2_ownership_transfers.py tests/test_database_journal_safety.py tests/test_quarantine_review.py -q
```

Pass both captured directories to `verify-journal-render.py` alongside the
sample directories. Keep these complete operands private: they are a parity
oracle and have no diagnostic redaction.
