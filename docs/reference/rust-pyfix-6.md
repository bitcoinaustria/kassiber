# Python fix round r7-6 in the Rust port

Branch `t3code/rust-pyfix-6` merges the round r7-6 Python reference fixes
(`t3code/python-reference-fixes-r7-6`, tip 8e2ed0807) into the integration
line r10b (c48358377) and makes the Rust port follow them.
[Plan 21](../plan/21-rust-backend.md) owns the parity rule: Python stays the
reference until a slice ports the change. Each r7-6 commit carries a repro or
check under `tests/rust_port/repro_*.py`; those scenarios are the vectors
here, and every one of them exits 0 (fixed) on the merged Python.

## How differences were shown

Each "differs" or "matches" is a differential run. The scenario of the
round's repro became a parity test that runs the merged Python reference and
the Rust binary (or crate) on copies of one book and compares envelopes,
events, broker or provider requests and the book's tables byte for byte.
"Differs" means that test failed against the unported Rust: the merge commit
4dfdcdd7e, built in its own scratch tree under `target/` with the branch's
test files copied in. Every port adds mutation entries that put the old
behaviour back; the named tests kill them (`scripts/mutations.json` of each
crate, see [mutation checks](#mutation-checks)). Table lists come from the
store's checked-in schema manifest (`crates/store/fixtures/current-schema.json`),
never a hard-coded count.

## Merge decisions

The merge (4dfdcdd7e) keeps both sides' Python. The r7-6 line also carries
main #672 to #714 and the S1 store work, which r10b lacked; they are merged
as they are and their Rust gaps stay with the slices in
[Rust main sync 1](rust-main-sync.md). Four repros were added on both sides
(`repro_ai_refresh_wire_name`, `repro_ai_wallets_sync_bad_config`,
`repro_chat_ctrl_c_kills_daemon`, `repro_chat_cursor_never_resumes`); the
merge keeps r7-6's wording, which describes the fixed Python. The test
inventory and its counts table are regenerated, not hand-merged.

The merged schema has 113 tables. The shared desktop parity harness
(`tests/rust_desktop_parity.py`) and `tests/test_rust_ai_tools.py` asserted
112, so every desktop parity test failed on this base; both now compare the
manifest's table list (3d3c4f47f). Other parity files still assert 112 (see
[still open](#still-open)).

## Per bug

| # | Python fix | Behaviour | Rust before | Verdict | Port |
| --- | --- | --- | --- | --- | --- |
| 1 | 91b024f89 | A restore staged under a symlinked temp root (macOS `/var`) installs; a link at or below the staging directory is still refused | `crates/store/src/restore.rs` refused every such restore with `unsafe_restore_target`: `durable::refuse_symlinks` walked every lexical ancestor of each staged source | Differs | 967b9ab9f |
| 2 | c40e8fb32 | Chat's daemon child leads its own process group, so a terminal Ctrl-C cancels the turn and keeps the daemon | `crates/cli/src/chat/client.rs` already started the child in its own group | Matches | none |
| 3 | 7e3e9e286 | Codex, Claude and OpenCode chats resume their native session: the lookup uses the transcript before the new prompt, and a tool turn binds its cursor to every leg's text | `crates/ai/src/chat/turn.rs` looked up the whole request and bound a tool turn to its last leg | Differs | e665da2ce |
| 4 | 09912e4c0 | Wire-name reads (`ui_reports_summary`) refresh stale journals like catalog names | `crates/ai/src/executor.rs` compared the name as called | Differs | ef30e6073 |
| 5 | 7bca137d4 | An unreadable stored wallet configuration is a typed `invalid_wallet_config` refusal naming the wallet, in the planner and `wallets sync --all` | AI `ui.wallets.sync` answered `not_implemented` (direct-sync path); the desktop freshness kinds and `wallets sync --all` answered `internal_error` ("Invalid wallet configuration JSON") | Differs | 488741ab4 |
| 6 | a125129ed | A list or object `ui.maintenance.run` sync mode is the ordinary `validation` refusal | `crates/daemon/src/settings.rs` copied the old `internal_error` "unhashable type: 'list'" | Differs (the brief expected a match) | 29f44c2a0 |
| 7 | 0e42e2694 | Linked transfer groups on an owned destination are sorted, not in set order | `crates/journals/src/transaction_graph.rs` already sorted by `py_str` | Matches | none |
| 8 | 4898634d4 | A CSV or non-UTF-8 file under `json`, `btcpay_json` or `wasabi_bundle` is a `validation` refusal with a hint | The three loaders answered `internal_error` with the decoder's message | Differs | 04b6a8af3 |
| 9 | fd44cccb3 | Dashboard period aliases bucket like their canonical names; "all" measures history to `until` when given | `crates/transactions/src/desktop/dashboard.rs` bucketed every alias (`30d`, `3m`, `5y`) by month; it has no yearly buckets, so the `until` half has no counterpart yet | Differs (alias half) | bbfef77a7 |

The parity tests per bug, and the unported Rust's result on them:

1. `tests/test_rust_backup_install.py`: `test_staging_under_a_symlinked_temp_root_installs_in_both_runtimes`
   (probe, both runtimes, durable step inventory),
   `test_a_link_at_or_below_the_staging_root_refuses_alike` (link at the
   staging directory, a staged file and an attachment) and
   `test_cli_restore_staged_under_a_symlinked_tmpdir_matches_wire_bytes`
   (`backup import --install` under a symlinked `TMPDIR`, wire bytes and the
   restored database's full logical dump, plaintext and SQLCipher). Rust unit
   test: `restore::tests::a_link_above_the_staging_root_installs_and_one_at_or_below_refuses`.
   Unported Rust: the probe and the CLI refuse with `unsafe_restore_target` (Python installs); the at-or-below refusals already agreed.
2. `tests/test_rust_chat_interrupt.py` (Rust, three terminal scenarios) and
   `tests/rust_port/repro_chat_ctrl_c_kills_daemon.py` (Python: the child
   leads its own group, survives the Ctrl-C and answers the cancel). Both
   pass. Python's cancel only takes effect at the provider's next chunk, so a
   held answer stalls its turn where Rust abandons the read at once; that
   latency difference predates r7-6 and is not part of the fix. The Rust
   test passes on the unported Rust too.
3. `tests/test_rust_ai_chat.py::RustAiChatParityTest::test_broker_chat_tool_round_and_resumed_cursor`:
   a broker tool turn ("Checking ", a tool, "all good") in a saved session,
   then the next prompt (resumes `native-cursor-1`), a branch holding only the
   last leg (resumes nothing) and a plain follow-up (resumes
   `native-cursor-2`); all response lines, the chat tables and every broker
   request line. The test masks only fields main #672 to #714 changed that
   the AI slices have not ported (provider `agent_access`, a tool call's
   `approval`, the advertised tool count, the broker's tool list and
   instructions). Rust unit tests: `chat::turn::tests::the_next_prompt_resumes_the_branch_before_it`
   and the registry's `None` lookup. Unported Rust: the broker never receives `provider_session_id` `native-cursor-1` that Python sends.
4. `tests/test_rust_ai_tools.py::AiToolExecutorParityTest::test_wire_name_reads_refresh_stale_journals`
   (`ui.reports.summary`, `ui.overview.snapshot`, `ui.reports.balance_sheet`,
   each by wire and catalog name on a stale book; envelope, model bytes and
   every table) and `crates/ai/tests/fences.rs::wire_name_reads_refresh_like_catalog_names`.
   `ui.review.worklist` refreshes too but is left out: its badges still lack
   main #707's fields in Rust (slice S4). Unported Rust: every wire-name read differs (no `auto_journal_process`, journals left stale).
5. `tests/test_rust_freshness_product.py::test_product_unreadable_wallet_config_is_a_typed_refusal`
   (`ui.wallets.sync`, `ui.freshness.run`, `ui.workspace.freshness.run`;
   stored `bad`, `[]`, `null`, `"text"`; envelope and book bytes, no chain
   traffic), `tests/test_rust_wallet_sync_cli.py::test_sync_cli_unreadable_wallet_config_matches_python`
   (`wallets sync --all`) and `tests/test_rust_ai_tools.py::AiToolExecutorParityTest::test_unreadable_wallet_config_refuses_wallet_sync_typed`
   (AI `ui.wallets.sync` with no wallet, `wallet: "Bad"` and `all`). Rust
   unit test: `sync::tests::sync_service_preflight_rejects_malformed_inputs_and_node_routes_without_io`
   (the CLI selection and both freshness source plans). The AI refusal also
   needed the error's hint, which Python's `_tool_result_from_app_error`
   (main #679) adds; the port adds the hint to Rust's mutating executor,
   while that helper's remediation and allowlisted details stay with the AI
   slices. Unported Rust: the desktop kinds and `wallets sync --all` answer `internal_error`, the AI tool `not_implemented` (Python: `invalid_wallet_config`).
6. `tests/test_rust_maintenance_run.py::RustMaintenanceRunTest::test_unhashable_sync_is_the_typed_validation_refusal`
   (`[]`, `{"a": 1}`, `["never"]`; wire bytes and every table). The old
   test pinned Rust to Python's traceback-era `internal_error`. Unported Rust: `internal_error` "unhashable type: 'list'" (Python: `validation`).
7. `tests/test_rust_transaction_graph.py::RustTransactionGraphTest::test_several_linked_groups_keep_one_order_under_every_hash_seed`
   (an owned destination with five linked groups, `PYTHONHASHSEED` 0 to 3,
   one order on both sides) and `tests/rust_port/repro_graph_group_order_hashseed.py`.
   The merged book's open-time migration resets the built journal semantics,
   so the test stamps them again; without that the projection is stale and
   the graph carries no group (the older `test_current_custody_projection_annotates_the_graph`
   is vacuous on this base for the same reason). Passes on the unported Rust too.
8. `tests/test_rust_wallet_import_desktop.py`: `test_non_json_files_under_json_formats_are_validation_refusals`
   (a CSV export, a Latin-1 file and a BOM-prefixed file under each of the
   three formats) and `test_sqlcipher_non_json_files_are_validation_refusals`.
   Rust unit test: `file_import::tests::json_formats_refuse_a_non_json_file_as_validation`,
   which also pins that nesting past Python's recursion limit stays
   `internal_error` (a `RecursionError` is not a `ValueError`) and that a
   missing file keeps the coordinator's `not_found`. Unported Rust: `internal_error` with the decoder's message, plaintext and SQLCipher.
9. `tests/test_rust_transaction_dashboard.py::RustTransactionDashboardTest::test_period_aliases_bucket_like_their_canonical_names`
   (eleven aliases over two windows) and the unit test
   `period_tests::every_period_spelling_buckets_like_its_canonical_name`.
   The test drops `bucketSize` and `history.years`, main #701's fields that
   S8 to S10 port. Unported Rust: monthly buckets (`2025-01` for `30d`, where Python has `2025-01-01`).

## Dashboard merge with S8 to S10

S8 to S10 (c62d918c4, `t3code/rust-sync-s8-s10`) rewrite the bucketing in
`crates/transactions/src/desktop/dashboard.rs`: `bucket` and `next` take a
size, `bucket_size` decides it, and `history_years` measures the history.
This branch's dashboard commit (bbfef77a7) touches only these lines, all of
which S8 to S10 also change, so expect conflicts there:

- `fn canonical_period` and its `period_tests` module, added before
  `fn empty_stats` (no S8 to S10 hunk);
- the four calls `bucket(start, ...)`, `bucket(cursor, ...)`,
  `next(cursor, ...)` and `bucket(date, ...)`, which pass
  `canonical_period(&period)`.

Resolve by taking S8 to S10's shape and moving the mapping into
`bucket_size` (Python sizes by the canonical period there), adding the
`4years`, `4year` and `4y` aliases. S8 to S10 also measure the "all"
cutover with `history_years(earliest, now_us(conn)?)`; r7-6 measures to
`until` when the request gives one, so that call must become
`history_years(earliest, if until.is_empty() { now_us(conn)? } else { parse(&until, "until")? })`.
The mutation entries `dashboard-alias-canonical`
(`crates/transactions/scripts/mutations.json`) and `dashboard-alias-buckets`
(`crates/daemon/scripts/desktop_projection_mutations.json`) name the lines
to re-point after the merge.

## Python reference bugs found here

Recorded with repros, not fixed:

- A tool turn's saved chat history message is only its last leg's text, not
  the whole answer the user saw
  (`tests/rust_port/repro_chat_history_tool_turn_last_leg.py`). r7-6 binds
  the provider cursor to the shown text, but the terminal record and the
  persisted message still use the last leg's `content`. Rust does the same
  (`turn.rs` passes the last leg's `content` to `terminal`), and the chat
  parity test compares the saved message byte for byte, so the two agree.
- `wallets sync --wallet X` with an unreadable configuration still crashes
  before planning with the decoder's error
  (`tests/rust_port/repro_cli_wallet_sync_bad_config_explicit.py`); r7-6 only
  guarded `--all`. Rust also answers `internal_error` there, with its own
  message, so the two messages differ; the CLI parity test compares `--all`.
- Observed while testing bug 3: a new history session's first turn has no
  session id yet, so neither runtime keeps that turn's provider cursor.

## Mutation checks

Twenty entries, each run in a scratch copy of the tree with the shared
`target/scratch-build`; every one was killed by its named test:

| Catalog | Entries | Killed by |
| --- | --- | --- |
| `crates/store/scripts/mutations.json` | `restore-staging-root-resolved` | `a_link_above_the_staging_root_installs_and_one_at_or_below_refuses` |
| `crates/ai/scripts/mutations.json` | `A-chat-resume-before-prompt`, `A-chat-resume-needs-a-prompt`, `R67-cursor-branch` (re-pointed) | `the_next_prompt_resumes_the_branch_before_it`, `provider_resume_cursor_is_bound_to_the_visible_chat_branch` |
| | `wire-chat-cursor-binds-every-leg`, `wire-chat-stream-resume-lookup` | `test_broker_chat_tool_round_and_resumed_cursor` |
| | `A-refresh-wire-name` | `wire_name_reads_refresh_like_catalog_names` |
| | `wire-ai-wallet-sync-invalid-config`, `wire-ai-write-refusal-hint` | `test_unreadable_wallet_config_refuses_wallet_sync_typed` |
| `crates/wallets/scripts/mutations.json` | `invalid-wallet-config-code`, `invalid-wallet-config-planner`, `invalid-wallet-config-cli-all` | `sync_service_preflight_rejects_malformed_inputs_and_node_routes_without_io` |
| | `json-import-decode-refusal`, `json-import-parse-refusal`, `json-import-recursion-stays-internal`, `json-import-btcpay-reader` | `json_formats_refuse_a_non_json_file_as_validation` |
| | `W-wasabi-file-open` (re-pointed) | `wasabi_missing_file_refusal_is_atomic` |
| `crates/daemon/scripts/desktop_projection_mutations.json` | `maintenance-unhashable-sync` | `test_unhashable_sync_is_the_typed_validation_refusal` |
| | `dashboard-alias-buckets` | `test_period_aliases_bucket_like_their_canonical_names` |
| `crates/transactions/scripts/mutations.json` | `dashboard-alias-canonical` | `every_period_spelling_buckets_like_its_canonical_name` |

`W-btcpay-json-newlines` survived and was removed: with the typed refusal a
raw CR in a JSON file is whitespace or an invalid string character under
either newline handling, and the decoder's position no longer reaches the
message, so the mutant is equivalent. The projection checker now builds in
the shared `target/scratch-build` instead of a cold cache per run.

## Verification

- Rust: `cargo test` of `kassiber-ai`, `kassiber-store` (74, fsync-bound on
  the build disk) and `kassiber-transactions` pass; `kassiber-wallets`
  `file_import` (114) and the new `sync::tests` case pass. The checked-in
  import, BTCPay and Wasabi vectors were regenerated from the merged Python
  (1c840bafd): they had frozen the pre-r7-6 `internal_error` answers.
- Parity: every test named [per bug](#per-bug) passes against this branch's
  harness build, and every "differs" test fails against the unported merge
  4dfdcdd7e (31 failed, 6 passed, the passes being the "matches" tests).
  Timeouts seen under machine load (average 40 to 80) passed when rerun alone.
- Every r7-6 repro exits 0 on the merged Python; the two new repros exit 0
  (bug reproduced).
- `kassiber-wallets` `sync::tests::sync_service_composes_all_102_python_publication_rounds`
  fails on the merged base and on this branch alike: its vectors predate S1's
  `ai_provider_checks` table.

## Still open

- Merge S8 to S10 with the dashboard resolution above.
- Replace the remaining hard-coded 112-table assertions in other Rust parity
  tests with the manifest's table list.
- Port main #679's `_tool_result_from_app_error` remediation and allowlisted
  details to the AI executors (this branch adds only the hint).
- `agent_access::tests::test_ending_the_desktop_session_ends_the_agent_lease`
  (`kassiber-daemon`) fails on this branch and on the unported merge alike:
  the fake broker's last command is `unlock`, not `lock`. It predates r7-6.
- Regenerate `crates/wallets/tests/vectors/python_wallet_publication.json`
  for S1's `ai_provider_checks` table.
- `W-blockpit-delimiter` and `W-cointracking-delimiter` in
  `crates/wallets/scripts/mutations.json` no longer apply (their `find` text
  is gone from `platform_csv.rs` on the base).
