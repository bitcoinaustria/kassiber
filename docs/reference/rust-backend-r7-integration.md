# R7 integration audit (plan 21, round 4)

Local branch: `t3code/rust-backend-r7`, based on `1e6565c8` (`t3code/rust-backend-r6`).
This audit continues [round 3](rust-backend-r6-integration.md).
No push, PR, merge commit, source-branch edit, stash operation or dependency upgrade
is part of this local integration. External dependencies are limited to the
versions already selected by the explicitly requested slices (including the
sync line's BDK mirror); integration adaptations add no dependency.

## Provenance

The requested inputs contain **57 candidate commits**. The stack applies **55**
original commits and skips **two** duplicates, preserving complete original messages
and requested order. The R5 slices' original round-1 base `0d4fbc28` is already
present as `3c91b02a`; only the explicitly requested commits are replayed.
`git cherry` evidence and subject/full-patch comparisons precede replay. The two
aliases are skipped after their fixes have been picked from the Python line:
`be959dd6` duplicates `def793f9`, and `8d7f1332` duplicates `133cde70`.
The excluded Python fixes under review, restore recovery fix and ledger-perf line
are not picked.

| Input | Candidates | Picks | Skips |
| --- | ---: | ---: | ---: |
| page | 1 | 1 | 0 |
| python | 5 | 5 | 0 |
| redaction | 1 | 1 | 0 |
| transfers | 14 | 13 | 1 |
| pairing | 1 | 1 | 0 |
| reports | 11 | 10 | 1 |
| sync | 16 | 16 | 0 |
| importers | 3 | 3 | 0 |
| r4 | 5 | 5 | 0 |
| **Total** | **57** | **55** | **2** |

The separately authored harness fix is `d7c17d21`. Integration adaptations follow
the replay, each with the Codex co-author trailer:

| Commit | Integration work |
| --- | --- |
| `d7c17d21` | Patched-policy fusion skip and regression |
| `73d8c1c6` | Shared UUID/32-hex redaction corpus |
| `90fd3ab8` | Rust partial-payout migration and historical-store tests |
| `71e694b6` | Custody/service API unions and retained CPython uppercase |
| `fc531dcd` | Bounded prices, fixed payout recovery and regenerated slice oracles |
| `f8244da9` | Generated test and contract inventories |
| `c60e7cd4` | Explicit legacy no-rate importer witness |
| `ad1438f5` | Shared gap worklist-range vector exporter |
| `58d3fbf3` | Derived zero-rate canonicalization and pricing-floor witness |
| `9ae4cba5` | Apply the historical-fixture distinction to shell parity callers |
| `4e1d7244` | Shared CLI runner for incoming witnesses and reference bench generation |
| `24593799` | Commercial document price boundary and current export-refusal witness |
| `c8b13768` | Wait for the real idle-watch database startup in desktop backup parity |
| `8d2fd783` | Re-anchor exact document-price mutation to the bounded production parser |
| `a61303d0` | Keep denominator-precision witnesses above the repaired pricing floor |
| `56a649a0` | Check refused authoring state before the caller's rollback |

This audit and documentation updates follow those adaptations: **72 commits**
above the base (55 picks, 16 adaptations and one audit/status commit).
Local provenance checks compare every original/picked full message and reject
merge commits.

## Conflicts

**34 distinct candidate commits** encounter textual conflicts: **98 path
occurrences across 41 paths**. Generated counts and contract snapshots retain a
temporary integrated baseline during replay and are regenerated from code.
All conflicts preserve the incoming behavior and the round-3 adaptations; none
requires a new accounting or authority design decision.

| Original | Conflicting paths |
| --- | --- |
| `36070685` | `docs/reference/rust-port.md` |
| `4ad4e698` | `kassiber/db.py` |
| `95f61f79` | `kassiber/core/custody_tax_projection.py` |
| `8a93e945` | `tests/test_audit_package.py` |
| `e99d622b` | `crates/cli/src/describe.rs`, `crates/daemon/src/lib.rs`, `crates/kassiber/Cargo.toml`, `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md`, `tests/rust_port/dispositions.json` |
| `c9bd660b` | `TODO.md`, `crates/pytext/src/lib.rs`, `crates/pytext/tests/data/upper.txt`, `crates/pytext/tests/upper.rs`, `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md`, `tests/rust_port/dispositions.json` |
| `0cd64ff9` | `THIRD_PARTY_LICENSES.md`, `crates/custody/src/components/commitments.rs`, `crates/custody/src/components/mod.rs`, `docs/reference/rust-port.md`, `tests/fixtures/rust_port/error_codes.json`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `3bd2370e` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json` |
| `b3721292` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json` |
| `19dd4d76` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `tests/fixtures/rust_port/error_codes.json` |
| `fea1f681` | `crates/pytext/src/lib.rs` |
| `c3d18194` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json` |
| `1f178e00` | `crates/custody/scripts/mutations.json`, `crates/custody/src/gaps/candidate.rs`, `crates/custody/src/gaps/mod.rs`, `crates/custody/src/gaps/normalize.rs`, `crates/custody/src/gaps/search.rs`, `crates/custody/src/interpretation/text.rs`, `crates/custody/src/lib.rs`, `crates/custody/src/python_text.rs`, `crates/custody/tests/gap_cases.rs`, `tests/fixtures/custody_gap_cases.json` |
| `f2464d20` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json` |
| `d8f3bac6` | `crates/custody/src/evidence.rs`, `crates/custody/src/graph.rs` |
| `3b20eecc` | `crates/reports/scripts/mutations.json`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md` |
| `9e138e0d` | `crates/reports/src/lib.rs`, `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `1eef14a8` | `crates/custody/src/graph.rs` |
| `b5cdb212` | `crates/cli/src/describe.rs`, `crates/runtime/src/store.rs`, `docs/reference/rust-port.md` |
| `85b971a7` | `Cargo.lock`, `crates/wallets/src/lib.rs`, `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `adcd3395` | `docs/reference/rust-port.md` |
| `711bf220` | `crates/wallets/src/lib.rs`, `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `1cb4981b` | `docs/reference/rust-port.md` |
| `3d6dce7d` | `docs/reference/rust-port.md` |
| `72c18e53` | `docs/reference/rust-port.md` |
| `f9ff09d0` | `crates/net/tests/reviewed-dependencies.txt`, `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `0da3346b` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `9517c6ff` | `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json` |
| `004f3963` | `docs/reference/rust-port.md` |
| `6af84154` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `5ef4aa2f` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-shell.md` |
| `5ade4b81` | `Cargo.lock`, `crates/journals/Cargo.toml`, `crates/journals/src/lib.rs`, `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md` |
| `ff7050c0` | `docs/reference/r4-journal-pipeline.md` |
| `6a94f676` | `crates/store/fixtures/compat.json` |

The resolutions use these rules:

- Union CLI registrations, daemon adapters, runtime/book methods, dependencies,
  CI assignments, review maps and documentation checkpoints. Daemon kind order
  follows Python, with transaction export beside transactions, transfer kinds
  after documents and review receipts after rates; retain filtered r6 services.
- Retain the optimized custody indexes, graph caches, pagination and stage APIs.
  Adapt component authoring to `Shared`/`Native` wrappers. Add gap worklist amount
  ranges and incoming witnesses without replacing the optimized search. Share the worklist
  range oracle across both exporters, retaining all 173 incoming range cases.
- Preserve both mutation catalogs. Colliding incoming gap IDs become
  `R7-worklist-GAP-136/137/138`; the original cases retain their IDs. Re-anchor
  mutations to the actual integrated production code.
- Retain the CPython uppercase implementation in `pytext::case` and its exhaustive
  scalar corpus. Remove the redundant unused incoming uppercase module/generator,
  whose different corpus delimiter would overwrite the retained oracle.
- Preserve optimized capital-gains and quantity-report behavior alongside the new
  CSV publication/export helpers. Initialize private custody caches through
  `Snapshot::default()` in new callers.
- Merge both `crates/journals` additions into one crate. Keep rendering in
  `lib.rs` with its boundary/model/render modules, move bench compute to
  `compute.rs`, re-export `compute`, `Computed`, `Fusion` and custody `Stages`,
  keep distinct compute/render refusals, and union features/dependencies. The
  timing feature and required bench feature remain intact.
- Retain owner decisions, bench status and storage design together. The compact
  calculation column is additive in Python and the Rust schema mirror. The audit
  package now expects seven migration audits.

Two overly broad early conflict edits temporarily removed a projection body and
an inventory tail. They were caught, disclosed and restored during replay
(`2d0c9fad` and `1c8642e8`); the generated inventory is then regenerated. No
unrelated user work was present or replaced.

## Integration adaptations

Rust operational redaction excludes the same UUID and 32-hex identity spans as
Python before and after keyed amount replacement. Eight shared-corpus cases pin
the behavior, including case-insensitive Unicode boundaries.

Rust price consumers follow the repaired Python `finite_decimal` boundary:
finite exact/legacy numeric output, exponent at most 308 in magnitude, minimum
nonzero magnitude `1e-13`, canonical zero and authored nonzero scale. Rates,
metadata, commercial documents, importers, payout authoring and recovery CSV projections share the
bounded parser. Payout writes validate both derived rates before persistence;
malformed legacy payouts remain listable with explicit recovery flags. Rust
custody vectors include bounded prices and refusal of a split reviewed payout
source. Derived zero rates also canonicalize after division, including wide
quantities; a frozen vector and mutation pin that regression. The whole-book
tiny-price writer witness meets both the value and derived-rate floors. The
AST-only payout-price oracle supplies the source identity now used by fixed Python. Full-book fixtures are regenerated for the partial-payout
migration, compact column and recovery projection fields.

The Rust store recognizes `custody-partial-payout-pricing-v1` in migration and
admission inventories. Its one-shot invalidation uses effective authored
memberships, source principals and Python's ordered payout projection. Audit
insertion and affected-profile invalidation commit together; retained rows stay
intact. Cross-runtime tests inspect Rust's raw database before Python can repair
an omitted migration.

The harness detects patched `tax_policy.POLICY_BUILDERS` against the production
mapping. Such builds compare the other custody stages but skip fusion with the
explicit reason `patched POLICY_BUILDERS: Rust fusion uses production profile
policy`. Production generic correctly retains global pooling only. Patched and
unpatched regressions validate both comparison and skip inventories.

## Verification

Verification uses Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest --`, `MATURIN_NO_INSTALL_RUST=1`, offline Cargo
and disposable books. Tooling `HOME` is never overridden. Scratch and evidence
live under `target/r7-integration`, with a short disposable `target/t7` for Unix
socket paths. One verification job runs at a time. Mutation and verification
build trees are deleted after their summaries are retained.

### Native and custody gates

| Gate | Result |
| --- | --- |
| Workspace formatting / clippy `-D warnings` | Passed; harness and journals timing features also checked |
| Rust workspace tests and doc tests | 1,492 unique native cases passed; no remaining failed or ignored cases |
| Storage migration suite | 281 passed, including bidirectional page-index upgrades, historical books, rollback and interrupted recovery |
| Shared UUID corpus / webview redaction | Native shared corpus passed; 19 webview tests passed |
| Custody stage/port/generated/fusion/harness modules | 183 unique cases passed after focused corrections |
| Generated custody campaign | 24,000 books and 1,030 permutations; all stages and fusion; zero differences |
| Retained request replay | 159 requests and 159 permutations; zero differences or pre-request refusals |
| Focused census | 192 passed, 12 subtests; 88 snapshots |
| Focused shadow | 192 passed, 12 subtests; 88 snapshots and frozen Python comparisons |

The focused modes cover ownership transfers, database journal safety,
quarantine review, pool policy, direct-payout slices and nonfinite pricing.
Each compares components/evidence/interpretation/gaps/quantity on all 88 builds,
finalized projection on 84, and fusion on 82. Four builds have an unavailable
finalized/fusion result; two additional patched-policy builds explicitly skip
fusion with the recorded reason. No unexpected skip or discrepancy remains.

Workspace evidence is cumulative across resolved retries, keyed by source path
and test name in `workspace-result.json`; doc tests pass separately. Native
wallet tests initially inherited a test-only no-egress switch without harness
hooks. Removing that variable only for the native unit lane exposed one stale
legacy no-rate fixture, which now seeds its intended old state explicitly.
Storage tests originally treated the new pair-free partial-payout historical
book as an overhaul fixture containing a legacy pair. The two applicable legacy
pair fixtures are now explicit; all historical books retain complete-dump parity.
The final 281-case migration run passes. The same distinction is applied to
shell callers: both historical tests and all five applicable subtests pass on
retry, while the other shell cases retain their successful full-run evidence. Custody retries correct the missing gap
exporter union, invalid tiny-price fixture, and derived zero-rate scale; all 30
affected native projection cases pass after the production correction.

The generated campaign records 16,757 ledger requests, 7,948 empty results,
147 builder refusals and 178 upstream refusals across originals/permutations.
Every successful request has a successful engine response, and all raw-stage
outputs and refusals match the Python reference.

### Hybrid acceptance

| Suite | Result |
| --- | --- |
| Shell | 53 parent tests and 339 applicable subtests passed cumulatively |
| Hybrid harness, CLI runner and inventories | 45 parent tests and 750 subtests passed cumulatively |
| Setup | 78 passed |
| Rates | 79 passed |
| Transactions | 53 passed and 97 subtests passed cumulatively |
| Wallets/importers | 422 passed and 56 subtests passed |
| Sync | 160 passed |
| Chain | 30 passed |
| Transfers/review | 25 passed and 33 subtests passed |
| Secrets | 120 passed cumulatively |
| Backup | 1,411 passed cumulatively |
| Reports and CSV exports | 39 passed |
| Diagnostics | 16 passed |
| Hybrid legacy-price recovery | 5 passed |
| Additional store interoperability / partitioning | 378 passed; two native macOS/Windows platform skips |
| Shipped Python corrections, compact storage, explanations and authoring atomicity | 269 passed and 34 subtests passed |
| Frozen render regeneration | 2 passed |

The harness includes 447 JSON importer and 247 River importer cases through
Rust, with whole-database comparisons. Transaction retries update the obsolete
CSV refusal witness and align commercial price validation with fixed Python;
all four document methods and the refusal method pass on retry. The other 50
transaction tests retain their successful original run evidence.

The initial secrets run passed 91 ordinary cases but could not start the
29-case disposable native keyring fixture. Its long workspace temporary path
exceeded the Unix socket limit. All 29 pass with `--basetemp=target/k7`;
no keyring error or skip remains unresolved. Tooling HOME is unchanged.

Backup's initial run passed 1,407 cases and exposed four desktop failures.
A reference-side fingerprint trace showed Python's idle local-watch worker
opening its second SQLCipher connection concurrently with preview: the
fingerprint captured a transient rollback journal and then treated the unchanged
target as stale. The fixture now waits for that real worker's database startup
before requests. It retains the worker and compares unmodified raw responses;
all 11 desktop cases pass in the two focused retries. Production watch,
fingerprint and restore code are unchanged. Concurrent idle-watch startup is
outside this controlled slice comparison; no acceptance of that race or of the
excluded restore recovery fix is implied.

### Routing evidence

Routing records cover all **145 registered CLI leaves** (four shell and 141
service leaves) and all **110 product daemon kinds**. Every supported service
variant is observed in Rust with **zero Python service fallbacks**.

| Owner | Rust CLI leaves | Python service fallbacks | Native daemon kinds |
| --- | ---: | ---: | ---: |
| Shell | 4 | 0 | 4 / 4 |
| Setup | 35 | 0 | 28 / 28 |
| Rates | 6 | 0 | 4 / 4 |
| Transactions | 44 | 0 | 29 / 29 |
| Wallets | 8 | 0 | 5 / 5 |
| Imports | 4 | 0 | 2 / 2 |
| Chain | 0 | 0 | 5 / 5 |
| Transfers | 19 | 0 | 18 / 18 |
| Secrets | 8 | 0 | 2 / 2 |
| Backup | 2 | 0 | 4 / 4 |
| Reports | 14 | 0 | 9 / 9 |
| Diagnostics | 1 | 0 | 0 / 0 |

Counts are distinct leaves/kinds. Raw mirrored route records and daemon replies
can contain duplicates, so their row totals are not invocation counts. Daemon
requests are associated with replies by request ID; readiness advertisements
supply the expected set and are never counted as served requests. Nine existing
witnesses (23 subtests) and two additional typed attachment refusals fill the
earlier recorder's missing request-kind associations; every supplementary
response matches Python bytes.

`transactions.export` is registered for CSV only. Explicit non-CSV exports
continue through reference routing; the captured XLSX witnesses are outside
that support claim. The harness's whole-session Python daemon policy is also
explicit and is separate from service fallback counts. Pairing, sync preparation/
persistence probes and journal rendering retain their unregistered boundaries.

### R4 probes and final checks

| Gate | Result |
| --- | --- |
| Pairing release harness | Built offline |
| Pairing projections | 11 books, four projections each; byte parity throughout |
| R4-1 timing harness | Built with the merged journals crate; 159 requests, zero differences |
| Test classification | Regenerated and checked: 5,684 = 4,200 rust-test + 522 oracle + 543 subprocess + 370 boundary + 49 Python-only |
| Contracts | Regenerated; final check passed |
| Preflight shard | 57 passed, 60 subtests passed |

Captured render parity matches every byte on **811 samples**: 508 across the
12 generator families, 145 additional tax cases (including 25 rendering
refusals), 77 from each focused census/shadow run, and four complete bench
books (generic and multi-wallet at 2k and 20k). The largest multi-wallet book
contains 300 paired pegs, 19,983 rendered entries and 17 quarantines.
All four disposable databases are removed after retaining their manifests and
render captures. Three generator and 21
tax cases refuse before rendering; four generator scenarios refuse upstream.
Those are tracked separately and are not counted as rendering parity. The
88 custody builds per focused mode and the 77 captured render inputs measure
different boundaries.

### Mutation catalog

| Catalog | Recipes | Killed | Survivors / errors |
| --- | ---: | ---: | ---: |
| backup | 107 | 107 | 0 / 0 |
| chain | 22 | 22 | 0 / 0 |
| custody | 2,292 | 2,292 | 0 / 0 |
| diagnostics | 98 | 98 | 0 / 0 |
| imports | 14 | 14 | 0 / 0 |
| journals | 39 | 39 | 0 / 0 |
| rates | 9 | 9 | 0 / 0 |
| reports | 532 | 532 | 0 / 0 |
| secrets | 18 | 18 | 0 / 0 |
| setup | 59 | 59 | 0 / 0 |
| transactions | 227 | 227 | 0 / 0 |
| transfers | 215 | 215 | 0 / 0 |
| transfers: pairing | 35 | 35 | 0 / 0 |
| wallets/importers | 169 | 169 | 0 / 0 |
| wallets: acquisition | 19 | 19 | 0 / 0 |
| wallets: BDK | 114 | 114 | 0 / 0 |
| wallets: discovery | 20 | 20 | 0 / 0 |
| wallets: observer | 82 | 82 | 0 / 0 |
| wallets: persistence | 112 | 112 | 0 / 0 |
| wallets: records | 83 | 83 | 0 / 0 |
| tax-engine | 186 | 186 | 0 / 0 |
| **Total: 21 catalogs** | **4,452** | **4,452** | **0 / 0** |

Every named unmutated baseline passes. The checker treats build failures as
errors, never kills. The final `mutation-catalog-final.json` verifies the exact
current recipe ID set, catalog and per-recipe hashes, and uniqueness within
every catalog. Catalogs run serially; each build tree is removed immediately
after its summary is saved. The settled production implementation is
`c8b13768`; subsequent commits change mutation anchors and test witnesses only.

The transaction `document-exact-fiat` recipe initially had a stale anchor to
the removed `decimal(v)?` parser. It now changes the bounded production
`finite_price(&py_str(v))` call to `finite_price("0")`. Static checks and the
full transaction catalog validate that corrected production mutation.

The initial custody run killed 2,291 recipes and left `projection2-btc-context`
alive. The repaired price floor makes its old wide-quantity witness refuse
before denominator rounding matters. Two additional priced vectors preserve
the same wide quantity while keeping both value and derived rate admissible.
All old vectors and activity counts remain unchanged; five Python port cases
and all 25 native projection cases pass. The focused retry passes its baseline
and kills the remaining recipe. The final receipt combines the disjoint
2,291 original kills and that one retry; the original survivor log is retained.

The initial transfers run killed 211 recipes and left four operation-atomicity
recipes alive. Their Python witnesses compared the database only after the
caller's rollback. The shipped explicit-BEGIN fix lets that cleanup conceal
a removed operation savepoint. The witnesses now also compare the complete
database immediately after refusal, before caller cleanup. Both affected
authoring modules pass all 58 cases; the focused retry passes its baselines
and kills all four recipes. The final receipt combines the disjoint 211 and
four kills, retaining the initial survivor log. Neither witness correction
changes production code or removes an existing assertion.

## Evidence and limits

Local evidence is in `target/r7-integration`: ordered `inputs.json` and
`replay.json`, `conflicts.jsonl`, provenance output, per-gate logs and summaries,
served-by records and mutation kill IDs. `final-gates.json` records the refreshed
classification, contracts, preflight, recipe-ID and provenance checks;
`cleanup.json` records removal of build and disposable test trees.
The duplicated conflict-log record for
`8a93e945` is deduplicated in this audit's counts. Original replay messages are
never rewritten to describe later adaptations.

The owner runs the full census and custody-shadow shards. Pairing, native sync
preparation/persistence fixtures and journal rendering retain their documented
unregistered boundaries; this integration does not enable production journal
routing or claim completion of R4-11 performance/legacy-input acceptance.

A manual second pass checks the authored resolutions, shared redaction corpus,
bounded price consumers, partial-payout migration order and audit admission,
custody cache preservation, merged compute/render crate, policy skip inventory,
strengthened mutation witnesses and generated inventories. Staged and unstaged
diffs are reviewed separately. No unresolved P1/P2 correctness or consistency
finding remains, and no conflict requires an accounting or authority design
decision. Full census/shadow and the explicitly incomplete slice acceptance
remain with their owners as stated above.

## Ordered replay map

| Input | Original | Integrated |
| --- | --- | --- |
| page | `36070685` | `5fabc039` |
| python | `133cde70` | `6352441b` |
| python | `def793f9` | `64331648` |
| python | `4ad4e698` | `9ea9532d` |
| python | `95f61f79` | `2d0c9fad` |
| python | `8a93e945` | `1c8642e8` |
| redaction | `e60dc50c` | `54efa465` |
| transfers | `e99d622b` | `aa4e13cf` |
| transfers | `c9bd660b` | `2f89f895` |
| transfers | `26e64be8` | `bcb85c43` |
| transfers | `2091516c` | `c59a5ccc` |
| transfers | `0cd64ff9` | `78f8f780` |
| transfers | `3bd2370e` | `5d796079` |
| transfers | `b3721292` | `f307e2c3` |
| transfers | `be959dd6` | Skipped (duplicate) |
| transfers | `19dd4d76` | `a89ab115` |
| transfers | `01e113a8` | `179cba3a` |
| transfers | `fea1f681` | `90972c59` |
| transfers | `c3d18194` | `2f318293` |
| transfers | `1f178e00` | `92001fcd` |
| transfers | `f2464d20` | `475078f5` |
| pairing | `d8f3bac6` | `221361e3` |
| reports | `3b20eecc` | `d578e477` |
| reports | `2bc2285e` | `ee9d2851` |
| reports | `d251b6b0` | `0f36b6c8` |
| reports | `8d7f1332` | Skipped (duplicate) |
| reports | `6d7c43e5` | `eae5f2de` |
| reports | `9e138e0d` | `a651e853` |
| reports | `af2c50d2` | `54760e81` |
| reports | `1eef14a8` | `f9faed76` |
| reports | `a97296d3` | `fbc57014` |
| reports | `eb625be4` | `4c9149b6` |
| reports | `b5cdb212` | `2eb0b09e` |
| sync | `85b971a7` | `80367577` |
| sync | `adcd3395` | `51e70475` |
| sync | `711bf220` | `413c0bf6` |
| sync | `1cb4981b` | `26e9b266` |
| sync | `3d6dce7d` | `bd4c3132` |
| sync | `72c18e53` | `4593abc2` |
| sync | `f9ff09d0` | `c5f4db4a` |
| sync | `0da3346b` | `426660db` |
| sync | `9517c6ff` | `56512de5` |
| sync | `ad51a96a` | `60748d57` |
| sync | `f9cb0008` | `2a1d7300` |
| sync | `77610082` | `c1eafdbd` |
| sync | `adba1f6f` | `50a90bb2` |
| sync | `25f67c77` | `48a94491` |
| sync | `004f3963` | `5d43fca3` |
| sync | `6af84154` | `3039f92c` |
| importers | `5ef4aa2f` | `7a7ebd1a` |
| importers | `86d364cf` | `acb3c3a2` |
| importers | `1b3d5ae2` | `5d5b6da1` |
| r4 | `52b1e2b9` | `904ad810` |
| r4 | `ae79db2a` | `baa5c730` |
| r4 | `5ade4b81` | `5591fee0` |
| r4 | `ff7050c0` | `53625d38` |
| r4 | `6a94f676` | `cf370bf5` |
