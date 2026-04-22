# External Document Reconciliation — Scope and Direction

**Status:** Active design direction. This doc sets the product boundary for
merchant and accountant-facing reconciliation work before the schema and CLI
surface lands in code.
**Scope of this doc:** external business documents, BTCPay provenance, matching,
review, and the handoff into Kassiber's tax-normalization seam.

---

## Problem statement

Some Kassiber users will not just buy and sell BTC. They will:

- sell goods or services and receive BTC as payment
- pay supplier invoices in BTC
- receive or store invoice PDFs, BTCPay records, contracts, and related
  bookkeeping evidence outside Kassiber

Kassiber should help reconcile those external documents to BTC flows without
turning into an invoicing system, VAT tool, or general ledger.

## Scope verdict

In scope:

- BTC-side provenance for incoming and outgoing payments
- ingesting or linking externally created business documents
- matching those documents to BTC transactions
- review and confirmation of proposed matches
- commercial annotations that explain what a payment represents
- tax normalization that turns a confirmed annotation into RP2-facing
  primitives such as `income`, `buy`, `sell`, `move`, or `fee`
- accountant-facing exports of the BTC subledger with document references

Out of scope:

- issuing invoices
- AR / AP workflow, dunning, or customer/vendor master data
- VAT close, sales-tax returns, RKSV, or ERP features
- full company P&L / balance sheet / statutory books
- generic document management unrelated to BTC flows

## Ownership boundary

| Concern | Owner |
|---|---|
| BTC system of record, transaction provenance, attachments, review | Kassiber |
| External business documents, structured extraction, and payment matching | Kassiber |
| Typed commercial annotations such as `sales_receipt`, `supplier_payment`, `refund`, `payroll` | Kassiber |
| Lot math, basis carry, gain/loss, Austrian regime logic, country tax rules | RP2 / `bitcoinaustria/rp2` |
| Revenue recognition, VAT, COGS, company general ledger | Merchant ERP / accounting system |
| Invoice issuing and merchant workflow state | BTCPay / ERP |

The intended split stays simple:

- Kassiber owns facts, provenance, reconciliation, review, and normalization.
- RP2 owns tax primitives and computation.
- The accounting system owns the books.

## Three-layer model

Merchant reconciliation needs three separate layers inside Kassiber:

### 1. Source / provenance

Raw facts from outside or below the ledger:

- BTCPay invoice / payment ids
- raw BTCPay payload snapshots
- file paths, hashes, and attachment metadata
- external references such as invoice numbers or payment hashes

This layer answers: "Where did this fact come from?"

### 2. Commercial match

A confirmed business interpretation:

- this BTC receipt settles invoice `X`
- this BTC payment pays supplier bill `Y`
- this transaction is a refund, payroll payment, owner contribution, or
  treasury transfer

This layer answers: "What business event does this transaction relate to?"

### 3. Tax normalization

The minimal primitive Kassiber feeds to RP2:

- `income`
- `buy`
- `sell`
- `move`
- `fee`

This layer answers: "What tax-computation primitive should RP2 see?"

Do not collapse these layers into a single flag or table. The same on-chain
shape can map to different commercial and tax meanings depending on the
confirmed external context.

## Core primitive

The reusable primitive is:

**external business document/payment match**

Both of these are just producers for that primitive:

- BTCPay API-backed provenance ingest
- AI-assisted OCR / extraction from PDFs or images

This framing is better than a BTCPay-only design because it generalizes to
supplier invoices, contracts, scanned receipts, and accountant-provided files.

## Conservative defaults

Imports and sync should stay conservative until Kassiber has enough evidence to
classify a transaction.

Rules:

- BTCPay import or API ingest should not silently promote inbound receipts to
  `income`.
- Transport-level kinds such as `deposit` / `withdrawal` are correct defaults.
- A BTC receipt only becomes RP2 `INCOME` once a confirmed commercial match or
  explicit user action says it is income-like.
- Until then, inbound rows remain acquisitions (`BUY`) with normal basis
  handling.

This keeps Kassiber from inventing merchant income where the BTC flow is
actually a refund, treasury transfer, owner contribution, or something else.

