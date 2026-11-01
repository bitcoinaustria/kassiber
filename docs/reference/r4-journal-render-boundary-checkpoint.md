# R4-11 boundary continuation and acceptance record

This 2026-10-31 local continuation starts at `b5eb38ea8` on
`t3code/r4-11-render-3`. **R4-11 is not accepted.** The lossless scalar boundary,
exhaustive branch mutations and full inherited gates remain open. Python
retains production accounting authority; no native journal command is enabled.

## Requirement and changes

Preserve Python's complete state and refusals without changing custody,
ownership or tax decisions. The requested render-and-join CPU limit is 0.15 s
on both 20k books. Compatibility metadata and suggestions grant no authority.

- Direct payouts use fallible Python integer coercion, including Unicode
  digits, whitespace, underscores, booleans, float truncation and falsey
  defaults. Witnesses cover source filtering, unique fallback and refusal order.
- Ownership and residual-suspense joins propagate integer refusals. The lazy
  no-card path still validates inbound amounts. Same-wallet comparison uses
  Python stringification.
- Holdings merge Python-equal boolean, integer and integral-float tuple keys,
  retain the first key and refuse unhashable keys. Missing account fields
  refuse rather than becoming null. Entry lookup preserves unknown-wallet
  precedence, missing-row refusal and UUID consumption; unhashable wallet ids
  refuse before generating an entry id.
- Typed quarantine floats decode directly into `pyjson`: finite Decimal
  overflow remains Infinity, and malformed canonical details refuse instead
  of panicking. Summary floats keep nonfinite scalar tags in canonical state.
- Candidate enrichment shares persistence's lossless parser, preserving
  escaped surrogate text, NaN and Infinity with compact sorted ASCII output.
- Calculation methods use Python whitespace stripping. Holdings precede
  quarantine rendering, and compatibility payouts precede summaries.

The base manifest repeated store/wallet dependencies and could not load in
Cargo. Removing duplicates normalizes the existing lockfile; no dependency is
added. The loader also retained the old transaction shape and discarded typed
admission. Its compatibility repair wraps transaction records as values and
returns the admitted `BookSnapshot`; receipt acquisition reads the same rows.
Existing loader tests cover this repair.

## Stage handoff

Keep `boundary::build(profile, custody, wallets, response, ids)`, or call
`render_ledger(&LedgerInputs, &LedgerResponse, ids)` followed by
`boundary::finish(&custody, ledger_state)`. `finish` consumes owned ledger rows,
performs diagnostic joins and borrows custody facts through persistence. It
does not serialize the state, read SQL, create ids, read clocks or perform I/O.
The new `native_render_and_finish_match_combined_stage` checks every successful
frozen state and exact UUID-tape consumption. Custody operands still use the
transitional representation; final typed integration remains open.

## Verification record

The frozen corpus contains 366 reproducible synthetic cases: 206 states and
160 refusals, 101 more than the previous 265 cases. Tests compare canonical
bytes with complete error attributes and explicit UUID tapes. All 265
original records remain unchanged. Tapes are checked on refusals as well as
successful states. All six native oracle tests and all six Python
oracle/capture tests pass. The all-feature journal crate suite passes all 34
tests, including full logical SQLite/SQLCipher dumps, rollback injection,
loader admission and storage tests. Scoped formatting and all-target,
all-feature clippy with `-D warnings` pass. Campaign/timing evidence follows.

The required classifier update refreshes the existing disposition inventory
and reference counts. The first CI preflight run passes 56 tests and fails
`DependencyDriftTests::test_development_workflows_use_locked_uv_and_pnpm`:
`docs/reference/r4-quantity-kernel-performance.md:203` puts `--offline` before
`--locked` in its already-locked sync command, while the check requires
`--locked` immediately after `sync`. Reordering those flags preserves the
command's behavior; no Python test or product source is changed.
The fresh CI preflight passes all **57 tests**.

All existing mutation recipes remain. Five anchors move to equivalent
operations; two enrichment anchors now target the shared persistence helper.
Twenty-two new recipes pin repaired boundary behavior. There are 83 journal and
51 persistence recipes. The inherited custody catalog has 2,327 applicable
recipes; applicability is not kill evidence.

| Reanchored recipe | Equivalent operation |
| --- | --- |
| `JR-quarantine-sort` | Change default separators to compact separators in the lossless detail serializer |
| `JR-enrichment-key` | Replace the established candidate key in the shared persistence helper |
| `JR-enrichment-layout` | Disable compact separators in that helper |
| `JR-owned-amount` | Change the sats-to-msat factor from 1,000 to 1,001 after fallible coercion |
| `JR-review-fee` | Add the already-coerced inbound amount instead of subtracting it |

