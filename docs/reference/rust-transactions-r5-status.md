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

## Metadata continuation

The continuation branch is `t3code/r5-transactions-2`. The requested branch was
created at `7b7e720d`, then fast-forwarded to the actual predecessor tip
`a0766f34` (the attachment/loan port and this acceptance record). The predecessor
branch checked out elsewhere was not modified. All continuation commits are
local; nothing is pushed or published.

`crates/transactions/src/metadata.rs` adds 18 CLI leaves:

- `metadata notes set/clear`;
- `metadata tags create/list/add/remove`;
- `metadata exclude/include`;
- `metadata records get`, `note set/clear`, `tag add/remove`,
  `excluded set/clear`, `kind set/clear`, and `history stale`.

`ui.activity.stale` is the existing desktop kind added here. CLI and daemon
use the same typed book service. Other metadata daemon kinds remain
`not_implemented`; in particular, the grouped metadata update kind is not
advertised with incomplete pricing/review behavior. AI projections stay Python.

Edits preserve Python's exact stored before/after/diff JSON, source/reason,
processed-version snapshots and version increments. Empty notes and null notes
remain distinct. Real edits append events and fields; no-ops append nothing.
A savepoint makes tags, transaction updates, audit rows and invalidation atomic.
The service never updates or deletes historical audit rows. History reverts
remain unported rather than rewriting history. Exact transaction IDs win over
external-ID aliases; ambiguous or cross-profile references fail. Tag editing
uses normalized **labels**, including Python's existing code/label collision
behavior. Exclusion checks active anchored custody legs before changing a row.
Kind overrides validate direction and preserve the importer's `kind`.

The shared `kassiber-pytext::lower` uses the existing Unicode 14 cased and
case-ignorable ranges; frozen CPython 3.11 vectors pin every scalar lowercase
mapping, contextual sigma, dotted I and post-Unicode-14 characters. No dependency,
schema, network path, or AI authority is added. The seven initial metadata Rust
tests plus two mutation-driven edge regressions cover this boundary; no
journal/tax/history-composition `rust-test` disposition is claimed complete.

Continuation verification (Rust 1.99.1, locked Python 3.11):

| Check | Result |
| --- | --- |
| Workspace/package fmt; workspace and test-harness clippy, `-D warnings` | Passed |
| `cargo test --workspace --exclude kassiber-net --locked` | 471 passed (includes initial 7 metadata tests and lowercase vectors) |
| Final metadata Rust tests after two added edge regressions | 9 passed |
| Test-harness build | Passed |
| Hybrid metadata boundary, edit-history module and existing metadata mutation regression | 16 passed, 191 deselected |
| Predecessor attachment/loan parity module | 4 passed |
| Requested shell/hybrid/CLI-runner/port modules | 53 passed, 68 subtests passed |
| Inventory, contracts, lowercase fixture and mutation-catalog checks | Passed |
| Transaction service mutation catalog | 16/16 killed; 13 distinct named tests passed unmutated |

The first sandboxed parity attempt failed because daemon children could not
write the fixed account ownership-lock namespace. It also caught and fixed a
Rust-only empty-note parsing mismatch. The targeted parity run was repeated
outside the filesystem sandbox, using disposable books, and passed. Full Python
shards remain the orchestrator's verification responsibility.
The predecessor parity rerun initially encountered transient temporary-storage
exhaustion (`ENOSPC`); the same four tests passed once storage recovered.

`KASSIBER_HYBRID_LOG` from the successful 16-test run records the counts below.
Stable read/error CLI envelopes and the daemon stale summary compare as raw
bytes. A separate pair of disposable books compares every stored history value,
diff, version snapshot and tag effect against Python; generated UUID/time fields
are excluded only from mutation response comparison. Direct byte-comparison
subprocess calls are additional evidence, not hybrid-log counts.

| Command | Rust | Python |
| --- | ---: | ---: |
| `metadata.exclude` | 2 | 0 |
| `metadata.include` | 2 | 0 |
| `metadata.notes.clear` | 1 | 0 |
| `metadata.notes.set` | 3 | 0 |
| `metadata.records.excluded.clear` | 1 | 0 |
| `metadata.records.excluded.set` | 2 | 0 |
| `metadata.records.get` | 2 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.kind.clear` | 3 | 0 |
| `metadata.records.kind.set` | 3 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.note.set` | 5 | 0 |
| `metadata.records.tag.add` | 2 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.create` | 25 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.remove` | 2 | 0 |
| `daemon` (stale-edit scenario) | 1 | 0 |

