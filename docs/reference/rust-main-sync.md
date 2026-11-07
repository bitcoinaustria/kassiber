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
| #679, #681 | new CLI leaves `journals quarantine groups`, `transactions verify`, `transfers dismissals list`, `transfers dismissals delete`; kind derivation for `transfers_dismissals_command` | the four leaves | Kind derivation **done (S1)**. `journals quarantine groups` **done (S5)**, `transactions verify` **done (S8)**; the dismissals leaves stay `not_implemented` until S6 | `crates/envelope/src/lib.rs` (subcommand attrs), `crates/cli` |
| #678, #684, #686 | changed arguments: `mcp enable` (`--grant`, `--agent-access`), `mcp call` (`--grant`, `--client`, effect now `mutating`), `chat` (`--permission-mode`, file options), `ai providers create/update` (`--agent-access`), `journals quarantine resolve price-override/exclude` (`--group-id`, `--reprocess`) | those leaves | `describe` **done (S1)**. None of these leaves is native | `crates/cli/src/describe.rs` |
| #679, #681, #684, #689 | `command_capabilities.py`: four CLI leaves and four daemon kinds | `ui.transfers.dismissals.list/delete`, `ui.chat.attachments.upload`, `ai.chat.permission_mode` | **Done (S1)**: 317 entries each. The kinds stay `not_implemented` until S6 and S12 | `crates/runtime/src/capabilities.rs` |

### Transactions, review and journals