All **83/83** journal recipes are killed by their declared named test, with
eight passing unmutated baselines and no build failures. The first run found
two survivors: the whitespace recipe named the wrong corpus group, and
removing an explicit zero branch was equivalent because serde normalizes
integer `-0`. The corrected recipe names the ledger test; the redundant
branch is removed and the zero recipe changes the conversion result. A fresh
complete 83-recipe run passes. No inherited recipe is removed.

| Named test | Killed recipes |
| --- | ---: |
| `ledger_rendering_matches_python` | 31 |
| `legacy_boundary_values_and_refusal_order_match_python` | 21 |
| `ownership_reviews_and_reconciliations_match_python` | 13 |
| `msat_conversion_matches_python_at_rounding_and_width_boundaries` | 8 |
| `balance_join_matches_existing_comparator_and_promotes_before_overflow` | 4 |
| `refusal_mapping_matches_python` | 3 |
| `quarantine_candidate_enrichment_is_compact_sorted_and_ascii` | 2 |
| `balance_current_keys_preserve_python_stringification` | 1 |

Local raw operands and logs live under `target/r4-render-3/`.
`verification-index.json` binds the final probe/source digests to the results;
`mutation-audit.json` maps every killed recipe to its named test. The mutation
and verification build trees are removed after these summaries are saved.

The twelve generators' 24,000 books and 1,030 permutations match all seven
custody stages on 24,852 non-refused inputs and all 24,705 rendering states.
The 178 matched upstream refusals and 147 pre-render refusals are separate
from successful rendering comparisons. The run exercises 4,907 entries,
125,117 quarantines, 328 ownership candidates and 45,441 diagnostic quantity
differences. No rendering refusal occurs in this campaign; the frozen corpus
supplies rendering refusal witnesses. The summary retains every scenario's
activity across all 515 scenarios and distinct active seed inputs, including
the small active subsets in the component, reconciliation and gap families.
`generated-final/summary.json` records the fresh final-probe run; the first
full run has the same counts and no differences.

| Generator | Books | Permutations | Render matches | Distinct active seed inputs | Entries | Ownership candidates |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Evidence | 2,000 | 58 | 1,987 | 1,379 | 1,207 | 0 |
| Components | 2,000 | 312 | 2,312 | 177 | 0 | 0 |
| Samourai | 2,000 | 68 | 2,068 | 1,588 | 60 | 0 |
| Recorded | 2,000 | 92 | 2,092 | 1,655 | 814 | 0 |
| Fanout | 2,000 | 94 | 1,916 | 1,494 | 0 | 220 |
| Graph | 2,000 | 16 | 2,016 | 2,000 | 0 | 0 |
| Pipeline | 2,000 | 24 | 2,024 | 1,675 | 0 | 0 |
| Safety | 2,000 | 78 | 2,078 | 767 | 160 | 108 |
| Channels | 2,000 | 64 | 2,064 | 1,563 | 194 | 0 |
| Native | 2,000 | 110 | 2,034 | 1,601 | 803 | 0 |
| Reconciliation | 2,000 | 50 | 2,050 | 96 | 1,476 | 0 |
| Gaps | 2,000 | 64 | 2,064 | 249 | 193 | 0 |

The original 159 recordings and their 159 permutations match on every custody
stage and all 318 rendering states. They exercise 3,076 entries, 750
quarantines and 222 diagnostic differences, with no ownership candidates or
render refusals; other corpora are needed for those branches. The focused
boundary repro now matches nonfinite detail and malformed payout integer
cases; the actual surrogate scalar still fails at probe decoding. The final
evidence is in `recorded-final/summary.json` and `boundary-final/summary.json`.

Five alternating trials on each copied benchmark book first passed complete
state parity. Best process CPU seconds were:

| Book | Python CPU (s) | Rust CPU (s) |
| --- | ---: | ---: |
| Generic 2k | 0.112395 | 0.016900 |
| Generic 20k | 1.675372 | 0.379535 |
| Multi-wallet 2k | 0.166336 | 0.047097 |
| Multi-wallet 20k | 1.529882 | 0.248375 |

Both 20k CPU measurements **fail 0.15 s**. Shared-host one-minute load ranged
from 53.14 to 67.13; load is context, not an exemption. These snapshots were
copied from `/home/tg/.cache/kassiber-verify/perf/`; the previous checkpoint's
different-host-load wall results do not establish a regression comparison.
Raw final wall/CPU samples are in
`target/r4-render-3/benchmark-final/timings.json`. An earlier local CPU run
also missed both budgets (0.156140 s generic, 0.295192 s multi-wallet);
`target/r4-render-3/benchmark/timings.json` retains it. There is no controlled
same-load baseline comparison.

## Remaining acceptance work

