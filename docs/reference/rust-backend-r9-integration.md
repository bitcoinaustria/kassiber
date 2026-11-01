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

The inventory contains **183 candidate occurrences**, **82 distinct subjects**,
and **64 provisionally new subjects**. **62 source commits were picked and
121 occurrences skipped**, plus one consolidated stage-adaptation commit
(`e4a0d9fd3`). This gives **63 replay commits above `9f7471bf7`**, before the
subsequent integration-fix and verification commits. Range inventories exclude
already-satisfied ancestry before the requested continuation boundaries.

## Source provenance and replay counts

Each row's acceptance, campaign, mutation and timing evidence is
**inherited from the listed branch@sha; pending fresh r9 verification**.
The fresh R9 verification section below records the checks rerun here.
Source checkpoint documents retain historical results and their original scope.

| Line / inherited provenance | Picked | Skipped |
| --- | ---: | ---: |
| `t3code/r4-9-loader-9@e15900677` | 7 | 3 |
| `t3code/r4-9-loader-blob@d03c0b9a1` | 1 | 9 |
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
  retained once in `e4a0d9fd3` (Integrate journal process stage adaptations).

## Conflict resolutions and compatibility fixes

The automated replay receipts record **47 conflicted candidates, 163 path
occurrences and 39 distinct paths**, including the skipped `59c5f062f`.
The seven initial loader picks add **26 path occurrences**. These are separately
recorded counts; the manual stage adaptation is not counted as an additional
cherry-pick conflict. Rerere resolutions were inspected rather than trusted.
No irreconcilable code-behavior conflict was found.

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

The loader-BLOB continuation supersedes the old BLOB exception: BLOB parity is
closed in that branch's evidence. **40 measured seed-dependent Python messages**
remain its storage-audit exception, pending the Python determinism fix. The old
16-message/non-BLOB-only status is historical. Renderer, persistence and process
remain partial, unregistered stages. Lightning acquisition/preparation does not
establish product routing or capability cutover. SOF mutations remain a frozen-case
persistence foundation. The SOF reads continuation supplies the honest native debug
contract; privacy's corrupt-marker diagnostic gap remains explicitly scoped.

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
| All-target workspace clippy `-D warnings` | Initial pass; daemon ordering changed afterwards, final rerun pending |
| Workspace tests | Initial run in progress; restricted loopback fixture failures require elevated rerun |
| Mutation catalog anchors | 6,541 / 6,541 apply exactly once across 42 catalogs; kills pending |
| Test classification | Regenerated 6,020 definitions; final contract check/preflight pending |
| Custody generated/replay/focused lanes | Pending fresh run |
| Slice/hybrid parity and no-fallback assertions | Pending fresh run |
| Full mutation kills in resumable shards | Pending fresh run |
| Combined journals process / custody CPU | Pending fresh run |

