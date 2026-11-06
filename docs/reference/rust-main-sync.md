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
| #679 | `core/transaction_lookup.py`, `handlers.resolve_transaction`: a reference resolves by id, then an unambiguous external id, then a qualified `<external id>:<wallet or in/out>`; ambiguity lists every candidate | every leaf and kind taking a transaction reference | **Differs**: `metadata notes set --transaction <txid>:in` succeeds in Python, Rust answers "not found"; the bare shared txid's `ambiguous_reference` hint differs | eight resolver copies: `crates/transactions/src/{metadata,attachments}.rs`, `crates/transfers/src/{dismissals,pair_authoring,component_reads}.rs`, `crates/reports/src/source_funds_reads.rs`, `crates/journals/src/evidence_summary.rs`, `crates/kassiber/src/real_store/journals.rs` |
| #680 | `review_workflow.py`: plans carry journal-meaning term operations (`pair`, `dismiss_pair`, `undismiss_pair`, `custody_gap`) and a per-year tax `delta`; apply checks the delta; receipts keep it | `ui.review.plan`, `ui.review.apply`, `ui.review.receipt`, `ui.review.cases` | **Refused**: a `pair` or `dismiss_pair` plan returns a plan in Python and "Review operation type is unsupported" in Rust. No Rust artifact carries `delta`, so an artifact either side makes is refused by the other | `crates/transfers/src/review_operations.rs`, `crates/daemon/src/review.rs`, `crates/journals/src/review_effects.rs` |
| #708 | `pricing.reviewed_price`, `metadata.py`, `transaction_history.py`, `ledger_request._coarse_needs_review`: an accepted coarse price is no longer held for review while the price is unchanged; a price edit drops the acceptance; history shows it | `ui.review.plan/apply` (`accept_coarse_prices`), `ui.transactions.metadata.update`, `ui.transactions.history`, `journals process` | **Refused**: `accept_coarse_prices` is an unsupported operation in Rust (Python: `review_case_changed`, nothing waiting). Rust's request builder sends `coarse` for every `coarse_fallback` row (`crates/custody/src/ledger_request.rs:654`), so after S1 an accepted book would quarantine again under Rust. Today such a book is refused by S1 | `crates/custody/src/ledger_request.rs`, `crates/journals/src/prebuild/pricing.rs`, `crates/transactions/src/metadata/history.rs` |
| #707, #711, bd162caca | `review_workflow.py`, `ui_snapshot.py`, `daemon_freshness.py`: cases name `fiat_currency`; badges add `review` and `definitions`; freshness pairs automatically before journals | `ui.review.cases`, `ui.review.badges`, `ui.freshness.run`, `ui.workspace.freshness.run` | **Differs**: `ui.review.cases` lacks `fiat_currency`; `ui.review.badges` lacks `review` and `definitions` (fresh root too). Freshness: inspection | `crates/daemon/src/review.rs`, `crates/daemon/src/workspace_reads.rs`, `crates/wallets` (freshness) |
| #681 | `quarantine_review.py` (+453): quarantine rolled up by root cause, groups, attention count, definitions; resolving by `group_id` with `reprocess` | `ui.journals.quarantine`, `journals quarantine groups`, `ui.journals.quarantine.resolve`, `ui.review.badges`, AI kind `ui.journals.quarantine.groups` | **Differs**: summary lacks `definitions` and `fiat_currency`; resolve with `group_id` is refused as an unsupported field (Python validates it). The books have no quarantine, so group rows were not compared | `crates/journals/src/quarantine.rs`, `crates/journals/src/views.rs`, `crates/daemon/src/journals.rs` |
| #681 | `wallet_completeness.py`, `transaction_verify.py` | `ui.wallets.list`, `transactions verify`, AI kind `ui.transactions.verify` | **Differs**: wallets lack `completeness_class` and `scan_bounds`; `transactions verify` is not native | `crates/daemon/src/wallets.rs`, `crates/wallets`, `crates/transactions` |
| #679, #683 | `custody_review_terms.dismiss_pair_candidate`, `transfer_matching.active_dismissed_pairs`, `handlers.suggest_transfer_candidates`: a dismissal marks journals stale, can be listed and deleted, and suggestions report exclusions (active pair, already journaled moves, dismissal) | `ui.transfers.dismiss`, `transfers dismiss`, `ui.transfers.suggest`, `ui.transfers.dismissals.list/delete`, `transfers dismissals list/delete` | **Differs**: after `transfers dismiss` Python bumps `journal_input_version`, Rust does not; `ui.transfers.suggest` lacks `exclusions`; list/delete kinds and leaves are not native | `crates/transfers/src/dismissals.rs`, `crates/transfers/src/pairing/service.rs`, `crates/daemon/src/transfers.rs` |
| #679 | `custody_components.py`, `custody_component_planner.py`: revision planning and activation errors with remediation; qualified references | `ui.transfers.components.plan/apply`, review `custody_component` operations | Inspection: a revise plan for an unknown component answers the same; activation remediation and qualified references follow S2 | `crates/transfers` (components), `crates/custody` |
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

The Rust daemon advertises 258 of Python's 307 kinds. 43 of the 49 others
are called by `ui-tauri/src`; all 49 are in the Tauri allowlist except
`ui.connections.bullbitcoin_wallet.create` and `ui.transfers.dismissals.list`.

