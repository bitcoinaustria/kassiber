# R5 local transaction evidence services

This is a partial boundary of [plan 21](../plan/21-rust-backend.md), based on
`82f6b77b`. Python remains the reference. The branch is
`t3code/r5-transactions`; commits are local only.

The latest continuation is `t3code/r5-transactions-6`; its
[observed-list acceptance](#observed-cli-list-continuation) and remaining work
are recorded at the end of this page. Earlier continuation results are retained
as historical evidence.

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

- `metadata bip329 import/preview/list/export` now have a partial local-index
  implementation; see the BIP329 continuation below. Descriptor-backed and
  complex-input matching still need the complete ownership-index port;
- remaining metadata daemon kind: `ui.transactions.metadata.update`. The three
  `ui.metadata.bip329.preview/import/export` kinds use the partial service below.
  The existing activity kind is
  `ui.activity.history`, now ported; `ui.activity.list` was a documentation error;
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

## Record/history continuation

`t3code/r5-transactions-3` starts at the requested `185c3707`, without changing
`t3code/r5-transactions-2` in its other worktree. This run stops at a verified
partial boundary: four of the eight requested remaining metadata leaves.
Transactions and documents have not been ported in this run. No push or PR.

New CLI leaves: `metadata records list`, `metadata records history list`,
`activity`, and `revert`. New existing desktop kinds: `ui.transactions.history`,
`ui.activity.history`, and `ui.transactions.history.revert`. These use the same
typed service through the admitted book connection. AI projections stay Python;
no schema, dependency, egress path or generic dispatch is added.

Record pages preserve all three descending tie-breakers, exact msat values,
notes/exclusion/tag/wallet/date filters, Python cursor bytes and scope/filter
binding. Wallet selectors prefer IDs and refuse ambiguous labels. History
selects whole events, retaining their complete field set after field/family
filtering. Values and diffs use the existing secret redactor; labels, summaries,
source names and report anchors match Python. History cursors include their own
timestamp/ID order and full filters; `include_stale=false` skips the summary.
Frozen CPython 3.11 vectors pin 40,665 timestamp inputs and 2,009 `.12g` legacy
price values. Run `crates/transactions/scripts/gen_metadata_vectors.py --check`.

Reverts restore all 21 supported fields, including exact price strings and
legacy float columns, through one new forward audit event. Tags retain label
normalization and identity reuse. Original events and field rows are never
rewritten or deleted. Repeated reverts and timestamp-only pricing changes are
no-ops. Grouped fields, tags, the new audit rows and journal invalidation share
one savepoint and roll back together. GUI reverts default to source `gui`.

Reference fix `df73a011` applies the active-custody exclusion guard to reverts.
The new `test_revert_cannot_exclude_a_newly_reviewed_custody_leg` failed against
the unmodified Python reference, then passed. It asserts the transaction,
history count and profile version remain unchanged on refusal. Rust has the
same regression and a killed mutation. This is the only new Python product
change; frozen error-source locations and the test inventory accompany it.

Final verification uses the pinned Rust 1.99.1 and locked Python 3.11.
All book roots and mutation trees are disposable; HOME is never redirected
where tools are installed. The final harness build follows workspace tests.

| Check | Result |
| --- | --- |
| Workspace fmt and both clippy configurations, `-D warnings` | Passed |
| `cargo test --workspace --exclude kassiber-net --locked --offline` | 481 passed |
| Final transaction domain tests | 32 passed: 16 metadata, 8 attachment, 7 loan, 1 CPython-vector test |
| `cargo build -p kassiber --features test-harness --locked --offline` | Passed |
| Hybrid metadata/history and inherited transaction-service modules, plus two existing metadata review regressions | 25 passed |
| Requested shell/hybrid/CLI-runner/port-contract modules | 53 passed, 72 subtests passed |
| Frozen metadata vectors and mutation catalog applicability | Passed |
| Inventory and contract checks (`tests/rust_port`) | 10 passed |
| Transaction service mutation catalog, isolated target | 30/30 built and killed; 19 distinct named tests passed unmutated |

The first exploratory boundary run caught a missing central daemon registration;
the successful final run includes that registration. Invalid numeric daemon
limits that raise untyped Python exceptions retain Rust's existing no-traceback
difference; successful limits and typed validation failures compare as raw bytes.
Stable CLI reads/errors and daemon history/stale reads/errors compare raw bytes.
Grouped CLI and GUI revert mutation responses omit only their generated audit
UUID from comparison; stored field JSON, source/reason and version snapshots
compare independently. The final counts below exclude exploratory runs and
exclude direct byte-comparison subprocesses.

An intermediate mutation attempt and ordinary domain rerun incorrectly shared
one Cargo target with earlier scratch-tree mutations, reusing a mutated artifact
and failing the custody-guard baseline. Those failures are not acceptance
evidence. Cleaning the affected package restored the passing domain baseline;
the complete 30-entry mutation run passed with a separate disposable
`CARGO_TARGET_DIR` and no concurrent build sharing it. Final workspace checks
and the harness build use the ordinary target after that cleanup.

The successful slice modules and two existing review regressions record 85 Rust
metadata calls and zero Python metadata fallbacks. The predecessor's two
history-list fallbacks and record-list fallback now run in Rust. The daemon
count represents three Rust sessions; history/stale reads, validation errors
and a successful GUI revert are compared inside those sessions.

| Command | Rust | Python |
| --- | ---: | ---: |
| `metadata.exclude` | 2 | 0 |
| `metadata.include` | 2 | 0 |
| `metadata.notes.clear` | 1 | 0 |
| `metadata.notes.set` | 7 | 0 |
| `metadata.records.excluded.clear` | 1 | 0 |
| `metadata.records.excluded.set` | 2 | 0 |
| `metadata.records.get` | 2 | 0 |
| `metadata.records.history.activity` | 3 | 0 |
| `metadata.records.history.list` | 4 | 0 |
| `metadata.records.history.revert` | 5 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.kind.clear` | 3 | 0 |
| `metadata.records.kind.set` | 3 | 0 |
| `metadata.records.list` | 6 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.note.set` | 5 | 0 |
| `metadata.records.tag.add` | 2 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.create` | 27 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.remove` | 2 | 0 |
| `daemon` (metadata sessions) | 3 | 0 |

Attachments and loans in the inherited boundary module also have zero Python
fallbacks. Unported workspace/profile/wallet setup and journal processing use
Python on the same disposable books.

No new `rust-test` disposition for journal/report composition is claimed. The
BIP329, transaction and document scenarios are not accepted by this partial run.
Full Python shards and native platform checks remain with the orchestrator.

| New mutation | Named Rust test |
| --- | --- |
| `metadata-record-page-order` | `record_pages_bind_filters_and_keep_all_three_descending_tiebreakers` |
| `metadata-record-page-boundary` | `record_pages_bind_filters_and_keep_all_three_descending_tiebreakers` |
| `metadata-cursor-filter-binding` | `record_pages_bind_filters_and_keep_all_three_descending_tiebreakers` |
| `metadata-history-page-order` | `history_pages_filter_events_preserve_all_fields_and_redact_values` |
| `metadata-history-page-boundary` | `history_pages_filter_events_preserve_all_fields_and_redact_values` |
| `metadata-history-field-selection` | `history_pages_filter_events_preserve_all_fields_and_redact_values` |
| `metadata-wallet-reference-ambiguity` | `wallet_filters_prefer_ids_and_refuse_ambiguous_labels` |
| `metadata-revert-before-state` | `reverts_append_one_forward_event_and_repeating_is_a_noop` |
| `metadata-revert-field-selection` | `reverts_append_one_forward_event_and_repeating_is_a_noop` |
| `metadata-revert-forward-append` | `reverts_append_one_forward_event_and_repeating_is_a_noop` |
| `metadata-revert-grouped-source` | `grouped_revert_restores_tax_pricing_tags_and_rolls_back_invalidation_failure` |
| `metadata-revert-custody-exclusion` | `revert_cannot_exclude_a_newly_reviewed_custody_leg` |
| `metadata-record-tag-scope` | `record_pages_bind_filters_and_keep_all_three_descending_tiebreakers` |
| `metadata-history-stale-selection` | `history_pages_filter_events_preserve_all_fields_and_redact_values` |

## Combined R5 custody corpus alignment

The integration base contains a newer custody timestamp corpus than this slice
used in isolation. Regenerating metadata vectors retains every previous input
and outcome and adds 34,989 timestamp inputs. The metadata parser's existing
copy of the component calendar parser now carries the custody base's ISO week
day disambiguation and timezone-adjacent separator/fraction rules. Metadata's
validation messages, empty-value handling, UTC formatting and range errors stay
at their own boundary. The existing native vector test passes all 40,665
CPython timestamp inputs and the unchanged 2,009 legacy float vectors.

## BIP329 continuation

`t3code/r5-transactions-4` starts at the requested `f417d50a`, without touching
`t3code/r5-transactions-3` in its other worktree. This run stops at a clean,
verified **partial BIP329 boundary**. It does not complete the full requested
service slice. All commits are local; nothing is pushed or published.

Four CLI leaves are registered: `metadata bip329 preview/import/list/export`.
Existing desktop kinds `ui.metadata.bip329.preview/import/export` use the same
typed metadata service. AI projections remain Python. There is no new schema,
dependency, spending capability, external service, or generic dispatch.

The local matching boundary covers profile-scoped transaction external IDs,
address lists (including retired address policies), cached output inventory,
and exact stored Core receive outpoints whose address agrees exclusively with
the row wallet. Two transaction rows in one wallet remain ambiguous. Matching
never establishes ownership, custody or tax facts. By default ambiguous labels
are preserved without projecting tags; `--apply-ambiguous` explicitly opts in.
Only the final duplicate label projects a tag. Labels, tags, audit rows and
journal invalidation roll back together. Existing audit rows remain untouched.

Duplicate imports retain non-null authored standard fields, merge unknown
fields, preserve nested Kassiber data and replace its match annotation. Unknown
JSON keys and values retain all code points, including escaped lone surrogates.
Conflict comparison follows Python numeric/dict equality. List cursors retain
Python bytes, descending timestamp/ID ties and scope binding. Stored,
synthesized and combined exports retain Python record and key order, escaping,
float spelling, integer sat conversion, trailing newline, wallet order and
stored-object precedence. Desktop exports use the resolved managed reports
root and existing timestamp/collision naming policy.

**Explicit unaccepted cases:** active or retired descriptor/xpub derivation
causes preview/import and wallet-scoped stored/all export to return
`not_implemented` before changing labels or an output file. Input labels whose
stored observation contains complex transaction graphs also refuse. A complete
observation/descriptor ownership index is required before these cases can be
accepted. Standard BIP329 string fields containing lone surrogates also refuse;
unknown JSON fields preserve them. Global stored export and synthesized export
work without that index. Native Windows newline/MIME/platform acceptance is not
claimed by this Linux run. These barriers are explicit Rust refusals, never
Python fallback or an incomplete guessed match.

The new differential module compares CLI/daemon reads and typed errors as raw
bytes, compares every exported file byte for all three modes with/without wallet
scope, and compares stored label data, audit values and version transitions on
two disposable books. Only generated desktop export paths/filenames are removed
from that successful export envelope comparison; the actual file bytes are
compared separately. Seven inherited BIP329 CLI scenarios also pass. New Rust
regressions cover duplicate merges, conflicts, matching, export/list ordering,
rollback and unported-case refusals. No Python product bug was found or fixed.

Verification uses Rust 1.99.1 through mise, cargo `--locked --offline`, and the
locked uv Python 3.11 environment with `MATURIN_NO_INSTALL_RUST=1` for setup.
All book roots, mutation targets and the task cache are disposable; HOME is
never redirected to an installation root. The final harness build follows the
workspace tests.

| Check | Result |
| --- | --- |
| Workspace fmt | Passed |
| Workspace clippy and `-p kassiber --features test-harness`, `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 508 passed |
| Transaction domain suite | 41 passed: 16 metadata, 8 BIP329 integration, 8 attachment, 7 loan, 2 unit/vector tests |
| Harness build | Passed |
| Final hybrid slice scenarios (new BIP329, inherited metadata/evidence, seven BIP329 regressions) | 21 passed, 185 deselected |
| Requested shell/hybrid/CLI-runner/port-contract modules | 53 passed, 76 subtests passed |
| Inventory, contract and shard validation | Passed |
| Transaction mutation catalog | 50/50 built and killed; 20 new entries, 26 distinct named unmutated tests passed |

The first sandboxed workspace attempt failed four loopback socket tests in
`kassiber-net`. The full run then passed outside that sandbox. Exploratory
parity caught a Rust-only capitalization difference in a daemon validation hint;
it was corrected before the final run. The shell catalog/kind projections and
one new scenario's SQLite-row assertion were also updated. An overlapping
workspace rebuild replaced the harness during an intermediate hybrid run,
invalidating that run; acceptance counts come only from the subsequent isolated
run after rebuilding the harness. No auto-review rejection occurred.

<!-- bip329-served-counts:start -->
| Command | Rust | Python |
| --- | ---: | ---: |
| `daemon` | 4 | 0 |
| `metadata.bip329.export` | 12 | 0 |
| `metadata.bip329.import` | 17 | 0 |
| `metadata.bip329.list` | 9 | 0 |
| `metadata.bip329.preview` | 10 | 0 |
<!-- bip329-served-counts:end -->

All four BIP329 commands have zero Python fallbacks in these hybrid scenarios.
Direct byte-comparison subprocess calls are additional evidence and are not
included in hybrid-log counts. Workspace/profile/wallet setup and journal/report
processing continue using Python. Daemon kind invocations execute within the
logged Rust daemon session. The new BIP329 session contains the following
candidate requests (successful calls and typed validation errors); reference
comparison sessions are separate and excluded from these counts.

| Daemon kind | Rust requests | Python fallbacks |
| --- | ---: | ---: |
| `ui.metadata.bip329.preview` | 3 | 0 |
| `ui.metadata.bip329.import` | 2 | 0 |
| `ui.metadata.bip329.export` | 4 | 0 |

| New mutation | Named Rust test |
| --- | --- |
| `bip329-last-duplicate-tag` | `duplicate_merge_keeps_null_fields_and_nested_kassiber_data_and_projects_only_last_tag` |
| `bip329-null-keeps-stored` | `duplicate_merge_keeps_null_fields_and_nested_kassiber_data_and_projects_only_last_tag` |
| `bip329-unknown-merge` | `duplicate_merge_keeps_null_fields_and_nested_kassiber_data_and_projects_only_last_tag` |
| `bip329-kassiber-merge` | `duplicate_merge_keeps_null_fields_and_nested_kassiber_data_and_projects_only_last_tag` |
| `bip329-conflict-equality` | `conflicts_preserve_unknown_surrogate_keys_and_python_numeric_equality` |
| `bip329-case-insensitive-tx` | `matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity` |
| `bip329-profile-match` | `matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity` |
| `bip329-exact-needs-one-row` | `matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity` |
| `bip329-wallet-order` | `matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity` |
| `bip329-wallet-export-exact` | `matching_requires_one_transaction_and_wallet_exports_exclude_ambiguity` |
| `bip329-stored-order` | `export_keeps_stored_order_then_synthesized_and_first_object_wins` |
| `bip329-synthesized-order` | `export_keeps_stored_order_then_synthesized_and_first_object_wins` |
| `bip329-stored-precedence` | `export_keeps_stored_order_then_synthesized_and_first_object_wins` |
| `bip329-page-order` | `label_pages_keep_descending_id_ties_and_bind_scope` |
| `bip329-page-boundary` | `label_pages_keep_descending_id_ties_and_bind_scope` |
| `bip329-receive-exclusive` | `local_address_inventory_and_receive_outpoint_matches_preserve_sources` |
| `bip329-outpoint-match` | `local_address_inventory_and_receive_outpoint_matches_preserve_sources` |
| `bip329-base58-case` | `local_address_inventory_and_receive_outpoint_matches_preserve_sources` |
| `bip329-derivation-refusal` | `unported_descriptor_and_complex_graph_matching_refuse_before_writes` |
| `bip329-graph-refusal` | `unported_descriptor_and_complex_graph_matching_refuse_before_writes` |

Remaining requested work, in order:

1. Complete BIP329 descriptor and input-graph matching plus standard-field
   surrogate/platform parity; the entire BIP329 family is not accepted yet.
2. Grouped metadata updates, including pricing/review/tax validation and atomic
   append-only audit changes, and `ui.transactions.metadata.update`.
3. `transactions list` and its desktop list/dashboard/extremes/search/resolve/
   graph/review-context/ledger-template surfaces.
4. All four `documents list/create/attach/import-report` leaves and
   `ui.documents.list/create/attach/import_report`, plus
   `ui.transactions.commercial_context`. Prior reports remain local evidence,
   never authority for ledger totals.

Explicitly out of scope and still `not_implemented`: `journals process`, every
`reports` command/kind, `transactions export` CSV/XLSX/platform formats,
`ui.transactions.export_csv/export_xlsx/export_platform_csv`,
`btcpay provenance sync`, and `ui.btcpay.provenance.sync`. AI projections stay
Python. Full Python shards and native platform verification remain with the
orchestrator.

## Grouped metadata continuation

`t3code/r5-transactions-5` starts at the requested `a6359897`, without touching
`t3code/r5-transactions-4` in its other worktree. This run stops at a verified
**grouped metadata boundary**, the first requested step. Transactions and
documents remain unported. All commits are local; no push or PR.

`crates/transactions/src/metadata/grouped.rs` implements the complete grouped
save through the typed metadata book service. The existing desktop kind
`ui.transactions.metadata.update` is registered. There is no new CLI leaf;
Python has no grouped-update CLI command. The catalog retains the 26 metadata
leaves already implemented. AI projections remain Python.

Notes preserve empty/null distinctions. Nullable review/tax fields distinguish
omission from an authored clear. Null tags preserve the current set; an empty
array clears it. Duplicate labels retain the first normalized spelling, reuse
profile tag identities, and use Python's canonical audit ordering. Exclusion
retains the active-custody guard. Kind overrides validate direction and leave
the imported kind and observed integer msat quantities unchanged.

Pricing retains authored decimal scale and derives missing rate/value using
Kassiber's application-wide 32-digit, half-even context. A missing source or
explicit `missing` quality clears pricing/provenance together. Capture time is
recorded for real edits; changing only capture time is a no-op. The existing
workspace `kassiber-pydecimal` crate supplies this arithmetic; no new third-party
dependency or license entry is needed. Currency uppercasing and alphabetic
validation follow CPython 3.11 / Unicode 14, including multi-character case
expansion and newer scalars. Frozen uppercase vectors check every scalar;
the Unicode-table generator also checks the alphabetic ranges.

All validation precedes writes. Tags, transaction columns, one ordered audit
event, its fields and journal-input invalidation share a savepoint. Failed
invalidation rolls everything back. Historical events are never rewritten.
There is no schema, egress, credential or generic-dispatch change.

Reference fix `d3ec0f94` rejects NaN and infinity in grouped price inputs with
typed validation. Before the fix, infinity could be stored and NaN could raise
an untyped decimal exception. The new
`test_grouped_pricing_rejects_nonfinite_decimals_without_side_effects` failed
before the fix (all eight subcases) and passed after it. It checks both rate
and value against infinities, NaN and signaling NaN, preserving the transaction,
tags, history and profile version on refusal. The full reference history module
passed: 13 tests, 8 subtests. This is the only new Python product change.

`tests/test_rust_grouped_metadata_services.py` compares successful mutation
envelopes as bytes after replacing only their generated audit UUID. Stable
no-op and validation records compare as raw bytes. Separate disposable books
compare all stored audit values/diffs/order, source/reason/version snapshots,
observed transactions and tag effects. Generated capture times are checked for
presence/format separately; their audit rows are counted rather than compared
as equal timestamps. The module covers 109 candidate requests for the new kind
across four Rust daemon sessions, with zero Python kind fallbacks.

Verification uses Rust 1.99.1 through mise, cargo `--locked --offline`, locked
uv Python 3.11 and `MATURIN_NO_INSTALL_RUST=1`. All books, HOME/XDG inputs and
mutation targets are disposable; tools install outside those HOME roots.
The mutation run uses its own `CARGO_TARGET_DIR` and never shares artifacts
with the ordinary workspace or parity binary. The final harness build follows
workspace tests; no concurrent Cargo command replaces it during parity.

| Check | Result |
| --- | --- |
| Workspace fmt | Passed |
| Workspace clippy and `-p kassiber --features test-harness`, all targets, `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | Passed; includes 46 transaction-domain tests |
| Harness build | Passed |
| Final hybrid grouped/inherited metadata/BIP329/evidence/history modules | 30 passed, 8 subtests passed |
| Requested shell/hybrid/CLI-runner/port-contract modules | 53 passed, 76 subtests passed |
| Inventory, contracts and CI shard validation | 28 passed, 47 subtests passed |
| Frozen Unicode tables and uppercase vectors | Passed |
| Transaction mutation catalog | 62/62 built and killed; 31 distinct named baseline tests passed |

The first sandboxed parity attempt failed because Python's daemon could not
write its fixed account ownership-lock namespace. Subsequent parity runs use
disposable books outside that sandbox. An exploratory differential run caught
and corrected a Rust-only use of the default 28-digit Decimal context; Kassiber
sets 32 digits in its initializer. An initial uppercase-test build preceded
successful fixture generation; the final workspace run includes that fixture.
No automatic approval rejection occurred.

<!-- grouped-served-counts:start -->
| Command | Rust | Python |
| --- | ---: | ---: |
| `attachments.add` | 3 | 0 |
| `attachments.gc` | 1 | 0 |
| `attachments.list` | 1 | 0 |
| `attachments.remove` | 1 | 0 |
| `attachments.rename` | 1 | 0 |
| `daemon` | 8 | 0 |
| `loans.link` | 1 | 0 |
| `loans.mark` | 3 | 0 |
| `loans.unmark` | 1 | 0 |
| `metadata.bip329.export` | 6 | 0 |
| `metadata.bip329.import` | 9 | 0 |
| `metadata.bip329.list` | 2 | 0 |
| `metadata.bip329.preview` | 9 | 0 |
| `metadata.exclude` | 2 | 0 |
| `metadata.include` | 2 | 0 |
| `metadata.notes.clear` | 1 | 0 |
| `metadata.notes.set` | 7 | 0 |
| `metadata.records.excluded.clear` | 1 | 0 |
| `metadata.records.excluded.set` | 1 | 0 |
| `metadata.records.get` | 1 | 0 |
| `metadata.records.history.activity` | 3 | 0 |
| `metadata.records.history.list` | 4 | 0 |
| `metadata.records.history.revert` | 5 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.kind.clear` | 3 | 0 |
| `metadata.records.kind.set` | 3 | 0 |
| `metadata.records.list` | 2 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.note.set` | 4 | 0 |
| `metadata.records.tag.add` | 1 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.create` | 29 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.remove` | 2 | 0 |
<!-- grouped-served-counts:end -->

| Daemon kind | Rust requests | Python fallbacks |
| --- | ---: | ---: |
| `ui.transactions.metadata.update` | 109 | 0 |

The following new entries supplement all 50 predecessor mutations, which were
rerun. Every entry builds and is killed by the named Rust test; compiler errors
are never counted as kills.

| New mutation | Named Rust test |
| --- | --- |
| `grouped-null-note` | `grouped_save_merges_only_present_fields_and_appends_one_ordered_event` |
| `grouped-null-tags` | `grouped_save_merges_only_present_fields_and_appends_one_ordered_event` |
| `grouped-first-tag-label` | `grouped_save_merges_only_present_fields_and_appends_one_ordered_event` |
| `grouped-audit-order` | `grouped_save_merges_only_present_fields_and_appends_one_ordered_event` |
| `grouped-present-fields` | `grouped_canonical_tags_and_capture_time_only_changes_are_noops` |
| `grouped-rate-from-value` | `grouped_prices_derive_exact_amounts_and_clear_all_provenance_when_missing` |
| `grouped-value-from-rate` | `grouped_save_merges_only_present_fields_and_appends_one_ordered_event` |
| `grouped-decimal-context` | `grouped_prices_derive_exact_amounts_and_clear_all_provenance_when_missing` |
| `grouped-missing-pricing` | `grouped_prices_derive_exact_amounts_and_clear_all_provenance_when_missing` |
| `grouped-capture-only-noop` | `grouped_canonical_tags_and_capture_time_only_changes_are_noops` |
| `grouped-kind-direction` | `grouped_validation_and_failed_invalidation_preserve_all_rows` |
| `grouped-custody-exclusion` | `grouped_nonfinite_prices_fail_before_writes_and_exclusion_preserves_custody` |

Remaining requested work, in order:

1. `transactions list`, with existing desktop list/dashboard/extremes/search/
   resolve/graph/review-context/ledger-template kinds. The existing editor smoke
   session also reads `ui.transactions.list`, so that whole session is not yet
   accepted against Rust; the new grouped-only sessions exercise native saves.
2. All four `documents list/create/attach/import-report` leaves,
   `ui.documents.list/create/attach/import_report`, and
   `ui.transactions.commercial_context`. Prior reports must remain evidence,
   without granting lot or ledger authority.

BIP329 descriptor/xpub and input-graph matching are deliberately deferred to
the wallet lifecycle work. Standard-field surrogate and native platform parity
also remain unaccepted. AI projections stay Python.

Explicitly out of scope and still `not_implemented`: journal processing
(`journals process` and its kinds), every `reports` command/kind,
`transactions export` CSV/XLSX/platform formats and
`ui.transactions.export_csv/export_xlsx/export_platform_csv`,
`btcpay provenance sync`, and `ui.btcpay.provenance.sync`. Full Python shards
and native platform verification remain with the orchestrator.

## Observed CLI list continuation

`t3code/r5-transactions-6` starts at the requested `17a69a38`, without changing
`t3code/r5-transactions-5` in its other worktree. This is a **partial CLI-list
boundary**, not acceptance of the transaction desktop or document families.
All commits are local; no push or PR. No Python product source is changed.

`crates/transactions/src/listing.rs` implements `transactions list` through the
typed runtime book service and its own CLI family module. It preserves wallet,
direction, asset, timestamp, repeated/comma-separated ID, period, status, flow,
payment method, network, fee and quick filters. Wallet selectors reuse the
existing scoped resolver, including ID precedence and ambiguous-label refusals.
Flow filters use only current stored custody projections in the same profile.
They never author ownership, custody, commercial or tax facts.

All four sorts preserve their primary order and chronological tie breakers.
Cursor JSON/base64 bytes, scope/filter/sort/order binding and keyset boundaries
follow Python. Counts cover the complete filtered set before the cursor. The
page preserves observed kinds, exact integer amount/fee msat, authored decimal
price strings, ordered tags and empty/null presentation. BTC display converts
the exact decimal once, avoiding premature rounding above `2^53` msat. No raw
wallet configuration enters the result. No schema, dependency, egress path,
credential surface or AI authority changes.

One reference limitation remains: time filters compare canonical timestamps
lexically. A stored `2025-02-01T00:00:00Z` sorts after the fractional bound
`2025-02-01T00:00:00.000001Z`, so that start bound includes the stored row and
that end bound excludes it. This port preserves Python's behavior; the Rust
regression pins it explicitly. A semantic correction needs a separate Python
reference-fix commit with a failing regression, then matching Rust changes.
No such fix is claimed by this partial boundary.

Seven Rust tests cover the service. The new Python differential module
`tests/test_rust_transaction_listing.py` compares successful pages, all filter
families, typed refusals, all four CLI renderers and pagination envelopes as raw
bytes; no fields are normalized. It also pins the explicit unported desktop
and export barriers. The two inherited pre-bootstrap refusal scenarios now use
`transactions export`, because list is implemented.

Verification uses Rust 1.99.1 through mise, Cargo `--locked --offline`, locked
uv Python 3.11 and `MATURIN_NO_INSTALL_RUST=1`. All books and HOME/XDG inputs
are disposable; HOME is never a tool installation root. Mutation builds use
their own target directory. The final harness build follows workspace tests,
with no overlapping workspace build during the final parity run.

<!-- observed-list-verification:start -->
| Check | Result |
| --- | --- |
| Workspace fmt | Passed |
| Workspace and test-harness clippy, all targets, `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 521 passed; includes 53 transaction-domain tests |
| Final listing regression suite, after stronger final-page assertion | 7 passed |
| Harness build | Passed |
| Final hybrid listing and inherited evidence/metadata/grouped/BIP329 modules | 21 passed, 57 subtests passed |
| Requested shell/hybrid-candidate/CLI-runner/port-contract modules | 53 passed, 77 subtests passed |
| Inventory, contracts, CI shards and mutation applicability | 31 passed, 61 subtests passed; 90 catalog entries apply |
| Final transaction mutation catalog, fresh isolated target | 90/90 built and killed; 37 distinct named baseline tests passed |

The first exploratory differential run caught a test-only assumption that
pagination fields were nested under `meta`; Python places them at the envelope
top level. All 57 filter subtests passed in that run. The final run uses the
corrected page assertions and additionally checks all four CLI renderers.
Initial Rust fixture corrections fixed an overlong synthetic explorer ID and
used case-distinct duplicate wallet labels to reach the ambiguity guard.

The first mutation run killed 89/90 entries: a page-boundary mutation survived
because the test allowed a full final page to advertise an empty follow-up.
The stronger assertion kills it. An intermediate rerun reused a target
directory retaining a compiled mutant and failed its unmutated baseline;
that run supplies no acceptance evidence. The successful final run uses a new
target directory and checks the entire catalog. No automatic approval rejection
occurred. The workspace socket tests and daemon scenarios ran outside the
filesystem sandbox, with disposable books.
<!-- observed-list-verification:end -->

<!-- observed-list-served-counts:start -->
| Command | Rust | Python |
| --- | ---: | ---: |
| `attachments.add` | 3 | 0 |
| `attachments.gc` | 1 | 0 |
| `attachments.list` | 1 | 0 |
| `attachments.remove` | 1 | 0 |
| `attachments.rename` | 1 | 0 |
| `daemon` | 8 | 0 |
| `init` | 21 | 0 |
| `loans.link` | 1 | 0 |
| `loans.mark` | 3 | 0 |
| `loans.unmark` | 1 | 0 |
| `metadata.bip329.export` | 6 | 0 |
| `metadata.bip329.import` | 9 | 0 |
| `metadata.bip329.list` | 2 | 0 |
| `metadata.bip329.preview` | 9 | 0 |
| `metadata.exclude` | 2 | 0 |
| `metadata.include` | 2 | 0 |
| `metadata.notes.clear` | 1 | 0 |
| `metadata.notes.set` | 7 | 0 |
| `metadata.records.excluded.clear` | 1 | 0 |
| `metadata.records.excluded.set` | 1 | 0 |
| `metadata.records.get` | 1 | 0 |
| `metadata.records.history.activity` | 3 | 0 |
| `metadata.records.history.list` | 2 | 0 |
| `metadata.records.history.revert` | 5 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.kind.clear` | 2 | 0 |
| `metadata.records.kind.set` | 2 | 0 |
| `metadata.records.list` | 2 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.note.set` | 2 | 0 |
| `metadata.records.tag.add` | 1 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.create` | 3 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.remove` | 2 | 0 |
| `transactions.list` | 147 | 0 |

All 147 candidate `transactions list` calls ran in Rust, with zero Python
fallbacks. Reference comparison subprocesses are additional evidence and are
not hybrid fallback calls. The inherited implemented commands also have zero
fallbacks in this run. Workspace/profile/wallet setup remains Python; the
daemon row counts Rust sessions, not individual kinds. There are no new
implemented daemon kinds. The direct four-format export refusals and twelve
unported desktop-kind refusals are deliberately outside the hybrid counts.
<!-- observed-list-served-counts:end -->

The 28 new mutations below supplement the 62 predecessor entries. Every entry
builds and fails its named Rust test; compiler errors never count as kills.

<!-- observed-list-mutations:start -->
| New mutation | Named Rust test |
| --- | --- |
| `transaction-profile-scope` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-wallet-filter` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-asset-filter` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-start-inclusive` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-end-inclusive` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-id-normalization` | `transaction_filters_keep_profile_wallet_dates_assets_and_normalized_ids` |
| `transaction-time-order` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-primary-ties` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-page-boundary` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-cursor-direction` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-cursor-ties` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-cursor-sort` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-cursor-order` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-review-precedence` | `transaction_quick_status_and_price_filters_keep_quarantine_precedence` |
| `transaction-exact-price` | `transaction_quick_status_and_price_filters_keep_quarantine_precedence` |
| `transaction-fee-filter` | `transaction_quick_status_and_price_filters_keep_quarantine_precedence` |
| `transaction-explorer-id` | `transaction_quick_status_and_price_filters_keep_quarantine_precedence` |
| `transaction-liquid-precedence` | `transaction_payment_and_network_filters_keep_family_precedence_and_json_guards` |
| `transaction-projection-version` | `transaction_flows_use_only_fresh_profile_scoped_custody_projections` |
| `transaction-projection-processed` | `transaction_flows_use_only_fresh_profile_scoped_custody_projections` |
| `transaction-projection-scope` | `transaction_flows_use_only_fresh_profile_scoped_custody_projections` |
| `transaction-incoming-direction` | `transaction_flows_use_only_fresh_profile_scoped_custody_projections` |
| `transaction-tag-order` | `transaction_rows_preserve_msat_decimal_rounding_tags_and_observed_fields` |
| `transaction-observed-kind` | `transaction_rows_preserve_msat_decimal_rounding_tags_and_observed_fields` |
| `transaction-cursor-filter-binding` | `transaction_pages_keep_all_sort_ties_counts_and_bound_cursors` |
| `transaction-msat-rounding` | `transaction_rows_preserve_msat_decimal_rounding_tags_and_observed_fields` |
| `transaction-move-projection-scope` | `transaction_flows_use_only_fresh_profile_scoped_custody_projections` |
| `transaction-network-config` | `transaction_payment_and_network_filters_keep_family_precedence_and_json_guards` |
<!-- observed-list-mutations:end -->

Remaining requested work, in order:

1. Transaction desktop `ui.transactions.list/dashboard/extremes/search/resolve/
   graph/review_context/ledger_template`. Existing history, reverts and grouped
   metadata saves retain their predecessor implementation; the editor's whole
   smoke session is still unaccepted because its list read is unported.
2. All four `documents list/create/attach/import-report` commands,
   `ui.documents.list/create/attach/import_report`, and
   `ui.transactions.commercial_context`. Matching and every refusal must follow
   Python and plan 08. Prior tax reports remain evidence, without granting
   ledger totals, lots, ownership, custody or tax authority.

BIP329 descriptor/xpub and input-graph matching, standard-field surrogate and
native platform parity remain deferred. AI projections remain Python.
Explicitly still `not_implemented`: `transactions export` CSV/XLSX/platform
formats and `ui.transactions.export_csv/export_xlsx/export_platform_csv`,
journal processing (`journals process` and its kinds), all `reports`
commands/kinds, `btcpay provenance sync` and `ui.btcpay.provenance.sync`.
Full Python shards and native platform verification remain with the orchestrator.
