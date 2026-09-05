# Opt-in general accounting (implementation in progress)

See the [acceptance record](general-accounting-acceptance.md) for technical
coverage, actual pilot prerequisites, and remaining verification limits.

The `accounting` command family uses a separate general ledger. Existing
wallet/reporting `accounts` and RP2 `journal_entries` retain their meaning.
Private portfolio and personal-tax use does not require accounting setup.

This branch is **not the completed organizational-accounting delivery** in
[the consolidated specification](../plan/17-general-accounting-and-private-ai-spec.md).
The integrated branch includes retained Bitcoin/RP2 projections, Austrian 2025
K2/annex working papers, and explicitly scoped accounting AI. Final independent
review, CLI/agent acceptance and the pilot organization's actual coverage remain
outstanding; these are implementation capabilities, not a filing certification.
An accounting period lock is not a tax-ready declaration or evidence that all
external sources have been imported.

## Current local contract

- Full accounting is CLI/Agent-only. There is no Accounting route, sidebar
  item, Settings toggle or dedicated bookkeeping form. Existing portfolio UI
  remains unchanged. Ledger enrollment is explicit through `accounting configure`;
  metadata-only `capabilities` does not enroll or encrypt a book. Old local
  visibility preferences are ignored, not migrated into financial data.
- Enabling accounting requires an already unlocked, keyed SQLCipher database.
  Opening a private plaintext project does not enroll it or migrate it.
  The check requires codec salt and a successful encrypted-schema read, and
  honors `cipher_status` when available. It does not require SQLCipher 4.12:
  that release first introduced the status pragma. See the
  [SQLCipher API](https://www.zetetic.net/sqlcipher/sqlcipher-api/).
- Functional currency, exponent, timezone, entity description, and regime are
  explicit (`accrual` or `cash_basis`). They cannot be silently changed after configuration. Fiscal
  intervals are explicit, non-overlapping inclusive calendar dates.
- Chart accounts have immutable code/name/type definitions; add new accounts
  instead of changing the meaning of posted lines.
- Drafts do not affect balances. Posting checks a reviewed payload digest,
  balance, fiscal interval, and account scope atomically. Mistaken unposted
  drafts can be discarded. Posted corrections are new reversal entries.
- Input amounts are integer minor units; JSON outputs encode monetary values
  as decimal strings. There are no floating-point amounts in this ledger.
- Bank/file/period controls are local and deterministic; no network is used.
- New accounting tables are never replicated and are absent from default
  diagnostics, ordinary audit packages, and the general in-app AI catalog.
  The scoped accounting assistant uses a separate explicit disclosure workflow,
  not generic SQL, shell, filesystem, or unrestricted accounting tools.
- Evidence bytes live inside SQLCipher, are immutable, and survive database
  backup/restore. Legacy attachment files are **not** retroactively encrypted.
- Book reset and books-set deletion refuse retained accounting books.

## CLI

Discover exact commands with `kassiber --machine commands describe accounting`.
Every book operation requires explicit `--workspace` and `--profile`.
Use `--payload-stdin` for JSON to avoid financial content in shell history.
`--payload-fd FD` supports inherited descriptors and operator-broker transport;
the existing secret-transport limit is 8 KiB, including brokered stdin. Direct
CLI stdin accepts at most 32 MiB of characters. For larger brokered inputs use
`--payload-file PATH --payload-sha256 HEX`: the daemon reads only the bounded
UTF-8 file whose exact bytes match the explicitly supplied lowercase SHA-256.
Changed file contents fail closed. No document bytes enter the broker's
argument list. `--payload` is available for small synthetic examples but is
visible in process arguments/shell history. Diagnostics redact that field.

Example JSON for `accounting configure`:

```json
{"currency":"EUR","minor_unit_exponent":2,"timezone":"Europe/Vienna","entity_kind":"association","accounting_regime":"accrual"}
```

This is configuration, not an Austrian legal classification. Account creation
uses `account-create` with `code`, `name`, and `kind` (`asset`, `liability`,
`equity`, `income`, or `expense`). `period-create` takes `period_id`,
`start_date`, and `end_date`.

Example JSON for `accounting draft` (synthetic EUR 100 receipt):

```json
{
  "idempotency_key":"receipt-001",
  "period_id":"2025",
  "entry_date":"2025-02-03",
  "description":"Reviewed receipt",
  "lines":[
    {"account_code":"bank","debit_minor":"10000"},
    {"account_code":"income","credit_minor":"10000"}
  ]
}
```

Review the returned lines before `post` with `draft_id` and `expected_digest`.
Reversals cannot themselves be reversed; correct a mistaken reversal with a
new reviewed entry (or a new closing appropriation where appropriate). Adding
an earlier fiscal interval requires explicitly reopening every later closed or
review-required interval first, so historical changes cannot silently stale a close.

Repeating an idempotency key with a different payload fails. `reverse` requires
the original entry, target date/period, reason, and new idempotency key.

`reports` requires `period_id` and returns trial balance, period P&L and cumulative
balance sheet. `close-readiness` returns the same local draft/period/source/bank/
cash controls checked by `close`, with all recorded blockers and a revision.
It does not certify absent external sources or tax readiness. `close` requires
the current book `expected_revision`; `reopen`
also requires a reason. Reopening an earlier year flags later periods for
review. Earlier close snapshots remain unchanged. A closing appropriation entry
must clear the period's P&L exactly to equity; reports retain the historical P&L.

`journal` accepts `status`, optional `period_id`, `limit` (1–500), and `cursor`.
It returns `entries` and `next_cursor`; cursors bind to the book, filters, and
current revision. A changed book requires restarting pagination instead of
silently skipping or duplicating records.

## CLI / Agent workflow

The complete deterministic CLI works without AI or a running desktop. Use
`accounting workbench` with `period_id` for scoped counts, selected statement
identities, local close blockers and an honest external-completeness flag.
It is a local worklist, not an accounting screen or a whole-book AI disclosure.

1. Configure the encrypted book, chart and period through the CLI. Retain
   evidence/import statements, review extracted fields and explicitly approve
   any proposal rules through their scoped commands.
2. `task-create` freezes `period_id`, `idempotency_key` and explicit
   `statement_ids` (or `include_period_statements: true`), optional
   `evidence_ids`, `draft_ids` and `tax_workpaper_id`. No first-page-only scope.
3. `task-get` / `task-preview` inspect actual state and an exact step:
   `prepare`, `post`, `close`, `tax_finalize`, `export_close`, `export_tax`.
   `task-apply` requires `task_id`, `step`, `expected_revision`,
   `expected_digest`, `idempotency_key`, and `confirmed: true`. Exports also
   require a separate `confirm_plaintext: true`.
   Review that exact preview before constructing approval; use a fresh preview
   after any mutation. These are explicit operator commands, not automatic AI consent.
4. The ordinary Assistant / interactive `kassiber chat` may instead receive
   an opaque task ID (“Continue accounting task …”). Its four tools are
   `ui.accounting.task_get`, `task_preview`, `task_apply`, `task_cancel`.
   They expose bounded state/counts, opaque approval IDs and committed receipts,
   not bank names, amounts, descriptions, evidence text or tax fields.
   Initial task creation, source assignment and rules remain explicit CLI
   operations; ordinary remote chat cannot independently browse/classify the book.
5. Every mutating agent step requires a new human approval. Full financial
   effects are shown locally by the existing Assistant consent dialog or CLI
   terminal, never supplied as provider tool context. The daemon recomputes
   the preview, checks scope/revision/digest and rejects expired grants.
   `--yes`, `--allow-tool` and session grants cannot bypass this. Noninteractive
   chat denies these actions; authorized automation uses the guarded direct CLI.
6. Retry reads durable receipts; cancellation does not erase committed work.
   `task-source-assign` records reviewed evidence-only links or standalone
   expense assignments. Rules are book-scoped, explicit and revocable, not consent.
7. For a genuinely new missing document, retain it in the same encrypted book,
   then use `task-amend-preview` with `task_id`, `period_id`, exact additive
   `evidence_ids` and `reason`. Locally approve `task-amend` with that selection,
   `expected_digest`, `expected_revision`, `idempotency_key` and `confirmed: true`.
   An append-only receipt extends effective source scope; the original selection
   and completed receipts stay immutable. Old step approvals become stale.
   Cancelled tasks and closed periods cannot acquire new sources; reopen is a
   separate action. Neither amendment command is an ordinary AI tool or an
   automatic file-discovery/disclosure grant.

Task export prepares reproducible bytes and an artifact identity; it does
**not** claim to have saved a file or filed a return. Use the explicit CLI
`--machine --output close.json` destination for the returned JSON export envelope (plaintext financial
content), then verify close packages with `accounting verify-package`.
The verifier accepts raw close packages, `accounting.export-close` envelopes,
and genuine `accounting.task-apply` close-export envelopes. Tax HTML/JSON is a
working paper for manual use, not official form submission.

Interactive chat can deliver the actual approved export without giving the
model filesystem access: `kassiber chat --accounting-export TASK_ID export_close
/absolute/existing-parent/close.json` (use `export_tax` for the full tax JSON,
including its HTML rendering). Repeat for another exact task/step/destination.
The path stays in the CLI; the local once-only approval shows it and authorizes
plaintext release. No directory grant, automatic output location or overwrite
is permitted. The CLI verifies the approved artifact identity, content hashes
and close-package arithmetic, publishes a private file exclusively, and checks
its exact bytes on readback. Only the deterministic **LOCAL EXPORT RECEIPT**
confirms a saved and verified file; provider responses still report preparation,
not saving, certification or filing. A byte-identical existing file is a safe
retry; conflicts or failed delivery leave the retained artifact prepared and
require fresh approval. Failures after publication report an unknown saved state
and `may_exist: true`; they never remove a possibly changed target. Symlink
parents/targets and unsupported safe filesystem
operations fail closed. This is a JSON handoff, not a standalone HTML export.
The local stream sideband uses the existing 64 MiB accounting-package bound,
including JSON escaping; larger
artifacts stay prepared and require the explicit direct CLI export above.

CLI `--transcript` strips the local accounting consent preview and export bytes.
Export sidebands are also excluded from `--stream-json`, model context,
tool history, logs and diagnostics; no Assistant export UI is added.
Explicit `--stream-json` is raw local event output and **can contain that
financial preview**; treat it as sensitive. Raw-stream chat does not approve
financial mutations. User-authored chat messages and explicit file exports
still have their normal disclosure/storage semantics; do not paste financial
documents into an ordinary hosted chat as a shortcut.

The experimental full UI is recoverable on local branch
`codex/accounting-ui-preserved-20260905` at `fbfce410`; current delivery branch
is `codex/accounting-cli-agent`. No financial data was deleted.

## Supporting records

`evidence-add` takes bounded `content_base64`, `name`, `media_type`, and optionally
an existing same-book `source_document_id`. `evidence-list` returns metadata,
not bytes. The CLI supports encrypted chunked upload with cancellation. `evidence-upload-begin`, `-append`, `-finish`, and
`-cancel` expose the same protocol; `evidence-upload-list` lists unfinished
uploads for cleanup after a lock or restart. Files are limited to 20 MiB,
chunks to 256 KiB, and unfinished uploads to ten per book. Finishing verifies
the complete content digest before retaining immutable evidence.

`document-extract` defaults to local text extraction. UTF-8 text and manual
`document-transcribe` need no extra runtime. Native PDF text and OCR run in a
bounded cancellable POSIX subprocess; unsupported platforms fail explicitly.
OCR is opt-in (`method: "ocr"`), requires already installed Tesseract (plus
Poppler for PDFs), and accepts explicitly selected PDF pages and languages.
There is no installer, model download, network fallback, or plaintext document
cache. This checkout has no Tesseract binary: real OCR execution is still an
unverified optional integration, while bounded worker and Poppler tests run.
`document-review` records typed fields with literal page/span provenance;
search queries retained metadata/text locally. Lock or scope changes discard
pending worker results. Original evidence bytes are never rewritten.

`bank-preview` / `bank-import` use the explicitly versioned
`kassiber-canonical-bank-csv-v1` interchange adapter, **not a verified
bank-specific export adapter**. Required CSV columns, in order:

```csv
row_id,date,amount_minor,description
bank-reference-001,2025-02-03,10000,Receipt
bank-reference-002,2025-02-04,-2500,Payment
```

Import adds `account_code`, `statement_id`, `start_date`, `end_date`, and optional
`opening_minor`, `closing_minor`, `evidence_id`. Stable row IDs belong to the
statement; two identical payments with distinct IDs remain two payments.
Overlapping active statements and reuse of one statement evidence record for
another active statement are rejected. Missing control balances/evidence or
unallocated row amounts cannot count as reconciled. Arithmetic agreement is
separate from source completeness. Complete reconciliation requires
`control_evidence_id`, `control_locator`, and `control_review_reason`, independently
retaining the statement's reviewed PDF control totals or exact
`kassiber-bank-control-v1` JSON. A row-only CSV is not balance-control evidence.

Preview/import accepts exactly one of `csv_text` or `csv_evidence_id`. The latter
loads the retained, digest-verified UTF-8 source in the same book, and import
binds that exact evidence record to the statement. It cannot be replaced with
an unrelated `evidence_id`. Retain the selected CSV explicitly before importing it. The canonical parser is bounded to 4 MiB and 10,000 rows.

`bank-allocate` links signed bank rows to compatible posted bank-account lines
with explicit partial amounts. `bank-list` and `bank-reconcile` show remaining
amounts and allocation IDs. Corrections use `bank-void-allocation`, then
`bank-void-statement` where needed, followed by a separately identified corrected
statement. Cancellation events retain their reasons and original records.

`item-create` records a payable/receivable backed by evidence and its posted
control-account line. `item-allocate` settles it against compatible posted
lines, up to both remaining budgets. `item-void-settlement` and `item-void`
provide the reviewed correction path before reversing affected entries.
`item-revise` corrects document reference/due-date metadata through an
effective-dated, evidence-backed digest chain; it never changes the financial
origin, amount, or settled balance. It requires the current metadata revision
and digest. Changes affecting a closed/review-required interval are blocked;
later effective corrections preserve earlier close metadata.

`schedule-create` / `schedule-revise` retain bounded exact manual fields for
supporting schedules. They do not implement a tax/depreciation calculator.
Effective date and authoring revision are separate: `head_revision` is the
optimistic update token, while `revision` identifies the applicable record.
Closed/review periods must be reopened before affecting their supporting records.

`evidence-list`, `bank-list`, `item-list`, and `schedule-list` accept `limit`
(1–500) and `cursor`, returning the corresponding collection, `next_cursor`,
`binding`, and `total_count`. Continuations bind to book, collection, and current
inputs; stale pages require refresh. CLI callers must follow cursors to inspect every record, not just the first 500.

## Source calculations and book valuation

`source-preview`/`source-capture` retain canonical source commitments;
`source-bind` allocates exact observed quantities with an economic identity and
explicit recognition/settlement role. Coverage and remaining quantities remain
visible. `calculation-capture` retains actual outputs from the pinned RP2
adapter with input/method/pool/dependency commitments and an exclusive cutoff.
Opening captures end before the first day; closing captures can explicitly
select an `as_of_date` within the fiscal interval.

`projection-policy-create` maps asset/transit/settlement/income/gain/fee accounts.
The reviewed compatible policy follows RP2 basis without claiming universal
corporate-tax suitability. `projection-create` produces ordinary balanced
drafts; publication rechecks exact source/artifact commitments. Pure quantity
events do not fabricate a one-cent posting. Reversals retain dated source and
quantity history. `opening-preview`, `opening-bind`, and `opening-create`
carry reviewed historical quantities/basis and additional fiat balances into
the first ledger opening without duplicating historical revenue.

Book-only `valuation-create`/`valuation-post` records impairment, write-back or
reviewed revaluation against retained evidence and actual open RP2 lot quantities.
The adjustment changes book value, not tax basis. Later disposals release the
layer using the actual captured consumed lots; custody transfers carry it.
The layer does not determine whether a valuation is legally permitted.
Close compares quantities, book balances plus these retained adjustments, and
unchanged tax artifacts. An arbitrary scalar schedule cannot override that check.
Currency representation uses the HALF_EVEN-rounded cumulative exact basis of
each asset/transit account, not independently rounded disposal fragments.
Each publication retains its rounding remainder and policy offset. Dependent
proposals must be reviewed against the current preceding postings; an earlier
publication invalidates an already prepared sibling proposal rather than
silently recomputing an approved batch. Reversals unwind dependent later
currency movements first.

## Cash and reviewed AI assistance

The optional cash book selects cash/bank/loan accounts explicitly. It retains
physical counts and partial payment-flow classifications backed by posted
lines and evidence. Internal transfers, borrowing and equity are non-result
flows. Missing counts are unknown, not zero. Its income/expenditure statement
is payment-based, separate from accrual P&L; its asset statement uses posted
carrying values. Configuration is not a legal determination of the correct regime.
Activity on a selected liquidity account before its effective selection date
is surfaced as a coverage gap, not retroactively classified or silently omitted
from a complete annual statement. Such gaps block a cash-basis close.

Accounting AI works only with explicitly selected pages/fields/entries/periods
and chart accounts. The tax-explanation purpose may additionally include one
explicitly selected working paper; no other papers or evidence bytes are added.
Its exact preview is recomputed at consent consumption, so a tax-only revision
invalidates earlier consent even without a book-revision change.
A local preview binds the exact content, provider/model,
unlocked book and revision to a short-lived, single-use consent. Hosted use is
an explicit disclosure; local use never silently falls back to a hosted model.
Sensitive CLI providers must prove supported no-history/no-tools settings.
On Windows the sensitive CLI path fails before provider startup with
`ai_sensitive_provider_unavailable`: reliable provider-process-tree cancellation
has not been verified there. Explicitly configured HTTP providers remain
available; ordinary portfolio CLI assistance is unchanged. On supported
platforms cancellation is latched during CLI discovery/startup, so an early
lock or scope change cannot be forgotten before the selected prompt is sent.
Prompts, results and tokens are not chat history, diagnostics or replication.
Output is plain text, never automatically fetched links or embedded content.

Structured AI output must pass the same draft/document validators as manual
input. Review previews use rollback-only savepoints. A separate human approval
creates drafts or reviewed fields atomically and retains proposal provenance;
it cannot post entries. Redacted pages cannot supply unreliable original-span
offsets. `batch-preview`/`batch-post` separately bind up to 50 reviewed drafts
and atomically post all or none. AI remains optional for every accounting step.

### Selected financial assistance from the CLI

Select the intended book first with `context set --workspace … --profile …`.
Create a UTF-8 JSON selection file containing only this explicit request:

```json
{
  "profile_id": "CURRENT_PROFILE_UUID",
  "question": "Suggest a draft from this reviewed source",
  "purpose": "draft_entry",
  "selection": {
    "extractions": [{"id": "EXTRACTION_ID", "pages": [1], "fields": []}],
    "period_ids": ["2025"],
    "include_chart": true
  }
}
```

The IDs above are placeholders, not discovery permissions. The explicit purpose
is one of `document_fields`, `document_sorting`, `draft_entry`,
`reconciliation`, `closing_checklist`, or `tax_explanation`.
Then run, with the lowercase SHA-256 of the exact file bytes:

```sh
kassiber --data-root ROOT chat \
  --accounting-selection selection.json \
  --accounting-selection-sha256 SHA256 \
  --provider NAME --model MODEL
```

Selection metadata is capped at 64 KiB; the existing disclosure budget still
bounds the selected financial context. The daemon checks the explicit profile
against its current book; the command never silently switches books. Inspect
the full local context and provider/model preview, then separately approve
disclosure. This runs one tool-free, no-history request through the existing
provider protections. Any structured candidate gets another canonical local
preview and separate consent before creating drafts or reviewed fields; never
posting. Denial, stale inputs or shutdown invalidate pending grants.

Both input and output must be interactive terminals. This mode rejects
`--yes`, tool grants, transcripts, raw-stream/machine output, output files,
session/history continuation, other file attachments, custom prompts and
generation overrides. Reasoning effort and bounded timeout remain supported.
The selection filename/hash are redacted in public diagnostics. Treat this
file and terminal scrollback as local sensitive information.

## Austrian working papers

The bundled `AT`/2025 pack covers K2, K2kv, K2a, K2b, K11, K12 and K12a with
versioned official-field mappings, applicability decisions, exact calculations
where supported, and explicit evidence-backed specialist fields elsewhere.
It is not K1/K3 support or a universal autonomous corporate-tax calculator.
An assessment year aggregates all relevant fiscal periods ending in that year.
Tax previews precede closing entries; finalized working papers bind final closes.
Reopening any included period makes the final paper stale without rewriting it.

`tax-create`, `tax-review`, `tax-preview`, `tax-finalize`, and `tax-export` share
CLI/internal daemon contracts. Exports are plaintext HTML/JSON working papers for manual
filing, not an automatically filled official PDF or FinanzOnline submission.
CLI exports require explicit plaintext consent and an explicit output destination
when saving files. JSON exports include the exact retained `report_json` string as the
SHA-256 input for `report_digest`; the separate `report` object is a wire-safe
display projection, not that hash input. `html_sha256` hashes the current HTML
bytes, including its current staleness warning. Reopening never rewrites the
original report string or digest. Every special-case applicability decision and carryforward requires
review; missing data is not assumed to be zero. Further countries require
versioned packs/adapters, not changes to the country-neutral ledger.

## Close packages

`export-close` requires `close_id` and `confirm_plaintext: true`. The result is
a **plaintext financial disclosure**; use existing `--output` deliberately.
The package contains the original immutable `snapshot_json` string and digest,
not a fresh reconstruction from today's chart or journal. JSON inside the
string preserves exact integers even in a JavaScript consumer.

`accounting verify-package --payload-stdin` works without a database or book
selection. It accepts the package or the `accounting.export-close` CLI envelope.
It independently checks snapshot/entry commitments and ledger/report arithmetic.
It explicitly does **not** claim independent bank-source completeness, RP2/tax
replay, or externally anchored authenticity. A self-consistent package can be
constructed by anyone; trusted original commitments must come from elsewhere.
The verifier accepts snapshots up to 64 MiB, with a separate bounded input
allowance for JSON escaping; it is not constrained by the normal 32 MiB input
limit. Monetary report values inside the committed snapshot must be integers,
not numerically equivalent floating-point values or booleans.
Close enforces the same snapshot bound before committing a period lock; an
oversized close fails without leaving an unexportable locked period.

No new mandatory runtime dependencies were added. The implementation reuses
SQLCipher, RP2 and existing report/cryptography libraries. Optional native
Poppler/Tesseract capabilities are explicitly detected and never installed or
downloaded by accounting operations. RP2 remains the Bitcoin tax calculation
authority, not the general ledger. A real encrypted tar/age backup round-trip
test restores evidence, text, open items, projections and two retained closes.
