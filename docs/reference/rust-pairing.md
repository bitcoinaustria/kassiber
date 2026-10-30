# Rust automatic transfer pairing (R4-14)

`kassiber_transfers::pairing` ports the automatic pairing used before journal
processing. This is a library checkpoint on `t3code/r4-14-pairing`, based on
`c3d18194`; it registers no CLI command or daemon kind. The transfers worker can
continue independently. The [service slice contract](../plan/21-rust-backend.md#service-slice-contract-r4-onward)
and [transfers classification](rust-shell.md#transfers-and-review-classification-r5)
continue to govern route activation.

## Callers and integration

Python's `core/transfer_matching.py` owns the reference matcher;
`cli/handlers.py` adds projection guards, filters, policies, rule selection and
writes. Its consumers are:

| Consumer | Reference boundary |
| --- | --- |
| CLI suggestions, rules apply, bulk pairing | `suggest_transfer_candidates`, `apply_transfer_rules`, `bulk_pair_transfers` |
| CLI `journals process --auto-pair` | `daemon_freshness.refresh_journals_step` |
| Desktop journal button | `ui.journals.process`, then the same refresh step |
| Freshness journal job | `journal_refresh`, then the same refresh step |
| Desktop transfer operations | `ui.transfers.suggest`, `ui.transfers.bulk_pair`, `ui.transfers.rules.apply` |
| AI tools and automatic review context | Python `ai/tools.py`, daemon transfer dispatch and `daemon_swap_review.py` |

R4-13 calls this entry after resolving book/workspace/profile scope:

```rust,ignore
use kassiber_transfers::{pairing, Scope, SystemSources};
let summary = pairing::auto_pair_before_journals(
    connection,
    &Scope { workspace_id, profile_id },
    &SystemSources,
)?;
```

The journal caller owns `BEGIN IMMEDIATE`, its journal savepoint, commit and
rollback. Rebuild a stale journal projection before pairing, as
`refresh_journals_step` does. Pairing captures current MOVE and ownership guards
before the rules pass and reuses them for bulk exact pairing even after a rule
invalidates journals. Then rebuild journals when required and commit both
changes together. Preserve the existing refresh counts, progress phases and
`skip_rebuild_when_current` behavior. Do not commit progress while pairs are
pending. R4-15 uses the same boundary for the freshness job.

The library opens a transaction if necessary, rolls its own operation back on a
late error, and leaves successful writes pending. It does not rebuild journals,
probe providers, discover books, or change routing. The supplied `Sources`
provides clocks and UUIDs; production uses `SystemSources`. The returned value
has Python freshness's exact automatic-pairing summary shape.

Standalone `suggestions`, `apply_rules` and `bulk_pair` take `PairingOptions`.
The two write methods accept optional captured `ProjectionGuards`; they also
leave commit to the caller. `Population`/`suggest` expose the pure matcher for
oracles. `examples/pairing-bench.rs` is an explicit disposable-database adapter,
not a product command.

## Preserved behavior

The full imported population determines hash/provider/refund uniqueness before
pair occupancy and dismissals remove emitted edges. Provider amount claims,
network domains, canonical txids, HTLC templates and observation attestations
retain their separate proof requirements. Same-wallet refunds remain eligible.
A suggestion never establishes ownership or carries basis.

Candidate ordering, evidence precedence, inclusive tolerance/window boundaries,
fee-inclusive amounts, zero/negative amounts, unknown timestamps, global
conflict cardinalities and policy/rule tie-breaks match Python. Ownership cards
compete before filtering but never auto-apply. Automatic journal runs preserve
user unpairs; explicit bulk pairing may recreate them. All writes reuse the
existing authored component/pair services, including immutable evidence,
revision terms, residual validation and invalidation.

The automatic entry loads observations once. Parsed identities and times are
reused; only effective pair occupancy, active dismissals, projection guards and
options determine whether matcher results can be reused. Expiry is evaluated
before cache lookup. The cache is local to that immutable population. Candidate
values use copy-on-write sharing; returned values cannot alter cached results.
Configuration-only network evidence is cached separately from raw evidence.
Shared custody identity reads preserve nonfinite Python scalar spellings so
pair commitments retain the same event id and hashes as Python.
Encoded provider text containers retain Python JSON object insertion order;
ordinary scalar and graph payloads use the faster parser.
Rule fee caps divide the integer quantities before rounding to binary64, as
Python does. The oracle includes an inclusive cap above 2^53 msat where rounding
each operand first would incorrectly reject the rule.

## Parity evidence

| Oracle | Inputs / operations | Comparison |
| --- | ---: | --- |
| Existing transfer fixture calls | 99 | Ordered dataclass JSON bytes |
| Identity encodings, evidence controls and reference refusal | 24 | Ordered JSON bytes / exact error |
| Transfer fixture populations hydrated into books | 123 / 984 | Ordered service JSON and every application-table row |
| Generated books, seeds 1–12 | 132 / 876 | Ordered service JSON and every application-table row |
| Supplied generic and multi-wallet books, 2k and 20k | 4 books, alternating repeats | Automatic summary, every application-table row, pre/post-write suggestion bytes |

UUIDs, clocks and initial-book identity are injected before execution. No output
UUID, timestamp, commitment, serialized field or row is masked. SQL restores
retain scope/immutability triggers and assert foreign-key consistency. Initial
migration audit times use an injected SQLite clock, before immutable audits are
created. Only normalized dump row ordering changes.

Each of the 12 generated seeds has **one distinct active input in each of 11
shapes**: exact, ambiguous, tolerance boundary, multi-wallet, rule selection,
expired dismissal, explicit unpair, ownership conflict, booked MOVE,
guards-after-rule and fee variants. Activity requires the intended candidate,
conflict and application signals, with an unguarded positive control for booked
MOVEs. The deliberately vacuous control fails with:

```text
seed 1, scenario deliberately-vacuous: distinct-input activity 0 < 1
```

The hash sweep uses `PYTHONHASHSEED=0,1,2,42,123,4294967295,random` on identical
initialized inputs. Its matcher and complete database/service exports must
remain byte-identical to each other and to the checked-in fixtures.

Python's provider amount reader has a separate reproducible reference bug:
`"²".isdigit()` succeeds, then `int("²")` raises `ValueError`. Rust preserves
that exact refusal, including suppressing it when a booked MOVE removes the
observation from matching. Python production is unchanged. Run
`crates/transfers/scripts/repro_pairing_provider_digit.py` with the locked Python
environment to reproduce it.

## Reproduction and acceptance

Use Rust 1.99.1, locked Python 3.11 and offline Cargo. Set
`TMPDIR="$PWD/target/r4-pairing"` and `MATURIN_NO_INSTALL_RUST=1`; create that
scratch directory first. Prefix commands with
`mise exec rust@1.99.1 uv@latest --`.

```sh
uv sync --locked --python 3.11
uv run --locked python crates/transfers/scripts/export_pairing.py
uv run --locked python crates/transfers/scripts/export_pairing_services.py
cargo test --offline --release -p kassiber-transfers --test pairing_matcher --test pairing_services
uv run --locked python crates/transfers/scripts/sweep_pairing_hashseed.py
cargo build --offline --release -p kassiber-transfers --example pairing-bench
uv run --locked python crates/transfers/scripts/bench_pairing.py SOURCE_BOOKS
uv run --locked python crates/transfers/scripts/verify_pairing_projections.py
uv run --locked python crates/transfers/scripts/pairing_mutation_check.py
```

The benchmark backs up each supplied book before use. It measures the pairing
library, excluding process startup/open and final commit for both languages.
Every run records the one-minute load. Above load 8, it alternates Python/Rust
order and reports the best of five for each implementation. The full raw run
record is retained alongside the report.

## Measured pairing time

Best of five alternating runs, seconds; 2026-10-30. One-minute load ranged from
4.33 to 6.00; five runs were retained even below load 8.

| Book | Python | Rust | Speedup | Load at Python / Rust best |
| --- | ---: | ---: | ---: | ---: |
| generic-2000 | 0.323586 | 0.032099 | 10.08× | 5.76 / 6.00 |
| multi-wallet-2000 | 0.425608 | 0.057988 | 7.34× | 5.54 / 5.54 |
| generic-20000 | 3.586817 | 0.327021 | 10.97× | 4.99 / 4.83 |
| multi-wallet-20000 | 5.009197 | 0.468366 | 10.70× | 4.36 / 4.50 |

The 20k target is met for both book types. At 2k, fixed service overhead keeps
the multi-wallet speedup below 10×. Every measured pair of runs passed complete
database and ordered suggestion parity. [Raw runs](rust-pairing-bench.json)
record both timings and the one-minute load for every execution.
Generic books have zero suggestions before and after pairing; multi-wallet
books retain 173 at 2k and 1,778 at 20k. The generated books and projection probe
exercise authored writes as well as candidate generation.

## Mutation and projection acceptance

All **35/35 mutations are killed by named tests**: 14 matcher, 11 complete
database/service, seven cache, one rollback, one exact fee-cap division and one
encoded provider-text ordering.
The catalog includes duplicate importer-hash and contradictory-provider-amount
controls, fee-inclusive rows and refusal to promote an unscoped chain label to
importer evidence. Mutation scratch trees are deleted after the summaries are
written.

The named killing tests are:

- `pairing_matcher_matches_python_transfer_fixtures_byte_for_byte`;
- `pairing_services_match_python_seeded_complete_database_rows_and_ordered_results`;
- `pairing_cache_rechecks_occupancy_dismissal_expiry_guards_and_options`;
- `pairing_auto_pair_rolls_back_late_failure_and_leaves_commit_to_caller`;
- `pairing_rule_fee_cap_rounds_the_integer_ratio_once`;
- `pairing_encoded_provider_metadata_preserves_python_mapping_order`.

The projection probe compares unchanged Python desktop/AI transfer list,
suggestions, review context and rule-list projections after Rust writes on 11
books: **44 byte-identical reads and seven newly authored pairs**. It also
compares every database table and the automatic summary before those reads.
The omitted rule confidence has Python's `strong` default; bulk's default
remains `exact`. Literal asset-code comparisons in view filters retain Python's
behavior independently of canonical Bitcoin alias matching.

The final source passes `cargo fmt --all --check`, workspace/all-target clippy
with `-D warnings` and the test-harness feature, and **975 workspace tests**
with that feature. Python transfer matching, rules, freshness, AI and native
transfer bridge modules pass **313 tests and 84 subtests**.
The reviewed disposition inventory is regenerated, and the required
`python_test_shards.py run preflight --ci` gate passes **57 tests**.