1. Replace `serde_json::Value`/Rust `String` throughout the native scalar
   boundary, including entries, provenance, joins and engine response decoding.
   The actual `workspace_id = '\ud800'` repro still cannot decode. Stored
   quarantine text is a separate repaired case; no sentinel is used.
2. Complete nonfinite handling for raw stage float tags, carried-rate overlays
   and transfer audit projection/consumption. Certify malformed row/list/dict
   operands and response refusal order. Eager response decoding, default-empty
   arrays, graph error-family mapping and remaining `expect` paths are open.
3. Finish the branch audit: component exclusions, ambiguous script owners,
   scope/txid filtering, amount tolerance and graph refusal propagation. Run
   the 51 persistence and 2,327 inherited custody mutations with named kills.
   Focused journal recipes do not replace those gates.
4. Finish typed loader/custody integration; rerun full census and custody-shadow
   with fresh render captures, the additional fresh-recording corpus and
   complete integrated logical SQLite/SQLCipher dumps, then the repository
   quality gate. Existing persistence fixtures do not certify the full
   generated encrypted rendering campaign.
5. Meet 0.15 s CPU on both 20k books after final integration. Historical wall
   measurements alone are not CPU acceptance evidence.

No Python product source changed. No Python reference bug was identified. No
push, PR or external comment was performed.

## Reproduction

Use `MATURIN_NO_INSTALL_RUST=1`, `CARGO_BUILD_JOBS=8` and
`mise exec rust@1.99.1 uv@latest --`. Synchronize with
`uv sync --locked --python 3.11`. Set `TMPDIR`, `CARGO_TARGET_DIR` and
`CARGO_BUILD_BUILD_DIR` under this worktree's `target/`; keep Cargo offline and
do not change `HOME`.

The release probe now reports wall `seconds` and process `cpu_seconds`, using
`getrusage(RUSAGE_SELF)` on Unix. The benchmark records Python `process_time()`
and alternates five Python/Rust trials per book after complete state parity.
Custody, engine computation, decoding, serialization and persistence are
outside both measured spans.

The focused mutation run selects the `JR-` recipes from the checked-in catalog
into `target/r4-render-3/journal-mutations.json`. The full catalog check includes
the 51 persistence recipes, but they were not rerun for kills. The boundary
repro below is expected to exit 1 while the actual surrogate scalar is open.

```sh
cargo test --offline --locked -p kassiber-journals --all-features -- --test-threads=1
cargo fmt -p kassiber-journals -- --check
cargo clippy --offline --locked -p kassiber-journals --all-targets --all-features -- -D warnings
uv run --locked python -m pytest tests/test_rust_journal_render.py -q
uv run --locked python -m tests.rust_port.journal_render_vectors
uv run --locked python crates/journals/scripts/mutation_check.py --check
uv run --locked python crates/journals/scripts/mutation_check.py --mutations target/r4-render-3/journal-mutations.json
cargo build --offline --locked --release -p kassiber-journals --example journal-render
uv run --locked python scripts/check-journal-render-boundary.py --probe PROBE --output target/r4-render-3/boundary-final
uv run --locked python scripts/check-journal-render-campaign.py --probe PROBE --output target/r4-render-3/generated-final
uv run --locked python scripts/check-journal-render-campaign.py --probe PROBE --recordings /home/tg/.cache/kassiber-verify/custody-recordings --output target/r4-render-3/recorded-final
uv run --locked python scripts/benchmark-journal-render.py --probe PROBE --snapshots target/r4-render-3/bench --output target/r4-render-3/benchmark-final
uv run --locked python -m tests.rust_port.classify_tests update
uv run --locked python scripts/python_test_shards.py run preflight --ci
```

## Fourth continuation: exact source reuse

This local continuation starts at `197f6cdec` on `t3code/r4-11-render-4`.
**R4-11 remains unaccepted.** This is a verified source-reuse checkpoint;
neither the CPU budget nor the remaining scalar and campaign gates are closed.

The controlled comparison reconstructs the pre-repair renderer and joins from
`b5eb38ea8`, retaining the current boundary adapter, and alternates that probe
with the unmodified `197f6cdec` probe on identical copied 20k operands. All book
states match before timing. Best of five process CPU seconds are:

| Book | Before boundary repairs | With boundary repairs | One-minute load range |
| --- | ---: | ---: | ---: |
| Generic 20k | 0.225057 | 0.387696 | 56.20–58.18 |
| Multi-wallet 20k | 0.425683 | 0.390397 | 53.84–58.92 |

Individual samples overlap substantially. For example, the generic pre-repair
probe ranges from 0.225057 to 0.421134 s. These results do not isolate the cause
of the historical regression or establish that the boundary repairs explain
it. The unmodified Python/Rust matrix separately measures 0.155323 s generic
and 0.298810 s multi-wallet. These are separate runs, not paired improvement
claims. Historical wall-clock measurements remain insufficient CPU evidence.