| PR | Python | Kinds and leaves | Rust state | Rust location |
| --- | --- | --- | --- | --- |
| #679 | `core/transaction_lookup.py`, `handlers.resolve_transaction`: a reference resolves by id, then an unambiguous external id, then a qualified `<external id>:<wallet or in/out>`; ambiguity lists every candidate | every leaf and kind taking a transaction reference | **Done (S2)**; see [after S2, S6 and S11](#after-s2-s6-and-s11). Before: `metadata notes set --transaction <txid>:in` succeeded in Python and Rust answered "not found"; the bare shared txid's `ambiguous_reference` hint differed | `crates/transactions/src/lookup.rs`, replacing nine copies (the eight listed before plus the planner's in `crates/transfers/src/component_review.rs`); `ui.transactions.resolve` in `crates/transactions/src/desktop/resolve.rs` |
| #680 | `review_workflow.py`: plans carry journal-meaning term operations (`pair`, `dismiss_pair`, `undismiss_pair`, `custody_gap`) and a per-year tax `delta`; apply checks the delta; receipts keep it | `ui.review.plan`, `ui.review.apply`, `ui.review.receipt`, `ui.review.cases` | **Refused**: a `pair` or `dismiss_pair` plan returns a plan in Python and "Review operation type is unsupported" in Rust. No Rust artifact carries `delta`, so an artifact either side makes is refused by the other | `crates/transfers/src/review_operations.rs`, `crates/daemon/src/review.rs`, `crates/journals/src/review_effects.rs` |
| #708 | `pricing.reviewed_price`, `metadata.py`, `transaction_history.py`, `ledger_request._coarse_needs_review`, `custody_tax_projection._base_row`: an accepted coarse price is no longer held for review while the price the journals read is unchanged (compared by decimal value); a changed price is held again; projected rows carry their transaction's reviewed price; history shows the acceptance | `ui.review.plan/apply` (`accept_coarse_prices`), `ui.transactions.history`, `ui.transactions.history.revert`, `ui.activity.stale`, `journals process` | **Done (S3)** for the request builder, the projection, the metadata service and history; see [after S3 and S5](#after-s3-and-s5). `accept_coarse_prices` stays an unsupported review operation until S4, which calls the ported `accept_coarse_price` | `crates/custody/src/coarse_review.rs`, `crates/custody/src/ledger_request.rs` (`coarse_needs_review`), `crates/custody/src/finalized_projection.rs`, `crates/transactions/src/metadata/{grouped,history}.rs` |
| #707, #711, bd162caca | `review_workflow.py`, `ui_snapshot.py`, `daemon_freshness.py`: cases name `fiat_currency`; badges add `review` and `definitions`; freshness pairs automatically before journals | `ui.review.cases`, `ui.review.badges`, `ui.freshness.run`, `ui.workspace.freshness.run` | **Differs**: `ui.review.cases` lacks `fiat_currency`; `ui.review.badges` lacks `review` and `definitions` (fresh root too). Freshness: inspection | `crates/daemon/src/review.rs`, `crates/daemon/src/workspace_reads.rs`, `crates/wallets` (freshness) |
| #681 | `quarantine_review.py` (+453): quarantine rolled up by root cause, groups, attention count, definitions; resolving by `group_id` with `reprocess` | `ui.journals.quarantine`, `journals quarantine groups`, `ui.journals.quarantine.resolve`, `ui.review.badges`, AI kind `ui.journals.quarantine.groups` | **Done (S5)** for the summary (`definitions`, `fiat_currency`), the groups core and its CLI leaf, and resolve by `group_id` with the measured or unmeasured effect; see [after S3 and S5](#after-s3-and-s5). The AI-only kind `ui.journals.quarantine.groups` follows with S12; the CLI leaves `journals quarantine resolve price-override/exclude` were not native before and stay with Python; `ui.review.badges` is S4 | `crates/journals/src/quarantine.rs`, `crates/journals/src/quarantine/groups.rs`, `crates/kassiber/src/real_store/journals.rs`, `crates/cli/src/run.rs` |
| #681 | `wallet_completeness.py`, `transaction_verify.py` | `ui.wallets.list`, `wallets list`, `transactions verify`, AI kind `ui.transactions.verify` | **Done (S8)**; see [after S8 to S10](#after-s8-to-s10). Before: wallets lacked `completeness_class` and `scan_bounds`; `transactions verify` was not native. The AI-only kind follows with S12 | `crates/wallets/src/completeness.rs`, `crates/journals/src/transaction_graph/verify.rs` |
| #679, #683 | `custody_review_terms.dismiss_pair_candidate`, `transfer_matching.active_dismissed_pairs`, `handlers.suggest_transfer_candidates`: a dismissal marks journals stale, can be listed and deleted, and suggestions report exclusions (active pair, already journaled moves, dismissal) | `ui.transfers.dismiss`, `transfers dismiss`, `ui.transfers.suggest`, `ui.transfers.dismissals.list/delete`, `transfers dismissals list/delete` | **Done (S6)**. Before: after `transfers dismiss` Python bumped `journal_input_version` and Rust did not; `ui.transfers.suggest` lacked `exclusions`; the list/delete kinds and leaves were not native | `crates/transfers/src/dismissals.rs`, `crates/transfers/src/pairing/{matcher,service}.rs`, `crates/transfers/src/review_context.rs`, `crates/daemon/src/transfers.rs` |
| #679 | `custody_components.py`, `custody_component_planner.py`: revision planning and activation errors with remediation; qualified references | `ui.transfers.components.plan/apply`, review `custody_component` operations | **Done (S11)**, except the AI-only revision boundary (`_validate_ai_custody_state_boundary`, `authored_source == "ai_tool"`), which belongs to the R6 tool executor (S12) | `crates/custody/src/components/remediation.rs`, `crates/transfers/src/component_{review,rows,store}.rs` |
| #701 | `ui_snapshot.build_transactions_dashboard_snapshot`: automatic bucket size and a `history.years` index for the period picker | `ui.transactions.dashboard` | **Done (S9)**. Before: lacked `bucketSize` and `history.years` (fresh root too), refused the four-year range and auto-selected the old ranges | `crates/transactions/src/desktop/dashboard.rs` |
| #682, #698, #714 | `document_import.py`: the read step needs no database and records the detected API format; chat imports reuse it | `ui.wallets.document_import.preview/import` | Inspection: needs an AI provider, not exercised | `crates/imports` (document core) |

### Reports