There are zero Python fallbacks for these 18 implemented metadata commands.
The whole metadata family is **not** accepted yet: the log also records two
Python `metadata.records.history.list` calls and one Python
`metadata.records.list` call. These are explicit unported dependencies in the
existing scenarios, along with setup and journal/report operations.

| New mutation | Named Rust test |
| --- | --- |
| `metadata-id-precedence` | `resolver_prefers_id_and_refuses_duplicate_external_ids_and_cross_profile` |
| `metadata-ambiguous-reference` | `resolver_prefers_id_and_refuses_duplicate_external_ids_and_cross_profile` |
| `metadata-profile-scope` | `resolver_prefers_id_and_refuses_duplicate_external_ids_and_cross_profile` |
| `metadata-tag-identity` | `tag_label_reuses_an_existing_profile_code` |
| `metadata-tag-noop` | `duplicate_case_labels_already_attached_are_a_canonical_noop` |
| `metadata-note-noop` | `notes_append_exact_json_and_noops_do_not_invalidate` |
| `metadata-custody-exclusion` | `exclusion_refuses_an_active_anchored_custody_leg` |
| `metadata-kind-direction` | `kind_requires_direction_and_preserves_observed_kind` |
| `metadata-stale-version` | `stale_counts_use_version_after_and_preserve_processed_events` |

An initial mutation run exposed a surviving tag-identity mutation and a
non-compiling duplicate-label mutation. The added edge regressions kill the
first, and the revised second mutation builds and is killed. Neither initial
result is counted as a kill. All seven predecessor mutations were rerun.

## Remaining work

This boundary does **not** complete the requested five-family service slice:

- `metadata` (8 remaining CLI leaves): `bip329 import`, `preview`, `list`,
  `export`; `records list`; `records history list`, `activity`, `revert`.
  BIP329 still needs byte-exact ordering, escaping and duplicate merge rules,
  ownership-index matching, and all export modes. History needs scoped/filter-bound
  pagination, redacted projections and forward-edit reverts across all fields;
- remaining metadata daemon kinds: `ui.metadata.bip329.preview/import/export`,
  `ui.transactions.metadata.update`, `ui.transactions.history`,
  `ui.transactions.history.revert`, `ui.activity.list`;
- `transactions list` and `transactions export`, and desktop kinds
  `ui.transactions.list/dashboard/extremes/search/resolve/graph/review_context`,
  `ui.transactions.ledger_template`, and the three export kinds listed below;
- `documents list/create/attach/import-report`, daemon
  `ui.documents.list/create/attach/import_report` and
  `ui.transactions.commercial_context`. Prior-report import is local evidence
  preservation, not authority for ledger totals; it remains unported with the
  document family, not because it generates reports or contacts the network;
- journal processing and reports, network-dependent work, and all other
  unregistered command/kind families remain `not_implemented` in Rust;
- the five `LoanTaxClassificationTest` Rust dispositions depend on finalized
  journal/tax composition and remain with that unported work. Hybrid end-to-end
  loan scenarios exercise Python's journal processor after Rust authors marks.

The CLI catalog advertises only implemented leaves. No new schema, arbitrary
filesystem/shell dispatch, spending capability or external I/O is introduced.
The full Python shards are left to the orchestrator's external verification.

Explicitly excluded report dependencies in these families are
`transactions export` (CSV, XLSX and platform CSV),
`ui.transactions.export_csv`, `ui.transactions.export_xlsx`, and
`ui.transactions.export_platform_csv`. Journal processing (`journals process`)
and all `reports` commands/kinds stay unimplemented. Adjacent network-dependent
commercial work (`btcpay provenance sync`, `ui.btcpay.provenance.sync`) also
stays unimplemented. None is invoked by these metadata services.

## Python reference correction

The separate reference-fix commit `7b7e720d` makes `ui.attachments.open` enforce the
same managed-root boundary as verify/remove. Its regression uses a disposable
book and tests parent and symlink escapes. The frozen error/kind inventories
are regenerated because their source locations change. No other Python
product behavior is changed.
The metadata continuation found no additional Python reference bug and changes
no Python product source.