A 500-sample gdb profile contains 315 samples inside `boundary::build`.
Source construction appears in 126 of those samples (40.0%), entry heads in
49 (15.6%), balance comparison in 28 (8.9%), and holdings-key validation in
three (1.0%); inclusive counts overlap. Generic 20k renders 39,976 fragment
sources from 19,989 distinct source rows. Response decoding is outside the
render timer and excluded from those 315 samples.

The renderer now shares an immutable `Source` for repeated uses of a row only
when asset, spot price, fiat fee and crypto fee match exactly. Decimal matching
includes sign, coefficient and exponent. Changed values replace the cached
source without modifying previously rendered fragments. Event and lot fields
hold `Arc<Source>` with transparent serialization. The existing provenance
cache, UUID consumption and refusal order are preserved. Entry hashability
validation no longer constructs and discards an allocated holdings key.
`render_ledger` and `boundary::finish` retain their signatures.

Thirteen new Python-generated successful states cover event and lot value
changes, equal-valued Decimal representations and asset changes. All original
366 records are unchanged. The complete frozen corpus now has 379 cases:
219 states and 160 refusals, with exact UUID-tape consumption. Five new mutation
recipes cover the cache guards; the entry-key recipe moves to the equivalent
hashability check. All inherited recipes remain.

### Fresh verification

- All 34 journal crate tests pass with all features in release mode, including
  the existing logical SQLite/SQLCipher dump and rollback fixtures. Formatting,
  all-target/all-feature clippy with denied warnings, six Python oracle tests
  and deterministic regeneration of all 379 vectors pass.
- One complete scenario cycle across the twelve generators matches 508 render
  states; three pre-render refusals are separate. Tax-corpus replay matches
  145 cases, including 25 rendering refusals, with 21 pre-render refusals separate.
- Focused census and custody shadow each pass 108 tests and 12 subtests, compare
  all seven stages on 65 builds, and observe 128 production tax calls. Both
  fresh capture directories match all 65 render states, each exercising two
  ownership candidates.
- Recorded custody and rendering replay matches all 318 inputs: 159 original
  recordings and 159 permutations, including 3,076 entries and 750 quarantines.
- The resumable, source-bound mutation shard kills all ten selected recipes
  after two passing named baselines. `ledger_rendering_matches_python` kills
  the three retained source recipes and all five new cache recipes;
  `legacy_boundary_values_and_refusal_order_match_python` kills
  `JR-holding-unhashable` and `JR-entry-key-hash`. Logs are incremental and
  the shard build tree is deleted. Applicability checks pass for all 139 journal
  recipes (88 renderer, 51 persistence) and 2,327 custody recipes. Applicability
  is not kill evidence for the unexecuted recipes.

The boundary reproducer still fails actual surrogate-scalar decoding. It now
also retains a malformed engine decimal, `00E-1`, whose native decoder returns
`EngineContractError` instead of Python's `ValueError`. Nonfinite detail and
malformed payout integer cases match. These two known differences remain
explicit failures, not passing corpus comparisons. No Python reference bug or
Python product-source change is introduced.

The final original/new probe comparison alternates five pairs per book:

| Book | Original CPU (s) | Source reuse CPU (s) | One-minute load range |
| --- | ---: | ---: | ---: |
| Generic 20k | 0.456467 | 0.435943 | 72.66–78.72 |
| Multi-wallet 20k | 0.423680 | 0.394396 | 83.42–89.83 |

The separate final Python/Rust matrix alternates five pairs for every book,
after comparing complete state bytes. Best process CPU seconds are:

| Book | Python CPU (s) | Rust CPU (s) | Load at best Rust trial | All-trial load range |
| --- | ---: | ---: | ---: | ---: |
| Generic 2k | 0.160682 | 0.022427 | 93.34 | 93.02–93.34 |
| Generic 20k | 1.466747 | 0.208215 | 71.90 | 71.90–75.06 |
| Multi-wallet 2k | 0.108363 | 0.022381 | 69.26 | 69.26–69.26 |
| Multi-wallet 20k | 1.382334 | 0.201380 | 48.54 | 48.54–51.10 |

Both 20k books still **fail the 0.15 s CPU budget**. Shared-host load is context,
not an exemption. These timings exclude custody, engine computation, response
decoding, canonical serialization and persistence on both sides.

Raw operands, all five trials, stack samples, logs and source/probe digests
remain under `target/r4-render-4/`; `verification-index.json` binds the evidence.
The final classifier update leaves the disposition inventory unchanged, and
CI preflight passes all 57 tests. Manual second review checks cache identity,
immutable sharing, serialization and refusal order against the corpus.
Mutation and verification build trees are removed after their summaries.

### Still required for acceptance

