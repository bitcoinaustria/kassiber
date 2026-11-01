# Rust backend R8 integration audit

Local integration of the finished branches onto `dd433e4ee` (R7, round 4),
on `t3code/rust-backend-r8`. The replay ends at `b5eb38ea8`; the final
integration adaptation/audit commit follows that checkpoint. No push, PR,
remote comment, stash pop/drop, or worker delegation occurred.

## Provenance and replay

The stack contains **56 cherry-picks** from **66 ordered candidates**, with
**10 skips**. Every picked commit retains its original message and order.
History above the requested base is linear. `git cherry`, subject matching,
and direct content/record comparisons distinguish actual missing work from
re-picks whose patch IDs differ because their context changed.
The completed stack has 57 commits above the base: 56 picks and one final
integration adaptation/audit commit.

| Line | Picked | Skipped |
| --- | ---: | ---: |
| harness | 1 | 0 |
| prerequisites | 0 | 3 |
| ledger | 19 | 0 |
| typed | 1 | 1 |
| interp | 2 | 1 |
| quantity | 2 | 2 |
| projection | 3 | 1 |
| persist | 5 | 1 |
| loader | 2 | 1 |
| transfers | 5 | 0 |
| sync | 4 | 0 |
| imports | 7 | 0 |
| reports | 5 | 0 |

The three design/storage/render prerequisites already exist on R7, before
persistence: `ff7050c00` → `53625d387`, `6a94f6767` → `cf370bf5c`, and
`5ade4b811` → `5591fee0f`.

`74f605e65` already includes the recovered baseline section and all four exact
baseline rows. Consequently its separate record `146281cf8` and the identical
baseline commits from the other five lines add no content. The merged
`r4-bench-records.jsonl` retains ten distinct rows, including the typed and
stopped-probe measurements. The stopped records on the typed-gaps,
typed-quantity and typed-projection aliases were also compared by subject,
`git cherry` and exact row inclusion; no additional record is missing.
The redundant alias pairs are `5012ab6f5`/`2b73364ee` (gaps),
`7dad367a0`/`5e8fd3ddc` (quantity), and `3cb1b899b`/`d2197da3b`
(projection). Their differing patch context explains the `git cherry +`
markers; their measurements are already present.

The persistence line's already inherited render/storage re-picks
`1e331d6f`, `3c6c324c`, `5ad2ab0c`, and `4b4b3446` are represented by
`4624073ee`, `db8534b0b`, `4100deb31`, and `4f6ca7d06`, respectively.
They are not applied again as separate commits.

For the split-payout injected failure, retain the precise declaration already
shipped by `951954d03`: the expected validation message and exact exception at
`finalized_projection`. Skip the broader redundant `c017987c9` declaration.
Loader-2 strengthens the same exact code/message declaration with assertions;
keep those assertions and `d89d335b9`'s compute-function replacement detector. This lets
intentional test injection remain explicit without accepting an unrelated
production refusal.

The authorized loader-2 addendum `0b78605a0` follows `569d80add` immediately,
before transfers. Its [direct SQLite record checkpoint](r4-native-loader-checkpoint.md) is retained alongside
the typed `BookSnapshot` checkpoint. Compatibility records use the same typed
accounting admission on receipt-error paths; ordinary direct loads retain their
record construction. Transaction maps move into the compatibility value wrapper
without cloning rows, and receipt extraction borrows those same object maps.
Typed admission still serializes and parses the compatibility snapshot;
direct typed admission remains future work. The loader checkpoint documents
that integrated boundary, and measured load/admission includes it.
The XLSX dependency is the explicitly
owner-approved `rust_xlsxwriter`; all 349 registry package identities in the
merged lockfile occur, with their exact versions and checksums, in an input
lockfile. No novel registry identity was introduced during integration.

## Conflicts and adaptations

There were **28 conflicted source candidates**, **54 path occurrences**, and
**15 distinct paths** across 30 conflict events (two source candidates conflict
again during the loader-2 replay), including the two conflicted candidates ultimately
skipped. No conflict required an accounting or architecture decision.

