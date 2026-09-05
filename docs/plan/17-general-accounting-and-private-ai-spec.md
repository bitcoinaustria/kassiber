# General accounting and private AI: consolidated implementation specification

**Status:** Implementation authorized and in progress; not production-ready.
**Updated:** 2026-09-05.
**Delivery:** One complete product outcome through the reviewed
[dependency-aware PR stack](18-general-accounting-pr-stack.md), with a retained
integration branch. This supersedes the earlier single-PR packaging, not its
accounting scope. The owner-approved CLI/Agent-only pivot below replaces the
dedicated UI requirements. Do not merge upstream without authorization.
**Execution backlog:** `TODO.md`. This document defines the accepted outcome
and acceptance criteria for that backlog.

This specification consolidates the full-accounting handoff and its subsequent
review. It supersedes the earlier K2-only slice, the blanket exclusion of
accounting data from AI assistance, and the product-wide prohibition of a
general ledger in plans 08 and 16. Existing shipped security requirements remain
binding. Historical plans are research inputs, not competing implementation
instructions. This is an implementation handoff, not evidence of shipped code.

Dependency policy: target zero additional mandatory runtime dependencies for
the general ledger. Own the accounting domain logic; reuse the existing
SQLCipher, cryptography, RP2, document-processing, and report writers. Any new
dependency must explain why existing code/stdlib is insufficient and document
offline behavior, egress, platform support, licensing, and update responsibility.
Beancount, hledger, GnuCash, TigerBeetle, and Paperless-ngx are engineering
references for specific invariants/workflows, not imported engines or legal
authorities. Vendored code and model/native-tool downloads are dependencies too.

## Delivery interface decision (2026-09-05)

Full accounting is CLI/Agent-only, as explicitly requested by the owner.
Keep the complete ledger/evidence/source/close/tax scope; remove dedicated
accounting routes, navigation, Settings switches, forms and native save pickers.
The existing portfolio/private UI remains unchanged. Only exact action review
in the existing Assistant is retained. Manual fallback means the deterministic
CLI, not a second graphical bookkeeping application. Do not rebuild that UI as
an acceptance prerequisite. Core arithmetic, privacy, recovery, agent consent
and actual CLI artifact tests remain mandatory. The prototype UI is retained
in a recovery branch, not silently destroyed.

## 1. Product and acceptance scope

Kassiber serves private individuals, businesses, and associations. Existing
portfolio, cost-basis, wallet, custody, and personal-tax workflows remain usable
without creating a chart of accounts, supplying corporate identity, closing a
period, selecting K2, or installing an AI model.

Organizational accounting is an explicit per-book capability. Entity kind,
accounting mode, accounting regime, jurisdiction, and tax obligations are
separate facts. Do not encode them as one mutually exclusive persona enum.
Private users can opt into richer accounting; organizations can use portfolio
features. Multiple separately scoped books are allowed; consolidated group
accounts are not required by this delivery.

The first complete acceptance case is the user's organization: keep its books
inside Kassiber and produce the relevant K2 declaration/annex working papers
for manual filing. Determine the exact required annexes rather than assuming
that “K2 annex” always means K2kv. Filing transport is excluded.

Before freezing posting and tax rules, record a coverage matrix of the real
organization's transaction categories, accounting regime, required reports,
bank formats, opening date, and target tax year. Use redacted or synthetic
examples. K2 applicability remains an explicit verified input; detect an
incompatible K1/K3 route without forcing the organization into K2. A missing
answer does not block independent ledger, CLI, or privacy implementation, but
does block claiming that the organization's affected workflow is complete.

For every actual category record: capture workflow, source evidence, accounting
recognition, tax treatment, closing control, and an acceptance fixture. A
required category cannot be excluded merely to make the acceptance tests pass.
“Manual support” requires usable structured records, provenance, and reports
inside Kassiber, not an undocumented external spreadsheet.

Do not assume technical or model limitations as product restrictions. Use the
smallest implementation that meets this complete workflow; capability, compute,
and storage requirements must be measured rather than guessed. This does not
authorize unrelated infrastructure, purchases, or deployment.

## 2. Required end-to-end accounting

- Opt-in setup, customizable chart, opening balances, and source scope.
- A real general ledger, separate from existing wallet/reporting `accounts`
  and RP2-derived `journal_entries`.