## Review discipline

Matching is propose-only.

The system may:

- propose a document/payment match
- explain why it matched
- assign a confidence score
- suggest a commercial annotation
- suggest the downstream RP2-facing tax primitive

The system must not:

- silently confirm the match
- silently rewrite tax semantics
- hide why a suggestion was made

This should follow the same discipline as Kassiber's quarantine/review model:
surface uncertainty, require confirmation, and preserve an audit trail.

## RP2 boundary

RP2 does not need to understand invoices, PDFs, BTCPay records, or ERP terms.

RP2 should only receive normalized tax primitives and whatever markers are
required for the math itself.

Example:

- a confirmed sales invoice paid in BTC becomes an inbound `income` event at
  FMV, which seeds basis at that same FMV
- a supplier invoice paid in BTC becomes an outbound disposal / fee story on
  the RP2 side, while the commercial expense recognition stays outside Kassiber

Only add new RP2 markers if they change computation. If a distinction affects
report packaging or accounting export but not tax math, keep it in Kassiber.

## Initial data-model direction

The exact schema can evolve, but the shape should stay additive and
many-to-many:

- `external_documents`
  One row per external invoice, receipt, contract, or similar business
  document. Holds origin, hashes, extracted fields, and reconciliation status.
- `document_payment_links`
  The allocation table between documents and transactions. Must support one
  document paid by many transactions and one transaction settling many
  documents.
- `match_suggestions`
  Proposed matches, their confidence, explanation, and generation method.
- `commercial_annotations`
  Confirmed business meaning such as `sales_receipt`, `supplier_payment`,
  `refund`, `payroll`, `owner_contribution`.

Idempotency should key off stable external ids when available and fall back to
content hashes plus transaction/document identity, so re-imports and re-OCR do
not create duplicate proposals.

## Matching pipeline

Cheapest, most deterministic signals should run first:

1. Deterministic
   Lightning `payment_hash`, BIP21 address, exact amount, tight timestamp
   window, explicit BTCPay invoice/payment ids.
2. Heuristic
   Amount tolerances, time windows, counterparty token overlap, OCR text
   overlap, and note/tag matches.
3. Multi-leg allocation
   Split payments, partial payments, or one payment covering several invoices.
4. AI-assisted extraction or tie-breaking
   Used when PDFs need structure extracted or when several candidates remain
   plausible after deterministic and heuristic passes.

AI should explain which fields or features drove the suggestion so the review
trail remains inspectable.

## BTCPay direction

BTCPay API-backed provenance import is the intended primary path.

The current file importers remain useful as fallback and migration tools, but
the long-term BTCPay value is richer provenance:

- stable invoice ids and payment ids
- paid / settled timestamps
- invoice currency and settled amount
- order or customer references
- refund / overpayment / partial-payment state
- raw payload snapshots for auditability

## AI direction

AI belongs in Kassiber, not RP2.

Use it for:

- local OCR / structured extraction from invoice PDFs and images
- confidence-scored match suggestions
- tie-breaking when deterministic rules narrow the field but do not finish it

Keep it optional and review-gated:

- deterministic matching should work without AI
- local-first models are preferred
- remote model use, if ever supported, must be explicit opt-in per document or
  workspace

## What stays out

On purpose, Kassiber should not grow:

- invoice authoring
- VAT period reports
- bank reconciliation
- general document management unrelated to BTC
- company-wide bookkeeping features already owned by ERP/accounting software

## Implementation order

1. Add BTCPay API-backed provenance ingest with stable external ids
2. Add external-document records and attachment-aware linking
3. Add deterministic matching and allocation tables
4. Add review / confirmation workflow for matches and commercial annotations
5. Feed confirmed annotations into the tax-normalization seam before RP2
6. Add accountant-facing export of matched BTC subledger rows
7. Add optional AI extraction / tie-breaking only after deterministic matching
   is solid

## One-line restatement

Kassiber is the BTC-side subledger with provenance, reconciliation, review, and
tax normalization. RP2 is the tax math core. Invoicing, VAT, and the general
ledger stay outside.