| Original candidate | Conflicted paths |
| --- | --- |
| `5c35fb65f` | `TODO.md`, `docs/reference/r4-journal-pipeline.md` |
| `74f605e65` | `TODO.md`, `docs/reference/r4-bench-records.jsonl`, `docs/reference/r4-journal-pipeline.md` |
| `146281cf8` | `TODO.md`, `docs/reference/r4-bench-records.jsonl`, `docs/reference/r4-journal-pipeline.md` |
| `4c130249c` | `TODO.md`, `docs/reference/r4-journal-pipeline.md` |
| `951954d03` | `TODO.md`, `crates/custody/scripts/mutations.json`, `docs/reference/r4-journal-pipeline.md` |
| `d2d3a66eb` | `TODO.md`, `docs/reference/r4-journal-pipeline.md` |
| `c017987c9` | `tests/test_pyfix_review.py` |
| `a687c07ca` | `crates/custody/scripts/mutations.json` |
| `25b87bfc8` | `TODO.md`, `docs/reference/r4-bench-records.jsonl`, `docs/reference/r4-journal-pipeline.md` |
| `78df09179` | `crates/custody/scripts/mutations.json` |
| `f985462ea` | `docs/reference/r4-journal-pipeline.md` |
| `db8534b0b` | `TODO.md`, `tests/custody_engine_compare.py`, `tests/rust_port/dispositions.json` |
| `adeefd597` | `docs/reference/rust-port.md` |
| `569d80add` | `Cargo.lock`, `TODO.md`, `docs/reference/r4-journal-pipeline.md`, `docs/reference/rust-port.md`, `tests/rust_port/dispositions.json`, `tests/rust_port/reviews.py` |
| `d64433d79` | `docs/reference/rust-port.md` |
| `78bda76b0` | `docs/plan/21-rust-backend.md`, `docs/reference/rust-shell.md` |
| `0207dbcfc` | `docs/reference/rust-port.md` |
| `5e39ba109` | `docs/reference/rust-port.md` |
| `2065ef67c` | `crates/cli/src/wallets.rs` |
| `d5d53ee6d` | `docs/reference/rust-port.md` |
| `15a100788` | `docs/reference/rust-port.md` |
| `3f8e12b0f` | `crates/cli/src/wallets.rs`, `docs/reference/rust-port.md` |
| `92ec79939` | `docs/reference/rust-port.md` |
| `da74dfe62` | `docs/reference/rust-port.md` |
| `560084e84` | `docs/plan/21-rust-backend.md` |
| `af6593627` | `tests/rust_port/dispositions.json` |
| `e193e6b10` | `docs/reference/rust-port.md`, `docs/reference/rust-shell.md` |
| `0b78605a0` | `TODO.md`, `docs/reference/rust-port.md`, `tests/test_pyfix_review.py`, `tests/test_tax_shadow_harness.py` |
| `78bda76b0` (loader-2 replay) | `scripts/quality-gate.sh` |
| `5e39ba109` (loader-2 replay) | `scripts/quality-gate.sh` |

Resolutions retain both the R7 behavior fixes and incoming speed changes:

- Union mutation catalog tails; all recipe IDs remain unique within each catalog.
- Union the custody compute-patch detector with context-bound injected fusion
  policy, retaining the final `if not recorded` guard.
- Union the wallet sync dispatch arm with every new importer alias.
- Restore Python ordering in the daemon capability announcement after the
  independently extended transfer and XLSX kind lists merge. The parity suite
  found eight failures in handshake bytes; request response bodies already
  matched. Keep the original strict byte assertions.
  The final transfer mutation baseline also exposes an older unit-test prefix
  that assumed the seven authoring kinds came first. Extend its exact prefix to
  include Python's leading `ui.transfers.suggest`; retain all argument/default
  assertions and the strict full-handshake gate. The initial failed baseline
  remains recorded separately; the complete transfer catalog is rerun.
- Merge prerequisite/checkpoint prose and measurements once; preserve R7's
  authority and refusal boundaries. Regenerate generated counts and inventories.
- Union the loader/persistence journal dependencies, then remove the duplicate
  manifest keys produced by the automatic merge and canonicalize lockfile order.
- Scope the compute-function baseline in the two harness controls that
  deliberately replace the Python oracle. The first focused census found
  14 failures when idempotent installation reused the enclosing census
  baseline. `monkeypatch` now restores that baseline after each control;
  production patch detection and exact stage/exception checks are unchanged.
- Remove obsolete XLSX refusal assertions from the older transaction-listing
  unported-surface test. Transaction XLSX is now supplied by the explicitly
  requested reports line and has its dedicated parity and no-fallback suites.
  Retain both unsupported platform CSV formats and the other unported daemon
  surfaces as exact `not_implemented` witnesses.
- Retain the loader's exact raw-v1 snapshot byte assertion. Its new typed-row
  adapter serializes the same f64 as `1.234e-7` where the compatibility row
  preserves `1.234e-07`. Compare the in-memory structures through the existing
  Python scalar codec, preserving integer/float distinctions and signed zero.
  This keeps the raw byte gate exact while comparing the typed scalar model.