- Balanced manual entries, reviewed automatic proposals, reversals, replacement
  entries, and retained evidence.
- Bank-file preview/import and reconciliation; existing BTC/LBTC observation
  and custody sources; cash records and count reconciliation if the organization
  handles cash.
- Receivable/payable records with document identity, partial settlements, and
  remaining amounts. Invoice issuing and dunning are unnecessary for this.
- Structured manual asset, depreciation, accrual, tax, or restricted-fund
  schedules where the coverage matrix requires them. Automatic specialist
  calculations are optional; complete supporting records are required.
- Journal, account ledgers, trial balance, balance sheet, P&L, reconciliation,
  tax-account controls, and book-to-tax bridge. Add association-specific
  income/expenditure and asset statements where the selected regime requires
  them; do not equate those with an accrual P&L.
- A usable deterministic CLI for the entire supported workflow and explicit
  scoped agent tools. Core accounting works with AI disabled; the existing
  Assistant provides only the minimal exact approval display.
- An independently verifiable close package and the required AT K2/annex
  working papers from the organization's own records.

EUR, calendar year, and Europe/Vienna are initial pilot candidates, not core
constants or verified facts about the organization. Confirm them in the
coverage matrix. Functional-currency exponent, timezone, fiscal intervals,
assessment periods, and form versions must be explicit types/configuration.
Implement any different setting actually required by the accepted pilot scope;
do not build speculative foreign-currency trading or consolidation features.

## 3. Modules, authority, and country seams

Use small Interfaces with behavior concentrated in deep Modules. The following
are responsibilities, not a mandatory count of packages or abstraction layers:

1. Observations and retained document evidence.
2. Economic interpretation and source allocations.
3. Accounting projection and balanced proposals.
4. General ledger, periods, and atomic persistence.
5. Reconciliation and closing controls.
6. Financial reports and saved artifacts.
7. Calculation artifacts and the RP2 Adapter.
8. Jurisdiction policies, form compilation, and tax working papers.
9. AI proposal orchestration through existing capability/consent machinery.

Preserve existing custody, commercial, loan, pricing, and tax interpretation
authority. Economic events are derived from those facts plus narrowly scoped
book decisions; no second competing set of classifications. Bank observations
do not become fake Bitcoin transactions. A document need not have a Bitcoin
transaction to exist or support a manual entry.

Sources feed separate accounting and tax projections. Neither the book ledger
nor a form compiler rewrites raw observations. Ledger/core Interfaces do not
import AT, K1, K2, K3, paragraph numbers, Kennzahlen, or RP2-internal rows.
Use a registry of versioned ChartPack, AccountingPolicyPack, TaxPolicyPack,
FormPack, and calculation Adapters where behavior actually varies.
JurisdictionPack is their manifest. Country/standard-specific financial report
presentation belongs in a pack as well as tax form presentation.

Ship Austria's required pack and a test-only contrasting Adapter. This proves
the seam technically, not legal support for another country. A further country
should need pack/Adapter additions, not a fork of custody or the ledger.

## 4. Monetary values, basis, and posting invariants

Use integer fiat minor units, existing integer Bitcoin atomic units, and
canonical decimal strings at rate/RP2 Interfaces. No floating-point accounting.
Specify decimal precision, rounding mode, remainder allocation, and overflow
handling. Retain sub-cent quantity events without inventing one-cent entries.
Distinguish quantity, economic value, book carrying value, tax basis, proceeds,
book P&L, and tax P&L.

RP2 remains the crypto lot/basis calculation Adapter. Persist complete result
artifacts with input commitments, revision, method, pool/election configuration,
per-disposal outputs, remaining quantities/basis, and unresolved cases. Prove
point-in-time opening support, organization/regime suitability, and pool/transfer
semantics against the pinned implementation and applicable primary sources.
Do not infer correctness from existing report output alone.

`follow_rp2` is allowed only for a proven compatible basis policy. Book-only
write-downs, write-ups, and other valuation adjustments need retained schedules,
including deterministic release on partial disposal. A quantity-free valuation
entry must not fabricate a taxable acquisition. If the pilot requires another
basis policy, resolve its calculation seam within the delivery or identify the
specific remaining prerequisite; do not silently apply personal tax rules or
claim an incomplete pilot is done.