Initial workspace failures and successful reruns will remain distinct receipts;
restricted-environment failures are not counted as successful coverage.
The owner will run full census and shadow shards afterwards; focused lanes
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
| `c47450757` | `387d52d92` | Complete automatic pairing stage parity and CPU acceptance |
| `bc9e126d3` | `b661d0225` | Port cached journal pricing to Rust |
| `4e2ae2f0b` | `baae7e7e5` | Port journal source-overlap pre-build repairs |
| `9ce29bfde` | `00d92f9f8` | Port drifted legacy pair refresh to Rust |
| `9fa4899af` | `09cf6e9f6` | Compose the ordered journal pre-build boundary |
| `9e8979d30` | `78a6085f9` | Add whole-stage journal pre-build scenario replay |
| `197f6cdec` | `fd90818a7` | Harden native journal rendering boundaries |
| `bd3690fa3` | `7c1d23e3a` | Reuse exact fragment sources when rendering journals |
| `4656ef30e` | `af660e580` | Reduce allocations in native journal rendering |
| `32767d331` | `0b92eae8c` | Reduce persistence overhead and pin journal crash recovery |
| `51bd20498` | `80d078c73` | Reuse journal bindings and correct append benchmarks |
| `925800ebe` | `2576ca4ea` | Compose journal processing behind the native test harness |
| `3856b335b` | `89c696b73` | Exercise cached pricing and abrupt journal recovery |
| `a7da5b474` | `dfb504449` | Reduce custody row copies and commitment conversions |
| `19792ea1a` | `1c84ad955` | Record custody mutation checkpoint and remaining work |
| `60b1a4506` | `e6266e0ca` | Pass typed ledger responses and borrow render operands |
| `0b8e25564` | `f4eb4a6d0` | Register native CLI Bitcoin wallet sync |
| `86e71ef68` | `6600cad9f` | Port the local freshness queue for daemon sync |
| `ab2581a73` | `6d71ca101` | Admit native freshness source plans |
| `16765561d` | `65a250f83` | Execute admitted native freshness jobs |
| `2a84706c5` | `6968c1623` | Stream freshness progress and fence fetch batches |
| `288ed149a` | `8750a1f96` | Wire guarded freshness request lifecycle controls |
| `821b481a3` | `2f883274c` | Publish native freshness graph grants after response flush |
| `bbb5c0166` | `7a06a9b70` | Add a bounded native graph reference worker |
| `d54f464d7` | `23e15f176` | Arm automatic graph consumption for native sync requests |
| `d2a30c40e` | `ac11bfb9b` | Validate native no-refresh daemon invocations |
| `a8511d099` | `5931c33fc` | Serve verified no-refresh wallet sync daemon routes |
| `f89a095f7` | `6c6a72c6a` | Port Ledger Live movement imports to Rust |
| `04c846575` | `52f822ada` | Port CoinTracking file imports in the Rust core and CLI |
| `c666ab691` | `0e6cf9dee` | Port Blockpit core and CLI file imports |
| `424d68180` | `33cc29952` | Port Binance supplemental file imports to Rust |
| `2a735ed22` | `cb670625e` | Port BTCPay file imports to Rust |
| `71b10d358` | `40b82d333` | Port generic ledger file imports to Rust |
| `5a0c768cc` | `72187fba1` | Port Wasabi file imports to the Rust core and CLI |
| `bcde492ff` | `3e5ecee10` | Port full-report XLSX exports and verification natively |
| `e5b555440` | `f3e6297f2` | Port exit-tax reads and XLSX exports to Rust |
| `be4aac52d` | `f11878cd8` | Port retained capital-gain explanations to Rust |
| `9338364db` | `f9a0910ee` | Port diagnostic cross-check reports to Rust |
| `0475d3e49` | `db8d855f4` | Port year-handoff XLSX reports to Rust |
| `013bb9ab2` | `203750524` | Port platform CSV exports to Rust |
| `4014edb6f` | `cae129d33` | Port configuration privacy hygiene reads to Rust |
| `35754114f` | `400946454` | Prepare native Lightning profitability reports |
| `ca645e224` | `52d1bdf1d` | Bind network consent to its owning action |
| `69955f7a0` | `2950e6974` | Prepare native LND profitability acquisition |
| `8b63484d6` | `ceca9c33a` | Gate native CLN profitability acquisition |
| `d1178136a` | `57da9ca97` | Preserve LND snapshot refusals in report acquisition |
| `a99fa77c8` | `fe4e6fc8e` | Port stored source-of-funds listings to Rust |
| `b9b050083` | `19f55b770` | Preserve native diagnostics for source-funds read failures |
| `62d4cd51a` | `5313ac976` | Port frozen source-of-funds case persistence to Rust |
| `4af9249bf` | `a2ab1eaf1` | Join native review build checkpoints |
| `c7e16d1d7` | `41ec1762d` | Implement native review-case reader checkpoint |
| `083d35db4` | `d6d148830` | Wire review cases to the live native assembler |
| `f2092193a` | `6f3e46102` | Normalize review operations and content digests |
| `61e6b66e7` | `9d1edc5a4` | Port review effects and portable artifact admission |