- Preserve the captured SQLite schema in the persistence replay, including the
  compact column's appended position on upgraded books. Capture explicit entry
  edits at the storage call, with the original render ID tape, and replay only
  fields the test actually changed. Reused state objects remain captureable for
  subsequent stores. The initial ten dump differences were four schema-order
  mismatches and six post-build timestamp edits; no writer regression remained.
  Complete dump equality and the independent render byte gate stay unchanged.

## R7 serial-daemon skip investigation

The original logs report R6 **270 passed / 83 skipped** and R7
**269 passed / 211 skipped / 2 failed**, both with 147 subtests.
The entire **+128 skips** is new native parity coverage guarded by an unset
`KASSIBER_RUST_BIN`. It is not an additional sandbox restriction, missing
Python fixture, changed skip predicate, or a previously passing case made
optional. Exact archived R7 modules reproduce the following 121 skips:

| New serial-daemon module | New skips |
| --- | ---: |
| `test_rust_bdk_execution` | 30 |
| `test_rust_bitcoin_records` | 3 |
| `test_rust_observer_state` | 38 |
| `test_rust_report_exports` | 13 |
| `test_rust_wallet_acquisition` | 23 |
| `test_rust_wallet_discovery` | 14 |
| **New modules** | **121** |

The remaining seven are additions inside already guarded modules:

- `HybridCandidateTest.test_json_import_has_no_fallback_and_matches_every_reference_dump`
  and `test_river_import_has_no_fallback_and_matches_every_reference_dump` (2).
- `RustRealStoreParityTest.test_json_import_wire_bytes_full_state_and_rollback`
  and `test_river_import_wire_bytes_full_state_and_rollback` (2).
- `test_rust_reports.test_snapshot_creation_rows_and_frozen_dependency_copy`
  for generic/FIFO, generic/HIFO and Austrian/moving-average (3).

Thus **83 + 121 + 7 = 211**. One additional unguarded drift check,
`RustShellDriftTest.test_file_import_vectors_match_python`, accounts for the
one new passing parent case. The two R7 failures are the existing BTCPay
smokes `test_ui_btcpay_connection_discover_accepts_inline_instance_credentials`
and `test_ui_btcpay_connection_test_uses_raw_token_and_single_probe`; neither
is part of the skip increase. Those R7 failures saw an unexpected
unauthenticated `GET /` on the local fake. Both tests pass in isolation on
R8 (2 passed, 8.77 s); the original full-shard interaction is not reproduced
by that focused run. No Python product change was made for it.

The standalone shard runner does not build/export a native binary. The regular
CI serial-daemon job and `quality-gate.sh` do build the fixtures and export
`KASSIBER_RUST_BIN`. The specialized tax/custody shadow jobs do not set it;
native shell/service parity belongs to the regular native-enabled lane.
R8's integration verification supplies the binary and probes explicitly.
Exact archived node IDs and skip reasons are retained in `skip-r7.xml`.

**All 128 added R7 skips pass with the native binary enabled on R8.**
The exact archived-to-current node mapping is in `skip-native-coverage.json`.

The restricted R8 continuation additionally hit the account owner namespace:
older daemon fixtures call `_owner_lock_root()` at `/home/tg/.kassiber/run`,
which is intentionally independent of `HOME` and read-only in this sandbox.
Python emits an `OSError` traceback for each request while Rust returns
`project_owner_lock_unavailable`. These are test-environment failures, not
acceptable parity results. The interactive component-review test then blocks:
its parent reads stdout while the reference daemon fills stderr (8,007 bytes
buffered, child wait state `anon_pipe_write`). The captured error and wait-state
receipt are retained. Only that identified disposable daemon was terminated;
the restricted runner was then interrupted to retain its completed results.
An elevated continuation reruns every failed or unfinished case. Zero-time
JUnit placeholders from interruption are excluded from the completed set.

Reproduce the restricted reference failure with a test-harness binary and a
scratch root under `target/` (the timeout bounds the pipe deadlock):

```sh
TMPDIR="$PWD/target/r8-integration/tmp" KASSIBER_RUST_BIN="$PWD/target/r8-build/debug/kassiber" \
  mise exec rust@1.99.1 uv@latest -- timeout --kill-after=5s 60s \
  uv run --locked python -m pytest -q \
  tests/test_rust_transfer_services.py::RustTransferServicesTest::test_native_daemon_component_review_matches_python
```

This reproduces an existing Python-reference error path under restricted
ownership access; no Python product fix is made in this integration.

## Verification

Verification uses Rust 1.99.1 and locked Python 3.11 through
`mise exec rust@1.99.1 uv@latest --`, `MATURIN_NO_INSTALL_RUST=1`, offline Cargo
and disposable books. Tooling `HOME` is unchanged. Scratch, logs and summaries
are under `target/r8-integration`, build output under `target/r8-build`, and
short native keyring fixture paths under `target/k8`. One test job runs at a
time. This audit reports only measurements rerun on R8; inherited branch
receipts are not new integration verification.

