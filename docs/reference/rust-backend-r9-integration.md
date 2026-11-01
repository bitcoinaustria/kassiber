# Rust backend R9 integration audit

**Status: ordered replay complete; combined verification in progress.**
The local `t3code/rust-backend-r9` stack starts at `9f7471bf7`, retaining
R8 (`3d88f24fa`) and the first-attempt checkpoint. All requested source lines
are now integrated through transfers `61e6b66e7`. No push, PR, remote comment,
stash pop/drop, or worker delegation occurred.

## Requirement and method

Preserve both branch behaviors, original source messages and source order in
one linear history. Follow the [R8 method](rust-backend-r8-integration.md):
inventory each requested range, inspect `git cherry` and matching subjects,
compare repeated stage patches, and pick duplicated records only once.
Documentation status is evidence, not a design conflict. Newest evidence wins;
inherited results below do not constitute fresh combined R9 acceptance.

The inventory contains **186 candidate occurrences** including the three-commit loader
determinism addendum. **63 source commits were picked and 123 occurrences
skipped**, plus one consolidated stage-adaptation commit
(`f8219f49b`). This gives **64 replay commits above `9f7471bf7`**, before the
subsequent integration-fix and verification commits. The inventory records repeated ancestry as skipped occurrences and uses the
requested continuation boundaries for the later branch lines.

## Source provenance and replay counts

Each row's acceptance, campaign, mutation and timing evidence is
**inherited from the listed branch@sha; pending fresh r9 verification**.
The fresh R9 verification section below records the checks rerun here.
Source checkpoint documents retain historical results and their original scope.

| Line / inherited provenance | Picked | Skipped |
| --- | ---: | ---: |
| `t3code/r4-9-loader-9@e15900677` | 7 | 3 |
| `t3code/r4-9-loader-blob@d03c0b9a1` | 1 | 9 |
| `t3code/r4-9-loader-determinism@aaf6216e3` (late addendum) | 1 | 2 |
| `t3code/r4-14-auto-pair@c47450757` | 1 | 0 |
| `t3code/r4-10-prebuild-4@9e8979d30` | 5 | 0 |
| `t3code/r4-11-render-5@4656ef30e` | 3 | 0 |
| `t3code/r4-12-persist-3@51bd20498` | 2 | 1 |
| `t3code/r4-13-journals-process-2@98402b6b4` | 2 | 13 |
| `t3code/custody-perf-4@19792ea1a` | 2 | 7 |
| `t3code/perf-conversions@60b1a4506` | 1 | 7 |
| `t3code/r5-sync-22@a8511d099` | 11 | 4 |
| `t3code/r5-importers-9@71b10d358` | 6 | 7 |
| `t3code/r5-importers-b@5a0c768cc` | 1 | 11 |
| `t3code/r5-reports-16@9338364db` | 4 | 5 |
| `t3code/r5-reports-b@0475d3e49` | 1 | 8 |
| `t3code/r5-reports-platform-csv@013bb9ab2` | 1 | 9 |
| `t3code/r5-reports-privacy@4014edb6f` | 1 | 9 |
| `t3code/r5-reports-lightning-4@d1178136a` | 5 | 9 |
| `t3code/r5-sof-reads@b9b050083` | 2 | 9 |
| `t3code/r5-sof-mutations@62d4cd51a` | 1 | 10 |
| `t3code/r5-transfers-13@61e6b66e7` | 5 | 0 |

The custody performance row includes `a7da5b474` and its `19792ea1a`
checkpoint. Both SOF continuations share `a99fa77c8`, picked once. Importer-B
shares `424d68180`; loader-BLOB shares `c5966c1ff`; those ancestors occur once.

Specific deduplication decisions:

- `1f064db49` repeats the four baseline records already retained on R8 via
  `2d245d19e`; no additional record is appended.
- `59c5f062f` only restacks loader compatibility and would regress newer scalar
  fidelity; the later native-loaded composition preserves its useful caller
  contract without reintroducing the old conversion.
- Thirteen repeated journal-stage subjects were compared with `git range-diff`.
  `98402b6b4` contains real process/lifecycle-clock changes beyond its repeated
  subject. These and the `658e91260` shared-row constructor adjustment are
  retained once in `f8219f49b` (Integrate journal process stage adaptations).

