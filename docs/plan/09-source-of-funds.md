# Source of Funds Reports

**Status:** v1 backend, CLI, and desktop review workstation landed; optional
chain-observation adapters remain follow-up work.
**Current source of truth:** shipped transactions, transfer pairs, attachments,
BTCPay wallet history sync, `TODO.md`, and this boundary doc.
**Core rule:** Kassiber may explain reviewed BTC flow evidence; it must not
pretend that public chain heuristics prove ownership or source where they do
not.

## Problem

Users sometimes need to sell bitcoin through an exchange or broker and provide
a credible, limited source-of-funds document. Real self-custody history can
contain many hops: wallet rotations, exchange deposits and withdrawals,
Lightning or Liquid swaps, peg-ins, payjoin, coinjoin, manual OTC flows, gifts,
mining income, and older records that predate the Kassiber import.

Kassiber should help the user build a report that reveals enough reviewed
evidence for a compliance team without exporting unrelated wallet history,
descriptors, xpubs, secrets, or unnecessary counterparties.

## In Scope

- choosing a report purpose: planned exchange sale / bank pre-disclosure, or
  an already-completed transaction
- choosing the current funds-history anchor for a planned sale, or the target
  sale / withdrawal / exchange deposit for a completed case
- walking backwards from that target through reviewed transaction-flow links
- deterministic link suggestions from existing Kassiber evidence
- manual root sources such as fiat purchase, gift, mining, income, opening
  balance attestation, or prior exchange withdrawal
- attachment-backed evidence for source claims
- explicit missing-history and ambiguity markers
- source mix rollups by root-source category such as fiat purchase, exchange
  withdrawal, income, gift, mining, opening-balance attestation, and unknown
- simplified and full flow graph data
- a machine-readable source-of-funds envelope
- immutable case snapshots so a generated report can be re-rendered from the
  same reviewed evidence later
- a compliance-facing PDF export after the graph data is reliable

## Out Of Scope

- chain-surveillance scoring
- automatic deanonymization or address clustering as a source of truth
- exchange acceptance guarantees
- legal, tax, or AML advice
- exposing raw wallet files, descriptors, xpub material, seeds, keys, backend
  tokens, env files, or unrelated transaction history
- fabricating links to make a path look complete

## Bitcoin Evidence Boundary

Bitcoin transactions prove that specific inputs were spent into specific
outputs. They do not prove that every input and output belongs to the same
person, and they do not define an exact sat-by-sat mapping from inputs to
outputs across multi-input transactions.

Privacy techniques make this boundary more important:

- A simple self-transfer can be shown when Kassiber has both owned legs or a
  reviewed manual link.
- A swap can be shown when the two sides are linked by imported provider data
  or a reviewed manual link.
- A payjoin or coinjoin can be shown as a privacy-preserving hop, but the
  report must not claim exact upstream ownership unless the user adds reviewed
  evidence. Wallet-specific privacy importers may set the typed transaction
  privacy-boundary marker when a transaction crossed an opaque boundary, but
  that evidence alone must not be expanded into exact participant lineage.
  Wasabi imports, for example, can mark likely CoinJoin transactions and current
  payment-in-CoinJoin state, but those markers are not a completed round ledger
  or participant ownership map.
  Samourai/Whirlpool suggestions use this same boundary: Kassiber can show a
  reviewed Deposit/Premix/Postmix transition, but it must not traverse or
  disclose unrelated participant inputs.
- If an exchange, broker, or old wallet export is missing, the report must show
  a missing-history node and tell the user what evidence is needed.

Allocation is part of the evidence boundary. Kassiber should default to
explicit reviewed allocations between nodes. If a multi-input or privacy-hop
transaction cannot be allocated without guessing, the unallocated amount must be
reported as `ambiguous_allocation` or `privacy_hop_unresolved`; it must not be
silently spread across parents. A proportional or haircut allocation can appear
only as a labeled heuristic suggestion, never as final proof.

## Data Model Direction

Do not overload `transaction_pairs`. It is currently a tax/journal pairing
surface with constraints that are too narrow for many-to-many source tracing.