| Native gate | Result |
| --- | --- |
| Locked Python 3.11 setup | Passed, offline |
| Workspace fmt / all-target clippy `-D warnings` | Passed |
| Workspace unit, integration and doc tests | 1,630 passed across 124 harness results; zero failed or ignored |
| Harness-feature clippy / tests | Passed; 14 cases |
| Network observer feature | 63 passed |
| Journal `test-harness,timing` all-target clippy `-D warnings` | Passed |
| Workspace examples and harness binary | Built offline |

The earlier workspace pass exposed the loader's in-memory exponent-spelling
comparison described above. The focused correction and full final workspace
rerun pass; the initial failure log is retained. The table reports the completed
run after all integration adaptations, including loader, persistence probe/support
and daemon ordering changes.

Initial classification regenerates **5,775 definitions**: 4,227 rust-test,
586 oracle, 543 subprocess-scenario, 370 boundary-fixture and 49 Python-only.
The contract check passes without regeneration. Final classification retains
those 5,775 definitions, and the required preflight passes **57 / 57** immediately
before the integration commit. Its initial run passed 56 cases and rejected the
incoming quantity checkpoint's locked-sync flag order. That command now uses
the repository's canonical `uv sync --locked` spelling; the strict drift check
is unchanged. The initial failure receipt and successful rerun are both preserved.

| Custody gate | Result |
| --- | --- |
| Stage/port/generated/fusion/snapshot and custody/tax harness modules | 234 passed; 16 subtests passed; affected controls rerun in focused lanes |
| Focused census, including `test_pyfix_review.py` and harness controls | 403 passed; 10 subtests passed; 81 custody builds, 80 exact loader matches |
| Focused custody/tax shadow, same modules | 403 passed; 10 subtests passed; 81 custody builds, 80 exact loader matches |
| Generated and archived custody campaign | 24,000 books + 1,030 permutations; 159 + 159 archived replays; zero differences |
| Ledger feature parity | 145 outputs byte-identical with and without custody serde_json features |
| Ledger/reference, preparation, contract and legacy suites | 964 passed; 52 subtests passed; one intentional panic-hook skip |

`tests/test_tax_engine_backend.py::BindingRoundTripTest::test_rust_panic_surfaces_as_runtime_error`
remains the existing unconditional skip: the public binding exposes no function
that deliberately panics. RP2 is installed and the oracle runs with
`KASSIBER_CI=1`, so missing reference dependencies cannot become silent skips.

The generated campaign reaches **16,757 ledger requests**, **7,948 empty
results**, **147 fusion-builder refusals** and **178 upstream refusals**.
All stage outputs and refusal boundaries match; refusals are not counted as
successful requests. The archived replay adds 312 requests and six empty
results across originals/permutations.

| Generator family | Books | Permutations | Distinct seed inputs | Distinct active seed inputs |
| --- | ---: | ---: | ---: | ---: |
| evidence | 2,000 | 58 | 2,000 | 1,379 |
| components | 2,000 | 312 | 2,000 | 177 |
| samourai | 2,000 | 68 | 2,000 | 1,588 |
| recorded | 2,000 | 92 | 2,000 | 1,655 |
| fanout | 2,000 | 94 | 2,000 | 1,494 |
| graph | 2,000 | 16 | 2,000 | 2,000 |
| pipeline | 2,000 | 24 | 1,841 | 1,675 |
| safety | 2,000 | 78 | 2,000 | 767 |
| channels | 2,000 | 64 | 2,000 | 1,563 |
| native | 2,000 | 110 | 2,000 | 1,601 |
| reconciliation | 2,000 | 50 | 100 | 96 |
| gaps | 2,000 | 64 | 2,000 | 249 |
| Archived replay | 159 | 159 | 159 | 156 |

Distinct counts exclude permutations. Each seed row retains its input hash,
request hash when active, and cumulative per-scenario and per-family counts
in `campaign/activity.jsonl`. Reconciliation repeats 100 distinct template
inputs across 2,000 calls; pipeline has 1,841 distinct inputs. These call
counts do not imply 2,000 distinct inputs for either family.