| Screen | Kinds |
| --- | --- |
| Review (#707, #711) | `ui.transfers.dismissals.delete` (undo a dismissal) |
| Reports (#706) | `ui.reports.export_pdf`, `ui.reports.export_summary_pdf`, `ui.reports.export_austrian_e1kv_pdf`, `ui.reports.export_exit_tax_pdf`, `ui.reports.export_audit_package` (PDF work is on `t3code/r5-pdf-1`) |
| Settings → Assistant, provider wizard (#673, #674, #675, #690) | `ai.providers.list`, `ai.providers.create`, `ai.providers.update`, `ai.providers.delete`, `ai.providers.set_default`, `ai.providers.set_api_key`, `ai.providers.move_api_key`, `ai.providers.acknowledge`, `ai.test_connection`, `ai.list_models`, `ai.provider_runtime.status` |
| Settings → External agents (#686) | `ui.agent_access.status`, `ui.agent_access.configure`, `ui.agent_access.lock`, `ui.agent_access.unlock`, `ui.agent_access.pairing` |
| Chat and docked right panel (#673, #682–#700, #709) | `ai.chat`, `ai.chat.cancel`, `ai.chat.permission_mode`, `ai.tool_call.consent`, `ui.chat.attachments.upload`, `ui.chat.history.configure`, `ui.chat.sessions.list`, `ui.chat.sessions.get`, `ui.chat.sessions.delete`, `ui.chat.sessions.clear`, `ui.chain_analysis.ai_context` |
| Connections | `ui.connections.btcpay.create`, `ui.connections.btcpay.discover`, `ui.connections.btcpay.key_guide`, `ui.connections.node.snapshot`, `ui.connections.bullbitcoin_wallet.create`, `ui.btcpay.provenance.review` |
| Other | `ui.reports.privacy_mirror`, `ui.source_funds.export_bundle`, `ui.source_funds.export_pdf`, `ui.transactions.ledger_template` |

Not called by the UI: `ai.providers.clear_default`, `ai.providers.get`,
`ui.btcpay.provenance.sync`, `ui.connections.btcpay.test`,
`ui.connections.sources`, `ui.transfers.dismissals.list` (the AI tool reads it).

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
2. ★ **S2 Qualified transaction references** (M). One resolver following
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
6. ★ **S6 Transfer dismissals** (M). Stale journals on dismiss, list and
   delete leaves and kinds, suggestion exclusions. Needs S2.
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
11. **S11 Custody component revision** (S). Activation remediation and the
    planner changes. Needs S2.
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
| `ui.transfers.suggest` | `exclusions` missing | S6 |
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

S1b brought the other crates' fixtures up to that schema. On 6159f8f39,
running each crate's test binaries one cargo command at a time (178 targets)
failed 139 tests in 35 targets, all from S1 except the egress chokepoint:

- Whole-book comparisons asserted 112 tables, but current books have 113.
  The kassiber privacy and PSBT tests, the reports lightning, platform CSV
  and source-funds tests, and the Python harnesses and generators behind
  them now compare table names with the store manifest
  (`kassiber_store::table_names()`, `tests/rust_port/book_schema.py`).
  Checks over the frozen `privacy_hygiene_seed.sql` book keep 112.
- 17 test binaries in journals, wallets, transfers, setup and reports
  rebuild their books from the manifest. They failed against Python dumps
  and `iterdump()` seeds cut from the old schema: 47 values for 48
  transaction columns, or no `ai_provider_checks`, `agent_access` or
  `pricing_review_accepted_price`. The journals `crash.rs` test
  `killed_writes_recover_complete_stale_books_on_sqlite_and_sqlcipher` is
  one of them, and it passes on a0b4d0342.
  `tests/rust_port/upgrade_schema_dumps.py` upgrades such vectors the way
  Python's compatibility pass upgrades a book: the declared added columns
  read their defaults and whole-book dumps gain the empty table. It refuses
  anything else. Regenerating these vectors would also take in behaviour
  other slices port: S2's qualified references in `component_reads.json`
  and `source_funds_passes.json`, and S3's custody record fields in the
  journal oracles, along with those generators' clock and UUID churn.
  `pairing_services.json` would take in S6's suggestion `exclusions` and
  #707's `ownership` candidate count. The inventory doesn't list the second
  yet: `daemon_freshness._review_move_count` leaves ownership-graph moves
  out of the cached `profiles.swap_candidate_count`, which drops from 2 to 1
  in 72 stored rows.
  On every schema column the generators agree with the upgrade, and the
  wallets generators agree byte for byte. Run the tool only on vectors whose
  tests replay into a manifest-built book; `loader.json` and the transfer
  review vectors carry their own schema.
- `lightning.json` and `python_networks.json` are regenerated. Apart from
  the schema, only the partition plans' `authored_digest` and `plan_id`
  change. They hash the wider transaction rows, and Rust computes the same
  new values.
- The chokepoint refused two journals files that open no connection: an
  IPv4 literal parse and a comment naming the egress crate. Both came with
  2e263cbcd, before S1.

Every workspace crate's test binaries then pass (178 targets, 2259 tests),
including the owned-loopback tests in chain, net, rates and wallets, which
this machine allows.

`DesktopParity`'s SQLCipher variants never unlocked the book, so the 29
encrypted tests only ever compared two `auth_required` refusals.
`compare()` now sends `daemon.unlock` first unless the session already does,
as `SyncParity` on the S2/S6/S11 branch and the wallet onboarding and reveal
tests do. 17 of the 29 then pass. The other 12 fail on the same differences
their plaintext twins show (41 plaintext failures in those files, debug
harness binary): `definitions` (S10), the capital-gains payload (S7),
`fiat_currency` and the plan's `tax` delta (S4, which also makes Rust refuse
Python's apply artifact), the dashboard `years` (S9), transfer `exclusions`
(S6), and an `assetId` beside the label of foreign-asset legs in
`ui.transactions.graph`. The last one came with #679
(`transaction_graph._apply_foreign_asset`) and no slice lists it yet.

The comparison is plaintext: the harness does not unlock SQLCipher books.
Encrypted admission is covered by the store differential tests, which open a
current Python SQLCipher book in both Rust open modes without changing it.

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