1. Isolate the remaining regression and meet 0.15 s CPU on both 20k books after
   final typed-stage integration, preserving the existing `boundary::finish`
   handoff to the concurrent conversion and orchestration work.
2. Carry actual surrogate scalars throughout inputs, entries, provenance,
   response decoding and joins. Complete raw nonfinite, carried-rate and audit
   fidelity; malformed row/list/dict parity, lazy refusal order, graph error
   families and remaining panic paths stay open. The malformed canonical-decimal
   repro above is an additional concrete witness.
3. Complete the exhaustive renderer branch audit: component exclusions,
   ambiguous script owners, scope/txid filtering, amount tolerance and graph
   refusal propagation remain open. Rerun the other 78 renderer recipes,
   all 51 persistence recipes and all 2,327 custody recipes
   in resumable shards with named kills and build-tree deletion. The ten kills
   above do not substitute for this gate.
4. Run the full 25,030-input generated campaign, complete census/shadow lanes
   and additional fresh recordings against the final integrated renderer.
   The scenario cycle and focused captures above do not certify those gates.
5. Run complete integrated logical SQLite/SQLCipher campaigns and the repository
   quality gate, then write production acceptance. Existing persistence fixtures
   do not certify that integrated campaign.

No push, pull request or external comment is performed. Python retains production
accounting authority.

## Fifth continuation: native fragment quantities and allocation checkpoint

This local continuation starts at `bd3690fa3` on `t3code/r4-11-render-5`.
**R4-11 remains unaccepted.** The work below is a verified allocation checkpoint,
not completion of the speed, fidelity, mutation or integrated campaign gates.

Fragment and source msat quantities now retain ordinary signed machine integers
until serialization. Wide values keep the arbitrary-precision JSON path and
serialize identically. Decimal coefficient reads avoid constructing a discarded
BigUint. Integral zero fixed-point text shares literals while preserving its
sign; negative exponents retain every trailing decimal place. Source cache keys
borrow immutable engine Decimals and still compare their exact representations.

Entry heads cache validated metadata by immutable wallet reference, rather than
wallet id, and preserve the first validation and UUID-consumption order. The
calculation method is cached after its first use. Balance comparisons skip an
empty observation index, and no-card ownership review avoids allocating a
discarded BigInt for ordinary integer amounts. Malformed inbound amounts retain
their fallible conversion. `render_ledger` and `boundary::finish` keep their
signatures. No dependency or Python product source changes.

### Controlled CPU comparisons

`scripts/compare-journal-render.py` checks complete canonical state bytes before
timing identical captured operands. It records operand/probe SHA-256 digests,
five alternating samples, CPU affinity and each trial's load, with incremental
JSONL output. Decoding, canonical serialization, persistence and destruction of
the completed state stay outside the measured build span.

The exploratory checkpoint comparison pins all four probes to CPU 0. The full
historical `1e331d6f8` source is reconstructed with only the process CPU timer
adapter and its existing locked dev dependency added. Best of five CPU seconds:

| Checkpoint | Generic 20k | Multi-wallet 20k |
| --- | ---: | ---: |
| Historical `1e331d6f8` | 0.152400 | 0.172835 |
| Boundary repairs | 0.158094 | 0.175164 |
| Committed source reuse | 0.148833 | 0.181941 |
| Intermediate native fragment quantities | 0.137009 | 0.174918 |

The ranges overlap and do not isolate the reported regression. The historical
wall-clock result does not establish a passing CPU baseline; the reconstructed
historical multi-wallet renderer already exceeds 0.15 s CPU. These exploratory
probes do not replace a final comparison with matching build flags.

The immediate baseline and final candidate are therefore rebuilt separately
with Rust 1.99.1, offline Cargo, `--release --features timing` and eight build
jobs. They use independent build directories. Both books match complete state
bytes. Each table entry below is the best of five alternating pairs:

| Affinity | Book | Baseline CPU (s) | Candidate CPU (s) | Load 1m range |
| --- | --- | ---: | ---: | ---: |
| Unpinned | Generic 20k | 0.160988 | 0.169458 | 50.33–50.78 |
| Unpinned | Multi-wallet 20k | 0.296345 | 0.369133 | 44.92–50.89 |
| CPU 0 | Generic 20k | 0.193970 | 0.151179 | 33.69–35.88 |
| CPU 0 | Multi-wallet 20k | 0.193626 | 0.179153 | 32.70–34.39 |

Both final books still **fail the 0.15 s CPU budget**, including the pinned
generic result by 0.001179 s. The final unpinned comparison does not demonstrate
an improvement. Shared-host variability is recorded, not used to waive the
budget. Entry rendering remains the largest recorded stage.

### Verification and remaining gates