The first native-enabled parity segment recorded **1,657 passed and 2,642
subtests passed**, with no failure, before a long wait during the commercial
report export module. It was interrupted with SIGINT; the last completed case
was the `unbounded` commercial subledger, leaving the `empty` case incomplete.
Host ptrace policy refused diagnostic attachment. Preserve this interrupted
receipt separately; it is not a completed suite pass. The complete 21-case
report-export module passes on rerun with `--full-trace` and a 90-second
faulthandler report, including the previously incomplete empty-state case.
The initial wait does not reproduce there. The later restricted continuation passes 723 cases before interruption at the
confirmed owner/socket restrictions described above. Its failures and the
manually terminated deadlocked reference child are excluded from successful
coverage. The elevated continuation passes 553 cases with two platform skips;
its two failures are the obsolete XLSX refusal witness (corrected) and a
120-second Python `secrets init` timeout in the Bull parity case. Both rerun
successfully together (2 passed, 10 subtests, 26.59 s). No timeout increase or
Python product change is used.

The completed native suite covers **3,077 distinct collected nodes: 3,075
passed and two platform-gated skips**, through the hybrid candidate. The
coverage map excludes interrupted zero-time placeholders and binds every node
to a completed receipt. The two remaining gates are
`tests/test_rust_store_interop.py::test_native_case_aliases_keep_python_project_identity`
and `tests/test_rust_store_interop.py::test_windows_broker_secret_uses_inherited_os_handle`; this Linux checkout
does not supply the required native case-alias or Windows-handle environment.
All loopback, ownership, fresh-keyring and native Secret Service tests otherwise
run successfully after elevation. No sandbox-blocked parity test remains.

The separate hybrid harness passes **18 tests and six subtests**. Supplemental
import, wallet, transfer/component and gap suites through the hybrid candidate
pass **138 tests and 19 subtests**.

Persistence reproduces the checkpoint's **502 generator samples / 13 upstream
or schema refusals** and **10 payout samples / three pre-persistence refusals**.
All 578 initial complete-dump captures match: 502 generators, 10 payouts and 33
from each focused mode. The storage/backup/filed-report modules pass 96 tests
in each mode, with 109 exact loader matches per mode. Their 194 additional
complete-dump captures also match, including repeated rebuilds and explicit
post-build edits. The 26 frozen native cases pass separately, bringing the
combined complete-dump coverage to **798 / 798**, with zero masked columns,
rowids, timestamps or scalar types. Captures from different modes can overlap
inputs; this is a count of complete comparisons, not distinct books.

The full render campaign matches **24,705 complete state byte strings** on
24,000 generated books and 1,030 permutations. Its 178 upstream and 147
pre-render refusals match the custody campaign and remain separate from
successful rendering; no render refusal or byte mismatch occurs. All **318**
archived originals/permutations also match complete render state bytes.

The full mutation campaign passes **5,132 / 5,132 kills**, across **25 catalogs
and 17 serial checkers**. Every named unmutated baseline passes. Final receipts
have zero survivors, errors or failed checkers. Exact multiset reconciliation
finds no missing or extra IDs; catalog and recipe hashes bind that reconciliation
to the integrated sources. The initial transfer baseline failure is retained
separately, and its corrected full 280-entry retry supplies all transfer kills.
Each checker writes its summary before deleting its build and temporary trees;
the final absence of every `target/r8-mut-*` tree is independently checked.

| Mutation checker | Killed / catalog |
| --- | ---: |
| XLSX | 6 / 6 |
| Native loader | 59 / 59 |
| Journal rendering/persistence | 112 / 112 |
| Custody, including fusion | 2,327 / 2,327 |
| Ledger/tax engine | 187 / 187 |
| Transfers | 280 / 280 |
| Pairing | 35 / 35 |
| Wallet/importer basics | 478 / 478 |
| Sync/observation (nine catalogs) | 547 / 547 |
| Reports | 569 / 569 |
| Transactions | 227 / 227 |
| Backup | 107 / 107 |
| Diagnostics | 98 / 98 |
| Imports | 14 / 14 |
| Rates | 9 / 9 |
| Secrets | 18 / 18 |
| Setup | 59 / 59 |
| **Total** | **5,132 / 5,132** |

Aggregate hybrid logs also include fixture/setup work for unported
`wallets.import-ledger`, `journals.process`, `journals.quarantined`, and
default Python daemon sessions. Zero-fallback assertions apply to each
ported slice command; those setup calls do not establish native journal
or default-daemon acceptance.

The retained aggregate mirror has 1,643 Rust routing records across 135 command
strings (including aliases, usage refusals and daemon probes). Per-test routing
logs and their strict assertions cover additional commands independently of
this mirror, including all five incoming importer families, pairing, component/
gap reads and XLSX exports. The mirror's Python records are exactly seven
`wallets.import-ledger`, 27 `journals.process`, two `journals.quarantined` and
four default daemon setup calls. None is counted as a ported service fallback.

