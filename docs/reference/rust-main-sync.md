# Rust main sync 1: parity gaps from main #672 to #714

The Rust backend stack (integration tip `t3code/rust-backend-r10b`,
a0b4d0342) sat on main as of #654. `t3code/rust-main-sync-1` merges
`t3code/tax-engine-phase-4` at c4f82d3db, which carries main up to #714
(045c36880), into it. This page lists every Python behaviour change in that
range that the Rust port serves, whether the Rust side now differs, and the
worker-sized slices that close the gaps. [Plan 21](../plan/21-rust-backend.md)
owns the parity rule: Python stays the reference until a slice ports the
change.

## How the differences were measured

`crates/daemon/scripts/desktop_read_diff.py` runs the Python and the Rust
daemon side by side, each on its own copy of a book with its own XDG roots
and `KASSIBER_NO_EGRESS=1`, sends the same requests in the same order and
records every differing path. The default set is every renderer-allowed,
non-streaming `read` kind the Rust daemon advertises (103 kinds); the report
reads (capability `operator`), `ui.review.plan`, `ui.review.worklist`,
`ui.review.receipt` and the transaction and wallet reads that need arguments
were added. Each run was repeated with the pre-merge Python (an export of
a0b4d0342) against the same Rust binary, so a difference present in both runs
predates the merge and is listed separately below.

Books:

- a fresh, empty data root;
- the two report-design sample books (`crates/reports/scripts/report_design_books.py`
  on `t3code/r5-pdf-1`): `austria` (moving average, history from 2020, Alt
  and Neu) and `generic` (FIFO). They were built twice, once with the
  merged Python CLI and once with the Rust CLI (same seeded CSVs, same
  commands; the Rust CLI serves all of them).

