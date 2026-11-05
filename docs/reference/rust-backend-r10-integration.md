# Rust backend R10 integration audit

**Status: ordered replay complete, including the four queued lines;
integration repaired and verified except the items under
[remaining work](#remaining-work).**
`t3code/rust-backend-r10b` starts at R9 (`b10e90e54`) and continues the paused
`t3code/rust-backend-r10` worker from its tip `1baf45765`. All 22 requested
source lines, then the queued `chain-reads-2`, `chain-ops-2`,
`desktop-projections-2` and `desktop-settings-1` lines, are integrated as one
linear stack. No push, PR, remote comment, stash pop/drop, or merge commit
occurred. Results below are fresh R10
measurements unless a row says it is inherited.

## Requirement and method

Preserve both branch behaviours, the original source messages and the source
order, following the [R8](rust-backend-r8-integration.md) and
[R9](rust-backend-r9-integration.md) method: inventory each range above R9,
skip work already present by `git cherry`, subject match or patch equivalence,
pick duplicated records once, and resolve conflicts by keeping both sides.
Documentation and status conflicts keep the newest evidence. Earlier
checkpoint evidence is inherited from its source branch and is not fresh R10
acceptance until this audit says it was rerun.

## Provenance and replay counts

The replay records **306 unique source commits: 167 picked and 139 skipped**.
Repeated ancestry shared by later lines is counted once. Across the 22 lines,
**86 picks conflicted** (326 path occurrences in 102 distinct paths); across
the four queued lines, **15 of 23 picks conflicted** (56 path occurrences in
19 distinct paths). Thirty-eight integration commits sit between or after the
picks: five by the paused worker (`7177e766c`, `c82562ef5`, `239cf5117`,
`e9b8c035f`, `1ecb55487`) and the 33 below. The stack has **205 commits above
R9**, before this audit's own commit.

| Line / inherited provenance | Picked | Skipped |
| --- | ---: | ---: |
| `t3code/r4-9-loader-determinism@aaf6216e3` | 0 | 12 |
| `t3code/r4-10-prebuild-5@a0393276a` | 9 | 5 |
| `t3code/r4-11-render-fidelity-5@b588bce80` | 13 | 3 |
| `t3code/r4-12-persist-accept@9ddee98cf` | 19 | 4 |
| `t3code/r4-12-persist-9@01620ce79` | 1 | 0 |
| `t3code/r4-13-journals-process-6@fff1e85e0` | 9 | 41 |
| `t3code/r4-15-journal-refresh@dfeaf1533` | 8 | 11 |
| `t3code/custody-perf-5@238422f90` | 1 | 2 |
| `t3code/perf-conversions-3@28d191796` | 0 | 3 |
| `t3code/r5-transfers-17@00f298a1e` | 4 | 14 |
| `t3code/r5-importers-b2@b191bc3da` | 1 | 12 |
| `t3code/r5-importers-b3@1c26e1a55` | 1 | 0 |
| `t3code/r5-importers-b4@a0884cdc0` | 2 | 0 |
| `t3code/r5-importers-b5@a3444a901` | 3 | 0 |
| `t3code/r5-sof-mutations-5@ada70d166` | 15 | 11 |
| `t3code/r5-reports-lightning-5@59b87ce56` | 3 | 5 |
| `t3code/r5-privacy-psbt-4@1bf6c7cbf` | 8 | 2 |
| `t3code/r5-privacy-legacy-scorer@fb54a31b6` | 2 | 0 |
| `t3code/r5-privacy-mirror-5b@b258fb1c9` | 16 | 1 |
| `t3code/r5-privacy-mirror-5a@e4c26e5f7` | 1 | 2 |
| `t3code/r5-exit-tax-mirror@2254f3a42` | 22 | 0 |
| `t3code/r5-mirror-6e-7b@f73668f25` | 6 | 7 |
| `t3code/r5-chain-analysis-reads-2@4690dcee2` | 3 | 2 |
| `t3code/r5-chain-analysis-ops-2@6688c95d9` | 7 | 1 |
| `t3code/r5-desktop-projections-2@3d36e8909` | 8 | 0 |
| `t3code/r5-desktop-settings-1@4ebb68e8e` | 5 | 1 |

The loader line was already fully present in R9. Its R4-9 acceptance is
inherited from `t3code/r4-9-loader-determinism@aaf6216e3`.

### Skips that are not subject matches

- `1f064db49`: its four baseline records were already retained through R8.
- `59c5f062f`: a restack-only loader adaptation that R9 superseded.
- `cb0cd4d1f`: R9 already carries the capability order and refusal fixes.
- `92a2545a5` and `28d191796` (perf-conversions-3): the paused worker's
  trial failed the strict "engine no slower than R9" gate on generic-2000:
  `engine_compute` best/median CPU 0.0416/0.0487 s against 0.0410/0.0483 s at
  R9. Under a one-minute load of about 38 this is within noise, and the
  same pairs show the fusion stage faster (0.0613/0.0675 against
  0.0902/0.1052 s) and whole-process CPU lower (0.739/0.906 against
  0.812/0.923 s). The skip follows the brief's strict gate; owner review may
  revisit it on a quiet machine. The trial is retained in
  [rust-backend-r10-conversion-trial.json](rust-backend-r10-conversion-trial.json).
- `a9ad2d491`: a merge commit. Both parents are replayed linearly, and the
  PSBT line had already squashed its resolved analysis code into `e3597426c`.
- `014ca5694` (mirror-6e-7b): a second native implementation of Python's
  round 7/7b cooperative sync cancellation. R4-15 already carries the
  accepted implementation (`17c2db85a`, `f30175465`, `5cca8bd9a`,
  `e5b34797f`, `2216235b0`). Its alternative `HttpClient::with_cancel_check`
  API conflicts with that implementation, so its implementation-specific
  tests are not carried. Python rounds 7/7b themselves are present.

The replay helper stages untracked files under `docs/`, so the first queued
pick (`489f80a4d`, from `00636290f`) also committed this audit's draft and its
two JSON records. No later commit touches them, and this audit's own commit
supersedes the draft.

The Mirror commits `f94d5b16d` to `4d2e0c289` overlapped with the PSBT
squash `e3597426c`, which already contained their code. They were picked for
their messages, checkpoints and backlog entries; their code paths resolved to
the existing integrated files.

## Conflict resolutions and integration fixes

Both sides were kept in every conflict. The shapes that recurred:

- Daemon report kinds are kept in Python's `SUPPORTED_KINDS` order, which the
  ready list must follow.
- Mutation catalogs are unioned by recipe identity. Exact duplicates are
  dropped; a differing duplicate keeps both under distinct identities
  (`sof-debug-normalization-scope-wire`).
- `AppError` carries both `message_codepoints` (journal line) and
  `sqlite_origin` (source-funds coverage).
- The source-funds honest-native diagnostic is implemented once:
  `request_error_envelope` delegates to the existing kind-aware
  `app_error_envelope`.
- `compose.rs` keeps the parallel fragment builder and adds the consumed-regime
  grouping key from the exit-tax mirror.
- The registered PSBT, legacy-hygiene, Lightning and document dispatch arms
  are unioned in front of the existing report and wallet dispatch.
- Generated contract fixtures (`tests/fixtures/rust_port/*.json`) and Python
  oracle fixtures are regenerated from the integrated Python, not merged by
  hand.

Integration commits:

| Commit | Change |
| --- | --- |
| `d8106071b` | Repairs the transfer-review build: relocated fixture paths, a crate-private helper and the lockfile edge. |
| `4c1de5fe1` | Keeps the refresh line's Python sqlite3 refusal families and the process line's BLOB identity, and hashes byte wallet labels in holdings. |
| `3d9f49efc` | Retargets 41 mutation anchors that later lines reshaped. Two strict-admission recipes are dropped because the raw review path replaced them, and duplicates are deduped. All 82 catalogs (8,042 recipes) apply. |
| `f7150dbd2` | Reviews the merged test-only launches and harness features for the network chokepoint scan. |
| `ec8856fc8` | Updates the daemon ready-list witness to Python's order. |
| `c0ab1be81` | Native journal stores stamp `journal_semantics_version` after the input markers, as Python round 6 does. Refresh skips only for current semantics. The journal oracles are refrozen. |
| `4f30e6405` | Applies the semantics check in readers ported on other lines (review cases, privacy hygiene, source-funds custody inputs and the Mirror sources) and refreezes their fixtures. |
| `2dd04f7ab` | Counts CLN launches from the ledger, so the witness no longer depends on an ambient egress guard. |
| `8cd9b7b57` | Refreezes the setup fixtures with the semantics columns. |
| `5ce0c1e75` | Mirrors Python's Mirror projection version 3 and refreshes the frozen privacy seed to the integrated schema. Every fixture built on the seed is refrozen. |
| `cb1fcebc7` | Makes the descriptor-close witness immune to descriptor-number reuse by parallel tests. |
| `dd2093d1c` | Refreezes the checked-in custody stage vectors; the only change is the two semantics columns in their profile rows. |
| `173715593` | Resolves `target/` in scratch-location guards so a symlinked target is accepted, and refreezes the renderer branch witnesses. |
| `4844ba8b8` | Places the legacy hygiene snapshot and Lightning profitability kinds where Python lists them; the ready list now equals Python's order. |
| `ab44b59bc` | Stamps semantics in the current automatic-pairing command seed. |
| `2d58edb28` | Refreezes the journal render vectors in dependency order (`render.json` before `render-native-ledger.json`). |
| `2eee3740c` | Owner fix `47a58f4c4`, cherry-picked: the default-home regression test no longer inherits `XDG_DATA_HOME`. |
| `4c33b5deb` | Restores the shell-parity probe line a paused-worker merge dropped; probes `ai providers list`, still unported. |
| `805d3ec4b` | Makes the reviewed sync seed and the paired desktop seed canonically current (count and semantics) and refreezes the publication vectors. |
| `9db04258d` | Empty-scope `ui.freshness.run` leads with Python's keys; the product-default acquisition test compares each route's raw answer with Python. |
| `cddb765c3` | The CLI run tests compile without `kassiber-cli/test-probe`, as the gate's workspace commands build them. |
| `6b475d61d` | Restamps semantics when the CSV refusal test restores current journals. |
| `3defac865` | Custody admits Python-sized (up to 4,300-digit) reviewed amounts with exact `BigInt` fallbacks instead of panicking; the intra-transfer `TypeError` carries Python's text. |
| `d855136db` | Seeds the refresh failure books like the exact book recovery starts from, so Mirror's change clock in the expected partial is no longer one revision ahead. |
| `2fa022810` | Refreshes the port classification after the parity fixes. |
| `fc21d0340` | Clippy across the merged lines: a collapsible `if let` and the entropy hook literal under Mirror's gated fields. |
| `75f11fed6` | Advertises the merged chain-analysis and desktop kinds in Python's order. |
| `1e4deecaf` | Keeps the chain-analysis operations clear of the network chokepoint scan (`import_labels`/`import_dataset`, no bare `net`). |
| `d71ede5a5` | Refreezes the nine chain-analysis vector files against the integrated reference. |
| `f93e3bd6c` | Retargets 16 mutation anchors moved by the merges; all 16 are killed. |
| `262372d98` | The desktop parity harness recognises the Rust daemon by its ready list and closes its seed connections. |
| `a31f092d1` | Binds the analysis-operations witnesses to Mirror's projection version and ignores test-only kinds. |
| `809b168c6` | Rerecords Python's chain-analysis hash-seed variants on the integrated schema. |

### Journal calculation semantics

The exit-tax line brought Python rounds 6 to 6e. Python now stamps
`journal_semantics_version` after `store_ledger_state` publishes the input
markers. A trigger clears the stamp whenever those markers move.
`evaluate_projection_freshness` also requires the current semantics.

The R4 journal pipeline and several R5 readers were ported on lines without
that round. The integration mirrors the stamp in native persistence and the
freshness check in each reader. Fixture generators that mark a book current
directly in SQL now stamp the semantics after the markers. Without that,
Python's trigger would make every "current" scenario stale, and those tests
would silently lose their current-state branch. The frozen privacy seed is
refreshed to the integrated schema for the same reason. Every fixture built on
it is regenerated, including Mirror's `custody_fresh` paths, which keep their
228 and 6 current cases.

### Queued chain-analysis and desktop lines

The read and operation lines both branched from R7 and each carried a copy of
the shared analysis runtime. The integration keeps one
`analysis_index::runtime` (the operations line's copy was byte-identical),
keeps `stream_claims` beside `preview` in `reads/preview.rs`, and deletes the
operations line's `dataset_stream` shim, which loaded that file a second time.
CLI dispatch checks the exact operation commands before the read prefix, so
mutations never route to the read service. The daemon's `lock` path reaps
freshness workers, clears analysis grants and receipts, and stops the watch
worker.

`daemon.ready` lists every native `ui.chain_analysis.*` kind first, sorted,
and `internal.chain_analysis.stage` after `ui.wallets.create`, matching
Python's `SUPPORTED_KINDS` order for all 219 native kinds. The report and
transfer witnesses now include the desktop line's capital-gains and
review-context kinds where Python lists them.

The R7-era vectors of both chain-analysis lines were refrozen from integrated
Python: the semantics columns and Mirror's projection version 3 change the
schema hash and snapshot identities, not any finding. The integrated network
scanner also rejects `::import(` calls and the bare word `net`, so the label
and dataset imports take Python's names `import_labels` and `import_dataset`.
The desktop line moved the time-zone port into `kassiber-pytext` and reworked
its fold handling; its eleven zone recipes were retargeted, and the report
mutation checker now also runs `kassiber-pytext`.

## Fresh R10 verification

All commands use Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest --`, `MATURIN_NO_INSTALL_RUST=1`, offline Cargo,
`CARGO_BUILD_JOBS=8` and disposable data roots. `HOME` is unchanged. Scratch
and build trees live under this worktree's `target/`, a symlink to the shared
build disk. Product storage refuses symlinked paths, so `TMPDIR` and the Cargo
directories use the resolved path. One test job runs at a time. Mutation
builds use their own target and build directories, removed afterwards.

Most runs shared the machine with other workers at one-minute loads of 100 to
400, and the build disk briefly filled. Every failure listed as load-bound
below was a timeout, `SQLITE_BUSY`, `SQLITE_FULL` or disk I/O error that passed
when rerun alone or at lower load. Python's `secrets init` migration is the
usual victim: `sqlcipher_export` spends about 7 s waiting on synchronous writes
even for an empty book, and contention stretches that past the tests' 90 to
120 s limits.

| Check | Fresh R10 result |
| --- | --- |
| Formatting | `cargo fmt --all --check` passes at the tip |
| Clippy `-D warnings` | Passes at the tip in all three configurations: the integration feature set (journal harness/timing, binary, chain/wallet harness, network observer), the gate's workspace command (`--features kassiber-journals/test-harness`) and the gate's binary/daemon/runtime command (`--features kassiber/test-harness`) |
| Workspace tests | Post-merge run `workspace-merge1`: **2,177 passed, 24 failed** across 171 harness results. All 24 were fixed and rerun: the network chokepoint scan (9/9 after `1e4deecaf`) and 23 `kassiber-wallets` cases (stale R7-era vectors, refrozen in `d71ede5a5`, and one `SQLITE_FULL`). The rerun passes 363 of 364; the remaining `analysis_dataset_jobs_match_python_receipts_and_tables` hit `database_busy` at load ~140 and passes 3/3 alone. `kassiber-daemon` passes 46/46 after the kind-order change, `kassiber-journals` 212/212 and `kassiber-custody`, `kassiber-journals` `process_native` and `kassiber-cli` 686/686 after the overflow fix, `kassiber-cli` also without `test-probe` |
| Mutation catalogs | All **87 catalogs, 8,201 recipes**, apply. The 16 recipes retargeted after the merges are all killed by their own runners (custody 1, chain 1, Lightning 1, transfers 2, reports 11). Full kills of the other recipes are not rerun here |
| Custody generated lanes | Rerun at `3defac865`, after the custody change: **24,000 books + 1,030 permutations**, 24,852 complete custody stage matches and 178 matching upstream refusals; renderer 24,705 / 24,705. The merges do not touch custody, journals or the tax core |
| Archived custody recordings | Rerun at `3defac865`: 159 recorded requests + 159 permutations, 0 differences; archived renderer 318 / 318 |
| Focused census and shadow | Rerun at `3defac865`: each passes **300 tests and 32 subtests**, one platform skip |
| Ledger feature check | Passes at `3defac865` |
| Registered `journals process` | See the table below (measured before the merges; the merges do not touch the journal pipeline) |
| Parity suite, before the merges | All **110 modules** (the 75 slice modules and the 35 modules R9 left unrun, every `tests/test_rust_*.py` and `tests/rust_port/test_*.py` module) ran to receipts. 100 passed outright. The other 10 were fixed or traced to load and rerun green: journal process and refresh (overflow and seed fixes), shell parity, transaction desktop, wallet acquisition and publication, reports, the inventory classifier, and backup desktop/install (load-bound, 26 nodes pass serially) |
| Parity suite, the 24 merged-line modules | **24 / 24 pass** after the harness and witness fixes |
| Parity suite, the 110 modules after the merges | In progress: **44 of 110** rerun on the merged stack. 41 pass, including journal process (51/51) and refresh (21/21); three have only load-bound failures (freshness native 2, freshness product 14 ready-line timeouts, Samourai 1 `secrets init` timeout) |
| Classifier | `classify_tests update` rerun at the tip; the inventory and contract checks pass after the earlier refresh |

### Registered `journals process`, Python against native

Five alternating pairs per book run the registered CLI from process start to
exit on fresh copies of each pre-migrated committed-seed book: Python
`python -m kassiber` against the release `kassiber` binary
(`a6adbd66…`). CPU is child user plus system time. In every pair the
payloads match without the processing clock, and the journal tables match by
count and by content with identities and clocks removed. The record is in
[rust-backend-r10-process-bench.json](rust-backend-r10-process-bench.json).

| Book | Median CPU, Python / native (s) | Ratio | Median wall, Python / native (s) | All pairs equal | One-minute load |
| --- | ---: | ---: | ---: | --- | ---: |
| Generic 2,000 | 1.820 / 0.770 | 2.36× | 3.26 / 0.96 | yes | 10.8–12.9 |
| Generic 20,000 | 12.330 / 6.960 | 1.77× | 13.32 / 7.20 | yes | 6.5–10.8 |
| Multi-wallet 2,000 | 2.290 / 1.140 | 2.01× | 2.52 / 1.36 | yes | 5.2–6.0 |
| Multi-wallet 20,000 | 19.890 / 10.060 | 1.98× | 21.65 / 11.43 | yes | 4.8–8.1 |

These are registered whole-process numbers. They sit in line with R9's
prepared-harness ratios (1.65–2.26×) and remain below the ≥4× end-to-end
target.

## Remaining work

- **Post-merge parity:** 66 of the 110 prior modules, shell parity among them,
  still need a post-merge receipt (`parity.sh` is resumable), and the three
  load-bound modules need a quieter rerun. The final preflight runs after them.
- **Full mutation kills:** only catalog applicability (all 8,201 recipes) and
  the 16 retargeted recipes were run. The full kill sweep, in resumable shards,
  is not rerun on the integrated stack.
- **`perf-conversions-3` skip:** owner review on a quiet machine, as described
  under the skips above.
- **Load-sensitive witnesses:** `analysis_dataset_jobs_match_python_receipts_and_tables`
  opens its worker connection with SQLite's default 5 s busy timeout and can
  report `database_busy` under heavy load. Several product suites wrap
  Python's `secrets init` in 90–120 s limits that contention can exceed. Both
  pass whenever the machine is quieter. No test was blocked by the sandbox;
  the loopback suites ran here.
- **Python reference observations (not fixed here):** the plaintext-to-encrypted
  migration's `sqlcipher_export` takes about 7 s of synchronous write waits on
  an empty book (SATA SSD). Python's dataset manifest field order depends on
  `PYTHONHASHSEED` (recorded in `chain_analysis_reads_hashseeds.json`). A
  truncated Electrum transaction aborts a whole acquisition with an uncaught
  `RuntimeError` (`tests/rust_port/repro_acquisition_electrum_truncated.py`).
- **Product gaps carried by the merged lines:** `ui.maintenance.run`, report
  blockers, report-read wallet sync, `ui.transactions.graph` and
  `ui.transactions.review_context`, `ui.audit.evidence.summary` and
  `ui.review.request_input` stay unported; see [TODO](../../TODO.md).
- **Owner runs:** the full census, shadow and native-lane shards beyond the
  focused selections above.
