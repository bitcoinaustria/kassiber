# Test speed

> **Historical.** This measurement predates Python's removal under
> [plan 22](../plan/22-python-removal.md). The pytest shards, the shadow lanes
> and the Python tests named below no longer exist; the
> [Rust test binaries](#rust-test-binaries) and
> [dependency optimization](#optimizing-dependencies-in-test-builds) sections
> still describe the Rust suites.

Where the suites spend their time, what was changed to shorten the critical
path, and what is left. Measured 2026-11-06/07 from 06e253ea6 on the shared
48-thread build machine (about 12 agents; load between 15 and 175), so wall
times carry noise and CPU times are the steadier figure.

## How it was measured

Every Python shard of `scripts/python_test_shards.py` ran once with
`-n 16`, the gate's environment (`KASSIBER_TEST_TAX_ENGINE=census`,
`KASSIBER_TEST_CUSTODY_ENGINE=census`, the release custody loader and journal
process examples, `KASSIBER_RUST_BIN` at the debug test-harness build), the
test-only no-flush shim, `--durations=0 --durations-min=2`, and a junit file.
`--basetemp` sat under `target/`, because several fixtures accept disposable
books there only. A scratch pytest plugin recorded each test's worker and
start/stop times. CPU is the process tree's user plus system time.

Serial lanes also ran with 16 workers here; CI keeps them serial. With
`--dist load`, `CliSmokeTest` fails in daemon-cli because its numbered
methods share state; CI's `loadscope` keeps the class on one worker.

Rust test binaries were built with the gate's
`cargo test --workspace --features kassiber-journals/test-harness --no-run`
and run one at a time from their package directory, as `cargo test` does.

## Shards

Wall / CPU seconds at 16 workers. "After" is this branch with `--dist load`
(the same scheduling as before); the last column is `--dist worksteal`,
measured for the shards where scheduling, not work, set the wall time.

| Shard | Before | After (`load`) | After (`worksteal`) |
| --- | --- | --- | --- |
| preflight | 61 / 192 | 20 / 75 | |
| cli-smoke | 11 / 54 | 8 / 32 | |
| core-accounting | 522 / 3366 | 301 / 2804 | |
| wallets-sync | 1384 / 1506 | 382 / 1358 | 118 / 1372 |
| daemon-cli | 81 / 956 | 79 / 854 | (keeps classes together) |
| security-replication | 37 / 417 | 34 / 404 | |
| reports-contracts | 356 / 2020 | 300 / 1708 | 202 / 1713 |
| serial-network | 64 / 198 | 90 / 205 | |
| serial-daemon | 3419 / 14454 | 4293 / 13170 | 1250 / 14068 |
| serial-regressions | 38 / 447 | 42 / 476 | |
| serial-integration | 130 / 1641 | 124 / 1281 | |
| **All, one after another** | **6103** | **5673** | **2268** |

The last total uses `worksteal` where measured and `load` elsewhere.

`--dist load` hands every worker a backlog up front and never moves it, so
consecutive heavy items pile up on one worker: after the splits, wallets-sync
still ran 376 s of Samourai slices on one worker while ten others were idle
after 5 s, and serial-daemon queued 4,155 s of shell-parity slices on one
worker (mean parallelism 3.2 of 16). `worksteal` lets idle workers take
queued tests, which raised mean parallelism to 14.8 (wallets-sync) and 11.9
(serial-daemon) with the same tests and the same CPU. For local runs:

```bash
uv run --locked python scripts/python_test_shards.py run wallets-sync --ci --workers 16 --dist worksteal
```

CI keeps two workers and `loadscope`; see [recommendations](#recommendations).

## What changed

Every split keeps the same inputs and assertions. For the top items a seeded
fault fails the original test and exactly the split test or slice holding the
faulted case, with the same message or subtest (run in one 16-worker job).

| Item | Before | After | Change | Seeded-fault proof |
| --- | --- | --- | --- | --- |
| Contract snapshots (preflight) | 42 s, one test | 10 s longest of four | one snapshot per test id | a drifted `error_codes.json` fails the original and only `[error_codes.json]` |
| Shard-manifest tests (preflight) | 21-22 s each | about 4 s | the overlay's import scan is cached by file content | |
| Native journal vectors | 50 s (`True`), 28 s | 1 s each, 230 ids | one vector per test id | a changed `all-stages` result fails the original with `all-stages` and only `[all-stages-False/True]` |
| Journal refresh vectors | 23 s | 2 s each, 114 ids | one vector per test id | |
| PSBT product comparison | 120 s, 98 s | 24-28 s, 8 ids | one selected case per test id | |
| Samourai reference corpus | 974 s | 56 s longest of 16 slices | slices of the 518 files, same book per slice, per-file subtests | a relabelled account fails the original and only `[slice06]`, subtest `account-main-3-False`, both codecs |
| Samourai hybrid corpus | 401 s | 44 s longest of 17 | slices, plus the coordinator calls as their own test | |
| Samourai codepoints | 75 s | 44 s + 23 s | wire and routing halves as two tests | |
| Shell-parity reference corpora (Blockpit, Binance, Wasabi, Ledger, Ledger preview, BTCPay JSON/CSV) | 296-983 s each | 91-248 s per slice | four slices each | a changed Binance description fails the original and only `[slice03]`, same subtest, both codecs |
| Watch-worker reorg test | 261 s (210 s alone) | 38-40 s | a session that sees no backend request ends after two finished worker passes, not the 90 s limit | |

Slices replay their files in order from the same baseline book the unittest
method built, so each slice still exercises the setup; the Samourai and
shell-parity corpora use slices instead of one test per file because that
setup (`import_run_book` plus `secrets init`, about 3.4 s unloaded) would
otherwise run 518 or 341 times. Four parity slices, not sixteen, because
serial-daemon's total work, not one test, bounds its wall time.

The watch-worker change adds a test-only marker: both workers append one line
per finished background pass to a file the test names
(`KASSIBER_RUST_TEST_WATCH_PASS_LOG` behind the daemon's `test-kinds`
feature; the Python reference launcher wraps its tick). The passes run on the
pinned analysis clock, a session that has seen requests still waits for them
to go quiet, and without the marker the wait is the old limit.

## Tests that failed only under load

| Test | Cause | Fix |
| --- | --- | --- |
| `test_rust_journal_desktop.py` pause tests (two seen by the integrator with all ten journal modules on 16 workers) | `KASSIBER_RUST_TEST_JOURNAL_WAIT_PHASE` held the native worker for a fixed 3 s; a test slowed by load cancelled, locked or rewrote settings after the worker had moved on. A seeded 3.5 s delay after each test saw its phase failed all 50 pause tests. | The hook waits until the request is cancelled or the test creates `KASSIBER_RUST_TEST_JOURNAL_WAIT_RELEASE`; the same 50 tests pass with the delay. |
| `test_rust_source_funds_links.py::...[daemon]` | Links created at the pinned time list by id, and both runtimes drew random ids: 12 of 24 repeated runs failed with or without load. | `KASSIBER_RUST_TEST_REPORT_IDS=sequential` in Rust and the same numbering in the Python reference; 48 of 48 pass. |
| `test_rust_wallet_publication.py::test_wallet_publication_vectors_still_match_python` | Fixed loopback ports (part of the vectors) were held by another session on the machine: `EADDRINUSE`. | `serve()` waits for a fixed port to free up (up to 600 s). |
| `test_rust_wallet_document_import.py::...test_sqlcipher_import_matches_python` | The Python reference daemon sometimes answers the import with `database_busy` (SQLITE_BUSY from sqlcipher3) instead of waiting, only on encrypted books; probably its watch worker's second connection right after unlock. | Python reference behaviour, left unchanged: repro in `tests/rust_port/repro_document_import_busy.py` (24 of 96 copies on 2026-11-07). |

No test ran Kassiber against the account's real offline preference: an
`LD_PRELOAD` audit over the whole suite (every Python shard after these
changes and every Rust test binary) logged no open or stat of
`~/.local/share/kassiber/config/offline-mode.json` in any process, including
children that rebuild their environment, which it followed. Its only hits
were helper programs started with a minimal environment that resolves to the
real state root (`/sbin/ldconfig -p` from `ctypes.util.find_library`, and
`sh`/`ps` from `kassiber-ipc` and `kassiber-operator` unit tests); none of
them reads the file.

The conftest's session-end owner-lock sweep runs in every xdist worker when
that worker drains, unlinks lock files in the account-wide namespace that it
can lock, and takes no admission lock, unlike the product's own pruner. It
did not cause any failure here, because the parity daemons lock under
per-test owner roots, but it can race other sessions that use the shared
namespace.

## Failing before and after

These fail at 06e253ea6 too: the serial rerun of every baseline failure
reproduced all of them except the four load-only tests above and
`CliSmokeTest` (which passes as a class).

- `test_audit_package.py::AuditPackageCoreTest::test_audit_summary_retains_filed_snapshot_and_amendment_history` (9 migration audits, expects 8)
- `test_cli_runner.py::ConstructionSiteTest::test_no_test_builds_the_product_command_itself` (four modules build `python -m kassiber` themselves)
- `test_rust_ai_tools.py::...test_mutating_tools_match_python_on_every_fixture`, `test_rust_mcp.py::...test_transcripts_match_python_on_every_fixture`
- `test_rust_backup_install.py::test_ordinary_cli_restart_recovers_before_layout_and_database_open[*-restore.after_install.config/settings.json-False-False-*]` (4)
- `test_rust_document_cli.py` (8) and `test_rust_document_ingestion.py` (3)
- `test_rust_hybrid_candidate.py::HybridCandidateTest::test_inherited_descriptors_reach_either_side`
- `test_rust_journal_process.py::test_native_real_prebuild_preserves_blob_consumers_without_v1_encoding` (6, with the census's custody differences)
- `test_rust_metadata_services.py`, `test_rust_transaction_services.py` (unported families refuse before bootstrap), `test_rust_psbt_transport.py::test_psbt_transport_mutations_stay_applicable`, `test_rust_review_cases.py` (2)
- `test_rust_shell_parity.py`: five `RustDaemonParityTest` cases and two `RustRealStoreParityTest` cases
- core-accounting's census: 41 custody differences and `tests/test_journal_process_campaign.py` missing from the shadow overlay
- Rust (through `cargo test -p kassiber-daemon -p kassiber-net --no-fail-fast`): `kassiber-net` `chokepoint` (an unreviewed `kassiber-pdf` dependency; network use found outside `kassiber-net`), `kassiber-daemon` `agent_access::tests::test_ending_the_desktop_session_ends_the_agent_lease` and `protocol::malformed_requests_match_python`. The daemon code they run is unchanged, because the only daemon change is behind `test-kinds`. Run directly from the package directory, `kassiber-net`'s `dependencies` binary also fails; under `cargo test` it passes.

## Slowest Python tests before

Seconds per test (setup, call and teardown) at 16 workers, before the changes.

| # | s | Shard | Test |
| --- | --- | --- | --- |
| 1 | 983 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_blockpit_reference_files_wire_bytes_full_state_and_rollback` |
| 2 | 974 | wallets-sync | `tests/test_rust_samourai_import.py::RustSamouraiImportTest::test_reference_files_match_raw_cli_wire_and_full_books_on_both_codecs` |
| 3 | 703 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_binance_reference_files_wire_bytes_full_state_and_rollback` |
| 4 | 653 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_wasabi_reference_files_wire_bytes_full_state_and_rollback` |
| 5 | 640 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_ledger_reference_files_wire_bytes_full_state_and_rollback` |
| 6 | 514 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_btcpay_json_reference_files_wire_bytes_full_state_and_rollback` |
| 7 | 424 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_ledger_import_and_preview_have_no_fallback_and_match_every_reference_dump` |
| 8 | 407 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_ledger_preview_reference_files_wire_bytes_and_no_mutations` |
| 9 | 401 | wallets-sync | `tests/test_rust_samourai_import.py::RustSamouraiImportTest::test_hybrid_reference_files_and_coordinator_calls_have_zero_importer_fallbacks` |
| 10 | 318 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_coinfinity_import_has_no_fallback_and_matches_every_reference_dump` |
| 11 | 300 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_pocket_aliases_have_no_fallback_and_match_every_reference_dump` |
| 12 | 296 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_btcpay_csv_reference_files_wire_bytes_full_state_and_rollback` |
| 13 | 263 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_blockpit_import_has_no_fallback_and_matches_every_reference_dump` |
| 14 | 261 | core-accounting | `tests/test_rust_analysis_watch_worker.py::test_source_runner_reorg_and_resume_match_python` |
| 15 | 258 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_json_import_has_no_fallback_and_matches_every_reference_dump` |
| 16 | 236 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_bull_import_has_no_fallback_and_matches_every_reference_dump` |
| 17 | 222 | serial-daemon | `tests/test_rust_document_ingestion.py::RustDocumentIngestionTest::test_reference_drafts_match_complete_books_and_managed_bytes_on_both_codecs` |
| 18 | 208 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_wasabi_import_has_no_fallback_and_matches_every_reference_dump` |
| 19 | 186 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_strike_import_has_no_fallback_and_matches_every_reference_dump` |
| 20 | 177 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_btcpay_import_has_no_fallback_and_matches_every_reference_dump` |
| 21 | 174 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_cointracking_import_has_no_fallback_and_matches_every_reference_dump` |
| 22 | 170 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_binance_import_has_no_fallback_and_matches_every_reference_dump` |
| 23 | 143 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_river_import_has_no_fallback_and_matches_every_reference_dump` |
| 24 | 130 | core-accounting | `tests/test_rust_analysis_grants.py::test_grant_product_cli_bytes_all_tables_and_zero_fallback[False]` |
| 25 | 129 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_twentyone_import_has_no_fallback_and_matches_every_reference_dump` |
| 26 | 120 | core-accounting | `tests/test_rust_analysis_dataset_import.py::test_dataset_import_product_cli_bytes_all_tables_and_zero_fallback[False]` |
| 27 | 120 | core-accounting | `tests/test_rust_psbt_product.py::test_psbt_product_bytes_files_all_tables_and_zero_hybrid_fallback[False]` |
| 28 | 114 | core-accounting | `tests/test_rust_analysis_watch_worker.py::test_watch_worker_emits_one_event_and_matches_python_tables` |
| 29 | 111 | serial-daemon | `tests/test_rust_transaction_desktop.py::RustTransactionDesktopTest::test_pages_grouped_totals_queue_scope_and_cursor_bytes` |
| 30 | 105 | core-accounting | `tests/test_rust_analysis_acquisition_apply.py::test_acquisition_apply_cli_matches_python_requests_bytes_and_tables[esplora-transaction-limit]` |
| 31 | 98 | core-accounting | `tests/test_rust_psbt_product.py::test_psbt_product_bytes_files_all_tables_and_zero_hybrid_fallback[True]` |
| 32 | 97 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_ledger_spreadsheet_integrity_import_and_preview_wire_bytes_and_full_state` |
| 33 | 97 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_ledgerlive_import_has_no_fallback_and_matches_every_reference_dump` |
| 34 | 89 | core-accounting | `tests/test_rust_analysis_watches.py::test_watch_product_cli_bytes_all_tables_and_zero_fallback[False]` |
| 35 | 89 | core-accounting | `tests/test_rust_transfer_reads.py::RustTransferReadsTest::test_component_cli_bytes_private_views_filters_and_unchanged_rows` |
| 36 | 86 | core-accounting | `tests/test_rust_analysis_watches.py::test_watch_product_cli_bytes_all_tables_and_zero_fallback[True]` |
| 37 | 83 | core-accounting | `tests/test_rust_pairing_commands.py::RustPairingCommandsTest::test_pairing_cli_bytes_dry_run_rows_hybrid_and_python_projections` |
| 38 | 83 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_json_import_wire_bytes_full_state_and_rollback` |
| 39 | 77 | serial-daemon | `tests/test_rust_source_funds_report.py::test_source_funds_complete_report_cli_bytes_files_tables_and_zero_fallbacks[stale]` |
| 40 | 77 | serial-daemon | `tests/test_rust_source_funds_report.py::test_source_funds_complete_report_cli_bytes_files_tables_and_zero_fallbacks[source-blocked]` |
| 41 | 75 | wallets-sync | `tests/test_rust_samourai_import.py::RustSamouraiImportTest::test_codepoints_match_raw_wire_books_and_native_hybrid_routing` |
| 42 | 75 | reports-contracts | `tests/test_custody_quantity_stage.py::test_every_quantity_raw_generator_matches_both_engines[gaps-scenarios11-snapshot]` |
| 43 | 63 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_wallet_derive_wire_bytes_match_reference` |
| 44 | 63 | core-accounting | `tests/test_rust_ai_tools.py::AiToolExecutorParityTest::test_mutating_tools_match_python_on_every_fixture` |
| 45 | 61 | serial-daemon | `tests/test_rust_source_funds_report.py::test_source_funds_complete_report_cli_bytes_files_tables_and_zero_fallbacks[no-context]` |
| 46 | 60 | serial-daemon | `tests/test_rust_source_funds_report.py::test_source_funds_complete_report_cli_bytes_files_tables_and_zero_fallbacks[ready]` |
| 47 | 59 | serial-daemon | `tests/test_rust_wallet_onboarding.py::RustWalletIdentifyTest::test_tokens_and_formats_match_wire_bytes` |
| 48 | 54 | reports-contracts | `tests/test_custody_gap_port.py::test_every_gap_generator_matches_and_counts_both_engines[gaps-scenarios11-snapshot]` |
| 49 | 53 | core-accounting | `tests/test_rust_ai_tools.py::AiToolExecutorParityTest::test_read_tools_match_python_on_every_fixture` |
| 50 | 52 | serial-daemon | `tests/test_rust_shell_parity.py::RustRealStoreParityTest::test_wasabi_coordinator_wire_bytes_full_state_and_rollback` |

## Slowest Python tests now

The top 25 after the changes (worksteal runs where measured). The
`HybridCandidateTest` corpus tests import every reference file into one
growing book, so each case depends on the ones before it and they cannot be
sliced without changing their inputs.

| # | s | Shard | Test |
| --- | --- | --- | --- |
| 1 | 383 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_ledger_import_and_preview_have_no_fallback_and_match_every_reference_dump` |
| 2 | 304 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_pocket_aliases_have_no_fallback_and_match_every_reference_dump` |
| 3 | 299 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_coinfinity_import_has_no_fallback_and_matches_every_reference_dump` |
| 4 | 261 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_json_import_has_no_fallback_and_matches_every_reference_dump` |
| 5 | 254 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_bull_import_has_no_fallback_and_matches_every_reference_dump` |
| 6 | 248 | serial-daemon | `tests/test_rust_shell_parity.py::test_blockpit_reference_files_wire_bytes_full_state_and_rollback[slice01]` |
| 7 | 237 | serial-daemon | `tests/test_rust_shell_parity.py::test_blockpit_reference_files_wire_bytes_full_state_and_rollback[slice00]` |
| 8 | 235 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_blockpit_import_has_no_fallback_and_matches_every_reference_dump` |
| 9 | 229 | serial-daemon | `tests/test_rust_shell_parity.py::test_blockpit_reference_files_wire_bytes_full_state_and_rollback[slice03]` |
| 10 | 227 | serial-daemon | `tests/test_rust_shell_parity.py::test_blockpit_reference_files_wire_bytes_full_state_and_rollback[slice02]` |
| 11 | 214 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_wasabi_import_has_no_fallback_and_matches_every_reference_dump` |
| 12 | 206 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_btcpay_import_has_no_fallback_and_matches_every_reference_dump` |
| 13 | 186 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_strike_import_has_no_fallback_and_matches_every_reference_dump` |
| 14 | 183 | serial-daemon | `tests/test_rust_document_ingestion.py::RustDocumentIngestionTest::test_reference_drafts_match_complete_books_and_managed_bytes_on_both_codecs` |
| 15 | 177 | serial-daemon | `tests/test_rust_shell_parity.py::test_wasabi_reference_files_wire_bytes_full_state_and_rollback[slice02]` |
| 16 | 173 | serial-daemon | `tests/test_rust_hybrid_candidate.py::HybridCandidateTest::test_cointracking_import_has_no_fallback_and_matches_every_reference_dump` |
| 17 | 172 | serial-daemon | `tests/test_rust_shell_parity.py::test_binance_reference_files_wire_bytes_full_state_and_rollback[slice00]` |
| 18 | 170 | serial-daemon | `tests/test_rust_shell_parity.py::test_ledger_reference_files_wire_bytes_full_state_and_rollback[slice02]` |
| 19 | 168 | serial-daemon | `tests/test_rust_shell_parity.py::test_binance_reference_files_wire_bytes_full_state_and_rollback[slice02]` |
| 20 | 164 | serial-daemon | `tests/test_rust_shell_parity.py::test_binance_reference_files_wire_bytes_full_state_and_rollback[slice03]` |
| 21 | 162 | serial-daemon | `tests/test_rust_shell_parity.py::test_binance_reference_files_wire_bytes_full_state_and_rollback[slice01]` |
| 22 | 161 | serial-daemon | `tests/test_rust_shell_parity.py::test_wasabi_reference_files_wire_bytes_full_state_and_rollback[slice00]` |
| 23 | 158 | serial-daemon | `tests/test_rust_shell_parity.py::test_ledger_reference_files_wire_bytes_full_state_and_rollback[slice00]` |
| 24 | 158 | serial-daemon | `tests/test_rust_shell_parity.py::test_wasabi_reference_files_wire_bytes_full_state_and_rollback[slice03]` |
| 25 | 156 | serial-daemon | `tests/test_rust_shell_parity.py::test_wasabi_reference_files_wire_bytes_full_state_and_rollback[slice01]` |

## Rust test binaries

`cargo test --workspace` runs 176 binaries one after another: 1,227 s wall,
7,083 s CPU in total. The top 20:

| # | Wall s | CPU s | Binary | Tests |
| --- | --- | --- | --- | --- |
| 1 | 281 | 4977 | `kassiber-wallets` lib `kassiber_wallets` | 386 |
| 2 | 112 | 183 | `kassiber-transfers` test `review_plans` | 8 |
| 3 | 61 | 71 | `kassiber-transfers` test `gap_reads` | 2 |
| 4 | 58 | 57 | `kassiber-net` test `chokepoint` | 8 |
| 5 | 55 | 89 | `kassiber-journals` lib `kassiber_journals` | 94 |
| 6 | 51 | 230 | `kassiber-reports` lib `kassiber_reports` | 57 |
| 7 | 44 | 189 | `kassiber-store` lib `kassiber_store` | 69 |
| 8 | 42 | 71 | `kassiber-transfers` test `pairs` | 13 |
| 9 | 38 | 43 | `kassiber-reports` test `full_pdf` | 1 |
| 10 | 35 | 49 | `kassiber-journals` test `prebuild_pipeline` | 8 |
| 11 | 30 | 33 | `kassiber-reports` test `austrian_pdf` | 1 |
| 12 | 28 | 101 | `kassiber-journals` test `persistence` | 31 |
| 13 | 26 | 81 | `kassiber-transfers` test `pairing_services` | 7 |
| 14 | 20 | 42 | `kassiber-transfers` test `review_apply` | 13 |
| 15 | 20 | 19 | `kassiber-journals` test `crash` | 2 |
| 16 | 19 | 20 | `kassiber-journals` test `process_native` | 4 |
| 17 | 19 | 21 | `kassiber-reports` test `source_funds_report` | 3 |
| 18 | 16 | 19 | `kassiber-transfers` test `component_reads` | 3 |
| 19 | 15 | 15 | `kassiber-reports` test `report_design` | 5 |
| 20 | 13 | 4 | `kassiber-chain` lib `kassiber_chain` | 40 |

`kassiber-wallets`'s lib tests set the Rust wall time, and most of their CPU
is contention: run one at a time, the 386 tests take 495 s user and 7 s system
time (the longest, `analysis_index::mirror::tests::mirror_matches_python_report_bytes_and_all_112_tables`,
93 s); with the default 48 test threads they take 230 s wall with 766 s user
and 4,294 s system time. Fewer threads are faster:

| `RUST_TEST_THREADS` | Wall | CPU |
| --- | --- | --- |
| 48 (default here) | 230 s | 5,061 s |
| 16 | 198 s | 2,115 s |
| 8 | 165 s | 949 s |
| 4 | 175 s | 599 s |

The kernel time points at contention inside the one test process (page
faults or allocator locks across threads); it was not traced further.

## Optimizing dependencies in test builds

`cargo test --config 'profile.dev.package."*".opt-level=1'` optimizes
third-party crates in debug and test builds. Measured cold in one scratch
build directory (build scripts and proc-macros, which this setting does not
cover, were already built for the second run):

| | Deps at opt-level 0 (now) | Deps at opt-level 1 |
| --- | --- | --- |
| Cold build of the `kassiber` test-harness binary | 90 s wall, 735 s CPU | 146 s wall, 1,572 s CPU |
| Then `kassiber-wallets` lib tests (`--no-run`) | 39 s | 42 s |
| `kassiber-wallets` lib tests (the slowest binary) | 232 s wall, 5,061 s CPU | 228 s wall, 4,965 s CPU |
| Python parity tests on that binary (Binance slices, four Samourai slices, desktop journals; 150 tests, 16 workers) | 327 s wall, 1,081 s CPU | 320 s wall, 1,031 s CPU |

Recommendation: do not change the profile for test speed. The gain is 2-5 %,
because these tests spend their time in the Python reference, process
start-up and SQLCipher key derivation rather than in dependency code; the
cost is about a minute per cold dependency build. Revisit it for a test that
profiles as dependency-bound.

## Recommendations

The pytest recommendations of this measurement (worksteal scheduling for the
Python shards, parallelizing serial-daemon, starting known-long Python tests
first) ended with Python. The Rust one still applies:

- Cap the `kassiber-wallets` lib test threads (8 here) or find the contention
  above; at 48 threads the binary uses 5,061 s CPU for 230 s.