The catalog inventory is **5,132 recipes across 25 catalogs**, versus R7's
4,452 across 21. Source comparison finds 681 new IDs, 67 changed recipes and
one replaced ID, for a net increase of 680. The deliberate replacement in
`560084e84` is `capture-raw-hex-refusal` → `capture-raw-hex-decode`:
R7's blanket refusal for graphless Liquid data is superseded by the supplied
reference-compatible local decoder. The new mutant bypasses successful parse
admission and is witnessed by `liquid_confirmation_scope_python_vectors`,
with 14 additional decoder mutants. Restoring the obsolete blanket-refusal
witness would contradict this authorized port. No other R7 recipe ID is lost;
all other changed recipes retain their IDs and target the integrated code.
`mutation-delta.json` retains the exact additions, replacements and changes.

## Combined performance

The four disposable benchmark copies have byte-identical CSVs to the committed
seeded generators: generic seed 1 and multi-wallet seed 7, 2,000/20,000 rows,
with 171/300 paired pegs on the multi-wallet books. Cached source books are
read-only inputs; profiling writes only the copies. Load is recorded per pair.

Five Python/Rust pairs per book use the recorder's loaded-host policy (limit 8,
no quiet-host wait). Each runtime selects its lowest total wall time separately;
all Rust spans below, including CPU, come from that same selected Rust trial.
The process CPU clock covers all threads inside `custody.total`, from admission
through finalization and operation-local memo cleanup. It excludes parsing,
fusion, the tax engine and oracle serialization, and can exceed elapsed wall
time because work runs concurrently. Temporary timing-feature instrumentation
is restored before native verification; it is absent from committed sources.

| Book | Python command end to end (s) | Python custody wall (s) | Rust snapshot compute wall (s) | Rust custody wall (s) | Rust custody CPU (s) | Load 1m range |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| gen2k | 1.920 | 0.864 | 0.481 | 0.300 | 0.552 | 14.42–15.35 |
| mw2k | 2.846 | 1.342 | 0.784 | 0.554 | 0.969 | 26.94–33.04 |
| gen20k | 13.913 | 5.742 | 4.691 | 3.015 | 5.675 | 15.47–18.13 |
| mw20k | 22.883 | 13.676 | 6.394 | 4.414 | 8.301 | 27.45–35.34 |

These are measurements of the combined ledger, interpretation, quantity,
projection and fusion changes. Every book additionally passes all stage outputs,
fusion request and ledger response hashes against Python. Custody wall speedups
are approximately 2.9× / 2.4× / 1.9× / 3.1× in table order; the 10× custody
target remains open. No complete-command speedup is inferred from compute time.
The measured Python command also uses the integrated Rust tax engine.

The loader has five alternating pairs per book and an exact complete-v1 byte
gate before timing. Its Rust total includes the direct SQLite record load and
typed admission; open/BEGIN and v1 serialization are outside both timers.
These separate loader times must not be added to compute and presented as a
measured command time.

| Book | Python loader wall (s) | Rust loader + admission wall (s) | Rust loader + admission CPU (s) | Load 1m range |
| --- | ---: | ---: | ---: | --- |
| gen2k | 0.106 | 0.075 | 0.074 | 12.62 |
| mw2k | 0.154 | 0.124 | 0.123 | 11.53 |
| gen20k | 1.159 | 0.782 | 0.774 | 12.01–12.53 |
| mw20k | 1.438 | 1.006 | 0.996 | 11.72–14.46 |

The 0.03/0.04/0.30/0.35-second loader targets remain unmet. This checkpoint
still constructs the compatibility snapshot, ownership index and gap worklist;
no new profile attributes the remaining cost to any one of those operations.

Rendering compares complete state bytes before five alternating Python/Rust
trials. Times cover render and joins, excluding parsing, encoding and persistence.

| Book | Python render/joins (s) | Rust render/joins (s) | Nominal Rust budget (s) | Observed result | Load 1m range |
| --- | ---: | ---: | ---: | --- | --- |
| gen2k | 0.096 | 0.013 | 0.020 | Within | 19.20 |
| mw2k | 0.091 | 0.017 | 0.020 | Within | 14.66 |
| gen20k | 1.004 | 0.144 | 0.150 | Within | 15.16–16.28 |
| mw20k | 1.275 | 0.168 | 0.200 | Within | 11.03–11.76 |

All four observed render/joins times fit the nominal budgets. These loaded-host
measurements do not replace the plan's quiet-host acceptance gate. The raw trials,
CPU spans, source/binary hashes, CSV hashes, loader byte digests and large-book
parity receipts remain in `target/r8-integration/perf`. Disposable benchmark
books and the profiler's copied books are deleted after the receipts are saved.