Before S1, Python-built books could not be compared at all: see
[S1](#s1-store-schema-and-command-registry). After S1, the Python-built
`austria` book (built by `report_design_books.py`'s steps with this
checkout's Python CLI) compares 113 kinds with 101 the same, 12 differing
and none refused; see [after S1](#after-s1).
The Rust-built books are opened by each side on its own copy; the Python
daemon migrates its copy on open.

Release binary, Rust-built books (kinds compared, differing):

| Book | merged Python vs Rust | pre-merge Python vs Rust |
| --- | --- | --- |
| fresh | 113, 4 | 113, 1 |
| austria | 113, 13 | 113, 4 |
| generic | 113, 13 | 113, 4 |

The pre-merge differences are `ui.logs.snapshot` (each side logs its own
lines; expected), `ui.journals.snapshot` and `ui.journals.events.list`
(`freshnessReason` omits "and calculation semantics"; fixed on
`t3code/r6-5b-ai-tools` by 172c5a8c7) and `ui.chain_analysis.query`
(`snapshot_id` differs on both books; not investigated here).

Targeted runs on the `austria` copy covered `ui.review.plan` with each new
operation type, `ui.journals.quarantine.resolve` by group, the export kinds
for tax years 2021 and 2025, the new CLI leaves, changed CLI arguments,
qualified transaction references and `transfers dismiss`. The Rust shell
crates' tests were run against the regenerated fixtures.

## Behaviour changes and their state

"Differs" means a run showed it. "Refused" means the Rust daemon or CLI
answers `not_implemented` or a validation error where Python answers.
"Inspection" means the books carry no data that reaches the change; the
Rust code was read instead.

### Store and command registry

| PR | Python | Kinds and leaves | Rust state | Rust location |
| --- | --- | --- | --- | --- |
| #675, #678, #684, #686, #708 | `db.py`: `transactions.pricing_review_accepted_price`, `ai_providers.agent_access`, table `ai_provider_checks`, custody AI audit `consent_decision` CHECK widened to `mode_auto`, `mode_full_access`, `mcp_host` (table rebuild) | every kind and leaf that opens a book | **Done (S1)**. Before: every book the merged Python created or opened answered `requires_python_migration` (`object: ai_provider_secret_refs`, `reason: schema_structure`) on every database kind, 99 of the 103 read kinds on the Python-built `austria` book | `crates/store` (`schema.rs`, `compat*.rs`, `creation.rs`) |
| #679, #681 | new CLI leaves `journals quarantine groups`, `transactions verify`, `transfers dismissals list`, `transfers dismissals delete`; kind derivation for `transfers_dismissals_command` | the four leaves | Kind derivation **done (S1)**. The leaves stay `not_implemented` until S5, S6 and S8 | `crates/envelope/src/lib.rs` (subcommand attrs), `crates/cli` |
| #678, #684, #686 | changed arguments: `mcp enable` (`--grant`, `--agent-access`), `mcp call` (`--grant`, `--client`, effect now `mutating`), `chat` (`--permission-mode`, file options), `ai providers create/update` (`--agent-access`), `journals quarantine resolve price-override/exclude` (`--group-id`, `--reprocess`) | those leaves | `describe` **done (S1)**. None of these leaves is native | `crates/cli/src/describe.rs` |
| #679, #681, #684, #689 | `command_capabilities.py`: four CLI leaves and four daemon kinds | `ui.transfers.dismissals.list/delete`, `ui.chat.attachments.upload`, `ai.chat.permission_mode` | **Done (S1)**: 317 entries each. The kinds stay `not_implemented` until S6 and S12 | `crates/runtime/src/capabilities.rs` |

### Transactions, review and journals

| PR | Python | Kinds and leaves | Rust state | Rust location |
| --- | --- | --- | --- | --- |
| #679 | `core/transaction_lookup.py`, `handlers.resolve_transaction`: a reference resolves by id, then an unambiguous external id, then a qualified `<external id>:<wallet or in/out>`; ambiguity lists every candidate | every leaf and kind taking a transaction reference | **Done (S2)**; see [after S2, S6 and S11](#after-s2-s6-and-s11). Before: `metadata notes set --transaction <txid>:in` succeeded in Python and Rust answered "not found"; the bare shared txid's `ambiguous_reference` hint differed | `crates/transactions/src/lookup.rs`, replacing nine copies (the eight listed before plus the planner's in `crates/transfers/src/component_review.rs`); `ui.transactions.resolve` in `crates/transactions/src/desktop/resolve.rs` |
| #680 | `review_workflow.py`: plans carry journal-meaning term operations (`pair`, `dismiss_pair`, `undismiss_pair`, `custody_gap`) and a per-year tax `delta`; apply checks the delta; receipts keep it | `ui.review.plan`, `ui.review.apply`, `ui.review.receipt`, `ui.review.cases` | **Refused**: a `pair` or `dismiss_pair` plan returns a plan in Python and "Review operation type is unsupported" in Rust. No Rust artifact carries `delta`, so an artifact either side makes is refused by the other | `crates/transfers/src/review_operations.rs`, `crates/daemon/src/review.rs`, `crates/journals/src/review_effects.rs` |
| #708 | `pricing.reviewed_price`, `metadata.py`, `transaction_history.py`, `ledger_request._coarse_needs_review`: an accepted coarse price is no longer held for review while the price is unchanged; a price edit drops the acceptance; history shows it | `ui.review.plan/apply` (`accept_coarse_prices`), `ui.transactions.metadata.update`, `ui.transactions.history`, `journals process` | **Refused**: `accept_coarse_prices` is an unsupported operation in Rust (Python: `review_case_changed`, nothing waiting). Rust's request builder sends `coarse` for every `coarse_fallback` row (`crates/custody/src/ledger_request.rs:654`), so after S1 an accepted book would quarantine again under Rust. Today such a book is refused by S1 | `crates/custody/src/ledger_request.rs`, `crates/journals/src/prebuild/pricing.rs`, `crates/transactions/src/metadata/history.rs` |
| #707, #711, bd162caca | `review_workflow.py`, `ui_snapshot.py`, `daemon_freshness.py`: cases name `fiat_currency`; badges add `review` and `definitions`; freshness pairs automatically before journals | `ui.review.cases`, `ui.review.badges`, `ui.freshness.run`, `ui.workspace.freshness.run` | **Differs**: `ui.review.cases` lacks `fiat_currency`; `ui.review.badges` lacks `review` and `definitions` (fresh root too). Freshness: inspection | `crates/daemon/src/review.rs`, `crates/daemon/src/workspace_reads.rs`, `crates/wallets` (freshness) |
| #681 | `quarantine_review.py` (+453): quarantine rolled up by root cause, groups, attention count, definitions; resolving by `group_id` with `reprocess` | `ui.journals.quarantine`, `journals quarantine groups`, `ui.journals.quarantine.resolve`, `ui.review.badges`, AI kind `ui.journals.quarantine.groups` | **Differs**: summary lacks `definitions` and `fiat_currency`; resolve with `group_id` is refused as an unsupported field (Python validates it). The books have no quarantine, so group rows were not compared | `crates/journals/src/quarantine.rs`, `crates/journals/src/views.rs`, `crates/daemon/src/journals.rs` |
| #681 | `wallet_completeness.py`, `transaction_verify.py` | `ui.wallets.list`, `transactions verify`, AI kind `ui.transactions.verify` | **Differs**: wallets lack `completeness_class` and `scan_bounds`; `transactions verify` is not native | `crates/daemon/src/wallets.rs`, `crates/wallets`, `crates/transactions` |
| #679, #683 | `custody_review_terms.dismiss_pair_candidate`, `transfer_matching.active_dismissed_pairs`, `handlers.suggest_transfer_candidates`: a dismissal marks journals stale, can be listed and deleted, and suggestions report exclusions (active pair, already journaled moves, dismissal) | `ui.transfers.dismiss`, `transfers dismiss`, `ui.transfers.suggest`, `ui.transfers.dismissals.list/delete`, `transfers dismissals list/delete` | **Done (S6)**. Before: after `transfers dismiss` Python bumped `journal_input_version` and Rust did not; `ui.transfers.suggest` lacked `exclusions`; the list/delete kinds and leaves were not native | `crates/transfers/src/dismissals.rs`, `crates/transfers/src/pairing/{matcher,service}.rs`, `crates/transfers/src/review_context.rs`, `crates/daemon/src/transfers.rs` |
| #679 | `custody_components.py`, `custody_component_planner.py`: revision planning and activation errors with remediation; qualified references | `ui.transfers.components.plan/apply`, review `custody_component` operations | **Done (S11)**, except the AI-only revision boundary (`_validate_ai_custody_state_boundary`, `authored_source == "ai_tool"`), which belongs to the R6 tool executor (S12) | `crates/custody/src/components/remediation.rs`, `crates/transfers/src/component_{review,rows,store}.rs` |
| #701 | `ui_snapshot.build_transactions_dashboard_snapshot`: automatic bucket size and a `history.years` index for the period picker | `ui.transactions.dashboard` | **Differs**: lacks `bucketSize` and `history.years` (fresh root too) | `crates/transactions/src/desktop/dashboard.rs` |
| #682, #698, #714 | `document_import.py`: the read step needs no database and records the detected API format; chat imports reuse it | `ui.wallets.document_import.preview/import` | Inspection: needs an AI provider, not exercised | `crates/imports` (document core) |

### Reports

| PR | Python | Kinds and leaves | Rust state | Rust location |
| --- | --- | --- | --- | --- |
| #706 | `ui_snapshot.build_capital_gains_snapshot`, `tax_estimate.py`: the Reports screen payload: `breakdown` by Kennzahl, `indicativeTax`, `e1kvApplies`, `rowsBeforeCryptoKennzahlen`, `fiatCurrency`, `longTermDays`, `totals`, lots with asset, category, entry type, Kennzahl and exact quantity. A root with no book, or a missing profile, reports `jurisdictionCode` `GENERIC`, and a profile without a country falls back to `generic`; Rust hard-codes `AT` in the empty payload (`gains_desktop.rs` `empty`) and falls back to `AT`. Not a store or setup default, so S1 left it to S7 | `ui.reports.capital_gains` | **Differs** on every book, with and without `year` | `crates/reports/src/gains_desktop.rs` |
| #706 | `austrian.py`, `reports.py`: E 1kv crypto Kennzahlen start in 2022; earlier disposals state no Kennzahl | `ui.reports.export_austrian_e1kv_csv/xlsx`, `reports export-austrian-e1kv*`, `ui.reports.export_capital_gains_csv` | **Differs, tax-relevant**: for 2021 Python refuses the E 1kv export ("The Austrian crypto Kennzahlen (KZ 172, 174, 176) start with tax year 2022"); Rust writes both files. The 2021 capital-gains CSV has an empty Kennzahl column in Python and 172/174 in Rust. 2025 matches | `crates/reports` (Austrian export, `full_csv.rs`, `tax_summary.rs`) |
| #706 | `ui_snapshot.build_journal_events_list_snapshot`: each event carries an `explanationReference` for `ui.reports.explain_capital_gain` | `ui.journals.events.list` | **Differs**: field missing | `crates/journals/src/views.rs` |
| #706 | `year_handoff.py`: asset presentation | `ui.reports.export_year_handoff` | Inspection: XLSX bytes differ in both runs (timestamps), contents not compared | `crates/reports` |
| #677 | `ui_snapshot.py`, `tax_estimate.py`: core tax figures | `ui.workspace.health`, `ui.next_actions` | **Differs**: `definitions` missing | `crates/workspace/src/readiness.rs`, `crates/daemon/src/workspace_reads.rs` |

Unchanged in every run: `ui.overview.snapshot` (#713 changed only the
UI), `ui.reports.tax_summary`, `ui.reports.summary`, `ui.reports.balance_sheet`,
`ui.reports.balance_history`, `ui.reports.portfolio_summary`,
`ui.reports.exit_tax_preview`, `ui.transactions.list`, `ui.transactions.history`,
`ui.transactions.graph`, `ui.wallets.utxos` and the other read kinds not named
above.

## Desktop kinds the Rust daemon does not serve

The Rust daemon advertises 260 of Python's 307 kinds (258 before S6 added
`ui.transfers.dismissals.list` and `.delete`). 42 of the 47 others are
called by `ui-tauri/src`; all 47 are in the Tauri allowlist except
`ui.connections.bullbitcoin_wallet.create`.

| Screen | Kinds |
| --- | --- |
| Reports (#706) | `ui.reports.export_pdf`, `ui.reports.export_summary_pdf`, `ui.reports.export_austrian_e1kv_pdf`, `ui.reports.export_exit_tax_pdf`, `ui.reports.export_audit_package` (PDF work is on `t3code/r5-pdf-1`) |
| Settings → Assistant, provider wizard (#673, #674, #675, #690) | `ai.providers.list`, `ai.providers.create`, `ai.providers.update`, `ai.providers.delete`, `ai.providers.set_default`, `ai.providers.set_api_key`, `ai.providers.move_api_key`, `ai.providers.acknowledge`, `ai.test_connection`, `ai.list_models`, `ai.provider_runtime.status` |
| Settings → External agents (#686) | `ui.agent_access.status`, `ui.agent_access.configure`, `ui.agent_access.lock`, `ui.agent_access.unlock`, `ui.agent_access.pairing` |
| Chat and docked right panel (#673, #682–#700, #709) | `ai.chat`, `ai.chat.cancel`, `ai.chat.permission_mode`, `ai.tool_call.consent`, `ui.chat.attachments.upload`, `ui.chat.history.configure`, `ui.chat.sessions.list`, `ui.chat.sessions.get`, `ui.chat.sessions.delete`, `ui.chat.sessions.clear`, `ui.chain_analysis.ai_context` |
| Connections | `ui.connections.btcpay.create`, `ui.connections.btcpay.discover`, `ui.connections.btcpay.key_guide`, `ui.connections.node.snapshot`, `ui.connections.bullbitcoin_wallet.create`, `ui.btcpay.provenance.review` |
| Other | `ui.reports.privacy_mirror`, `ui.source_funds.export_bundle`, `ui.source_funds.export_pdf`, `ui.transactions.ledger_template` |

Not called by the UI: `ai.providers.clear_default`, `ai.providers.get`,
`ui.btcpay.provenance.sync`, `ui.connections.btcpay.test`,
`ui.connections.sources`.

The lock screen (#704, #705), the light theme (#695) and the Overview (#713)
need no new kinds. The Overview, wallet detail (#703) and transaction panel
(#688, #701) call native kinds; their gaps are the payload rows above
(`ui.wallets.list`, `ui.transactions.dashboard`, `ui.review.badges`).

## AI catalog and tools (R6-5b, R6-8)

`crates/ai/src/data/catalog.json` on `t3code/r6-5b-ai-tools` is byte-identical
to `gen_catalog.py` over the pre-merge Python. Over the merged Python the
catalog changes as follows; both R6 branches need it regenerated when they
move onto this base.

- **15 new tools** (161 to 176), all also in `CORE_TOOL_NAMES`:
  `ui.attachments.add_from_chat`, `ui.imports.list`, `ui.imports.rollback`,
  `ui.imports.rollback.plan`, `ui.journals.quarantine.groups`,
  `ui.transactions.verify`, `ui.transfers.dismissals.list`,
  `ui.transfers.dismissals.delete`, `ui.visuals.render`,
  `ui.wallets.import_attachment.plan`, `ui.wallets.import_attachment.apply`,
  `ui.wallets.import_document.plan`, `ui.wallets.import_document.apply`,
  `ui.wallets.read_document`, `ui.wallets.request_input`. Eleven are AI-only
  kinds dispatched inside the tool loop, not daemon kinds (all but
  `ui.imports.list`, `ui.imports.rollback` and the two dismissal kinds);
  `CHAT_DOCUMENT_TOOL_NAMES` and `WALLET_IMPORT_TOOL_NAMES` are new sets.
- `CORE_TOOL_NAMES` also gains `ui.transactions.history.revert`,
  `ui.transfers.dismiss` and `ui.transfers.unpair`; `REVIEW_TOOL_NAMES` gains
  `ui.journals.quarantine.groups` and `ui.transactions.verify`.
- **Consent**: `AI_TOOL_ONCE_ONLY_CONSENT` gains `ui.imports.rollback`,
  `ui.wallets.import_attachment.apply`, `ui.wallets.import_document.apply`.
  `egresses` turns true for `ui.maintenance.run`, `ui.rates.latest`,
  `ui.rates.rebuild` and `ui.wallets.sync`, so the egress consent axis now
  applies to them.
- **Permission modes and undo** (#684, #700, #709): every `ToolEntry` has
  `reversible` (true for 19 tools, among them `ui.journals.process`,
  `ui.transfers.pair`, `ui.transfers.unpair`, `ui.transfers.dismiss`,
  `ui.transactions.metadata.update`, `ui.custody.review.apply`); `ai/undo.py`
  takes changes back. `gen_catalog.py` does not record `reversible` yet.
- **Tool areas** (#677): `load_tools`, `LOADABLE_TOOL_AREAS` (11 areas) and
  the tool-area index for small models.
- `_AI_AUTO_JOURNAL_REFRESH_TOOL_NAMES` gains `ui.journals.quarantine.groups`
  and `ui.visuals.render`.
- Changed parameters: `ui.journals.quarantine.resolve`, `ui.reports.capital_gains`,
  `ui.review.plan`, `ui.review.apply`, `ui.transfers.components.plan`,
  `ui.transfers.components.apply`, `ui.transfers.pair`, `ui.wallets.analyze_file`.
  Changed descriptions only: `ui.custody.gaps.list`, `ui.custody.gaps.review_context`,
  `ui.journals.transfers.list`, `ui.reports.exit_tax_preview`,
  `ui.reports.tax_summary`, `ui.review.badges`, `ui.transactions.resolve`,
  `ui.transfers.dismiss`, `ui.transfers.suggest`, `ui.wallets.list`.
  The skill-reference index text is unchanged; eight reference files changed.
- **MCP** (#686, R6-8): grants (`read`, `accounting_decisions`, `operator`)
  and `agent_access` (`full`, `restricted`) in `agent_access.py`; writes within
  the grant run through the assistant executor and are audited as
  `mcp_host`; client names; `mcp/tools.py` replaces `_ENTRIES` and
  `MCP_SOURCE_TOOL_NAMES` with a registry, `TOOL_CATALOG`, grant instructions
  and report-export aliases.
- **Providers** (#674, #675, #678, #714, R6-6a): `agent_access` per provider,
  stored last checks (`ai_provider_checks`), saved model inventories and
  compatibility fixes.
- **Chat** (#676, #682, #689, #693, #697, #698, R6-7): attachments and
  uploads (`daemon_chat_uploads.py`, `core/chat_files.py`), attached-file and
  document import plans (`core/attachment_import.py`, `core/document_chat_import.py`),
  chart visuals (`core/chat_visuals.py`), permission modes and inline approvals.

## Slices

In dependency order. ★ marks the slices the new desktop screens need. Sizes
are Rust lines including tests: S below 300, M 300 to 1,000, L 1,000 to 2,500,
XL above.

1. ★ **S1 Store schema and command registry** (M). **Done**; see [after S1](#after-s1). The four schema changes
   above, in creation and the compatibility pass, with a fixture of a book the
   merged Python created; the `transfers_dismissals_command` kind attribute,
   `describe.rs` (four leaves, `mcp call` effect, new arguments) and
   `capabilities.rs`. It fixes the five failing shell tests
   (`argparse_parity`, three `describe_drift` tests,
   `capabilities::tables_match_python`). Everything else needs it: until it
   lands the Rust app cannot open a book the current Python app has opened.
2. ★ **S2 Qualified transaction references** (M). **Done**; see
   [after S2, S6 and S11](#after-s2-s6-and-s11). One resolver following
   `core/transaction_lookup.py`, replacing the eight copies. Needs S1.
3. ★ **S3 Coarse-price acceptance** (M). The request builder's accepted-price
   rule, the acceptance column on price edits and history, the history labels.
   Needs S1.
4. ★ **S4 Review workflow** (L). Term operations, `accept_coarse_prices`,
   the per-year tax delta in plan, apply and receipt, `fiat_currency` on
   cases, `review` and `definitions` on badges, automatic pairing before
   journals in freshness. Needs S2 and S3, and S6's dismissal storage for
   `dismiss_pair`.
5. ★ **S5 Quarantine root causes** (L). Groups, attention counts,
   definitions, `journals quarantine groups`, resolve by group with
   `reprocess`. Needs S1.
6. ★ **S6 Transfer dismissals** (M). **Done**. Stale journals on dismiss,
   list and delete leaves and kinds, suggestion exclusions. Needs S2.
7. ★ **S7 Reports screen** (L). The capital-gains payload and indicative tax
   (including the empty-root `jurisdictionCode` and country fallback),
   the E 1kv refusal before 2022 and the empty Kennzahl before 1 March 2022
   in the exports, `explanationReference` on journal events, the year handoff
   asset presentation. Independent of S2 to S6; it fixes a tax-relevant
   difference, so it can go first after S1.
8. ★ **S8 Wallet completeness and verification** (M). `completeness_class`
   and `scan_bounds` on `ui.wallets.list`, `transactions verify`. Needs S1.
9. ★ **S9 Transaction dashboard** (S). Bucket size and the years index.
10. **S10 Readiness definitions** (S). `definitions` on `ui.workspace.health`
    and `ui.next_actions`.
11. **S11 Custody component revision** (S). **Done**. Activation remediation
    and the planner changes. Needs S2.
12. ★ **S12 R6 rebase** (XL, on the R6 branches). Regenerate the AI catalog
    and add `reversible`; then providers (R6-6a: `agent_access`, checks,
    inventories), the tool executor (R6-5b: new tools, consent, refresh sets,
    tool areas, undo), MCP grants (R6-8) and chat (R6-7: attachments, uploads,
    permission modes, visuals, document import). The Settings and chat
    screens need these kinds.

## After S1

The Rust store now runs the four schema steps where Python does (see
[database compatibility](database-compatibility.md#storage-added-by-main-675-to-708)),
and the registry, `describe` and capability tables follow main. The five
shell tests pass.

`desktop_read_diff.py` on the Python-built `austria` book, release binary,
the default 103 read kinds plus the seven report reads and
`ui.review.plan/worklist/receipt` (113 kinds, no `--args-file`): 101 the
same, 12 differing, none refused. Before S1, 99 of the 103 default kinds were
refused. The 12, each with its slice:

| Kind | First difference | Slice |
| --- | --- | --- |
| `ui.review.cases` | `fiat_currency` missing | S4 |
| `ui.review.badges` | `definitions` missing | S4 |
| `ui.journals.quarantine` | `summary.definitions` missing | S5 |
| `ui.transfers.suggest` | `exclusions` missing | S6 (**done**) |
| `ui.reports.capital_gains` | `asset` and the Reports payload | S7 |
| `ui.journals.events.list` | `explanationReference` missing | S7 |
| `ui.wallets.list` | `completeness_class` missing | S8 |
| `ui.workspace.health` | `definitions` missing | S10 |
| `ui.next_actions` | `health.definitions` missing | S10 |
| `ui.journals.snapshot` | `freshnessReason` wording | pre-merge; fixed on `t3code/r6-5b-ai-tools` |
| `ui.chain_analysis.query` | `snapshot_id` | pre-merge, not investigated |
| `ui.logs.snapshot` | each side's own log lines | expected |

Verification of S1: `kassiber-store` (72 tests), the shell crates
(`kassiber-cli`, `-runtime`, `-envelope`, `-daemon`, every test binary)
and the store comparisons in `tests/test_rust_store_migrations.py` pass:
the four historical fixtures, the new `pre_sync_bae01537b` among them, give
identical first-pass dumps and ledgers in both runtimes, plaintext and
SQLCipher. The store mutation catalog (ten entries) is fully killed. The
preflight shard passes. Not rerun for S1: the other workspace crates and
the remaining Python lanes.

The comparison is plaintext: the harness does not unlock SQLCipher books.
Encrypted admission is covered by the store differential tests, which open a
current Python SQLCipher book in both Rust open modes without changing it.

## After S2, S6 and S11

**S2.** `crates/transactions/src/lookup.rs` ports `core/transaction_lookup.py`
and replaces nine resolver copies: the eight listed above and the component
planner's (`crates/transfers/src/component_review.rs`, which keeps its own
`not_found` wording). References are stripped and stringified as
`str(reference or "")` does, so a falsy non-string reads as empty, and a
string with surrogates fails at the bind as in Python. `ui.transactions.resolve`
falls back to the qualified lookup and adds `sharedExternalIdCount` and
`sharedExternalIdMatches`, which the inventory did not list;
`ui.transactions.review_context` resolves through it. Transaction graph
nodes in a foreign asset carry `assetId` (#679). The shared
`details.remediation` shape is `kassiber_envelope::remediation`.

**S6.** `transfers dismiss` and `ui.transfers.dismiss` mark journals stale.
`transfers dismissals list|delete` and `ui.transfers.dismissals.list|delete`
are native (`crates/transfers/src/dismissals.rs`): the list applies the
matcher's expiry rule (an expiry at or before now lifts the veto, an
unreadable one never does) and validates `limit` and, in the daemon,
`include_expired` before the scope as Python does; delete takes an id or two
(qualified) legs. `ui.transfers.suggest`, `transfers suggest` and
`ui.transfers.review_context` report `exclusions`.

**S11.** Membership conflicts carry `revise_conflicting_component`
remediation in the custody stage, plans, revisions and activations; a second
draft carries `revise_existing_draft`, a batch anchoring one transaction
twice `merge_planned_components`, and non-activatable and incomplete
refusals their issues' remediation. `review_field_order.json` and the
validation projection order the new fields. The AI-only revision boundary
(`_validate_ai_custody_state_boundary`) runs only for `authored_source ==
"ai_tool"`, which the R6 tool executor owns (S12).

### Verification

- Rust vectors from CPython: `gen_lookup_vectors.py` (64 references, seven
  modes each, and 12 `ui.transactions.resolve` snapshots),
  `export_dismissals.py` (dismiss, list, delete, suggest and review context
  with injected clock and ids, plus 15 exclusion-count cases), the regenerated
  `components.json` and `pairs.json`, the five changed custody stage vectors
  in `tests/fixtures/custody_component_cases.json`, the `resolve` group of
  `desktop-vectors.json` and the dismiss rows of `authoring.json`.
- Differential process tests on disposable roots, plaintext and unlocked
  SQLCipher, all 113 tables: `tests/test_rust_transaction_references.py`,
  `tests/test_rust_transfer_dismissals.py` and
  `tests/test_rust_component_remediation.py` (14 tests, all pass) over the
  shared `tests/rust_port/sync_parity.py`.
- Mutation catalogs, every entry killed: transactions 20/20, transfers
  23/23, custody 6/6, journals 1/1 (`sync_mutation_check.py` in each
  crate's `scripts/`). One S2 candidate, the `len(shared) > 1` guard on
  `sharedExternalIdMatches`, is equivalent (a one-row txid is the shown row,
  which the filter already drops) and has no entry.
- Crate tests: `kassiber-transactions` (lookup, desktop), `kassiber-custody`
  (all), `kassiber-transfers` (all but the two binaries below), the
  `kassiber-journals` graph unit tests, `kassiber-cli` and `kassiber-daemon`
  (all); clippy with `-D warnings` on every touched crate.
- No new egress: every change is local SQL, so no consent regression was
  added.
- Not run for these slices: the other workspace crates' tests, the full
  Python lanes and the loopback tests. `cargo fmt --check` and the preflight
  shard (57 tests) pass.

### Desktop read diff

`desktop_read_diff.py` on the Python-built `austria` book (built with this
checkout's Python CLI by `report_design_books.py`'s build steps), release
binaries of 6159f8f39 (before) and of this branch (after):

| Set | Before: same, differing | After: same, differing |
| --- | --- | --- |
| default 103 read kinds, the seven report reads, `ui.review.plan/worklist/receipt` (113) | 101, 12 | 102, 11 |
| S2/S6/S11 kinds with shared-txid arguments (7) | 0, 7 | 7, 0 |

The default set's one change is `ui.transfers.suggest` (`exclusions`); the
other 11 are the S1 list's other slices and pre-merge differences. The
targeted set ran `ui.transactions.resolve` on a txid the transfer legs share,
`ui.transactions.review_context` with `<txid>:in`,
`ui.transfers.components.list` with `<txid>:out`, `ui.audit.evidence.summary`
with `<txid>:<wallet label>`, `ui.transfers.suggest`,
`ui.transfers.dismissals.list` (not native before) and
`ui.transfers.review_context`. The Rust daemon now advertises 260 kinds.

### Found while porting

- `tests/rust_desktop_parity.py` asserted 112 tables; S1's
  `ai_provider_checks` makes 113, so every `DesktopParity` test failed before
  comparing. Fixed. Its encrypted sessions also compared only the
  `auth_required` answers; the main-sync harness unlocks first.
- `crates/transfers/tests/component_reads.rs` (3 tests) and
  `pairing_services.rs` (7) fail at 6159f8f39 already: their frozen SQL has 47
  transaction values for S1's 48 columns. Regenerating them from current
  Python also brings in #684's result changes that Rust lacks (`transfers
  rules delete` returns the deleted `rule`, `transfers unpair` the removed
  `pair`) and #711's auto-pair `ownership` counts, so it is left to a slice
  that ports those. `authoring.json` and `pairs.json` keep their pre-#684
  `rules delete` and `unpair` results for the same reason, so rerunning their
  exporters reproduces those differences.
- `ui.transfers.review_context` refreshes stale journals before it reads, in
  both runtimes; it is not a read-only kind.
- `tests/custody_component_vectors.py` regenerates inputs with S3's
  `pricing_review_accepted_price`; only the five changed outputs were taken.

## Verification of the merge

- `cargo build --release -p kassiber`, both clippy lanes with `-D warnings`,
  workspace `cargo fmt --check` and the desktop shell's, and the `ui-tauri`
  typecheck pass.
- The preflight shard passes (57 tests, 69 subtests).
- `gen_ledger_fixtures.py` regenerates the case-merged ledger goldens byte for
  byte; `kassiber-tax-core` and `kassiber-custody` pass 787 tests and the
  journals `fusion_parity` test passes 4.
- `kassiber-cli`, `kassiber-daemon`, `kassiber-runtime` and `kassiber-envelope`
  tests fail five against the regenerated fixtures, all S1: `argparse_parity`,
  three `describe_drift` tests and `capabilities::tables_match_python`.
  The other workspace tests and the Rust-candidate Python lanes were not run
  for this sync.

## Python reference notes

Python's `open_db` opens a book carrying tables, columns, indexes,
triggers or migration audits it does not know, leaving them in place. The
Rust store refuses such a book in both opens without writing. S1 keeps that
asymmetry deliberately; see [database compatibility](database-compatibility.md#storage-added-by-main-675-to-708).

No Python behaviour was changed by this sync. The pre-merge
`freshnessReason` difference and the `ui.chain_analysis.query` `snapshot_id`
difference predate the merge.
