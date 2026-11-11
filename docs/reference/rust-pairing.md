# Rust automatic transfer pairing (R4-14)

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

`kassiber_transfers::pairing` ports the automatic pairing used before journal
processing. The journal stage remains separate from R4-13's pipeline wiring:
CLI `journals process --auto-pair`, desktop `ui.journals.process` and freshness
journal jobs use the native refresh coordinator. The assistant's no-pairing
journal step is native and verified; its chat loop awaits the R6 port.
Python retains production authority until R4-16. The existing
native transfer commands reuse this same library. The [service slice contract](../plan/21-rust-backend.md#service-slice-contract-r4-onward)
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
invalidates journals. Then rebuild journals when required and commit pairing
and that rebuild together. The reference commits an earlier stale-projection
rebuild separately; a later failure retains it and rolls back pending pairing,
its candidate cache and the second rebuild. Preserve the existing refresh
counts, progress phases and
`skip_rebuild_when_current` behavior. Do not commit progress while pairs are
pending. R4-15 uses the same boundary for the freshness job.

The library opens a transaction if necessary, rolls its own operation back on a
late error, and leaves successful writes pending. It does not rebuild journals,
probe providers, discover books, or change routing. The supplied `Sources`
provides clocks and UUIDs; production uses `SystemSources`. The returned value
has Python freshness's exact automatic-pairing summary shape.

R4-13's `process::refresh_native` coordinates this contract through the shared
scoped book service and registered CLI flag.
Its captured entry, `auto_pair_before_journals_captured`, preserves SQLite
exceptions separately from application refusals for the exact skipped-pairing
summary. The ordinary pairing entry and its pending-write contract remain the
same. Complete refresh books verify that a failed second rebuild rolls back
pairs while retaining a successful earlier stale rebuild.

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
Pairing field reads compare the existing generalized UTF-8 key bytes directly,
retaining exact Unicode matches and last-duplicate precedence without repeatedly
validating every preceding key. Non-string keys remain invisible to field reads.
Rule fee caps divide the integer quantities before rounding to binary64, as
Python does. The oracle includes an inclusive cap above 2^53 msat where rounding
each operand first would incorrectly reject the rule.

## Parity evidence

| Oracle | Inputs / operations | Comparison |
| --- | ---: | --- |
| Existing transfer fixture calls | 99 | Ordered dataclass JSON bytes |
| Identity encodings, evidence controls and reference refusal | 24 | Ordered JSON bytes / exact error |
| Transfer fixture populations hydrated into books | 123 / 1,107 | Ordered service JSON and every application-table row, SQLite and SQLCipher |
| Generated books, seeds 1–12 | 132 / 1,008 | Ordered service JSON and every application-table row, SQLite and SQLCipher |
| Journal generator, generic and multi-wallet, 2k and 20k | 4 books; five SQLite repeats and one SQLCipher replay each | Automatic summary, every application-table row, pre/post-write suggestion bytes |
| Synthetic CLI and daemon recordings | 2 restored books, all five profile scopes, SQLite and SQLCipher | Automatic summary, every application-table row and ordered suggestions, including a nonempty suggestion case |

UUIDs, clocks and initial-book identity are injected before execution. No output
UUID, timestamp, commitment, serialized field or row is masked. SQL restores
retain scope/immutability triggers and assert foreign-key consistency. Initial
migration audit times use an injected SQLite clock, before immutable audits are
created. Only normalized dump row ordering changes.

Every oracle book runs automatic pairing twice. The second pass creates no additional
pairs, but Python's candidate-cache UPDATE still advances the chain-index clock
and dirty revision. Those rows are compared exactly, including on SQLCipher;
they are not masked as incidental state. A separate named test wraps both passes
in the journal caller's savepoint, rolls that savepoint back and compares all
112 application tables with the original book in both storage modes. Fixture
restoration is batched in a transaction; pairing uses the ordinary commit policy.

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
scratch directory first. Set `CARGO_BUILD_JOBS=8`,
`CARGO_TARGET_DIR="$PWD/target"`, `CARGO_BUILD_BUILD_DIR="$PWD/target/build"`
and `CARGO_NET_OFFLINE=true` so shared user Cargo settings cannot move build
scratch outside the worktree. Prefix commands with
`mise exec rust@1.99.1 uv@latest --`.

```sh
uv sync --locked --python 3.11
uv run --locked python crates/transfers/scripts/export_pairing.py
uv run --locked python crates/transfers/scripts/export_pairing_services.py
cargo test --offline --locked --release -p kassiber-transfers --lib --test pairing_matcher --test pairing_services
uv run --locked python crates/transfers/scripts/sweep_pairing_hashseed.py
cargo build --offline --locked --release -p kassiber-transfers --example pairing-bench
uv run --locked python crates/transfers/scripts/bench_pairing.py SOURCE_BOOKS
uv run --locked python crates/transfers/scripts/bench_pairing.py SOURCE_BOOKS --cipher --runs 1
uv run --locked python crates/transfers/scripts/verify_pairing_projections.py
uv run --locked python crates/transfers/scripts/verify_pairing_recordings.py RECORDINGS
uv run --locked python crates/transfers/scripts/verify_pairing_recordings.py RECORDINGS --cipher
uv run --locked python crates/transfers/scripts/pairing_mutation_check.py
```

The benchmark backs up each supplied book before use. Wall measurements cover
the pairing library, excluding process startup/open and final commit for both
languages. CPU scopes are described below. Every run records the one-minute
load. It alternates Python/Rust order and reports the best of five for each
implementation, including below load 8. The full raw run record is retained
alongside the report.

`bench_pairing.py` also accepts repeatable `--book DB` snapshots and visits every
profile, compares every table and pre/post suggestion bytes, and supports
`--cipher` with a synthetic key delivered to the native child over stdin.
`verify_pairing_recordings.py` restores completed synthetic session data roots
under `target/` before passing them to that same comparator. It never edits a
recording. `--out` is constrained to this worktree's `target/`.

CPU results use Python's `time.process_time()` around the stage and the native
child's user plus system CPU from `resource.getrusage(RUSAGE_CHILDREN)`. Native
CPU includes startup, database open/keying, output serialization and commit, so
it is an upper bound on stage CPU. Both implementations retain separate wall
measurements; the earlier wall-only record is historical.

## CPU acceptance, 2026-10-31

Best process CPU of five alternating runs on the committed journal-book
generator's plaintext books, seeds 1 (generic) and 7 (multi-wallet), seconds.
This is the R4 pipeline's plaintext performance budget. Native CPU is the
complete-child upper bound described above. One-minute load ranged from 47.37
to 71.43 on the shared 48-core host.

| Book | Python stage CPU | Native child CPU | CPU speedup |
| --- | ---: | ---: | ---: |
| generic 2k | 0.415573 | 0.041085 | 10.11× |
| multi-wallet 2k | 0.462281 | 0.067721 | 6.83× |
| generic 20k | 4.709525 | 0.361649 | **13.02×** |
| multi-wallet 20k | 5.823293 | 0.495431 | **11.75×** |

The 10× CPU target at 20k is met on both shapes. At 2k the multi-wallet book
remains below 10×. Every measured pair passed the automatic summary, complete
database dump and ordered pre/post suggestion comparisons.

Generic books have zero suggestions before and after pairing; multi-wallet
books retain 173 at 2k and 1,778 at 20k. The synthetic seeded books and projection
probe exercise new authored writes as well as candidate generation.
SQLCipher replays verify parity separately; their complete-child CPU includes
key derivation before the timed stage and is not used for stage CPU acceptance.

## Mutation and projection acceptance

The current campaign kills **all 38 mutations** with eight named tests, each of
which passes unmutated first. It retains the original 35 matcher/service/cache
controls and adds two exact-key/duplicate-order controls plus a forbidden
journal-transaction commit. Every mutated tree must compile before its named
test fails; compilation errors do not count as kills.
The catalog includes duplicate importer-hash and contradictory-provider-amount
controls, fee-inclusive rows and refusal to promote an unscoped chain label to
importer evidence. Mutation scratch trees are deleted after the summaries are
written.

The named killing tests are:

- `pairing_matcher_matches_python_transfer_fixtures_byte_for_byte`;
- `pairing_services_match_python_seeded_complete_database_rows_and_ordered_results`;
- `pairing_cache_rechecks_occupancy_dismissal_expiry_guards_and_options`;
- `pairing_auto_pair_rolls_back_late_failure_and_leaves_commit_to_caller`;
- `pairing_stage_repeats_match_python_and_outer_journal_failure_reverts_every_table`;
- `pairing_field_lookup_preserves_exact_keys_and_last_duplicate`;
- `pairing_rule_fee_cap_rounds_the_integer_ratio_once`;
- `pairing_encoded_provider_metadata_preserves_python_mapping_order`.

The projection probe compares unchanged Python desktop/AI transfer list,
suggestions, review context and rule-list projections after Rust writes on 11
books: **44 byte-identical reads and seven newly authored pairs**. It also
compares every database table and the automatic summary before those reads.
The omitted rule confidence has Python's `strong` default; bulk's default
remains `exact`. Literal asset-code comparisons in view filters retain Python's
behavior independently of canonical Bitcoin alias matching.

The R4-14 follow-up passes **16 focused Rust tests**, including all 2,115
operations on SQLite and SQLCipher, and transfer all-target clippy with
`-D warnings`. Python matching, rules and freshness pass **212 tests and 33
subtests**. The synthetic boundary recordings cover seven CLI smoke cases and
the daemon suggestion case. The original library checkpoint's broader
975-test workspace run remains historical evidence; it was not rerun here.
The current follow-up also passes formatting, regenerates the reviewed
disposition inventory, and passes the required **57-test preflight**. The
regeneration refreshes stale baseline counts; preflight also required correcting
an existing setup command's flag order to `uv sync --locked --offline`.

## Legacy pair repair identities

A `repair_legacy_pair` review apply revises the pair through
`custody_review_terms.update_pair_review`; Rust follows the same path through
`pair_authoring.rs`. The stored identities commit to the authoring time:

- The revision's `created_at` is `now_iso()`, in whole seconds
  (`revise_pair_review_component`; Rust `sources.now`).
- Its component id is `uuid5("transaction_pair_group", profile, sha256 of
  the pair fields)`, and those fields include `created_at`
  (`_pair_group_spec` and `_PAIR_HASH_FIELDS`; Rust `pair_spec::group`).
  Legs, allocations and terms take uuid5 ids under that component id.
- Each evidence commitment id is `sha256(["custody-component-evidence-v1",
  component_id, ordinal])`. The ordinals follow `(quantity_hash,
  detail_hash)` order on both sides.

So repairing the same pair at different times stores different revision ids,
in Python as in Rust, and the same book repaired at the same instant stores
the same rows in both runtimes. Neither runtime stores order-dependent data
here, and `PYTHONHASHSEED` plays no part.

| Writer | Ids | Order | Verdict |
| --- | --- | --- | --- |
| Review apply, `custody_component` | Content uuid5 (`_bulk_component_id`, `_revision_component_id`, `_deterministic_id`) | Spec order | Deterministic |
| Review apply, `repair_legacy_pair`, and `transfers update`/`unpair` of a pair group | uuid5 over fields including the authored `created_at` | Group by `created_at, id` (pair ids) | Deterministic given the clock |
| Pair and payout creation | uuid5 over a fresh uuid4 pair or payout id and `created_at` | n/a | Random by design. Parity binds ids and clocks (AI tool sessions) or maps all uuids (`test_rust_transfer_services.py`) |
| Gap bridge reviews | uuid5 over gap id and fingerprint; review row uuid4 | Spec order | Deterministic apart from the uuid4 row id |
| Custody allocations | uuid5 by ordinal under the component | `allocate_msat_fifo` over ordered legs | Deterministic |
| Evidence commitments | sha256(component id, ordinal) | Sorted `(quantity_hash, detail_hash)` both sides; Rust sorts the final pairs (`commitments::commitment_pairs`) | Deterministic. Ties are identical pairs |
| Authored evidence snapshots | Keyed by detail hash | Rows read `occurred_at, created_at, id`, deduplicated by detail hash, then sorted | Deterministic: the key covers the whole payload |
| Journal custody decisions | `custody_decision_id` hashes | Composition order (same-time source order, [tax: same-time order](tax.md#same-time-order)) | Deterministic |
| Review receipts | uuid4 id, clock | n/a | Random id and time by design |

The Rust writers in this path keep their order in `BTreeMap`/`BTreeSet` or
sorted `Vec`s; the `fxhash` sets in `commitments.rs` only intern strings and
events, and nothing iterates them into stored rows.

Tests: `ReviewParity.CLOCK` (`tests/test_rust_review_workflow.py`) runs the
Python reference through `tests/rust_port/journal_cli_reference.py` with its
authoring and receipt clocks bound and Rust with
`KASSIBER_RUST_TEST_JOURNAL_TIME`; `ReviewLegacyPairFeeParity` sets it. The
masker maps uuid4 only, so derived uuid5 ids and hashes compare as stored
([session replay policy](testing.md)).
`test_repair_apply_stores_the_same_rows_on_every_run` repairs two copies of
the book with fresh random ids in each runtime. Mutation catalog:
`crates/transfers/scripts/repair_determinism_mutations.json` (5 entries, run
with `review_process_mutation_check.py --mutations`) mutates the commitment
id, ordinal order, clock hashing and authoring clock, plus a random
commitment id that only the run-to-run test catches.