Posted entries are atomic and immutable, with exact balance, profile isolation,
stable posting identity, idempotency, source/decision/pack commitments, and
database-enforced mutation protection. Reversals are new entries and cannot be
partial by accident. Snapshot referenced account/report definitions so later
chart edits cannot change historical reports.

Manual BTC quantity postings require compatible source/custody evidence.
Multiple representations of one purchase or sale (invoice, bank, wallet) share
economic identity and explicit allocations. They must not book revenue or
expense twice. Support partial/split/grouped settlement and show all remainders.

## 5. Bank, opening, and multi-year close

Bank-file import preserves stable statement/row identity, exact values, parser
version, provenance, coverage, overlaps, and corrections. Identical legitimate
payments must survive. Import without control balances may be allowed, but
cannot independently prove completeness. Require separately evidenced control
balances before declaring reconciliation complete. Posted use prevents unsafe
rollback; timing differences need explained reconciling items.

Opening accepts reviewed balances, open items, valuation schedules, and tax
carryforwards. Historical Bitcoin input remains available for calculation but
does not create duplicate opening-era book entries. Detect duplicate opening
and out-of-scope/pre-opening proposals.

Define result appropriation/carryforward, subsequent-year opening, preserved
historical P&L, continued open items, and book/tax valuation continuity. Reopening
an earlier year creates a new revision and marks dependent later-year state for
review/reconciliation. It never overwrites an earlier close artifact.

Use this order to avoid a close/tax dependency cycle:

candidate book snapshot -> tax preview -> reviewed tax/closing entries ->
final accounting close -> final working paper referencing that close.

Separate accounting_closed, tax_workpaper_ready, and package_exported. Missing
form packs do not prevent ordinary book operations or accounting close where
all accounting-relevant controls are satisfied. Missing valuation needed by
the books still blocks close. A snapshot cannot require the hash of a future
document that itself contains that snapshot's hash.

Post/close recheck input revisions inside the atomic transaction. Single-writer
does not mean CLI, desktop, sync, or background updates cannot race. Replicated
source changes still invalidate affected proposals or identify later corrections
even while ledger tables themselves remain local-only.

## 6. Evidence, encryption, and local operation

Preserve existing project unlock, passphrase, secret-floor redaction, watch-only
wallet, and daemon trust contracts. Do not add telemetry, automatic uploads, or
an arbitrary file/shell/SQL tool for the in-app assistant.

Accounting-sensitive storage requires an encrypted project before ingestion.
Use encrypted document contents inside SQLCipher for the initial managed
accounting evidence implementation; adapt existing document metadata/links to
reference that storage without constructing a second interpretation system.
Persist extraction results, searchable text, schedules, thumbnails if retained,
and AI proposal provenance within that same protection.

Existing attachments can be plaintext files outside SQLCipher. Do not claim
that linking one encrypts it. Import/reuse with explicit provenance and storage
status, and retain immutable content for posted accounting references. Do not
silently delete user originals or plaintext migration backups. Existing private
portfolio use must not be silently enrolled into an encryption migration.

Extend deletion, GC, import rollback, profile reset, and attachment replacement
checks to retained accounting evidence. Existing posted/closed references must
survive. Backup and restore must recover document bytes as well as metadata,
open items, calculation artifacts, and close history.

Local OCR/rendering/search must not fetch document URLs, load remote images,
execute embedded content, or silently send crash/usage reports. Bound parser
resources and isolate unsafe parsing where practical. Avoid plaintext caches
and temp files; any required transient exposure needs an explicit lifecycle,
owner-only access, and error/cancel cleanup tests. Clear decrypted working state
on lock. Do not promise protection against a compromised operating system.

Model/runtime downloads are explicit setup actions and do not include book
content. Network-disabled accounting and document processing are acceptance
tests. Do not silently switch to a hosted model when a local model is missing.

New ledger/bank/evidence data is excluded from replication, diagnostics, and
ordinary audit-package defaults. This does not prohibit scoped AI assistance.
Exports are explicit disclosures: preview evidence inclusion and clearly label
plaintext outputs; reuse encrypted package support for confidential handoffs.

## 7. AI assistance across the product

