# R5 local transaction evidence services

This is a partial boundary of [plan 21](../plan/21-rust-backend.md), based on
`82f6b77b`. Python remains the reference. The branch is
`t3code/r5-transactions`; commits are local only.

## Implemented boundary

`crates/transactions` owns loan marks and managed attachment evidence.
Each family registers through its own module in `crates/cli` and
`crates/daemon`. The runtime book trait exposes typed family requests;
`crates/kassiber/src/real_store.rs` supplies the admitted store connection,
resolved book scope and attachment root from the existing paths resolver.
Attachment wallet labels pass through the single `attachment_rows` projection
function; no wallet configuration or ownership inference enters the service.

CLI commands: `loans mark`, `loans unmark`, `loans link`, `loans list`,
`attachments add`, `attachments list`, `attachments rename`,
`attachments remove`, `attachments verify`, `attachments gc`.

Daemon kinds: `ui.loans.list`, `ui.loans.mark`, `ui.loans.link`,
`ui.loans.unmark`, `ui.attachments.list`, `ui.attachments.add`,
`ui.attachments.copy`, `ui.attachments.rename`, `ui.attachments.remove`,
`ui.attachments.open`. These are the existing desktop/AI request kinds;
there is no new generic dispatch or AI authority. The daemon retains unlock,
project ownership, expected-scope checks and rollback before/after routing.

Loan role validation, request-order loan-ID reuse, oldest-first per-asset
collateral hints, soft retirement and journal-input invalidation follow Python.
Observed transactions and their integer msat amounts are unchanged.
The ten storage tests in Python's `CollateralMarkTest` are represented by
seven Rust integration tests, including failed-invalidation rollback.

Files retain Python's `<profile-id>/<attachment-id>-<sanitized-filename>`
layout, UUID identities, SHA-256 and size metadata, and ordinary file/directory
creation modes under the caller's umask. Evidence reuse copies bytes into
a fresh managed file and records both source identities. A corrupt source
rolls back the batch's rows and created files. Verify/remove/open resolve
managed paths before accessing them. URL attachments remain local literal
references; no network client, fetch or index is introduced. Local MIME
inference uses the frozen CPython 3.11 built-in table and Unix mime.types
overrides; Windows registry MIME overrides still require platform parity.

## Verification and evidence

All data roots,
HOME/XDG inputs and mutation workspaces are disposable. The harness binary is
built after workspace tests, which otherwise replace `target/debug/kassiber`
with a product build that correctly refuses the Python no-egress test guard.

- Workspace formatting, workspace clippy and test-harness clippy use
  `-D warnings` and the pinned Rust 1.99.1 toolchain.
- `cargo test --workspace --exclude kassiber-net --locked` verifies the workspace.
- `cargo build -p kassiber --features test-harness --locked` supplies the candidate.
- Existing loan end-to-end CLI scenarios and attachment regressions use the
  hybrid candidate. Journal/report setup and processing fall back to Python.
- `tests/test_rust_transaction_services.py` compares stable CLI reads and
  daemon reads/errors byte for byte on the same book. It exercises all ten
  CLI leaves through the hybrid candidate and checks unported families fail
  before creating a book.
- `test_daemon_transaction_attachments_round_trip` explicitly opts into the
  Rust daemon, retaining Python fallback for setup commands.
- The requested shell-parity, hybrid-candidate, CLI-runner and port-contract
  modules run with `KASSIBER_RUST_BIN` set to the test-harness binary.
- Inventory, contract and MIME fixture checks run against the locked Python 3.11.

Final results (Rust 1.99.1, locked Python 3.11):

| Check | Result |
| --- | --- |
| Workspace/package fmt and both clippy configurations (`-D warnings`) | Passed |
| Workspace tests, excluding `kassiber-net` | 463 passed; includes 15 domain tests (8 attachment, 7 loan) |
| Test-harness binary build | Passed |
| Hybrid slice scenarios, including existing CLI attachment lifecycle | 18 passed, 187 deselected, 2 subtests passed |
| Existing Rust-daemon attachment round trip | 1 passed |
| Requested shell/hybrid/CLI-runner/port-contract modules | 53 passed, 50 subtests passed |
| Inventory, contract and frozen MIME checks | Passed |
| Mutation catalog | 7/7 killed; six distinct unmutated named tests passed |

The hybrid slice and daemon runs produce these served-by counts. They include
the existing loan CLI end-to-end scenarios, attachment regressions, the six
shared CLI lifecycle/setup methods, the four new boundary scenarios, the
Python open-containment regression, and the existing daemon attachment round
trip. Direct byte-comparison subprocesses are additional checks and are not
counted as hybrid invocations. No attachment or loan CLI call falls back to
Python. The daemon log records the session; its individual kinds are exercised
inside that Rust process.

| Command | Rust | Python |
| --- | ---: | ---: |
| `attachments.add` | 9 | 0 |
| `attachments.list` | 3 | 0 |
| `attachments.rename` | 4 | 0 |
| `attachments.remove` | 3 | 0 |
| `attachments.verify` | 3 | 0 |
| `attachments.gc` | 3 | 0 |
| `loans.mark` | 7 | 0 |
| `loans.unmark` | 2 | 0 |
| `loans.link` | 2 | 0 |
| `loans.list` | 2 | 0 |
| `daemon` (attachment scenario) | 1 | 0 |

Setup commands for wallets/profiles/imports, metadata record reads and
journal/report processing use the reference fallback, as intended.

The mutation catalog and scratch-tree runner live in
`crates/transactions/scripts/`, following R2. Every entry builds and is killed
by a named test; compiler errors do not count as kills.

| Mutation | Named Rust test |
| --- | --- |
| loan-role-direction | `all_roles_require_the_exact_direction_and_refuse_channel_roles` |
| loan-link-first-requested | `linking_preserves_request_order_and_requires_two_distinct_active_marks` |
| loan-lock-oldest-first | `open_locks_match_oldest_first_per_asset_and_ignore_principal` |
| loan-principal-is-not-collateral | `open_locks_match_oldest_first_per_asset_and_ignore_principal` |
| attachment-root-containment | `traversal_verify_remove_and_open_never_touch_external_file` |
| attachment-copy-identity | `copy_creates_independent_file_identity_and_preserves_provenance` |
| attachment-copy-hash | `corrupt_copy_rolls_back_prior_rows_and_files` |

## Remaining work

This boundary does **not** complete the requested five-family service slice:

- `transactions` (2 CLI commands), including desktop/AI list/detail projections;
- `metadata` (26), including labels, tags, notes, BIP329 merge and byte-exact
  import/export, edit history and forward-only reverts;
- `documents` (4), including commercial evidence creation/attachment/matching;
- journal processing and reports, network-dependent work, and all other
  unregistered command/kind families remain `not_implemented` in Rust;
- the five `LoanTaxClassificationTest` Rust dispositions depend on finalized
  journal/tax composition and remain with that unported work. Hybrid end-to-end
  loan scenarios exercise Python's journal processor after Rust authors marks.

The CLI catalog advertises only implemented leaves. No new schema, arbitrary
filesystem/shell dispatch, spending capability or external I/O is introduced.
The full Python shards are left to the orchestrator's external verification.

## Python reference correction

The separate reference-fix commit `7b7e720d` makes `ui.attachments.open` enforce the
same managed-root boundary as verify/remove. Its regression uses a disposable
book and tests parent and symlink escapes. The frozen error/kind inventories
are regenerated because their source locations change. No other Python
product behavior is changed.