The v1 schema adds these source-funds tables:

- `source_funds_sources`: reviewed root claims such as fiat purchase, gift,
  mining, income, opening balance attestation, or prior exchange withdrawal
- `source_funds_cases`: one report target, recipient/exchange, report currency,
  reveal mode, generated-at metadata, and immutable snapshot metadata
- `source_funds_links`: reviewed and suggested links between transactions
  and/or root sources, including allocation amount, asset, link type,
  confidence, method, explanation, and review state
- `source_funds_link_attachments`: joins flow links to existing
  `attachments`
- `source_funds_source_attachments`: joins root source claims to existing
  `attachments`

Use the shipped attachment store for supporting documents. Do not create a
second blob store.

At PDF generation time, the case must freeze the exact envelope or a lossless
snapshot of the path: source/link ids, review states, allocations, pricing
provenance, attachment hashes, data-quality findings, reveal mode, and
schema_version. If later edits change the live evidence, Kassiber should create
a new case or explicitly refuse to re-render the old one from mutable state.
Report-currency conversions should use stored transaction/report pricing
provenance where available and label any conversion source in the envelope.

## Link Types

Initial link types should stay conservative:

- `self_transfer`: same user funds moved between owned wallets
- `exchange_transfer`: exchange deposit or withdrawal hop
- `trade`: in-exchange or imported wallet trade
- `swap`: cross-asset or cross-network swap
- `peg_in`: reviewed peg-in evidence, for example BTC to L-BTC
- `peg_out`: reviewed peg-out evidence, for example L-BTC to BTC
- `lightning_funding`: channel-open funding transaction
- `lightning_close`: channel-close transaction
- `lightning_routed`: off-chain Lightning payment or routing history backed by
  node/provider records
- `lightning_swap`: submarine or reverse-submarine swap where evidence exists
- `coinjoin`: reviewed privacy hop with explicit ambiguity
- `payjoin`: reviewed privacy hop with explicit ambiguity
- `manual_source`: root source claim supplied by the user
- `missing_history`: known gap that must not be hidden

Each link should carry:

- `state`: `suggested`, `reviewed`, `rejected`
- `confidence`: `exact`, `strong`, `weak`, `unknown`
- `method`: deterministic source such as `same_external_id`,
  `transaction_pair`, `provider_trade_id`, `manual`, or `chain_observation`
- `allocation_policy`: `explicit`, `heuristic`, or `unknown`
- `explanation`: short human-readable reason

Only `reviewed` links should be used as confirmed path evidence in a PDF.
Suggested links can appear in a review workflow, not as final proof.

Source-funds link review must not mutate tax/journal `transaction_pairs`.
Existing pairs may seed suggested links with `method: transaction_pair`, but the
source-funds review state belongs to `source_funds_links`. If a pair is deleted
after a case snapshot is generated, the snapshot remains reproducible while new
cases should surface the now-missing source evidence.

## Analysis Pipeline

The path builder starts from a target transaction and walks backwards through
reviewed links. It consumes explicit reviewed allocations, checks that reviewed
allocations sum to the target amount, and stops only at reviewed root sources or
explicit gaps. If allocations are incomplete, ambiguous, or heuristic-only, the
machine envelope must expose the unresolved amount and block PDF export.

Deterministic suggestions should run in this order:

1. Existing same-asset self-transfer detection by shared external transaction
   id across owned wallets.
2. Existing manual `transaction_pairs`, including cross-asset swap links.
3. Provider/import evidence such as trade ids, order ids, payment ids, or
   exchange ledger ids when stored in `raw_json`.
4. Tight time and amount matches across owned wallets, as opt-in broad hints.
5. Chain observations from configured Esplora, Electrum, or Bitcoin Core
   backends, stored as evidence only unless the ownership link is reviewed.
   Public Esplora or third-party Electrum usage must show a privacy warning
   because the queried txids reveal the report target and investigation path to
   that backend.