**Product clarification (2026-09-05): agent first, not agent only.** The primary
assisted workflow starts with the user's intent, not a sequence of forms they
must learn. The agent discovers permitted capabilities and current scoped state,
identifies missing facts, proposes a bounded plan, prepares reviewable actions,
and verifies the resulting records. The UI is a review, approval, correction,
and evidence workspace; complete manual operation remains available without AI.
This applies to private portfolio use as well as organizational books without
forcing private users into corporate setup.

AI supports private portfolio/cost-basis questions as well as organizational
document sorting, field extraction, document/payment matching, account and tax
classification proposals, manual-entry drafting, reconciliation explanations,
closing checklists, and explanations of K2/annex derivations.

Pipeline: local ingest -> text extraction/local OCR -> structured candidates ->
deterministic validation/matching -> optional AI proposals -> user review ->
normal daemon command -> normal accounting validation/posting.

Keep source spans/page references, extraction method/model version, proposed
values, confidence where meaningful, and human corrections. AI confidence is
not an accounting or legal approval. Existing commercial/custody authority is
updated through its supported commands, not bypassed by document extraction.

Local models may receive user-selected sensitive context through typed tools
while the project is unlocked. Hosted models receive only a selected,
minimized disclosure under existing remote-provider acknowledgement plus a
bounded consent for the newly sensitive document/accounting data. Do not
silently broaden permissions for existing AI tools.

Disclosure scope binds project/book, documents/fields, destination provider,
purpose, and lifetime. Enforce it on the first prompt, images, retrieved text,
tool outputs, retries, summaries, subsequent turns, and provider switching.
Masking names alone does not make a financial document anonymous. Show what
will be sent; keep the existing egress/privacy receipt behavior accurate.

Provider execution locality and data destination are separate: a local CLI,
LAN endpoint, proxy, or TEE can still send data off-device. Do not treat a
provider label or localhost URL as proof of fully local processing. Reuse
verified routing/trust configuration; describe remaining provider trust honestly.

Use existing advertised-tool scopes, schema validation, project pinning, and
separate mutation/egress consent. Drafting does not post. AI can execute a
reviewed bounded batch through the same guarded posting Interface after explicit
approval; bind approval to payload/input hashes and reject stale approvals.
Close/reopen, disclosures, and permanent changes require action-specific review.
No model may bypass balances, retention, or reconciliation controls.

Documents, OCR text, tool results, and imported metadata are untrusted data.
Embedded instructions cannot request extra access, approve actions, change
accounts, or cause outbound requests. Exact approved context is not a grant to
enumerate the whole book. Cancellation, denial, lock, book changes, or provider
changes invalidate the appropriate pending operations. Chat/index persistence
stays inside the configured encrypted storage and outside diagnostics.

### 7.1 Agent-first completion contract

CLI availability alone is not agent capability coverage. For each supported
workflow, record the discovery/read operation, proposal/preview operation,
approval requirement, guarded execution operation, and result verification.
Use explicit typed, advertised tools backed by the same deep Modules as CLI. Do not expose generic daemon dispatch, shell, SQL, or arbitrary files,
and do not implement orchestration through simulated UI clicks.

Cover book/chart/period preparation, user-selected evidence intake and local
extraction, bank/source matching, posting proposals, open items and schedules,
reconciliation, close readiness, and tax working-paper preparation. The agent
must explain unavailable capabilities or missing evidence and request the
specific missing input rather than inventing facts or silently omitting work.
Credentials, source-file selection, and action-specific user approvals remain
explicit user interactions, not gaps to bypass with broader permissions.

The agent can propose which records to inspect, but discovery must remain inside
the user's explicit book/task/data scope. Newly selected sensitive content needs
the appropriate disclosure preview and consent before reaching the model.
Local execution does not grant unrestricted book access; hosted execution does
not gain whole-book context from a task-level request. Apply scope, provider,
revision, budget, lifetime, cancellation and egress controls on every tool round.
The current one-question, tool-free disclosure token must remain tool-free;
adding orchestration requires explicit scoped tool contracts, not relaxing that
token's meaning or the provider's native filesystem/tools restrictions.

Show a task-level worklist with sources, proposed changes, totals, unresolved
items and consequences. Aggregate compatible actions into a reviewable bounded
batch rather than prompting once per field. Posting, close/reopen and exports
keep their separate existing approvals; no blanket "agent first" consent.
After approval, the daemon revalidates the exact payload and current inputs,
executes through ordinary guards and returns committed record identities.
The agent reports completion from those results, not its own proposed text.
Retries and resumption read actual current state and idempotency receipts;
they must not duplicate postings or silently reuse expired approval. Do not add
a new agent framework, background autonomy or chat-history persistence merely
to implement this workflow.