| PR | Python | Kinds and leaves | Rust state | Rust location |
| --- | --- | --- | --- | --- |
| #706 | `ui_snapshot.build_capital_gains_snapshot`, `tax_estimate.py`: the Reports screen payload: `breakdown` by Kennzahl, `indicativeTax`, `e1kvApplies`, `rowsBeforeCryptoKennzahlen`, `fiatCurrency`, `longTermDays`, `totals`, lots with asset, category, entry type, Kennzahl and exact quantity. A root with no book, or a missing profile, reports `jurisdictionCode` `GENERIC`, and a profile without a country falls back to `generic`; Rust hard-coded `AT` in the empty payload (`gains_desktop.rs` `empty`) and fell back to `AT`. Not a store or setup default, so S1 left it to S7 | `ui.reports.capital_gains` | **Done (S7)**. Before: differed on every book, with and without `year` | `crates/reports/src/gains_desktop.rs` |
| #706 | `austrian.py`, `reports.py`: E 1kv crypto Kennzahlen start in 2022; earlier disposals state no Kennzahl | `ui.reports.export_austrian_e1kv_csv/xlsx`, `reports export-austrian-e1kv*`, `ui.reports.export_capital_gains_csv` | **Done (S7)**. Before, tax-relevant: for 2021 Python refuses the E 1kv export ("The Austrian crypto Kennzahlen (KZ 172, 174, 176) start with tax year 2022") and a 2022 with January or February crypto rows; Rust wrote both files. The 2021 capital-gains CSV has an empty Kennzahl column in Python and had 172/174 in Rust. 2025 matched | `crates/reports` (Austrian export, `full_csv.rs`, `tax_summary.rs`) |
| #706 | `ui_snapshot.build_journal_events_list_snapshot`: each event carries an `explanationReference` for `ui.reports.explain_capital_gain` | `ui.journals.events.list` | **Done (S7)**. Before: field missing | `crates/journals/src/views.rs` |
| #706 | `year_handoff.py`: the journal sheet's `tax_form_field` states no crypto Kennzahl before 1 March 2022 (the only change there; asset presentation was already ported) | `ui.reports.export_year_handoff`, `reports export-year-handoff` | **Done (S7)**. Before: Rust wrote 174 for such rows. XLSX bytes differ in both runs (timestamps), so the contents are compared | `crates/reports/src/year_handoff.rs` |
| #706 | `exit_tax.py`: a departure before 1 March 2022 has no 27.5 % rate, no estimated tax, no KZ 174/176 and the EXIT-011 assumption | `ui.reports.exit_tax_preview`, `reports exit-tax`, the exit-tax exports | **Done (S7)**. Not in the first inventory: the books' default departure is in 2025 | `crates/reports/src/exit_tax.rs` |
| #677 | `ui_snapshot.py`, `tax_estimate.py`: core tax figures | `ui.workspace.health`, `ui.next_actions`, `health`, `next-actions` | **Done (S10)**. Before: `definitions` missing | `crates/workspace/src/readiness.rs` |

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
3. ★ **S3 Coarse-price acceptance** (M). **Done**; see [after S3 and S5](#after-s3-and-s5).
   The request builder's accepted-price rule, the acceptance column on price
   edits and history, the history labels. Needs S1.
4. ★ **S4 Review workflow** (L). Term operations, `accept_coarse_prices`,
   the per-year tax delta in plan, apply and receipt, `fiat_currency` on
   cases, `review` and `definitions` on badges, automatic pairing before
   journals in freshness. Needs S2 and S3, and S6's dismissal storage for
   `dismiss_pair`.
5. ★ **S5 Quarantine root causes** (L). **Done**; see [after S3 and S5](#after-s3-and-s5).
   Groups, attention counts, definitions, `journals quarantine groups`,
   resolve by group with `reprocess`. Needs S1.
6. ★ **S6 Transfer dismissals** (M). **Done**. Stale journals on dismiss,
   list and delete leaves and kinds, suggestion exclusions. Needs S2.
7. ★ **S7 Reports screen** (L). **Done**; see [after S7](#after-s7). The capital-gains payload and indicative tax
   (including the empty-root `jurisdictionCode` and country fallback),
   the E 1kv refusal before 2022 and the empty Kennzahl before 1 March 2022
   in the exports, `explanationReference` on journal events, the year handoff
   asset presentation. Independent of S2 to S6; it fixes a tax-relevant
   difference, so it can go first after S1.
8. ★ **S8 Wallet completeness and verification** (M). **Done**; see
   [after S8 to S10](#after-s8-to-s10). `completeness_class`
   and `scan_bounds` on `ui.wallets.list`, `transactions verify`. Needs S1.
9. ★ **S9 Transaction dashboard** (S). **Done**. Bucket size and the years index.
10. **S10 Readiness definitions** (S). **Done**. `definitions` on `ui.workspace.health`
    and `ui.next_actions`, and on `ui.report.blockers` through its embedded
    health.
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
| `ui.reports.capital_gains` | `asset` and the Reports payload | S7 (done) |
| `ui.journals.events.list` | `explanationReference` missing | S7 (done) |
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

## After S8 to S10

S8 (#681), S9 (#701) and S10 (#677, #681) are native:

- `wallets list` and `ui.wallets.list` append `completeness_class` and
  `scan_bounds` from one derivation, `crates/wallets/src/completeness.rs`.
  `wallets list` now fails a stored configuration that is not a JSON object
  (`[1,2]`, `null`, an undecodable string) with Python's exception text, as
  the leaf it extends does in Python.
- `transactions verify` runs `crates/journals/src/transaction_graph/verify.rs`
  over the graph read's local-only enrichment, parsing and owned index; it
  never contacts the network and never writes. The AI-only
  `ui.transactions.verify` tool comes with S12 and can call the same function.
- `ui.transactions.dashboard` reports `bucketSize` and `history.years`,
  accepts `4years`/`4year`/`4y`, groups "all" over ten or more years by year
  and auto-selects among `1year`, `4years`, `10years` and `all`.
- `ui.workspace.health` carries `definitions`, and with it `health`,
  `next-actions`, `ui.next_actions` and the `health` inside
  `ui.report.blockers`.

`desktop_read_diff.py` on the Python-built `austria` book (built with
`report_design_books.py`'s `build` from `t3code/r5-pdf-1` and this checkout's
Python CLI), debug harness binaries of 6159f8f39 and of this slice:

| Run | Before | After |
| --- | --- | --- |
| Default 103 read kinds plus the seven report reads and `ui.review.plan/worklist/receipt` (113 kinds) | 101 same, 12 differing | 104 same, 9 differing |
| `ui.transactions.dashboard` with `{"period": "all", "resolveAuto": true}`, `ui.wallets.list`, `ui.workspace.health`, `ui.next_actions` | 4 differing (`bucketSize`, `completeness_class`, `definitions`, `health.definitions`) | 4 same |

The default run sends the dashboard no arguments, so both sides refuse its
`1year` default the same way; the targeted run is what shows S9. The nine
still differing are the other slices' kinds and the known ones:
`ui.review.cases`, `ui.review.badges` (S4), `ui.journals.quarantine` (S5),
`ui.transfers.suggest` (S6), `ui.reports.capital_gains`,
`ui.journals.events.list` (S7), `ui.journals.snapshot`,
`ui.chain_analysis.query` and `ui.logs.snapshot`.

Verification: `tests/test_rust_wallet_completeness.py` (7 tests: daemon and
CLI `wallets list` in JSON, table, plain and CSV, malformed sources,
`transactions verify` over stored, reference, Liquid and graphless rows,
plaintext and SQLCipher, all 113 tables unchanged) and the dashboard tests
(`tests/test_rust_transaction_dashboard.py`, 8) pass. `tests/test_rust_workspace_shell.py`
passes except its sessions that include `ui.review.badges`, which differ
only there (S4). The workspace vectors replay (5), `kassiber-transactions`,
`kassiber-journals --lib`, `kassiber-cli`, `kassiber-runtime` and
`kassiber-daemon` pass. The mutation catalog
`crates/daemon/scripts/sync_s8_s10_mutations.json` (37 entries) is fully
killed. The preflight shard passes.

Found while verifying, not caused by these slices: `kassiber-wallets --lib`
fails nine publication and sync oracle tests whose frozen full-table dumps
predate `ai_provider_checks` (the 113th table, S1), and many Python harnesses
still assert 112 tables (`tests/test_rust_*.py`, `gen_lightning_vectors.py`).
This slice moved only the ones it uses (`tests/rust_desktop_parity.py`,
`tests/test_rust_workspace_shell.py`, the workspace vectors).

## After S3 and S5

### S3

`journals process` on a coarse-review book (`require_coarse_review`, two
coarse prices) gives the same envelope and the same book as Python in seven
shapes, plaintext and SQLCipher: unaccepted, accepted, accepted then rebuilt
with trailing zeros, accepted then changed, accepted as a value, accepted as
a value then priced by a rate, and review not required. Only the unaccepted
and changed prices are held. `ui.transactions.history`, `ui.activity.history`
and `ui.transactions.history.revert` render and revert the acceptance like
Python (`tests/test_rust_coarse_price_acceptance.py`, 18 cases). Thirteen
Python-oracle builder cases (`row-coarse-*` in `custody_fusion_builder.json`)
cover the rule's edges, including an sNaN acceptance, a NaN rate
(`InvalidOperation`) and a non-text acceptance (`AttributeError`).

Before S3, 26 Python/Rust oracle tests failed on this base: the merged
Python's projection rows carry `pricing_review_price`, which the committed
projection fixtures and the Rust projection lacked
(`test_custody_projection_port`, `_composition_port`, `_quarantines_port`,
`_stage` and the fusion-builder probe). The fixtures were regenerated from
the merged Python, refusing any change but that key and, in one DB-built
adapter case, the new column; all 26 pass, with the ledger goldens and the
builder probe over them. The S3 mutation entries (8 custody, 6 transactions)
are all killed.

Found while comparing, owned by S4: since #707 Python's journal rebuild
recounts transfer candidates and caches `profiles.swap_candidate_count`
(bumping the chain-index revision); Rust does not. Every `journals process`
book comparison differs there; the S3 and S5 tests mask exactly that column
and revision and compare the rest. The existing auto-pair comparison in
`test_rust_journal_commands.py` also fails on this base, on the envelope
(first case `exact-generic-False-False`); not investigated here.

### S5

`journals quarantine groups` answers byte for byte as Python's CLI on a
book with a root two rows wait on, a root of its own and an unrooted
downstream row: the overview, `--limit` bounds, each group kind, unknown and
malformed groups, and paging by Python's own cursors, including truncated,
foreign, retyped and stale ones (`tests/test_rust_quarantine_groups.py`,
plaintext and SQLCipher). `ui.journals.quarantine.resolve` by `root:` group
gives Python's envelope and book with and without a reprocess, for a root
with dependents, a root of its own, a transaction and an unmeasured
downstream row, and every refusal (target, group kind, stale group, price
arguments) leaves the book unchanged. Rust unit tests compare the overview,
member pages, cursors and the resolution effect with Python's
`json.dumps` of `quarantine_review` on one fixture
(`gen_quarantine_group_vectors.py`).

The quarantine summary now evaluates freshness with the calculation-semantics
term, as Python's `projection_freshness` does; before, Rust reported a book
whose journal semantics had moved on as current. This predates the merge.
`ui.journals.snapshot` and `ui.journals.events.list` keep that gap in
`views.rs` (`freshnessReason`, `needsJournals`; fixed on
`t3code/r6-5b-ai-tools`), so `test_rust_journal_views.py` still fails on
those kinds; none of its differences is in `ui.journals.quarantine`. The S5
mutation entries (10 journals, 6 desktop daemon and CLI entries run through
`crates/wallets/scripts/freshness_mutation_check.py`) are all killed.

### Verification of S3 and S5

Workspace fmt and clippy with `-D warnings` (custody, transactions,
journals, cli, daemon, kassiber with `test-harness`) pass. Every test binary
of `kassiber-custody`, `-transactions`, `-cli`, `-daemon`, `-runtime`,
`-envelope` and `kassiber` passes. In `kassiber-journals` the library and 12
integration binaries pass; seven fail identically at 6159f8f39, with the
same pass counts, so they predate these slices: `crash` (its `cipher`
vector, `kill=1`; not investigated), and `persistence` (15),
`prebuild_legacy` (6), `prebuild_pipeline` (4), `prebuild_pricing` (10),
`prebuild_sources` (10) and `probe` (1), whose stored Python table dumps
predate S1's schema (`ai_providers.agent_access` first). Those dumps need
regenerating against the 113-table schema; that belongs with S1, not here.

### Read diff after S3 and S5

`desktop_read_diff.py` on the Python-built `austria` book (built by
`report_design_books.py`'s steps with this checkout's Python CLI), the
default read kinds plus the seven report reads and
`ui.review.plan/worklist/receipt`, debug binaries: 113 kinds compared;
before (6159f8f39) 12 differ, after 11. `ui.journals.quarantine` no longer
differs; the other 11 are the after-S1 list's other kinds. A targeted run
with arguments (`ui.journals.quarantine` with `scope`,
`ui.transactions.history` for a transaction, `ui.activity.history`,
`ui.activity.stale`, `ui.journals.quarantine.resolve` with an unknown
`root:` group) has 2 of 5 differing before (`ui.journals.quarantine`; the
resolve answered `validation` for the unsupported field where Python answers
`quarantine_group_stale`) and none after. The book holds no quarantine and no
coarse price, so the differential tests above cover those paths.

## After S7

S7 ports #706's report data: the Reports screen payload of
`ui.reports.capital_gains` and the core's indicative tax
(`kassiber_reports::tax_estimate`, which S4's per-year delta and S12's AI
tax summary can reuse), the empty-root and country-less `jurisdictionCode`
`GENERIC`, and the crypto Kennzahlen from 1 March 2022 (Europe/Vienna) in
every report that states a field: the capital-gains report and its CSV,
full CSV/XLSX and verification exports, the E 1kv detail rows and Kennzahl
summary, the year handoff and the exit-tax preview. The E 1kv report and its
text, CSV and XLSX exports refuse a year before 2022 and a 2022 with January
or February crypto rows. Journal events carry `explanationReference`. The
cherry-picked 172c5a8c7 (from `t3code/r6-5b-ai-tools`) counts calculation
semantics in the journal views' freshness, which decides when an event has a
reference. Rendering stays with `t3code/r5-pdf-1`.

`desktop_read_diff.py` on the Python-built `austria` book (this checkout's
Python CLI, `report_design_books.py`'s steps), release binary, the same 113
kinds as after S1: 104 the same, 9 differing, none refused. Before S7: 101
the same, 12 differing. `ui.reports.capital_gains`, `ui.journals.events.list`
and `ui.journals.snapshot` (the cherry-pick) now match. Targeted runs of
`ui.reports.capital_gains`, `ui.reports.exit_tax_preview` (departure on
15 February of the year), `ui.reports.tax_summary` and
`ui.journals.events.list` for each tax year 2020 to 2025 match; that book
states 2020 and 2021 in KZ 801 and `unassigned` only, withholds 2022's
figure for one January/February row and estimates 2025.

Verification of S7:

- `kassiber-reports` lib (61), `vectors` (20), `gain_reads` (3) and
  `artifact_vectors` (1), and the `kassiber-journals` lib (95), pass against vectors regenerated from the merged
  Python. The generators gained cases for the cutoff instant, the E 1kv
  refusals, stored Kennzahl spellings, the breakdown and its link cap,
  swaps split across conversions, holding periods, countries and exit-tax
  departures around 1 March 2022.
- New Rust tests: `tax_estimate::tests` (the eight
  `AustrianIndicativeTaxTest` cases of `tests/test_tax_estimate.py`, ported,
  and a zero KZ 801),
  the calendar cutoff cases and
  `views::explanation_tests::journal_events_reference_only_explainable_gains`
  (from `test_review_regressions.py`).
- `tests/test_rust_reports_screen.py` (new, 8 tests) and
  `tests/test_rust_report_capital_gains.py` compare response bytes and all
  113 tables against the Python daemon, plaintext and SQLCipher; the
  report lanes (`test_rust_reports`, `_report_exports`, `_austrian_xlsx`,
  `_year_handoff`, `_exit_tax`, `_gain_reads`, `_full_xlsx`,
  `_export_preview`) pass where they run without the XLSX service probe.
  `test_rust_journal_views.py` matches for every events request; its
  quarantine comparisons fail on S5's `definitions`. `test_rust_reports.py`
  expected a 2020 E 1kv to succeed and now expects the refusal both sides
  answer.
- `crates/reports/scripts/reports_screen_mutations.json` (36 entries: 32 in
  `crates/reports`, 4 in `crates/journals`; `reports_screen_mutation_check.py
  --crate reports|journals` builds only the targets the entries name) is
  fully killed.

Found on the way, not S7: `ui.report.blockers` lacks `definitions` through
its embedded workspace health (four `test_rust_report_blockers.py` tests);
it follows S10. S1's 113th table broke the reports crate's 112-table
assertions and export-sequence fixtures, now updated.

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

The dashboard's bucket size compares the coerced period with the canonical
names only, so the aliases Python accepts bucket by month: `4y`, `4year` and
`4 Year` get monthly buckets while `4years` gets quarters, as `30d` and `3m`
already did before #701. Rust mirrors it. Repro:
`uv run --locked python -c "from kassiber.core import ui_snapshot as u; print([u._dashboard_bucket_size(u._coerce_ui_transaction_period(p), 1.0) for p in ('4years', '4y', '30d')])"`
prints `['quarter', 'month', 'month']`. The bucket size also reads the wall
clock rather than `until`, so "all" switches to yearly buckets ten years
after the first transaction regardless of the requested window.