When a target transaction is supplied, suggestion writes are target-scoped:
Kassiber only persists candidate links that touch the target or transactions
already reachable from the target through non-rejected source-funds links.
Broad account-scoped provider ids and same-day time/amount matches are not
persisted unless the user explicitly opts into broad hints. Every suggestion
run has a hard write cap and aborts without committing when the cap is exceeded.
Batch review must re-check deterministic predicates against the live database
before promotion. A stale `same_external_id`, deleted `transaction_pair`, or
provider id that is no longer one-to-one remains `suggested` for manual review.

Walkers must keep a visited set keyed by transaction and asset, enforce depth
and node-count caps, and emit `path_truncated` instead of silently stopping.
Coinjoin, payjoin, and wallet-specific privacy-boundary nodes should collapse
unrelated participant inputs into an opaque privacy-hop cluster. Kassiber must
not list unrelated participant addresses or txids as if they were user-owned
source parents, and automatic same-transaction-id self-transfer suggestions
must stop at the boundary.

Every ambiguity becomes a data-quality item:

- `missing_history`
- `ambiguous_allocation`
- `privacy_hop_unresolved`
- `missing_pricing`
- `unreviewed_suggestion`
- `unconfirmed_chain_data`
- `path_truncated`

## Report Shape

The report should follow the same broad shape as common exchange-facing
source-of-funds documents:

1. Overview of the target sale/withdrawal/deposit, value, date, source mix,
   transaction count, link count, and data sources.
2. Short narrative of the reviewed path and open limitations.
3. Data sources grouped by wallet, exchange, API/import/manual, and transaction
   count.
4. Source structure by root source category and amount.
5. Simplified flow graph with clustered hops.
6. Full flow graph with transaction-level nodes.
7. Transaction detail table by path level, including txid/external id, type,
   asset, amount, fiat value, source kind, and evidence state.
8. Notes and disclaimers: generated from user-provided data, not legal advice,
   not a government-certified source-of-funds confirmation.

The shipped local report envelope now carries those sections as structured
fields instead of PDF-only prose: `overview`, `narrative`, `data_sources`,
`data_provenance_summary`, `simplified_flow`, `flow_levels`, `source_mix`,
`graph`, `findings`, `disclosure_preview`, `report_context`, and (when
requested) `diagrams`. The narrative and simplified chart model are generated
deterministically from the saved review graph on the user's machine.

Per-transaction granularity matches strict exchange-facing reviews: every
transaction node carries `fee`/`fee_msat` and a `data_provenance` value
(`chain_sync` for descriptor/xpub watch-only sync, `platform_export` for
exchange/node CSV and API imports, `manual_import` for custom/manual rows),
derived from the owning wallet's kind. `flow_levels` nodes additionally carry
`direction`, and each level row carries `assets` plus a `fiat_value_total`
subtotal when the level's priced nodes share one fiat currency. The PDF
renders this as one "Transaction Details by Level" sub-table per level (Date,
Source, Type, In, Out, Fee, Fiat, ID/hash, Data source), with root sources
included as attested rows. `data_provenance_summary` rolls disclosed
transactions up by provenance for the data-sources ring; the data-sources
table keeps wallet/category rollups and gains a per-row provenance column.
`missing_history` gap findings carry the unexplained `amount`/`amount_msat`
where the walk knows it, and the PDF renders a dedicated "Missing History and
Gaps" section whenever `gaps` is non-empty.

`disclosure_preview` also quantifies the disclosure footprint: `wallets_named`
lists the wallet labels the report exposes, and `ownership_note` states the
core privacy consequence — sharing the report demonstrates common ownership
of those wallets to the recipient. This is a deliberate lightweight summary,
not a chain-analysis privacy scorer.