Acceptance: run the private and organizational lanes below through the actual
agent/tool/consent Interfaces, including a multi-step synthetic scenario from
selected evidence through matching and draft review to approved posting,
reconciliation, approved close and verified close/K2 artifact production, not
only close preparation or K2 explanation. Compare resulting
accounting facts with the equivalent manual path. Test missing facts, denied
actions, interruption/retry, cross-book/provider changes and scope escalation
across multiple rounds. Keep deterministic orchestration/guard fixtures and
real provider/interactive smoke evidence distinct; a scripted reply or green
form component test alone is not proof of agent-first completion.

### 7.2 Required work-saving outcomes

These five outcomes are requirements for the complete stack, not optional future
enhancements. Section 7.1 supplies the security and execution contract; these
outcomes measure whether that contract actually removes routine user work.

**AF-1 — A period from one task.** Starting with "Prepare June" and explicitly
selected book, interval and sources, the agent processes the complete selected
population, including pagination, into proposed routine actions and named
exceptions. Users provide genuinely missing facts and review consequences;
they must not transcribe already available amounts, counterparties, record IDs
or supported mappings repeatedly. Source/disclosure approvals may be grouped
within the existing bounded contracts; selecting a period does not authorize
arbitrary files, other books or whole-book provider disclosure.

Use a frozen synthetic monthly benchmark with at least 100 source records,
mixing documents, bank rows and BTC observations. Include recurring cases,
multiple representations of one event, partial settlements and ambiguous or
missing evidence; freeze the expected routine/exception classification before
the run. Record source coverage, correct routine proposals, missed cases,
incorrect proposals, user corrections, repeated data entry, clarification and
approval counts, active user time and elapsed time. Compare with the equivalent
manual workflow on the same fixture. Pass requires all records accounted for,
correct preparation of the designated routine cases without repeated manual
data entry, surfaced exceptions and no duplicate/unapproved/wrong final postings.
Approval counts alone are not errors; reducing clicks by weakening consent is
not a pass. Do not invent an automation percentage or infer external source
completeness from this benchmark. Repeat against the agreed pilot categories.

**AF-2 — Deliver artifacts, not instructions.** On a complete supported annual
fixture, after separate explicit posting, close and export approvals, the agent
executes the normal guarded operations through actual close, final K2/required
annex working papers and the selected export package. Verify retained identities,
revisions, hashes and produced artifact contents. No handoff consisting only of
"now click these screens" is completion of a supported action. User selection
of export destination and disclosure approval remain legitimate interactions.
With missing facts or denied approval, report precise partial state rather than
claiming the artifacts exist. Reopen/correction must identify affected artifacts
and require fresh approvals. Manual filing remains outside agent execution.

**AF-3 — Reuse reviewed decisions.** After a user corrects a recurring assignment,
offer an explicit, narrowly scoped reusable rule. The rule is book-bound,
versioned, inspectable and revocable, with applicability conditions and source
provenance. Retain it only within permitted encrypted storage; do not silently
train a model or add provider chat history. The next matching import proposes
the reviewed treatment and cites the exact rule; changed conditions, conflicting
rules or changed dependent account/policy state require review rather than an
arbitrary match. Rule approval is never posting or disclosure approval. Test
matching and nonmatching cases, another book, revocation and stale proposals;
historical posted records retain their original decision provenance.

**AF-4 — Resolve exceptions in the same task.** A missing document, ambiguous
partial payment and conflicting classification must be represented as distinct
actionable items in one task-level worklist. Newly supplied answers or explicitly
released evidence update the affected items and trigger their dependent checks
without requiring the user to repeat the original task or lose independent
completed work. Test answer -> re-evaluate -> reviewed action -> verified result,
plus interruption and restart. Resume from authoritative book state and retained
receipts; encrypted structured task state may be used where needed, not hidden
provider history. Clear decrypted state on lock, reacquire scope after restart
and never revive expired permissions. Existing private use must not acquire a
mandatory encryption migration; without permitted persistence, reconstruct from
current allowed records and ask only for transient facts that cannot be recovered.