All original 379 frozen records are unchanged. Four new cases exercise wallets
sharing an id with different account metadata, a missing second-wallet account
after a valid entry, and integral negative-zero fees in both event and lot
sources. The corpus has **383 cases: 222 states and 161 refusals**. Deterministic
regeneration, all six Rust oracle tests and all six Python oracle/capture tests
pass with exact UUID tapes. The all-feature release journal suite passes all
34 tests, including the existing SQLite/SQLCipher dump and rollback fixtures.
All 28 pydecimal tests, workspace formatting and all-target/all-feature Clippy
with denied warnings pass.

All 144 journal mutation recipes apply: 93 renderer and 51 persistence. The
source-bound focused shard kills eleven recipes after four passing named
baselines: six affected inherited guards and five new guards. Named results,
source digests and incremental logs are retained; its build tree is deleted.
These eleven kills are not a full mutation campaign.

The fresh boundary repro still fails actual surrogate decoding and malformed
canonical-decimal error fidelity (`00E-1` yields native `EngineContractError`
instead of Python `ValueError`). Nonfinite detail and malformed payout integer
repros match. No Python reference bug is identified or changed.

Acceptance still requires, in order:

1. Isolate the remaining regression and meet 0.15 s CPU for both books on the
   final integrated renderer with exact bytes and five alternating pairs.
2. Repair actual surrogate scalars, raw nonfinite/carry/audit values, malformed
   inputs and lazy refusal order, including both failing repros above.
3. Finish the exhaustive branch audit, including component exclusions,
   ambiguous script owners, scope/txid filtering, amount tolerance and graph
   refusal propagation. Rerun the outstanding 78 inherited renderer recipes,
   revalidate the affected/new recipes on final source (current renderer catalog:
   93), all 51 persistence recipes and all 2,327 custody recipes in resumable
   shards with named kills and build-tree deletion.
4. Run the full 25,030 generated inputs, complete census/shadow lanes and fresh
   recordings against final source. Earlier campaigns do not certify this change.
5. Run the complete integrated SQLite/SQLCipher campaigns and repository quality
   gate, then write production acceptance. The passing persistence fixtures above
   do not certify that campaign.

Raw operands, probes, all trials, repros and logs remain under
`target/r4-render-5/`; `verification-index.json` binds their digests. The required
classifier update leaves the checked-in inventory unchanged; CI preflight
passes all **57 tests** before the local checkpoint commit. Manual second review
checks integer widths, zero representations, wallet identity, immutable source
references, serialization and refusal order against the frozen witnesses.
Mutation and verification build trees are removed after summarizing their logs.
No push, pull request or external comment is performed. Python retains
production accounting authority.

## Sixth continuation: borrowed pricing and entry identities

This local continuation starts at `4656ef30e` on `t3code/r4-11-render-6`.
**R4-11 remains unaccepted.** This verified speed boundary meets the isolated
render-and-join CPU budget. Scalar fidelity, exhaustive branch mutations and
integrated acceptance campaigns remain open.

### Requirement and implementation

The renderer must preserve complete state bytes, Decimal representations,
fallible conversions and UUID consumption while meeting 0.15 s process CPU on
both 20k books. The inherited evidence index and captured operands were recovered
from `/home/tg/.t3/worktrees/kassiber/t3-6d875b3a/target/r4-render-5/` and copied
into this worktree's `target/r4-render-6/` before measuring the inherited probes.

Ordinary source pricing retains twelve references selected during rendering.
Carried rows retain owned pricing values; pointer identity prevents substituting
pre-carry facts. Serialization only encodes those selected values. Entry identity
fields borrow validated profile and wallet records. Entry/source strings borrow
the immutable engine response; calculation methods borrow stripped profile
strings, preserving Python stringification for other values. The returned
`LedgerState`, entry/source models and `JournalState` retain the input and engine
response lifetime through serialization and persistence. `render_ledger` and
`boundary::finish` keep their call shapes; the temporary `LedgerInputs` wrapper
need not outlive the result. The oracle caller now keeps its decoded response
alive rather than moving it into a closure.

Row indexes, entries and fragments reserve their known sizes before ordered
construction. Date-cache hits borrow their keys. A machine-width fixed-point
formatter writes directly into its final string, using a read-only u128 Decimal
coefficient accessor. It preserves negative zero, every fractional place and
wide/exotic fallback behavior. A differential test compares 4,405 signed,
full-width, exponent and refusal operands with the established Decimal formatter.
No arithmetic, rounding, input validation or tax decision changes.

A 500-sample profile contains 327 samples within `boundary::build`; 171 of those
include JSON string-key indexing. Hot reads now use direct map access, preserving
null on missing/non-object optional fields and the existing required-field
refusals. Entry transaction ids construct their JSON string directly rather than
serializing a temporary String. No dependency, lockfile, Python product source,
command registration or external I/O changes.