Diagrams are rendered by a single on-device substrate
([kassiber/core/source_funds_diagram.py](../../kassiber/core/source_funds_diagram.py)):
one `reportlab.graphics` `Drawing` per chart (a weighted Sankey-style
simplified flow with value-share edge percentages and a Bitcoin-native legend,
plus source-mix and data-source donut rings). The same builder renders the
native-vector chart embedded in the PDF and a web-safe SVG string for the
desktop disclosure preview, so the two cannot drift. SVG rendering is opt-in
via `build_report(..., include_diagrams=True)`; the `compute_coverage` sweep
leaves it off so its repeated per-transaction `build_report` calls never pay for
chart rendering. When `include_diagrams` is set the `diagrams` SVGs are frozen
into the case snapshot alongside the rest of the disclosure payload. `simplified_flow`
edges now also carry `percent_of_target` (target amount = 100% base; trading
gains/losses and fees are never folded into source percentages). They must not call an external AI service or upgrade weak heuristics
into proof. The simplified chart follows reviewed local sources, wallet
transfers, and consolidation-style reviewed hops. CoinJoin/PayJoin traversal is
deferred for now and rendered as an explicit privacy boundary, not as proof
through unrelated participant inputs.

`report_context` is the expansion point for country/currency presentation. The
first concrete profile is `template_key="at_eur_basic"` for Austrian EUR books:
the PDF title becomes `Mittelherkunftsnachweis / Source of Funds Report`, the
cover records Austria/EUR context, and an evidence checklist pins the minimum
workflow. This is not a full Austrian legal template or German localization yet.

The machine envelope should expose the graph nodes, edges, rollups, gaps, and
data-quality findings directly so the PDF is only a rendering of program
output.

Source mix rollups should be by root-source kind only: fiat purchase, exchange
withdrawal, mining, income, gift, opening-balance attestation, and unknown.
Transfers, swaps, pegs, and privacy hops are path/edge statistics, not final
root sources. Opening-balance attestations must remain visibly separate from
fully explained sources because they mean "prior history stops here with
supporting evidence."

## Export Gates

`reports source-funds` should be useful before a case is exportable. It should
return the full machine envelope, including blockers and an `explain_gates`
summary.

`reports export-source-funds-pdf` renders saved case snapshots only, using the
same ReportLab PDF renderer family as the styled tax reports so disclosed
labels, notes, and evidence names preserve Unicode text. It should refuse live
target-argument exports because the reviewed preview and the PDF must share the
same immutable disclosure payload. It should also refuse when:

- a path edge is still `suggested`
- a reviewed path forms a cycle or self-link
- the target amount has any unallocated or heuristic-only remainder
- a leaf is not a reviewed root source or a reviewed `missing_history` gap
- required pricing is missing for an amount included in the source mix
- a concrete source has no amount, is over-allocated, or uses a different asset
  than the reviewed link consumes
- a reviewed path requires more value from a transaction than that transaction
  contains
- a reviewed parent transaction or root source is dated after the transaction
  it claims to fund
- a `self_transfer` link declares an asset that differs from either transaction
- unconfirmed chain data is used as proof instead of context
- selected reveal mode would include unreviewed chain observations

Reviewed `missing_history` gaps may appear in the report, but they must be
labeled as gaps with the evidence the user attached. They are not equivalent to
fiat purchases, mining income, gifts, or exchange withdrawals.

## Audit Package Handoff

The Reports audit package export now reuses the same reviewed source-funds
state for trusted auditor handoff. The package manifest is DB-backed and
deterministic: it lists included transactions, direct attachments, source-funds
links, link/root-source evidence, journal/review state when enabled, copied-file
hashes, URL references, copied-evidence provenance, and missing-evidence
warnings.

This is not a source-funds PDF replacement and it does not mutate
`transaction_pairs`. Tax/journal pairs can seed source-funds suggestions, but
the audit package reads source-funds review state from `source_funds_links`,
`source_funds_sources`, and their attachment join tables.

AI/readiness summaries use the same persisted evidence query shape but redact
raw URL values, managed storage paths, descriptors, xpubs, backend endpoints,
credentials, raw wallet files, logs, AI settings, unrelated books, and
technical wallet evidence. OCR, photo understanding, invoice parsing, remote
document upload, automatic evidence pairing, and auto-review remain deferred.

## Privacy and Reveal Modes