**AF-5 — An equally real private workflow.** Test "Review my portfolio and help
resolve this month's outstanding assignments" through the actual scoped agent
tools. The agent identifies relevant permitted cases, proposes corrections and,
after the existing specific approvals, runs required recalculation and verifies
updated portfolio/cost-basis reports against the manual fixture. Include an
ambiguous case, denied correction and interrupted retry. Do not require entity
identity, a chart, a GL enrollment, a K2 pack or corporate setup. Preserve the
same custody/tax authority and no-unapproved-egress rules as the existing product.

## 8. K2 and tax completeness

Build an exact year-bound matrix for K2 and required annexes: each field is
derived, reviewed input, confirmed not applicable, or blocked. Include legal
identity/obligation facts and tax opening/carryforward records not derivable from
the current-year ledger. Explicitly handle applicable allowances, withholding,
losses, elections, and activity/sphere allocations; never reuse a personal tax
rate or silently classify all crypto into a capital-income annex.

Verify rules against official BMF/RIS/form sources before implementation, record
source identity/version/digest, and distinguish reviewer-approved rules from
open legal questions. No AI review substitutes for missing factual or legal
evidence. Unknown mandatory values are not zero.

Final figures trace to the closed ledger, calculation artifacts, reviewed
adjustments, carryforwards, and exact pack rules. Deliver human-readable
working papers plus machine-readable values and verified mappings usable for
manual form entry. An unsupported tax route has a precise blocker without
restricting the organization's otherwise supported bookkeeping.

## 9. Verification and delivery gate

The complete stack must pass three independent end-to-end acceptance lanes:

1. Existing private book: portfolio, cost basis, personal-tax reports, and
   optional AI assistance remain usable without organizational setup; AF-5
   proves the agent-led workflow, not just absence of regressions.
2. Organizational book: opening -> bank/cash/BTC/evidence -> review/manual
   entries -> open-item and valuation schedules -> reconciliation -> reports ->
   year-one close -> year-two continuation -> prior-year correction -> verified
   accounting package. All categories in the agreed coverage matrix pass.
3. Austrian filing preparation: exact K2/annex coverage, provenance, controls,
   carryforwards, draft/final state, and manual-entry package from that book;
   AF-2 proves approved agent execution through the actual retained artifacts.

Record AF-1 through AF-5 separately as pending, failed or passed with fixture,
source revision, executed path, review evidence and measured outcomes. Happy-path
tool mocks, the historical regression suite or missing pilot facts do not mark
these new gates passed. Manual fallback remains required but is not a substitute
for completing supported routine work through the agent-first path.

Add hostile and concurrency tests: double posting across sources, duplicates,
partial settlements, missing balances, pure value adjustments, tiny BTC fees,
stale chart/price/source/model inputs, interrupted import/post/close, locked DB,
cross-book access, evidence deletion/reset/GC, backup restore, prior-year reopen,
and checksum/report manipulation.

For AI test local processing with outbound traffic denied, disabled/unavailable
AI fallback to manual work, prompt injection in documents, unapproved tool calls,
secret leakage, hosted-context minimization across tool rounds, provider/book
switches, revoked/cancelled approvals, batch payload changes, and shared mutation
guards. Include image/text/OCR paths and local CLI provider routing.

Verify packages at separately named levels: content digests; arithmetic and
source reconciliation; calculation replay where the complete input recipe and
compatible engine are available. Do not call checksum matching independent tax
recalculation or externally anchored authenticity.

Run repository quality gates, relevant regression/regtest suites, CLI/daemon
tests, existing private UI regressions, minimal localized Assistant consent,
and packaged-build checks. Review with
independent accounting/spec/security reviewers and the previously requested
Claude CLI Fable 5.1 high check when actually available. Report any unavailable
review honestly. Fix evidenced findings in their owning PR. Every intermediate
PR must pass its scoped checks, and the integrated stack must pass the full gate;
do not merge upstream without authorization.

EBICS and FinanzOnline transmission remain excluded. No speculative ERP,
payment initiation, consolidation, extra countries, or multi-writer ledger is
required. Necessary records for the accepted organization are included, even
when their minimum useful implementation is manually maintained.

Deliver the PR stack links, final SHA, coverage matrix, test/review evidence, privacy
behavior, and exact remaining limitations. A functioning foundation or a set of
permanent blockers is not completion of the requested organizational workflow.