### Controlled CPU verification

The first five-pair measurement pins the inherited probes to CPU 0 and verifies
complete state bytes. The inherited final renderer gives 0.131090 s generic and
0.173923 s multi-wallet CPU; the latter still fails. One-minute load ranges from
6.77 to 10.26 during those samples.

The final candidate uses Rust 1.99.1, offline Cargo, eight build jobs and
`--release --features timing`, matching the inherited final probe's flags. Both
probes match complete canonical state bytes before five alternating pairs on
CPU 0. Decoding, serialization, persistence and destruction of the completed
state remain outside both build spans; all field selection remains inside.

| Book | Inherited CPU (s), best of five | Candidate CPU (s), best of five | Load 1m range |
| --- | ---: | ---: | ---: |
| Generic 20k | 0.147481 | 0.125116 | 31.86–33.65 |
| Multi-wallet 20k | 0.177463 | 0.145054 | 31.15–32.12 |

Both candidate books pass **0.15 s process CPU**. Probe and operand SHA-256
bindings, every sample and load live in `target/r4-render-6/fields-paired/`.
Earlier variants and measurements are retained as exploratory evidence; they do
not certify the final source. In particular, selecting fields only at
serialization was rejected, and its passing timing is not acceptance evidence.
Final fidelity or integration changes require another CPU measurement.

### Verification and remaining work

All **383** frozen cases remain unchanged: **222 states and 161 refusals**, with
exact UUID tapes. Deterministic regeneration and all six Python oracle/capture
tests pass. The all-feature release journal suite passes **35** tests, including
its existing complete SQLite/SQLCipher dump and rollback fixtures. All **28**
pydecimal tests, workspace formatting and scoped all-target/all-feature Clippy
with denied warnings pass.

The 144 inherited mutation ids and rules remain. Seven new recipes cover
borrowed/owned pricing, carry-row identity, coefficient width, sign, decimal
placement and leading fractional zero. The catalog now has **100 renderer and
51 persistence** recipes. Three inherited anchors move to the same operations:
engine-event id rendering, Python whitespace stripping and wallet-posting
selection. All **151** recipes apply. A source-bound resumable shard kills the
seven new and three reanchored recipes after three passing named baselines;
its incremental calls and summary are retained and its build tree is deleted.
These ten kills do not certify the complete mutation campaigns.

The fresh boundary repro still differs on actual surrogate scalar decoding and
malformed canonical-decimal errors. Nonfinite quarantine detail and malformed
payout integer repros match. No Python reference bug is identified or changed.

Acceptance still requires, in order:

1. Repair surrogate and malformed-decimal repros, raw nonfinite scalars,
   carry/audit projection and consumption, malformed shapes and lazy refusal
   order; finish the lossless typed consumer boundary.
2. Complete the branch audit, including component exclusions, ambiguous script
   ownership, scope/txid filtering, amount tolerance and graph refusal
   propagation. Run all 100 renderer recipes (the original 93 plus seven new),
   all 51 persistence recipes and all 2,327 custody recipes in resumable shards,
   retaining named kills and deleting build trees after each shard.
3. Run all 25,030 generated inputs, census and shadow lanes and fresh
   recordings against the final source. Earlier campaigns do not certify this
   source.
4. Run the complete integrated SQLite/SQLCipher campaigns and repository quality
   gate, repeat final integrated CPU measurements and write production
   acceptance. The passing persistence fixtures above do not replace those
   campaigns.

`target/r4-render-6/verification-index.json` binds the source, probes, operands,
logs, mutation summary and remaining gates. The required classifier update
leaves the checked-in inventory unchanged; CI preflight passes all **57 tests**
before the local commit. Manual second review checks input and engine-response
lifetimes, null/scalar field access, Decimal widths and zero representations,
UUID/refusal order and persistence callers. Mutation and verification build
trees are removed after their summaries are saved. No background job, push,
pull request or external comment is left; Python retains production accounting
authority.

## Seventh continuation: legacy transfer floats and carry refusals

This local continuation starts at `e723aacbf` on `t3code/r4-11-render-7`.
**R4-11 remains unaccepted.** This is a partial fidelity boundary; actual
surrogate scalars and lazy malformed-response decoding remain open.

Transfer audits now retain native `f64` values through entry consumption and
canonical serialization. Decimal-to-float overflow no longer becomes JSON null
or reaches the former `expect("audit float")` panic. Transfer quantities retain
finite Decimal representations or a typed Infinity/NaN variant. They preserve
Python's transfer-out negation, signed zero, underflow and float rounding.
Persistence refuses a nonfinite quantity at its msat conversion, retaining the
Python exception and restoring every table through the existing savepoint.
No nonfinite quantity becomes an integer accounting fact.