The default report should be scoped to the target path only. It must not include
unrelated UTXOs, descriptor paths, xpubs, raw wallet configuration, or backend
credentials.

Reveal modes:

- `labels_only`: source labels, dates, amounts, evidence labels/types, and no
  txids/external ids; attachment URLs, managed paths, hashes, and media types
  are omitted
- `minimal`: the selected target txid/external id, source labels, dates,
  amounts, and evidence labels/types; ancestor txids and attachment URLs,
  managed paths, hashes, and media types are omitted
- `standard`: full path txids/external ids plus wallet/source labels; evidence
  labels/types, media types, and hashes are included, but attachment URLs and
  managed storage paths are omitted
- `full`: full path txids/external ids and full attachment metadata including
  URLs and managed storage paths

Including a txid is already meaningful on-chain disclosure: a recipient can
inspect its inputs, outputs, amounts, and neighbors. The user should see a
preview of what will be disclosed before exporting a PDF, including what a
chain-analytics service can infer from each included txid. `full` reveal mode
adds reviewed addresses and observations; it never upgrades a weak
`chain_observation` into proof of ownership.

## Library Strategy

No single maintained Python library appears to solve this feature end to end.
Kassiber should keep the core local-first and deterministic:

- Use existing sync backends and Bitcoin Core/Esplora data for raw chain facts.
- Use the existing `embit` dependency for Bitcoin transaction and descriptor
  primitives where local parsing is needed.
- Consider an optional GraphSense adapter later for users who operate or trust
  a GraphSense endpoint. It is a full analytics stack, not a lightweight core
  dependency, and any score-like output must remain review context or an
  attachment, not a source-mix proof.
- Do not base the feature on BlockSci; upstream says it is no longer actively
  developed or supported.
- Treat browser-oriented tooling such as txray as useful prior art for privacy
  heuristics, not as a backend dependency.
- Treat `Copexit/am-i-exposed` as useful prior art, not as a core dependency:
  it uses client-side mempool.space tracing, bounded backward/forward
  traversal, a deterministic hop-column graph explorer, VisX/D3 Sankey views,
  proportional taint analysis, and Boltzmann WASM linkability. Kassiber should
  reuse the ideas of bounded trace fetchers and explicit graph data, while
  keeping final report evidence limited to reviewed links and attachments.

For graph rendering, start with structured JSON and plain text. Add a PDF graph
renderer only after the path builder is correct. If a third-party renderer is
needed, prefer a maintained, cross-platform PDF/drawing dependency and update
`README.md` plus `THIRD_PARTY_LICENSES.md` in the same change.

## Shipped v1

The first implementation adds the conservative, testable core path:

- `source_funds_sources`, `source_funds_links`, case/snapshot tables, and
  source/link attachment joins in SQLite
- `source-funds sources ...`, `source-funds links ...`,
  `source-funds suggest ...`, and `source-funds cases list`
- `reports source-funds --target-transaction ...` with graph nodes, edges,
  allocations, source mix, gaps, findings, disclosure preview, and
  `explain_gates`
- `reports export-source-funds-pdf --case ...`, which refuses unresolved
  blockers and renders only the immutable saved case snapshot
- immutable case snapshots for later re-rendering
- daemon kinds for source/link/evidence review, suggestion seeding, report
  preview, and PDF export
- a desktop source-of-funds workstation with purpose selection for planned
  exchange sale versus already-completed transaction, target/anchor selection,
  planned exchange/bank note fields, suggestion seeding, link accept/reject,
  explicit allocation edits, evidence attachment, source/gap creation, gate
  preview, disclosure preview, and PDF export
- a simplified desktop default path that keeps target selection, local case
  summary, review gates, and export visible while historical coverage, target
  filters, and full link/source editors stay optional advanced panels
- PDF sections for source overview, local narrative, data-source rollups, source
  mix, a simplified boxes-and-arrows flow path, level-by-level flow rows,
  transaction details, review gates, disclosure preview, and limitations