## Conflict resolutions and compatibility fixes

The automated replay receipts record **47 conflicted candidates, 163 path
occurrences and 39 distinct paths**, including the skipped `59c5f062f`.
The seven initial loader picks add **26 path occurrences**. These are separately
recorded counts; the manual stage adaptation is not counted as an additional
cherry-pick conflict. Rerere resolutions were inspected rather than trusted.
No irreconcilable code-behavior conflict was found. All 63 retained source
commit messages are byte-identical to their originals; no merge commit occurs
above the requested starting checkpoint.

Resolutions preserve shared-column `Record` storage and private cached row views,
strict typed JSON admission, native SQLite scalar fidelity, and literal mapping
identity for the internal byte-wire tags. Journal composition uses the native
loaded snapshot without routing BLOBs through JSON. Typed ledger responses and
borrowed renderer operands retain exact/dynamic storage-encoding fallback checks.

Independent importer, report, source-funds, sync and transfer registrations are
unioned. BTCPay, generic ledger and Wasabi tests/helpers retain their complete
ID tapes, per-step extras, refusal precedence and frozen inventory clock.
The daemon ready list follows Python ordering, including source-funds reads,
privacy hygiene, late cross-check and transaction export kinds.

The combined runtime dependency graph exposed `journals -> transfers -> journals`.
The unregistered `review_workflow` and `review_effects` composition modules now
live in `crates/journals/src/`, above transfer primitives. Transfers only uses
journals as a dev-dependency. No third-party package was added.
Mutation catalogs retain all recipe IDs; independent additions and both-changed
killer-test lists are unioned. Anchors affected by the integrated layout are
updated to the same behavior in its new location.

The loader determinism addendum supersedes both older loader status records:
R4-9 is fully accepted on `t3code/r4-9-loader-determinism@aaf6216e3`, with no
pending exceptions. The typed validation refusal closes all forty historical
seed-dependent messages. `e15900677` and BLOB integration `795458939` were
already represented, so only `aaf6216e3` was added, immediately after BLOB.
The later stack was replayed in place without changing source messages.
This adds four source-conflict path occurrences and five metadata-only replay
conflict occurrences; there is no new behavioral conflict. Renderer, persistence and process
remain partial, unregistered stages. Lightning acquisition/preparation does not
establish product routing or capability cutover. SOF mutations remain a frozen-case
persistence foundation. The SOF reads continuation supplies the honest native debug
contract; privacy's corrupt-marker diagnostic gap remains explicitly scoped.

The loader addendum passes a fresh combined R9 loader/BLOB oracle: **274 / 274 tests**, including the eight-seed revision refusals. Its full source acceptance and historical timing retain the scope of `t3code/r4-9-loader-determinism@aaf6216e3`; the remaining combined gates are tracked below.

Hybrid parity exposed one interaction between the custody performance conversion and the loader BLOB continuation: owned finalization retained the private byte tag as a literal mapping in four live BLOB-note cases. Reserved-tag objects now use the shared lossless codec, while ordinary objects keep the owned conversion. The regression proves both native-byte identity and escaped literal-map identity; all 274 loader/BLOB cases then pass with zero writes and exact projection comparisons. A new `projection-owned-byte-identity` mutation pins the fix. No oracle was relaxed.

## Fresh R9 verification