Carried rate strings use the existing Python-compatible float parser, including
Unicode digits, underscores, whitespace, infinities and NaN. Malformed strings
retain Python's `ValueError` message before consuming entry ids. Nonfinite
carried-rate overlays use the existing canonical float representation in the
transitional row tree. This does not complete the lossless typed row boundary.

All **383 original cases remain byte-for-byte unchanged**. A separate
`render-legacy-floats.json` corpus adds **39 cases: 35 states and four refusals**.
Its generator observes unchanged Python `build()` and `render_audit()` behavior;
the native oracle compares complete state bytes, every audit scalar and exact
UUID-tape consumption. It exercises overflow of each audit amount and price,
negative zero, underflow, float rounding, audits without entries, carried-rate
coercion and malformed-rate refusal text. The first new run exposed a generator
aliasing error between an audit-only variant and its preceding entry variant;
the generator now copies each response before capturing it.

All **37 all-feature journal tests** pass, including the existing complete
SQLite/SQLCipher dump and rollback fixtures and the new nonfinite-quantity
rollback test. Scoped all-target/all-feature Clippy passes with warnings denied.
These fixtures do not replace the complete integrated database campaigns.

All 151 inherited journal mutation ids remain. `JR-transfer-sign` moves to the
same negation in `JournalQuantity::from_audit`. Ten new recipes cover native
audit floats and serialization, infinity signs and identity, carried-rate
coercion/refusals and nonfinite persistence refusal. All **161 recipes apply**:
110 renderer recipes and 51 persistence recipes. The source-bound resumable
shard kills the ten new recipes and the reanchored transfer recipe after three
passing named baselines, without build errors. Its build trees are deleted
after the incremental call log and summary are saved. This is eleven focused
kills, not a complete mutation campaign.

Fresh release probes use the pinned toolchain, offline Cargo, eight build jobs
and `--release --features timing`. Both books first match complete state bytes.
Each run alternates five inherited/candidate pairs and records every CPU sample,
wall sample, load, probe digest and operand digest. CPU 18 is selected before
its run from a recorded two-second per-core utilization sample; the earlier
CPU-0 failure remains part of the evidence.

| Affinity | Book | Inherited CPU, best of five (s) | Candidate CPU, best of five (s) |
| --- | --- | ---: | ---: |
| CPU 0 | Generic 20k | 0.166397 | 0.162323 |
| CPU 0 | Multi-wallet 20k | 0.186418 | 0.181005 |
| CPU 18 | Generic 20k | 0.126380 | 0.131175 |
| CPU 18 | Multi-wallet 20k | 0.258975 | 0.270028 |

These fresh runs **do not meet 0.15 s CPU on both books**. The inherited probe
also misses that budget on multi-wallet 20k; these measurements do not establish
a causal regression from this change. Host load and affinity are context, not
exemptions. The sixth continuation's passing historical measurement does not
certify this source. All field selection remains inside the measured build;
decoding, canonical serialization, persistence and state destruction stay outside.

The remaining work, in acceptance order, is:

1. Close the actual surrogate-scalar and malformed-canonical-decimal repros,
   including their complete UUID/refusal order. Complete the lossless typed
   boundary across profile, rows, entries, provenance, joins and response
   decoding; raw nonfinite stage operands and carry/audit malformed shapes and
   refusal order remain open.
2. Finish the exhaustive branch audit for component exclusions, ambiguous
   script ownership, scope/txid filtering, amount tolerance and graph refusal
   propagation.
3. Run the complete source-bound catalogs: all 110 renderer recipes (the
   inherited 100 plus this continuation's ten), all 51 persistence recipes and
   all 2,327 custody recipes, in resumable shards with build-tree deletion.
4. Run all 25,030 generated inputs, full census/shadow lanes and fresh recordings
   against the final source.
5. Complete typed loader/custody integration, full integrated SQLite/SQLCipher
   campaigns and the repository quality gate.
6. Reestablish 0.15 s CPU on both 20k books, repeat the final integrated CPU
   measurements and write the acceptance record.

The fresh boundary repro exits 1: actual surrogate decoding and malformed
canonical-decimal refusals still differ; nonfinite quarantine detail and the
malformed payout integer match. All six Python oracle/capture tests pass,
including deterministic regeneration of both render corpora. Workspace
formatting passes. The required classifier update leaves the inventory
unchanged, and the required CI preflight passes all **57 tests** before commit.
Manual second review checks quantity signs and scalar identity, exact finite
representations, UUID consumption, persistence rollback and the unchanged
production-route boundary. Source/probe/operand digests and all results live in
`target/r4-render-7/verification-index.json`. Verification build trees are
removed after saving the probes and summaries. No background job remains.

Python product sources, dependencies, schema, routes and AI projections are
unchanged. No Python reference bug has been identified. Python retains
production accounting authority.
