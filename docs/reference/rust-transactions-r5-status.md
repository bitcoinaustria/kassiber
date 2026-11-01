# R5 local transaction evidence services

This is a partial boundary of [plan 21](../plan/21-rust-backend.md), based on
`82f6b77b`. Python remains the reference. The branch is
`t3code/r5-transactions`; commits are local only.

The latest continuation is `t3code/r5-transactions-9`; its
[document boundary](#document-continuation) and remaining work are
recorded at the end of this page. Earlier continuation results are retained
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

At this historical boundary, time filters compared canonical timestamps
lexically. A stored `2025-02-01T00:00:00Z` sorts after the fractional bound
`2025-02-01T00:00:00.000001Z`, so that start bound includes the stored row and
that end bound excludes it. This port preserves Python's behavior; the Rust
regression pins it explicitly. A semantic correction needs a separate Python
reference-fix commit with a failing regression, then matching Rust changes.
The continuation below corrects this limitation in separate Python and Rust
commits; these historical results predate that correction.

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
that run supplies no acceptance evidence. A CLI failure fixture initially inserted an arbitrary
trigger before reopening the book; the existing Rust schema gate correctly
refused that unadmitted schema. The trigger rollback/error check remains in the
open-book Rust regression; the CLI scenario instead checks a managed-root I/O
failure. The old unported-kind assertion was updated now that commercial context
is served. The successful final run uses a new
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

## Instant-bound Python reference correction

The continuation `t3code/r5-transactions-7` starts at `e66eafae`, leaving the
predecessor worktree unchanged. Its standalone Python correction replaces
lexical comparisons of user timestamp bounds with exact integer microseconds,
using `parse_iso_datetime_or_none` for persisted values. Transaction lists,
metadata records, edit history, journal-event lists, rate ranges/as-of lookups,
desktop transaction/review-queue/dashboard filters and audit changes-since
counts share the helper. Dashboard window validation also compares instants.
Malformed persisted timestamps are excluded from bounded reads. No schema,
stored observations, journal processing or reports change. AI projections stay
Python.

`tests/test_timestamp_filters.py` reproduces incorrect whole-second/fractional
boundary selection, including one microsecond, timezone-equivalent bounds,
inclusive equality and reversed dashboard windows. The tests fail against the
original filter implementations, then pass with the correction. It also pins
unknown timestamp handling and repeated filtered reads with an active cursor.
The targeted Python regression modules passed (95 tests, 82 subtests); the
additional active-cursor regression and inventory/contracts passed (20 tests,
24 subtests). Rust acceptance follows below.

## Instant-bound Rust continuation

`t3code/r5-transactions-7` starts at `e66eafae`. This continuation completes the
first requested step and stops at a verified **timestamp-correction boundary**.
It does not port the remaining transaction desktop kinds or documents. All
commits are local; nothing is pushed or published. Standalone Python reference
fix: `4f3cb12d`.

Transaction list, metadata record and edit-history bounds now use the same
exact integer microsecond comparison as the corrected Python reference. Stored
timestamps pass through the existing CPython-compatible ISO parser, including
terminal-Z and timezone normalization; unknown or calendar-overflow instants
cannot satisfy a bound. This retains sub-millisecond precision without floating
Unix time or SQLite date-function rounding. Registration is once per connection,
so another active read can safely use the function. No stored timestamps,
schema, dependencies, journal processing or AI authority change. Date predicates
now evaluate parsed instants within the scoped query; this run makes no large-book
performance claim for that per-row evaluation.

Existing canonical timestamp cursor bytes, sort/tie rules and filter binding
remain intact. Counts are still taken after filtering and before the page
cursor. New regressions independently assert fractional membership, filtered
counts and advancing pages. The SQL instant helper is checked against all 5,676
frozen CPython timestamp inputs. Differential scenarios compare transaction and
metadata pages, exact cursors and daemon history records as raw bytes, with no
normalized fields. There is no new CLI leaf or daemon kind in this continuation.

<!-- instant-bound-verification:start -->
| Check | Result |
| --- | --- |
| Workspace fmt | Passed |
| Workspace and test-harness clippy, all targets, `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 525 passed; includes 57 transaction-domain tests |
| Test-harness build, after workspace tests | Passed |
| Final hybrid listing/evidence/metadata/grouped/BIP329 modules | 22 passed, 57 subtests passed |
| Requested shell/hybrid-candidate/CLI-runner/port-contract modules | 53 passed, 77 subtests passed |
| Inventory, contracts, CI shards and mutation applicability | 31 passed, 61 subtests passed; 98 entries apply |
| Frozen metadata vectors | Passed; SQL instants also check all 5,676 timestamp inputs |
| Final transaction mutation catalog, fresh isolated target | 98/98 built and killed; 40 distinct named baseline tests passed |
| Python SQLCipher exact-bound smoke | Passed |

Rust uses the pinned 1.99.1 toolchain, all Cargo calls are offline, and Python
uses the locked uv 3.11 environment. Books, test HOME/XDG inputs and mutation
workspaces are disposable; tool installation HOME is never redirected. The
initial sandboxed workspace run could not bind four local test sockets. The
complete 525-test run and daemon scenarios passed outside that sandbox. A
preliminary 97-entry mutation pass predates the shared terminal-Z normalization
and its added mutation; only the fresh final 98-entry run supplies acceptance
for the final code. No automatic approval rejection occurred.
<!-- instant-bound-verification:end -->

<!-- instant-bound-served-counts:start -->
| Command | Rust | Python |
| --- | ---: | ---: |
| `attachments.add` | 3 | 0 |
| `attachments.gc` | 1 | 0 |
| `attachments.list` | 1 | 0 |
| `attachments.remove` | 1 | 0 |
| `attachments.rename` | 1 | 0 |
| `daemon` | 9 | 0 |
| `init` | 22 | 0 |
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
| `metadata.records.history.activity` | 15 | 0 |
| `metadata.records.history.list` | 2 | 0 |
| `metadata.records.history.revert` | 5 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.kind.clear` | 2 | 0 |
| `metadata.records.kind.set` | 2 | 0 |
| `metadata.records.list` | 14 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.note.set` | 2 | 0 |
| `metadata.records.tag.add` | 1 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.create` | 3 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.remove` | 2 | 0 |
| `transactions.list` | 159 | 0 |
<!-- instant-bound-served-counts:end -->

Every listed command has zero Python fallbacks. The only Python-served calls
in this run are unported workspace/profile/wallet creation (22 each). No
transaction desktop or document-family acceptance is implied.

The fractional-bound scenario sends eight requests each to
`ui.activity.history` and `ui.transactions.history` in the Rust daemon, with
zero Python fallbacks. Other daemon sessions retain the predecessor kind
coverage. Direct Python/Rust comparison subprocesses are additional evidence
and do not enter the hybrid served-by counts.

<!-- instant-bound-mutations:start -->
All eight new entries below build and are killed by the named test. The 90
predecessor entries, including the two adapted inclusive-bound anchors, were
also rerun and killed.

| New mutation | Named Rust test |
| --- | --- |
| `transaction-bound-lexical` | `transaction_fractional_bounds_compare_instants_before_counting_and_pagination` |
| `record-bound-lexical` | `metadata_record_and_history_bounds_keep_exact_microseconds` |
| `history-bound-lexical` | `metadata_record_and_history_bounds_keep_exact_microseconds` |
| `timestamp-fractional-precision` | `transaction_fractional_bounds_compare_instants_before_counting_and_pagination` |
| `timestamp-offset-normalization` | `persisted_time_bounds_normalize_offsets_and_exclude_invalid_instants` |
| `timestamp-unknown-is-null` | `persisted_time_bounds_normalize_offsets_and_exclude_invalid_instants` |
| `timestamp-calendar-range` | `persisted_time_bounds_normalize_offsets_and_exclude_invalid_instants` |
| `timestamp-terminal-z-normalization` | `persisted_time_bounds_normalize_offsets_and_exclude_invalid_instants` |
<!-- instant-bound-mutations:end -->

Remaining requested work, in order:

1. Transaction desktop `ui.transactions.list/dashboard/extremes/search/resolve/
   graph/review_context/ledger_template`, including custody grouping and the
   complete editor smoke session. Existing history/reverts/grouped saves stay
   implemented. No remaining transaction desktop read is advertised as ported.
2. All four `documents list/create/attach/import-report` commands,
   `ui.documents.list/create/attach/import_report`, and
   `ui.transactions.commercial_context`, preserving plan 08 matching and
   refusals. Prior reports must remain evidence without granting ledger, lot,
   ownership, custody or tax authority.

Explicitly still `not_implemented`: transaction exports in every format,
journal processing, reports, and BTCPay provenance sync. BIP329 descriptor/xpub
and input-graph matching remain deferred, as do standard-field surrogate and
native platform parity. AI projections remain Python. Full Python shards and
native platform verification remain with the orchestrator.

## Desktop page continuation

`t3code/r5-transactions-8` starts at the requested `d219bed5`, leaving
`t3code/r5-transactions-7` in its other worktree untouched. This is a partial
boundary: **three of the eight requested kinds**, not completion of the run's
eight-kind objective. All commits are local; no push or PR. Python remains the
reference. The standalone Python reference fix `4fc40b2f` rejects array/object
`sort` and `order` values before dictionary/set membership. Its regression fails
in all eight list/search cases before the fix, then passes with typed validation
refusals. The added cases also compare raw Rust/Python daemon JSONL.
Documents and AI projections stay outside this continuation.

`crates/transactions/src/desktop.rs` and its rendering/SQL modules implement
`ui.transactions.list`, `ui.transactions.search` and `ui.transactions.extremes`.
A typed `Projection` allowlist crosses the runtime book trait, with one daemon
family module. The daemon retains unlock, ownership, expected-book checks and
rollback. There is no generic dispatch, new schema, dependency, network path or
AI authority.

The projection keeps every desktop filter and alias, limit coercion/clamping,
SQLite LIKE query field, signed display sort and chronological tie. It uses
exact microsecond bounds before counting and cursor selection. Counts cover
the full filtered custody-group population before pagination; a page retains
pending pair IDs so a later counterpart does not render again. Wallet filters
select all matching IDs/labels as Python's desktop does, including ambiguous
labels; the CLI's wallet resolver remains separate. Queue status and its filters
use the most urgent leg within the page's wallet/date scope.

Stored custody decisions/economic relations and the projection relation view
supply grouping and pair presentation; legacy journal transfer rows supply the
same fallback as Python. No journal processing or ownership inference runs.
Freshness checks processed time, active transaction count and input version.
Same-asset moves do not invent a fee from the principal difference. Observed
kinds, authored kind overrides, ordered tags, nullable prices, payment/network
presentation, and explorer-ID case match the reference. Integer msat survive
until display; sub-satoshi float conversion rounds the exact rational once.
Wallet config is read only for the existing payment/chain/network presentation
and never enters the result. Errors use the shared secret redactor; successful
desktop values retain Python's existing disclosure boundary. No AI exposure is
added. This run makes no large-book performance claim.

Eleven Rust tests use 201 reproducible Python 3.11 vectors and independent checks
for relative UTC-midnight periods and empty-context behavior. Run
`PYTHONPATH=. uv run --locked python crates/transactions/scripts/gen_desktop_vectors.py --check`.
Only the fixture's current-UTC-year fallback is symbolic; the live daemon
oracle compares all JSONL response bytes without normalization. Three Python
scenarios cover filters/refusals, all sort pages and exact cursors, current/stale
custody grouping, queue scope, matching wallet populations and no active context.
The fixtures include isolated query fields and a >2^53 msat rounding boundary.

<!-- desktop-page-verification:start -->
Verification uses Rust 1.99.1, locked Python 3.11 and disposable books. Cargo
commands run offline. The harness is rebuilt after workspace tests.

| Check | Result |
| --- | --- |
| Workspace fmt; workspace and test-harness clippy, all targets, `-D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 536 passed, including 11 desktop tests |
| Test-harness binary build | Passed |
| Six hybrid parity modules (desktop, CLI listing, loans/attachments, metadata, grouped saves, BIP329) | 25 passed, 57 subtests passed |
| Requested shell/hybrid-candidate/CLI-runner/port-contract modules | 53 passed, 77 subtests passed; repeated after the reference fix |
| Inventory and CI-shard regressions | 28 passed, 47 subtests passed |
| Reference validation, timestamp and port regressions after the Python fix | 21 passed, 32 subtests passed |
| Final desktop Rust tests and live JSONL parity after the Python fix | 11 Rust tests and 3 hybrid scenarios passed |
| Inventory, contracts, frozen desktop vectors and mutation applicability checks | Passed; 201 vectors and 176 applicable entries |
| Final mutation mappings | 176/176 built and killed across two runs; 51 distinct baseline tests passed |

The final 171-entry mutation run killed 170 entries. Its default-list-limit
mutation survived because its assigned pagination test always supplied a limit.
The mapping was corrected to the filter test, which exercises the default. A
fresh six-entry run killed that corrected mapping and five additional explicit
status, stored-flow and cursor-binding mutations. Thus the final 176-entry
catalog has complete kill evidence, but no single successful all-176 run is
claimed. Preliminary surviving mutations and compiler failures are not kills.

The first shell-parity attempt used the product binary left by workspace tests
and refused the test guard with exit 78. Rebuilding the harness resolved it;
the successful runs above use the harness. The initial inventory check also
caught the new Python regression missing from the inventory; the updated
inventory and its follow-up checks pass. Full Python shards and native platform
checks remain with the orchestrator.
<!-- desktop-page-verification:end -->

<!-- desktop-page-served-counts:start -->
The final post-reference-fix desktop hybrid scenarios record these per-kind
responses, including typed refusals. Reference subprocesses compare every JSONL
byte and are excluded from fallback counts.

| Kind | Rust | Python |
| --- | ---: | ---: |
| `ui.transactions.list` | 158 | 0 |
| `ui.transactions.search` | 70 | 0 |
| `ui.transactions.extremes` | 13 | 0 |

All three ported kinds have zero Python fallbacks. The run starts 106 Rust daemon
sessions; only workspace/profile/wallet setup uses Python (three calls each).
The five deferred transaction kinds are not claimed by this acceptance table.
<!-- desktop-page-served-counts:end -->

The catalog adds 78 desktop mutations to the 98 inherited entries. Each entry
must build and fail its named test; compiler errors never count as kills.
The complete mapping follows.

<!-- desktop-page-mutations:start -->
| New mutation | Named Rust test |
| --- | --- |
| `desktop-profile` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-wallet-population` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-direction` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-asset` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-start-inclusive` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-end-inclusive` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-timestamp-instants` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-txids` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-tags` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-id` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-external-id` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-kind` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-description` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-counterparty` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-note` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-query-wallet` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-candidate-flow` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-incoming-direction` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-fee-filter` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-external-flow` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-review-queue` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-failed-import` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-explorer-filter` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-missing-price` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-network-config` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-network-asset` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-payment-filter` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-network-payment` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-sort-amount` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-sort-fiat` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-sort-fee` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-time-order` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-primary-ties` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-null-order` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-cursor-order` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-cursor-sort` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-cursor-ties` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-total-groups` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-page-limit` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-pending-pairs` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-pair-dedup` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-queue-rank` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-queue-end` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-fresh-count` | `desktop_freshness_requires_processed_count_version_and_timestamp` |
| `desktop-fresh-version` | `desktop_freshness_requires_processed_count_version_and_timestamp` |
| `desktop-fresh-processed` | `desktop_freshness_requires_processed_count_version_and_timestamp` |
| `desktop-search-required` | `desktop_search_requires_query_and_clamps_its_own_limit` |
| `desktop-extremes-order` | `desktop_extremes_apply_filters_before_both_amount_limits` |
| `desktop-extremes-limit` | `desktop_extremes_apply_filters_before_both_amount_limits` |
| `desktop-no-context` | `desktop_missing_context_short_circuits_list_search_but_extremes_validates` |
| `desktop-tags-order` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-kind-override` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-period-30days` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-3months` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-6months` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-1year` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-5years` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-10years` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-period-15years` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-ids-deduplicate` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-ids-count` | `desktop_refusals_keep_typed_validation_and_cursor_errors` |
| `desktop-candidate-count` | `desktop_refusals_keep_typed_validation_and_cursor_errors` |
| `desktop-list-default-limit` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-search-default-limit` | `desktop_search_requires_query_and_clamps_its_own_limit` |
| `desktop-list-maximum-limit` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-search-maximum-limit` | `desktop_search_requires_query_and_clamps_its_own_limit` |
| `desktop-extremes-maximum-limit` | `desktop_extremes_apply_filters_before_both_amount_limits` |
| `desktop-count-before-cursor` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-total-before-cursor` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
| `desktop-boolean-alias-priority` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-timestamp-alias-priority` | `desktop_relative_periods_use_midnight_and_alias_precedence` |
| `desktop-limit-truncation` | `desktop_refusals_keep_typed_validation_and_cursor_errors` |
| `desktop-sat-rounding` | `desktop_filters_preserve_wallet_population_query_ids_and_exact_instants` |
| `desktop-status-filter` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-stored-transfer-filter` | `desktop_moves_use_stored_decisions_and_journal_fallback_without_inventing_fees` |
| `desktop-stored-swap-filter` | `desktop_pairs_preserve_group_totals_queue_scope_and_pending_cursor_pairs` |
| `desktop-layer-transition-filter` | `desktop_flow_payment_network_fee_quick_and_status_filters_match_python` |
| `desktop-cursor-filter-binding` | `desktop_pages_preserve_all_orders_nulls_totals_and_bound_cursors` |
<!-- desktop-page-mutations:end -->

The five remaining kinds stay `not_implemented` as whole projections:

| Kind | Exact remaining dependency |
| --- | --- |
| `ui.transactions.dashboard` | Daemon `suggest_transfer_candidates` via `_ui_swap_matching_payload_from_conn`, non-conflicted candidate selection, and `build_transactions_dashboard_snapshot` calendar/timezone/auto-period/bucket composition. These suggestions are computed, not a saved snapshot to substitute. |
| `ui.transactions.resolve` | `_reviewed_pair_reading`, `custody_authored_migration.list_pair_review_records` and `quarantine_review.pair_fingerprint`, including component allocations and full observed leg context. The stored-state reader/fingerprint port is unfinished; journal processing is not required for this read. |
| `ui.transactions.graph` | `transaction_graph._compute_profile_semantics` and the full owned index, stored custody annotations, local spends/funding/block heights, graph parsing and reference-cache enrichment. Cache contents are inputs, not an already computed projection. Explicit public lookup must retain its exact consent gate. |
| `ui.transactions.review_context` | Resolve plus journal-event projection, commercial context, source-funds links, current pair/payout materialization, graph, privacy hygiene, audit/evidence readiness and section-specific redactors/refusals. Disabling optional sections still needs the mandatory stored-state sections. |
| `ui.transactions.ledger_template` | `_managed_report_export_path` collision naming and `write_generic_ledger_template`, including the generic CSV and styled XLSX template writer and its validation. This writer is unfinished, independent of journal/report processing. |

The complete transaction editor smoke session remains unaccepted. Documents,
commercial context, all transaction export formats, journal processing, reports,
BTCPay provenance sync, and the prior BIP329/platform deferrals retain their
existing barriers. Full Python shards and native platform verification remain
with the orchestrator.

## Document continuation

Branch `t3code/r5-transactions-9` starts at the requested `d5296a02`.
The predecessor branch in its other worktree is untouched. All commits are
local; no push or PR. Python remains the reference, with no Python product-source
change in this continuation.

All four CLI commands and desktop kinds are implemented:
`documents list/create/attach/import-report` and
`ui.documents.list/create/attach/import_report`.
Stored matching is exposed by `btcpay provenance suggest` and
`ui.btcpay.provenance.suggest`; the stored `list/links` commands and kinds and
`ui.transactions.commercial_context` supply the needed commercial reads.
The explicit enum requests cross the admitted runtime book interface, with
CLI/daemon family registration and the existing unlock, ownership,
expected-book and rollback boundaries.

Registration preserves document ID precedence, scoped reference ambiguity and
conflict refusals, exact decimal text, UTC second precision, the unknown-date
sentinel, and descending created-time/ID pages. Managed evidence reuses the
attachment layout, filename sanitizer, SHA-256/size and MIME inference. Document
attachments have no transaction identity. URL targets and labels retain the
document reference's literal behavior, including whitespace and scheme-free
targets; no URL is fetched. Unencodable text is refused rather than silently
dropping Unicode code points; malformed date/decimal fields keep the reference
refusal codes. Layout I/O failures also retain the reference's CLI error code.
Prior-report import atomically stores a document,
raw provider/year provenance and its file, without changing transactions,
journals, lots, custody, ownership or journal-input versions. Failure rolls back
the rows and removes files created by that request.

Matching reproduces the shipped plan-08 reference rather than inventing its
future heuristics: transaction IDs, payment hashes and payment-ledger stable keys
must identify exactly one active observation in the admitted profile. Payouts
require outgoing observations and matchable stored status. Exact confidence
uses absolute principal after included outbound fees. Documents match stored
invoice/order/payment-request references or equal stored currency/value text.
Ambiguity remains unresolved. Existing reviewed links retain their state and
confidence; a first matching document enriches the same payment/transaction
identity. Rejected rows remain historical, and suggestions never change prices,
kinds, tax inputs or ownership. A failing later suggestion rolls back the batch.

The pure context read preserves active-link ordering, document deduplication,
invoice-first origin/payment-request evidence, BTC-denomination fiat suppression,
integer-msat payout totals, 32-digit decimal sums and mixed/missing-currency
barriers. Sync freshness reads stored timestamps only and never probes a backend.
Successful desktop evidence retains Python's existing disclosure boundary;
AI projections stay Python.

<!-- document-verification:start -->
All checks use Rust 1.99.1 and the locked Python 3.11 environment. Cargo
builds/tests are offline. Books, HOME/XDG inputs, logs and scratch trees are
disposable. The harness is rebuilt after workspace tests. No Python product
bug was found or changed.

| Check | Result |
| --- | --- |
| Workspace fmt | Passed |
| Workspace clippy, all targets, warnings denied | Passed |
| Test-harness clippy, all targets, warnings denied | Passed |
| Full workspace tests, including `kassiber-net` | 547 passed, 0 failed |
| Final test-harness build | Passed |
| Seven hybrid document/transaction/metadata parity modules | 29 distinct tests passed across the broad run and focused rerun; 57 subtests passed |
| Requested shell/hybrid-candidate/CLI-runner/port modules | 53 passed, 84 subtests passed |
| Inventory, contracts, shard validation | Passed; 28 tests and 47 subtests passed |
| Mutation applicability | 226 entries apply |
| Transaction mutation checks | 226/226 built and killed across two acceptance runs; 62 named baseline tests passed |

A sandboxed preliminary workspace attempt could not bind its disposable local
sockets; the complete run with that permission passed. An interrupted attempt
also exhausted the host filesystem while compiling; no result from that attempt
is counted. The resumed run uses fresh isolated mutation targets. A reused-target
baseline picked up a prior mutated artifact and failed before mutation, so that
attempt supplies no kills. Compiler failures and incomplete runs likewise supply
no acceptance evidence. A CLI failure fixture initially inserted an arbitrary
trigger before reopening the book; the existing Rust schema gate correctly
refused that unadmitted schema. The trigger rollback/error check remains in the
open-book Rust regression; the CLI scenario instead checks a managed-root I/O
failure. The old unported-kind assertion was updated now that commercial context
is served.
The broad seven-module run passed 27 tests and failed the two assertions
described above. After the fixture, remaining-kind assertion and CLI layout
error-code corrections, the focused four document scenarios plus that
remaining-kind scenario passed (5 tests). Together these cover all 29 distinct
tests; a single clean 29-test run is not claimed.
<!-- document-verification:end -->

<!-- document-served-counts:start -->
The accepted five-test focused run gives these counts. The four document
scenarios exercise every new command/kind; the additional remaining-kind test
does not add document calls. Raw reference/candidate byte comparisons are
additional checks, excluded from hybrid counters. Setup workspace/profile/wallet
calls intentionally fall back to Python. The five hybrid daemon sessions run
in Rust; each kind count records requests inside those admitted Rust processes.

| Command | Rust | Python |
| --- | ---: | ---: |
| `documents.list` | 1 | 0 |
| `documents.create` | 5 | 0 |
| `documents.attach` | 5 | 0 |
| `documents.import-report` | 2 | 0 |
| `btcpay.provenance.list` | 1 | 0 |
| `btcpay.provenance.links` | 1 | 0 |
| `btcpay.provenance.suggest` | 1 | 0 |
| `daemon` | 5 | 0 |

| Desktop kind | Rust | Python |
| --- | ---: | ---: |
| `ui.documents.list` | 8 | 0 |
| `ui.documents.create` | 13 | 0 |
| `ui.documents.attach` | 8 | 0 |
| `ui.documents.import_report` | 5 | 0 |
| `ui.transactions.commercial_context` | 6 | 0 |
| `ui.btcpay.provenance.list` | 2 | 0 |
| `ui.btcpay.provenance.links` | 2 | 0 |
| `ui.btcpay.provenance.suggest` | 1 | 0 |
<!-- document-served-counts:end -->

Eleven Rust regressions and four hybrid scenarios cover the boundary. The
mutation runner builds every mutated tree and requires its named test to fail;
compiler errors and baseline failures are not kills. The full 224-entry run
killed all 224 entries. After the final Unicode refusal guards, all 50 document
entries were rerun in a fresh isolated target and killed, with their 11 named
baseline tests passing. These overlapping runs cover all 176 inherited and 50
new entries; a single 226-entry run is not claimed. The 50 new mappings follow.

<!-- document-mutations:start -->
| New mutation | Named Rust test |
| --- | --- |
| `document-id-precedence` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-reference-scope` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-reference-ambiguity` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-external-conflict` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-page-tie` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-second-precision` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-unknown-date` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-exact-fiat` | `document_registration_preserves_exact_values_refusals_scope_and_order` |
| `document-cleanup-files` | `document_evidence_is_literal_transactionless_hashed_and_atomic` |
| `document-transactionless` | `document_evidence_is_literal_transactionless_hashed_and_atomic` |
| `document-url-literal` | `document_evidence_is_literal_transactionless_hashed_and_atomic` |
| `document-prior-evidence-type` | `prior_reports_are_atomic_evidence_without_ledger_or_lot_authority` |
| `commercial-profile` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-excluded` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-txid-signal` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-payment-hash-signal` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-stable-key-signal` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-unique-transaction` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-principal-fee` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-principal-absolute` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-confidence` | `commercial_matching_requires_unique_in_scope_active_identifiers_and_scores_principal` |
| `commercial-payout-direction` | `commercial_payout_matching_requires_outgoing_proof_and_matchable_status` |
| `commercial-payout-status` | `commercial_payout_matching_requires_outgoing_proof_and_matchable_status` |
| `document-match-request-reference` | `document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment` |
| `document-match-order-reference` | `document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment` |
| `document-match-exact-fiat` | `document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment` |
| `document-match-currency` | `document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment` |
| `document-match-profile` | `document_matching_uses_reference_or_exact_currency_value_and_requires_unique_payment` |
| `document-match-unique-payment` | `document_transaction_matching_refuses_ambiguity_and_rolls_back_the_whole_suggestion_batch` |
| `commercial-rejected-identity` | `document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates` |
| `commercial-document-enrichment` | `document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates` |
| `commercial-first-document` | `document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates` |
| `commercial-document-fiat-provenance` | `document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates` |
| `commercial-suggestion-dedup` | `document_matching_enriches_identity_preserves_review_and_rejection_and_deduplicates` |
| `commercial-suggestion-atomic` | `document_transaction_matching_refuses_ambiguity_and_rolls_back_the_whole_suggestion_batch` |
| `commercial-context-rejected` | `commercial_context_keeps_order_invoice_preference_redaction_boundary_and_stored_freshness` |
| `commercial-context-reviewed-order` | `commercial_stored_pages_and_context_keep_scope_filters_limits_and_ties` |
| `commercial-context-id-tie` | `commercial_stored_pages_and_context_keep_scope_filters_limits_and_ties` |
| `commercial-links-id-tie` | `commercial_stored_pages_and_context_keep_scope_filters_limits_and_ties` |
| `commercial-records-id-tie` | `commercial_stored_pages_and_context_keep_scope_filters_limits_and_ties` |
| `commercial-invoice-origin-priority` | `commercial_context_keeps_order_invoice_preference_redaction_boundary_and_stored_freshness` |
| `commercial-bitcoin-is-not-fiat` | `commercial_context_keeps_order_invoice_preference_redaction_boundary_and_stored_freshness` |
| `commercial-payout-absolute-sum` | `commercial_context_payout_batches_preserve_exact_sums_and_currency_barriers` |
| `commercial-payout-decimal-precision` | `commercial_context_payout_batches_preserve_exact_sums_and_currency_barriers` |
| `commercial-payout-currency-barrier` | `commercial_context_payout_batches_preserve_exact_sums_and_currency_barriers` |
| `commercial-payout-missing-currency` | `commercial_context_payout_batches_preserve_exact_sums_and_currency_barriers` |
| `commercial-freshness-never-synced` | `commercial_context_keeps_order_invoice_preference_redaction_boundary_and_stored_freshness` |
| `commercial-corrupt-amount` | `document_transaction_matching_refuses_ambiguity_and_rolls_back_the_whole_suggestion_batch` |
| `document-prior-source-provenance` | `prior_reports_are_atomic_evidence_without_ledger_or_lot_authority` |
| `document-url-surrogate` | `document_evidence_is_literal_transactionless_hashed_and_atomic` |
<!-- document-mutations:end -->

Remaining work: the four deferred transaction desktop projections and full
editor smoke session, transaction exports in every format, journal processing
and reports, BTCPay provenance sync and commercial review, descriptor/input-graph
BIP329 matching, standard-field surrogate support and native-platform evidence.
The stored commercial section is now available to a future review-context port;
the whole review-context kind remains `not_implemented`. OCR/import-draft AI
workflows and AI projections are outside this document-family port. Native
platform checks and full Python shards remain with the orchestrator.

## Desktop resolution continuation (partial)

The [R5 resolver checkpoint](rust-shell.md#desktop-transaction-resolution-r5-partial)
registers `ui.transactions.resolve` on `t3code/r5-desktop-projections` from
`b10e90e54`. This supersedes the preceding deferred-resolver statements;
dashboard, graph, review context and ledger template remain unported. The
resolver uses the existing desktop renderer, exact integer msat amounts and
current-journal gate. A uniquely matched authored pair supplies its reviewed
identity and economic fingerprint through the shared transfer service. Missing
or ambiguous review matches carry null confirmation fields. The 32 frozen
resolver vectors and plaintext/SQLCipher daemon comparisons pin alias ordering,
focused principal and fee, grouped review status, profile scope and no-context
validation order. All 112 logical tables stay unchanged by these reads.

The new transaction mutation entries are `resolve-id-precedence`,
`resolve-id-lowercase`, `resolve-external-uppercase`, `resolve-order`,
`resolve-id-order`, `resolve-profile`, `resolve-focused-principal`, `resolve-focused-pair-fee`,
`resolve-queue-status`, `resolve-current`, `resolve-no-context` and
`resolve-query-trim`. The transfer catalog adds `desktop-reading-missing`,
`desktop-reading-observation`, `desktop-reading-allocations`,
`desktop-reading-source` and `desktop-reading-ambiguous`. Each entry names its
killer test in its owning catalog. The shell checkpoint records the run results;
previous mutation kills are inherited, not claimed as rerun here.