The registered Rust `journals process` command remains unimplemented at this
checkpoint. Rust snapshot-to-ledger compute excludes native SQL loading,
rendering, persistence and command orchestration. Its timing cannot establish
complete Rust command end-to-end acceptance against Python `journals process`.
The unregistered [loader](r4-native-loader-checkpoint.md) and
[persistence](r4-journal-persistence-checkpoint.md) probes retain their checkpoint
boundaries; R4-13 orchestration is still required.

## Evidence and remaining work

Local evidence lives in `target/r8-integration`: the ordered input/replay map,
conflict records, lockfile provenance, applicability inventory, per-gate logs,
per-seed activity, complete-dump comparisons and performance receipts. No
private book or request is committed.

The restricted continuation rejected creation of a disposable D-Bus socket
with `Operation not permitted`. With
`KASSIBER_TEST_DISPOSABLE_KEYRING=1`, the following
`tests/test_rust_secrets_lifecycle.py` cases initially skipped:

- `test_disposable_keyring_python_and_rust_enrollment_rotation_and_revocation`.
- `test_disposable_keyring_interruption_resumes_in_either_runtime` (28 cases):
  both `python-recovery` and `rust-recovery` for each checkpoint below.

| Operation | Checkpoints |
| --- | --- |
| remember | `remember.after_item_write`, `remember.after_prior_delete`, `remember.after_legacy_cli_delete`, `remember.after_legacy_shared_delete`, `remember.after_policy_publish` |
| forget | `forget.after_item_delete`, `forget.after_legacy_cli_delete`, `forget.after_legacy_shared_delete`, `forget.after_policy_publish` |
| rotate | `rotation.after_item_refresh` |
| rotate-failed-refresh | `rotation.after_item_delete`, `rotation.after_legacy_cli_delete`, `rotation.after_legacy_shared_delete`, `rotation.after_policy_disable` |

All **29** cases above subsequently pass in the elevated continuation on a
fresh bus and keyring. A disposable `dbus-run-session` wrapper supplies a
local bus configuration whose socket directory is `target/b8`; there are no
service activation directories and no account credential-store connection.
No test data or bus socket is created under `/tmp` by that continuation.

Remaining acceptance is R4-13 native command orchestration, the loader and 10×
custody performance targets, and quiet-host performance confirmation. Full
census and custody-shadow shards remain for the owner's requested follow-up.
The two platform-gated parity cases and existing deliberate-panic-hook skip are
named above. The R7 BTCPay full-shard interaction is not reproduced by its
successful focused rerun. No unexplained skip or design conflict remains.

After their summaries, all task-owned mutation and native verification build
trees are removed, including `target/r8-build` and its `target/debug` symlink.
Disposable keyring/bus roots and temporary fixtures are removed; an empty
scratch directory remains for the documented commands. Completed performance
inputs and the 194 additional storage captures are hashed before deletion.
Small receipts, raw timing trials, per-seed activity and verified executable
copies remain under `target/r8-integration`, with the final harness and probes
in `final-bin`. A host process check finds no task-owned test or daemon process;
no background job is left running. `cleanup-final.json` and
`processes-final.json` record the final checks.

## Ordered replay map

