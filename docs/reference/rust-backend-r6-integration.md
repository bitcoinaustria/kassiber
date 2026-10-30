# R6 integration audit (R4-0)

Local branch: `t3code/rust-backend-r6`, based on `50f67616` (`t3code/rust-backend-r5c`).
This audit follows [round 2](rust-backend-r5-integration.md#round-2-completed-audit-r5b).
No push, PR, merge commit, source-branch edit, stash operation, or dependency upgrade is part of this integration.

## Provenance

The requested ranges contain 87 candidate commits. The stack applies 86 original
commits, preserving their complete messages and relative order, and skips one
empty patch. Three integration adaptations and this audit/status commit are
separate from the picks: 90 commits above the base, with no merge commits.

| Input line | Candidates | Picks | Skips |
| --- | ---: | ---: | ---: |
| Custody through `1b62f405` | 28 | 27 | 1 |
| Projection review fixes | 2 | 2 | 0 |
| Custody performance | 40 | 40 | 0 |
| Fusion | 7 | 7 | 0 |
| Read performance | 6 | 6 | 0 |
| Diagnostics | 1 | 1 | 0 |
| Shipped Python fixes | 2 | 2 | 0 |
| R4 design | 1 | 1 | 0 |
| **Total** | **87** | **86** | **1** |

`git cherry 50f67616 1b62f405` reports all 28 custody candidates as unique.
The first, `cc18b710`, becomes empty after retaining the integrated lockfile,
license purposes and generated inventory context: its native HTLC code is
already present. It is skipped rather than manufacturing an empty commit.
`git cherry HEAD t3code/python-reference-fixes` confirms the fixed-point fix
`ca9a672f` is already present through `c80ee752`. The nonfinite-pricing fix
`117eb35d` remains `+` because its patch context differs from `077d9f28`;
comparing every added/deleted line and path proves the changes identical.
Neither fix is picked twice. `fc046429` and `95cd338b` are both picked.

All 86 original/picked full-message comparisons pass. The local evidence stores
an explicit original-to-picked mapping, input tips and the ordered replay log.

## Conflicts

Twenty-three candidate commits encounter textual conflicts, including the
already-integrated empty first patch. Generated count conflicts retain temporary
local counts and are regenerated with `classify_tests update`; generated
contracts are regenerated with `tests.rust_port.contracts update`. Review maps
retain every R5 entry and incoming custody entry, with newer custody review
wording replacing only the corresponding earlier entry.

| Original | Conflicting paths |
| --- | --- |
| `cc18b710` | `Cargo.lock`, `THIRD_PARTY_LICENSES.md`, `docs/reference/rust-port.md` |
| `16234030` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `23c059dc` | `docs/reference/rust-port.md` |
| `7969fa8e` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `180edd62` | `docs/reference/rust-port.md` |
| `a4151d44` | `docs/reference/rust-port.md` |
| `9d2ab49c` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `fc30eeb8` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `6ba5f6bf` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `dfb72b16` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `72325e38` | `docs/reference/rust-port.md` |
| `746cda8c` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `84b2181b` | `docs/reference/rust-port.md` |
| `1b62f405` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md` |
| `86cc2538` | `docs/reference/rust-port.md` |
| `f2c8bfdc` | `crates/custody/scripts/mutations.json`, `crates/custody/src/finalized_projection/runtime.rs` |
| `8ccb4707` | `docs/reference/custody-engine.md` |
| `5daa0367` | `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `580015d8` | `crates/custody/scripts/mutations.json`, `crates/custody/src/interpretation/holds.rs`, `docs/reference/rust-port.md`, `tests/rust_port/reviews.py` |
| `e2be9b41` | `crates/custody/Cargo.toml`, `docs/reference/custody-engine.md`, `docs/reference/rust-port.md` |
| `0907ca83` | `crates/custody/src/lib.rs`, `docs/reference/rust-port.md`, `tests/custody_engine_compare.py`, `tests/rust_port/dispositions.json`, `tests/test_custody_shadow_harness.py` |
| `ce362a66` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-port.md`, `docs/reference/rust-shell.md` |
| `9755246f` | `docs/plan/21-rust-backend.md` |

The authored resolutions preserve both sides:

- `Cargo.lock` retains the integrated graph; offline Cargo metadata confirms
  the required union. Existing broader license purposes remain intact.
- Plan, shell and custody reference conflicts retain the R5 checkpoints and
  add the custody, performance, fusion, diagnostics and R4 design sections.
- `f2c8bfdc` retains the move-based operand construction and the projection
  review's `blocked_ids(interpreter, projection)` union. Its mutation anchor
  follows the resulting construction.
- `c3a5e8f9` applies cleanly; review confirms its borrowed conversion rows retain
  the projection fixes. The reviewed row-sort mutation is re-anchored to
  `sort_by_cached_key`, preserving the same equivalent removal mutant.
- `580015d8` combines shared concurrent confirmation facts with fusion's
  authoritative captured-confirmation path. Ordinary interpretation passes
  `closed=false`; ledger conflict/pending adapters derive facts and pass
  `closed=true`. Both catalog tails remain; an array delimiter was corrected
  in the following fusion pick before validation.
- `0907ca83` retains concurrent/direct stage serialization and appends fusion.
  Decision-only output explicitly skips both finalization and fusion. The
  stronger projection-review refusal accounting, stage checks and regression
  cases are retained alongside fusion checks.
- Diagnostics and read-performance declarations coexist, as do the custody
  timing feature and the fusion probe's development dependency.

No conflict required an accounting or authority design decision.

## Integration adaptations

The fixed Python reference remains authoritative. Rust applies payout terms only
when the decision is a reviewed payout source and its component matches. Residual
slices retain ordinary pricing, and carrying payout rows are emitted once.
Real-writer cases now assert this corrected behavior and exact decimal terms;
synthetic payout witnesses explicitly identify their reviewed source decision.
Two additional mutation recipes independently remove the slice and component
checks. Legacy nonfinite tests and existing refusal barriers remain.

Automatic diagnostics filenames retain `generated_at` and sanitize only filename
characters. Exclusive creation adds `-2`, `-3`, etc. on collisions; explicit custom
targets retain overwrite behavior. CLI parity covers timestamp names, repeated
exports and preservation of earlier reports. Sequential byte comparisons use
equivalent initial output directories so a prior reference write does not create
an artificial native-only collision.

Reports now initialize custody snapshots with an empty internal row cache through
`Snapshot::default()` and then populate their component-admission facts. This
preserves the cache's private boundary and report admission behavior.

Fusion appends its result to lossless Python-model stage values rather than
round-tripping existing diagnostics through serde JSON, which replaced lone
surrogates. The migration-surrogate regression and full generators verify this.
Quantity checks explicitly expect fusion to be skipped with finalization.
The old partial-carry duplicate-row refusal witness now proves success; separate
raw component and finalized fanout refusals retain exact stage comparisons.
All newly picked CLI subprocess tests use `tests.cli_runner`.

## Verification

Verification uses Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest --`, `MATURIN_NO_INSTALL_RUST=1`, offline Cargo,
and disposable books/ownership roots. Tooling HOME is never overridden.
The settled implementation is `7e2da3fb`; the later mutation-anchor and final
audit commits change no production code. The stable copied harness avoids
overlapping Cargo rebuilds changing a running parity suite's executable. Elevated
fixture runs permit only the existing local socket/keyring test resources; no
live provider acceptance is claimed.

### Native and custody gates

| Gate | Result |
| --- | --- |
| Workspace formatting | Pass |
| Workspace and test-harness clippy, `-D warnings` | Pass |
| Workspace tests | 1,294 passed; zero failed or ignored |
| Custody stage/port/generated/fusion/harness modules | 181 passed |
| Generated books | 24,000 books and 1,030 permutations; zero differences |
| Generator outcomes, including permutations | 16,757 requests, 7,948 empty, 147 builder refusals, 178 upstream refusals |
| Recorded requests | 159 requests and 159 permutations; zero differences |
| Focused census | 63 passed, 21 subtests; 27 snapshots |
| Focused shadow | 63 passed, 21 subtests; 27 frozen Python comparisons |
| Gap fanout regression | Passed explicitly and in the complete custody gate |
| Interpretation fanout and full-stage vectors/wrappers/refusals | All 13 interpretation tests passed; included in complete custody gate |
| Projection parity focus | 17 passed |
| Quantity stage/port parity | 30 passed |
| Picked Python fixes and CLI construction checks | 35 passed, 6 subtests |
| Classification update and preflight shard | Updated; 57 passed |

For both focused census and shadow, components, evidence, interpretation, gaps
and quantity compare all 27 snapshots. Finalized projection and fusion compare
25 and skip the same two upstream-inadmissible snapshots. These are explicit
stage refusals/skips, not service fallbacks. No injected builds are used.

### Hybrid acceptance

| Suite | Result |
| --- | --- |
| Shell parity | 50 passed, 278 subtests |
| Hybrid candidate, CLI runner, inventory, contracts and CI shards | 43 passed, 53 subtests |
| Setup | 78 passed |
| Rates | 53 passed |
| Transactions | 53 passed, 97 subtests |
| Wallets/imports | 416 passed, 54 subtests |
| Chain | 30 passed |
| Secrets | 91 passed; 29 native keyring cases passed on retry |
| Backup | 1,411 passed |
| Reports | 19 passed |
| Diagnostics | 13 passed |

The first secrets run skipped 29 cases because its disposable Secret Service
could not unlock. Rerunning those cases with a short disposable socket directory
passed all 29; no native keyring skip remains unresolved. The native lifecycle
cases use the same copied candidate binary.

The initial temporary routing recorder tried to parse intentionally malformed
oracle output and wrapped `cli_runner.prefix` during its own unit test. Those
recorder failures are excluded. Shell parity passes both without recording and
with the corrected malformed-output handling; the remaining 43 tests pass with
the runner unit tests exempted from the temporary prefix wrapper. No product
behavior was changed to accommodate the recorder.

The suite module lists match round 2, with the diagnostics slice added. Routing
records cover all **116 service CLI leaves**, all four shell leaves and all
**88 product daemon kinds**. Every service leaf is observed in Rust with
**zero Python service fallbacks**. Ten supplementary native backend, import and
attachment daemon requests match Python response bytes, including typed errors.

| Owner | Observed Rust CLI leaves | Python service fallbacks | Observed native daemon kinds |
| --- | ---: | ---: | ---: |
| Shell | 4 | See routing policy below | 4 |
| Setup | 35 | 0 | 28 |
| Rates | 6 | 0 | 4 |
| Transactions | 44 | 0 | 29 |
| Wallets | 8 | 0 | 5 |
| Imports | 2 | 0 | 2 |
| Chain | 0 | 0 | 5 |
| Secrets | 8 | 0 | 2 |
| Backup | 2 | 0 | 4 |
| Reports | 10 | 0 | 5 |
| Diagnostics | 1 | 0 | 0 |

This table counts distinct leaves/kinds rather than invocations: mirrored
module logs contain duplicate invocation records. `served-by-final.json` retains
the complete per-command evidence and raw counts. The hybrid harness's explicit
whole-session Python daemon policy is exercised separately; its three recorded
Python session routes are not service fallbacks. Unported leaves continue to use
explicit reference routing and are outside this claim.

### Mutation catalog

| Catalog | Recipes | Killed | Survivors / errors |
| --- | ---: | ---: | ---: |
| backup | 107 | 107 | 0 / 0 |
| chain | 20 | 20 | 0 / 0 |
| custody | 2,285 | 2,285 | 0 / 0 |
| diagnostics | 98 | 98 | 0 / 0 |
| imports | 14 | 14 | 0 / 0 |
| rates | 9 | 9 | 0 / 0 |
| reports | 279 | 279 | 0 / 0 |
| secrets | 18 | 18 | 0 / 0 |
| setup | 59 | 59 | 0 / 0 |
| transactions | 226 | 226 | 0 / 0 |
| wallets | 83 | 83 | 0 / 0 |
| tax-engine | 186 | 186 | 0 / 0 |
| **Total** | **3,384** | **3,384** | **0 / 0** |

All named unmutated baselines pass before their recipes run. The checked-in
checker rejects build failures as errors; none is counted as a kill. The final
ID audit proves every current catalog recipe ran exactly once, with no missing
or duplicate IDs, including the two new payout-authority witnesses.

Custody starts in eight strided partitions. Two unstarted partitions are
rebalanced into disjoint 235- and 50-recipe parts to limit the final wait, with
at most eight catalog workers active. Each part uses the unchanged checker and
its own baselines. `mutation-rebalance.json` records the IDs; the original
scheduler's planned partition sizes remain in its raw metadata. Executed-ID
counts in `mutation-catalog-final.json` are authoritative. Per-worker summaries
are written before deleting their build targets; no mutation build tree remains.

The reports `quantity-unit` recipe initially survived because its old division
constant matched only the new scaling-equivalence unit-test oracle. The
read-performance implementation uses `Decimal::from_parts(..., -11)` in
production. The recipe now mutates that exponent to `-8`, retaining the same
holdings-vector killer. Its fresh unmutated baseline passes and the production
mutation is killed. The other 278 report recipes already passed their baselines
and kill checks; their raw catalog result retains the resolved survivor rather
than being rewritten. The final ID audit combines those 278 with the corrected
production recipe, excluding the stale test-only mutation.

The earlier interrupted attempt is excluded entirely after the lossless fusion
fix settled. Its logs are retained separately under `superseded-mutations/`;
none of its kills enters this table. The completed workspace verification build
tree is also removed after summaries and the copied parity harness are retained.

## Evidence and limits

Evidence is local under `/tmp/kassiber-r6-verification/`: `inputs.json`,
`replay.json`, `provenance.json`, generator `campaign-*.json`, native and Python
logs, slice served-by JSONL, and per-catalog mutation summaries. Mutation scratch
and target trees are deleted after summaries are written. Interrupted mutation
attempts are kept separate and never counted as kills.

The owner retains the full census and custody-shadow shards. The input slices'
platform and unported-feature limits still apply; this is local integration,
not completion of R3/R5/R7 or a release-readiness claim.

A manual second pass reviewed the authored resolutions, payout authority,
snapshot-cache construction, lossless fusion serialization, fixtures and test
inventories. No unresolved P1/P2 correctness or consistency finding remains.
No accounting or authority decision was needed to finish R4-0. The R4 design's
future owner decisions remain open.

## Ordered replay map

The adaptation commits are `3ad0623c`, `7e2da3fb` and `dd6cc183`; the
audit/status commit follows them. The table lists original picks in requested order.

| Input | Original | Integrated |
| --- | --- | --- |
| custody | `cc18b710` | Skipped (empty) |
| custody | `16234030` | `f5df40b1` |
| custody | `23c059dc` | `18336c2a` |
| custody | `7969fa8e` | `5ad056de` |
| custody | `180edd62` | `aeadbe0e` |
| custody | `a4151d44` | `db9e8ac0` |
| custody | `9d2ab49c` | `54e168f3` |
| custody | `2951cdc8` | `7f8ec83b` |
| custody | `122f64d6` | `39c16875` |
| custody | `618ae9f3` | `20530538` |
| custody | `a89ce4fd` | `e0977c6d` |
| custody | `0e1e270b` | `54ab0527` |
| custody | `fc63c1e0` | `d966e650` |
| custody | `fc30eeb8` | `dd60ca28` |
| custody | `aa44ab11` | `57ab9db3` |
| custody | `6ba5f6bf` | `eda353be` |
| custody | `dfb72b16` | `15ae13b5` |
| custody | `6300605e` | `7297d837` |
| custody | `077d9f28` | `5202baa3` |
| custody | `c80ee752` | `c65ee21e` |
| custody | `72325e38` | `2c59f074` |
| custody | `d5f196fb` | `ed782938` |
| custody | `8c6e26dc` | `fb9e86e2` |
| custody | `746cda8c` | `fb5bc9a1` |
| custody | `84b2181b` | `3cac87a5` |
| custody | `e9639bf2` | `32f59763` |
| custody | `e27fd57e` | `8fd3ead9` |
| custody | `1b62f405` | `cc2cc40d` |
| projection | `052f06fe` | `329a23df` |
| projection | `86cc2538` | `6a0fb3c7` |
| performance | `e417714a` | `58dd47b3` |
| performance | `5293a3f9` | `1a45a092` |
| performance | `d103bb54` | `ec96f57e` |
| performance | `947a7b46` | `a1a4271a` |
| performance | `bc4844f4` | `20dd528c` |
| performance | `a1cbaae2` | `73311081` |
| performance | `fa50f134` | `faf98961` |
| performance | `6d71f4b2` | `1cb073cd` |
| performance | `fdf27d92` | `65639337` |
| performance | `aa77740e` | `635ce594` |
| performance | `feb71b20` | `aac25d29` |
| performance | `148d5265` | `65704997` |
| performance | `6a48af66` | `9c1c8e1f` |
| performance | `4f082910` | `4e90bb02` |
| performance | `917265ab` | `33cf640a` |
| performance | `10082c2f` | `bca71b1e` |
| performance | `1e0d1bbe` | `8896e07d` |
| performance | `0cbb084e` | `ca388915` |
| performance | `f2c8bfdc` | `6f3c84ae` |
| performance | `6ba2d28c` | `6bdc2d55` |
| performance | `a5ba41b3` | `5a0bccb1` |
| performance | `36100630` | `efb34d33` |
| performance | `18d96d2c` | `e327a089` |
| performance | `b7dba17f` | `e8fefced` |
| performance | `ce4e794a` | `53ed3ce3` |
| performance | `b825b1d2` | `a96abb6d` |
| performance | `77bab52d` | `eeaa6adf` |
| performance | `8355a0e6` | `2454abf1` |
| performance | `32c2f443` | `546d7082` |
| performance | `bdc58c0c` | `3e216ade` |
| performance | `10645f48` | `046bcfe7` |
| performance | `c3a5e8f9` | `2086be1f` |
| performance | `4886a53c` | `6460d121` |
| performance | `3a36ecfb` | `e19d38b9` |
| performance | `dd50ce71` | `a46dc300` |
| performance | `988bc8bf` | `decc5278` |
| performance | `7cd01606` | `c33637f7` |
| performance | `8f1734dc` | `225c212b` |
| performance | `a127286c` | `e724ff0f` |
| performance | `8ccb4707` | `5e3f6c7e` |
| fusion | `b660c565` | `4e39e32b` |
| fusion | `5daa0367` | `0ed1754b` |
| fusion | `76375c27` | `5e844f18` |
| fusion | `580015d8` | `b2eefbd5` |
| fusion | `e2be9b41` | `a941e1c3` |
| fusion | `0907ca83` | `f32a38c1` |
| fusion | `76439c46` | `3a1a17cd` |
| reads | `2ede99fa` | `c17539b4` |
| reads | `3d898f9a` | `fd156664` |
| reads | `7a56cc1c` | `502296da` |
| reads | `5f3f5c62` | `56eeeb51` |
| reads | `71081073` | `7f49a0bf` |
| reads | `f261b9b2` | `e30e4ed3` |
| diagnostics | `ce362a66` | `dc53a1c9` |
| python | `fc046429` | `5360b17e` |
| python | `95cd338b` | `4c0fa18c` |
| design | `9755246f` | `222ba242` |