- a single on-device diagram substrate
  ([kassiber/core/source_funds_diagram.py](../../kassiber/core/source_funds_diagram.py))
  that renders a weighted Sankey-style simplified flow (value-share edge
  percentages, Bitcoin-native legend) plus source-mix / data-source donut rings,
  emitted both as native-vector PDF charts and as web-safe SVG strings frozen in
  the case snapshot; the desktop Export step embeds those same SVGs so the
  preview matches the PDF, and the cover gains an at-a-glance strip and mini-flow
- advanced, snapshot-frozen presentation options via `build_report(...,
  report_options=...)`, stored as `envelope["report_options"]`. The first option
  is `diagram_detail` (`summary` clusters long paths; `detailed` shows more hops
  before clustering), exposed as `reports source-funds --diagram-detail`, the
  daemon `report_options` arg, and a desktop Export-step control. The same
  normalize-store-freeze pattern now also carries `amount_precision`
  (`btc`/`sats`), `mask_recipient`, and `omit_sections` (verbose PDF sections
  from `OPTIONAL_REPORT_SECTIONS`), each exposed via `reports source-funds`
  flags (`--amount-precision`, `--mask-recipient`, `--omit-section`) and
  Export-step controls. Further options should follow the same pattern so
  simple UX stays the default
- a recipient-ready evidence bundle: `reports export-source-funds-bundle --case
  ...` (and the `ui.source_funds.export_bundle` daemon kind) zips the report PDF,
  the original evidence files attached to disclosed sources, and a SHA-256
  `manifest.json`. It reuses the same export gate as the PDF and is reveal-mode
  scoped: `standard`/`full` include the files, `labels_only`/`minimal` record
  them as `withheld_by_reveal_mode`. The report references originals by hash; it
  never transcribes them (`attachments.resolve_attachment_files` resolves the
  on-disk files).
- a desktop target picker rendered as the transaction table (parity with the
  Transactions screen) and the shared, editable transaction detail panel
  ([TransactionDetailController](../../ui-tauri/src/components/transactions/dashboard/TransactionDetailController.tsx))
  wired into the workflow: the picker's details affordance, review-step gate
  findings, and flow-path transaction nodes all open it. Because daemon
  mutations invalidate the source-of-funds queries, fixing pricing / exclusion /
  evidence there re-evaluates the gates, coverage, and source mix automatically.
- a basic Austrian/EUR report context with bilingual title, evidence checklist,
  and a checked-in fictitious demo generator at
  `scripts/generate-source-funds-demo-report.py`

The v1 suggestion pass seeds separate source-funds links from same
`external_id` transfers, existing `transaction_pairs`, and one-to-one
provider/import ids in `raw_json`; broad provider account ids and tight
same-day amount matches require explicit broad-hint opt-in. These links stay
suggested until reviewed; PDF export does not use them as proof. Exact/strong
deterministic suggestions from same external ids, existing `transaction_pairs`,
and one-to-one per-transaction provider/import ids may be batch-reviewed by the
user so long consolidation chains do not require one-click-per-hop review.
Batch review is target-scoped: it only promotes deterministic suggestions
reachable from the selected report target and still deterministic at review
time. Broad provider account ids, weak time/amount matches, stale provider or
external-id matches, amount-mismatched provider rows, and chain-observation
hints stay manual.

## Implementation Order

1. [x] Add source/root and flow-link schema, using existing attachments for
   evidence.
2. [x] Add CLI review surfaces for sources and links:
   `source-funds sources ...`, `source-funds links ...`, and a suggestion
   command.
3. [x] Add `reports source-funds --target-transaction ...` with a machine envelope
   and strict data-quality gates.
4. [x] Add plain/table rendering so users can fix gaps before PDF work.
5. [x] Add `reports export-source-funds-pdf` once graph nodes and edges are stable.
6. [x] Add first desktop review UX, deterministic-hop bulk review, and disclosure preview.
7. Add optional chain-analytics provider adapters only behind explicit user
   configuration.

## One-Line Restatement

Kassiber should produce a reviewed, path-scoped source-of-funds explanation,
not an opaque chain-analysis verdict.