All commands use Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest --`, `MATURIN_NO_INSTALL_RUST=1`, offline Cargo,
`CARGO_BUILD_JOBS=8` and disposable data roots. `HOME` is unchanged. Scratch is
under this worktree's `target/r9-integration`; Cargo output is under
`target/r9-build`, including an explicit build-dir override. One test job runs
at a time. Task-owned build trees must be summarized and removed at completion.

| Check | Fresh R9 result |
| --- | --- |
| Locked environment | Passed, offline, CPython 3.11.17 |
| Workspace formatting | Passed after compatibility fixes |
| All-target workspace clippy `-D warnings` | Passed with journal harness/timing, binary, chain/wallet harnesses and network observer features |
| Workspace tests | Ordered stack before the owned-converter fix: 1,889 passed across 143 harness results; zero failed or ignored. Fresh post-fix workspace run pending; the new focused regression passes |
| Binary / network-observer harness tests | 86 passed across nine harness results; zero failed or ignored |
| Mutation catalog anchors | All 6,543 integrated recipes across 42 catalogs apply with zero problems; the owned-converter fix adds one recipe (6,544 total), with final applicability and full kills pending |
| Test classification | Regenerated 6,022 definitions; final contract check/preflight pending |
| Custody generated/replay/focused lanes | 24,000 books + 1,030 permutations: 24,852 complete stage matches and 178 matching refusals; 159 archived requests + 159 permutations match. Focused census and shadow each pass 300 tests and 32 subtests, with 75 native-loader byte matches and one explained panic-hook skip |
| Complete renderer campaign | 24,705 / 24,705 generated admitted cases and 318 / 318 archived cases match |
| Slice/hybrid parity and no-fallback assertions | In progress: 2,246 distinct nodes pass through the loader/BLOB suites. Four BLOB-note failures are corrected and the full 274-case loader/BLOB rerun passes; remaining modules pending |
| Full mutation kills in resumable shards | Pending fresh run |
| Combined journals process / custody CPU | Pending fresh run |

The saved pre-fix workspace run uses the combined harness and observer features and
passes all 1,889 tests in one uninterrupted run. The owned-converter correction requires a fresh workspace result. The earlier run and its 73
initial failures/rechecks remain historical receipts, rather than the final
coverage claim. The all-target feature check had also exposed a freshness
fixture with obsolete `AppError` fields; the fixture is adapted and clippy passes.
After saving the exact log hash and counts, 221 completed test executables were
removed (4.988 GB unlinked), bringing the verification build tree to 7.3 GiB.
Rebuilt debug fixtures, the hybrid candidate and optimized journal probes pass.
The custody/render receipts above precede the late mixed-revision guard; the
generated snapshots do not exercise SQLite revision loading. The 274-case loader/BLOB suite reruns the affected boundary; the owned-converter fix also requires refreshed custody/render campaigns and native builds. The owner will run full census and shadow shards afterwards; focused lanes
remain part of this integration's required work.

## Remaining verification

Finish workspace checks and harness builds; run 24,000 generated books and 1,030
permutations, 159 retained recordings and permutations, focused census/shadow,
every slice's hybrid parity with no fallback for registered shapes, all mutation
recipes in resumable serial shards, combined process/custody benchmarks on four
books with host load, final classifier/preflight and manual review. Update this
audit with exact receipts and remove inherited-pending labels for rerun checks.

## Original-to-integrated replay map

| Source | Integrated | Original subject |
| --- | --- | --- |
| `e8387a966` | `dbc5156ad` | Share custody columns to meet the native loader CPU target |
| `05b0d32d4` | `cf98d67aa` | Correct native custody loader filtering and scalar refusals |
| `c60e0cb9f` | `a602e01c0` | Preserve native custody loader refusal boundaries |
| `39de991a3` | `d61482289` | Preserve custody loader values through projection checks |
| `efcbb7af7` | `d4e015b51` | Preserve downstream custody scalar refusals |
| `c5966c1ff` | `75ba3a23d` | Match mixed revision refusals across Python hash seeds |
| `e15900677` | `06653dee8` | Complete R4-9 native custody loader acceptance |
| `d03c0b9a1` | `91ee045d1` | Preserve BLOB parity in the native custody loader |
| `aaf6216e3` | `708039d14` | Make revision-reference refusals deterministic |
| `c47450757` | `cc365aa6d` | Complete automatic pairing stage parity and CPU acceptance |
| `bc9e126d3` | `43aa93def` | Port cached journal pricing to Rust |
| `4e2ae2f0b` | `2cf892c7d` | Port journal source-overlap pre-build repairs |
| `9ce29bfde` | `1a3d5844d` | Port drifted legacy pair refresh to Rust |
| `9fa4899af` | `ab6f90438` | Compose the ordered journal pre-build boundary |
| `9e8979d30` | `a08eb0b99` | Add whole-stage journal pre-build scenario replay |
| `197f6cdec` | `fb23fa249` | Harden native journal rendering boundaries |
| `bd3690fa3` | `f49b8394d` | Reuse exact fragment sources when rendering journals |
| `4656ef30e` | `c80d1bded` | Reduce allocations in native journal rendering |
| `32767d331` | `c418381f7` | Reduce persistence overhead and pin journal crash recovery |
| `51bd20498` | `3c6459382` | Reuse journal bindings and correct append benchmarks |
| `925800ebe` | `1ed97c397` | Compose journal processing behind the native test harness |
| `3856b335b` | `727c297c8` | Exercise cached pricing and abrupt journal recovery |
| `a7da5b474` | `e51b51012` | Reduce custody row copies and commitment conversions |
| `19792ea1a` | `eeacf4e89` | Record custody mutation checkpoint and remaining work |
| `60b1a4506` | `87e9abea7` | Pass typed ledger responses and borrow render operands |
| `0b8e25564` | `6ecd01be9` | Register native CLI Bitcoin wallet sync |
| `86e71ef68` | `5cc368559` | Port the local freshness queue for daemon sync |
| `ab2581a73` | `88f9134e5` | Admit native freshness source plans |
| `16765561d` | `a94ef50c7` | Execute admitted native freshness jobs |
| `2a84706c5` | `249a3b7d3` | Stream freshness progress and fence fetch batches |
| `288ed149a` | `a6a5a6be1` | Wire guarded freshness request lifecycle controls |
| `821b481a3` | `9518c4efe` | Publish native freshness graph grants after response flush |
| `bbb5c0166` | `76922d092` | Add a bounded native graph reference worker |
| `d54f464d7` | `fe3c6ae9f` | Arm automatic graph consumption for native sync requests |
| `d2a30c40e` | `35ce95b76` | Validate native no-refresh daemon invocations |
| `a8511d099` | `01512093e` | Serve verified no-refresh wallet sync daemon routes |
| `f89a095f7` | `944568458` | Port Ledger Live movement imports to Rust |
| `04c846575` | `49be03331` | Port CoinTracking file imports in the Rust core and CLI |
| `c666ab691` | `26231a595` | Port Blockpit core and CLI file imports |
| `424d68180` | `486d69bd4` | Port Binance supplemental file imports to Rust |
| `2a735ed22` | `3826b369a` | Port BTCPay file imports to Rust |
| `71b10d358` | `014899283` | Port generic ledger file imports to Rust |
| `5a0c768cc` | `41e2c4e15` | Port Wasabi file imports to the Rust core and CLI |
| `bcde492ff` | `98f16e17f` | Port full-report XLSX exports and verification natively |
| `e5b555440` | `f4ace9d4f` | Port exit-tax reads and XLSX exports to Rust |
| `be4aac52d` | `cabf1602c` | Port retained capital-gain explanations to Rust |
| `9338364db` | `3748e1051` | Port diagnostic cross-check reports to Rust |
| `0475d3e49` | `b78efd3bb` | Port year-handoff XLSX reports to Rust |
| `013bb9ab2` | `fdab7558c` | Port platform CSV exports to Rust |
| `4014edb6f` | `7b9af1e87` | Port configuration privacy hygiene reads to Rust |
| `35754114f` | `c5caa0621` | Prepare native Lightning profitability reports |
| `ca645e224` | `a9d21c925` | Bind network consent to its owning action |
| `69955f7a0` | `22782ca6b` | Prepare native LND profitability acquisition |
| `8b63484d6` | `e347a3049` | Gate native CLN profitability acquisition |
| `d1178136a` | `d9be6a6b5` | Preserve LND snapshot refusals in report acquisition |
| `a99fa77c8` | `7584b5d5c` | Port stored source-of-funds listings to Rust |
| `b9b050083` | `ab2e4c034` | Preserve native diagnostics for source-funds read failures |
| `62d4cd51a` | `7b0b5c2bf` | Port frozen source-of-funds case persistence to Rust |
| `4af9249bf` | `8cbe9d993` | Join native review build checkpoints |
| `c7e16d1d7` | `e10052002` | Implement native review-case reader checkpoint |
| `083d35db4` | `43304ae73` | Wire review cases to the live native assembler |
| `f2092193a` | `cba027396` | Normalize review operations and content digests |
| `61e6b66e7` | `fc41a9f43` | Port review effects and portable artifact admission |