| Original | Integrated / existing | Action | Subject |
| --- | --- | --- | --- |
| `d89d335b9` | `addedd20c` | pick | Skip custody comparison when a test replaces a compute function |
| `ff7050c00` | `53625d387` | skip | Design the R4 storage-format speedups from a measured prototype |
| `6a94f6767` | `cf370bf5c` | skip | Store retained tax calculations in a compact column |
| `5ade4b811` | `5591fee0f` | skip | Add the R4-11 journal rendering checkpoint |
| `de18d9afd` | `d634e590f` | pick | Time the ledger operation's stages behind an opt-in feature |
| `2ac6c521f` | `b21703131` | pick | Hand the walk's constructed entries to the lot engine |
| `06edc4cb4` | `28728613a` | pick | Reuse the rail carry's settled pass as the final pass |
| `41ec82236` | `abc9c32b1` | pick | Compute decimal coefficients that fit u128 with fixed-width integers |
| `f52d52981` | `672a1d0f8` | pick | Report the ledger bench's on-CPU time |
| `7ef06f2ef` | `a2e771350` | pick | Render an entry's text into one buffer |
| `2d92bb27a` | `9f381edac` | pick | Read a fragment's prices and fees from its entries directly |
| `03c343a08` | `9839b6e44` | pick | Read a whole ledger request once |
| `0537479d8` | `3f8eb0438` | pick | Read and write boundary decimals without intermediate strings |
| `31ac85163` | `4f16c6ab2` | pick | Count a u128's decimal digits from its bit length |
| `3a39f611c` | `f7aaf5d2b` | pick | Keep the request's book when the rail carry priced nothing |
| `c93a6b194` | `f509902f8` | pick | Let the lot engine borrow the walk's entries |
| `5bfbcaa26` | `fb3afa491` | pick | Write long response lists in concurrent chunks |
| `6dee1a9ff` | `3cbcaf241` | pick | Prepare and compute a book's assets concurrently |
| `16c79421e` | `6e747aa36` | pick | Run the walk's entry constructors concurrently after the walk |
| `0c6642677` | `77369cee2` | pick | Compose acquisitions and realized fragments concurrently |
| `0d1deebb9` | `2d6d74123` | pick | Derive a computed asset's fragment values concurrently |
| `de836bd23` | `f372b5547` | pick | Keep a decimal at its former size |
| `5c35fb65f` | `55301905a` | pick | Record ledger speed-pass performance and verification |
| `74f605e65` | `2d245d19e` | pick | Add typed custody rows and network admission checkpoint |
| `146281cf8` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `92ecc0562` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `4c130249c` | `7f8594eb2` | pick | Record the stopped R4 interpretation probe |
| `951954d03` | `77225f073` | pick | Accelerate custody interpretation and gap search |
| `3334d05d1` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `d2d3a66eb` | `a746ebd81` | pick | Record the failed R4-6 quantity target probe |
| `c017987c9` | `77225f073` | skip | Declare the split-payout test's injected stage failure |
| `a687c07ca` | `f2942bb0a` | pick | Accelerate Rust custody quantity kernels |
| `cf2d4a1b7` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `25b87bfc8` | `f3fb138f9` | pick | Record the R4-7 projection target miss and custody span |
| `78df09179` | `13d6164a1` | pick | Reduce custody projection and fusion kernel copies |
| `f985462ea` | `e8859a4ba` | pick | Record the projection and fusion kernel checkpoint |
| `81e7f221d` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `4624073ee` | `2dde4bd38` | pick | Reduce journal rendering and ownership join costs |
| `db8534b0b` | `1f8d20d59` | pick | Complete compact journal storage verification |
| `4100deb31` | `03e034810` | pick | Retain unchanged journal entries and quantity postings |
| `4f6ca7d06` | `6a2c44498` | pick | Scope the journal rebuild page cache |
| `adeefd597` | `2c44db454` | pick | Implement an unregistered Rust journal persistence checkpoint |
| `1f064db49` | `2d245d19e` | skip | Record the missing R4 baseline and failed row probe |
| `569d80add` | `66cf4e3ae` | pick | Checkpoint the native custody loader behind the test harness |
| `0b78605a0` | `9ba49cd8e` | pick | Build custody snapshot records directly from SQLite |
| `d2023f2a3` | `f3a70aa19` | pick | Serve transfer pairing commands through Rust |
| `e9806ca51` | `752b4445d` | pick | Serve stored custody component reads through Rust |
| `15c320977` | `28315951c` | pick | Serve custody gap snapshots through Rust |
| `7f2e02655` | `6ba03b424` | pick | Refresh transfer read fixtures for the R7 migration |
| `d64433d79` | `532aa3d56` | pick | Record the verified R7 transfer integration boundary |
| `78bda76b0` | `11d91df85` | pick | Assemble native Bitcoin wallet fetches |
| `d50d2ce2f` | `530bb0456` | pick | Refresh compatibility evidence when the chain tip changes |
| `0207dbcfc` | `0adacda0e` | pick | Refresh Rust wallet evidence when the tip changes |
| `5e39ba109` | `11c36fb52` | pick | Add a request-bound Bitcoin wallet sync service |
| `2065ef67c` | `143186a8a` | pick | Port Bull Bitcoin order imports to Rust |
| `f872c85fb` | `209665d69` | pick | Refresh Bull vectors against the integrated reference |
| `d5d53ee6d` | `d259539f7` | pick | Port Coinfinity order imports to Rust |
| `15a100788` | `5be83e42c` | pick | Port 21bitcoin custodial ledger imports to Rust |
| `3f8e12b0f` | `9a0d33c00` | pick | Port Pocket Bitcoin evidence imports to Rust |
| `92ec79939` | `91dfe121f` | pick | Port Strike custodial ledger imports to Rust |
| `da74dfe62` | `9fbe34a8b` | pick | Invalidate journals after legacy 21bitcoin reactivation |
| `560084e84` | `f2f323a64` | pick | Capture graphless Liquid report dependencies natively |
| `f5ca58c54` | `cb4281d34` | pick | Port reviewed commercial subledger reports |
| `af6593627` | `a42bc7bf7` | pick | Render deterministic XLSX workbooks in Rust |
| `f6f04d9db` | `942973675` | pick | Enable native transaction XLSX exports |
| `e193e6b10` | `b5eb38ea8` | pick | Port Austrian E1kv XLSX exports natively |
