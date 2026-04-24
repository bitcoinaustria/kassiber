# External Document Reconciliation

**Status:** Active design direction; schema and CLI surface have not landed.
**Current source of truth:** shipped transaction attachments, BTCPay wallet
history sync, `TODO.md`, and this boundary doc.
**Core rule:** Kassiber is the BTC-side subledger. It is not an invoicing, VAT,
ERP, or general-ledger product.

## Problem

Some users receive or spend BTC for real-world business activity and keep
evidence elsewhere: invoices, receipts, contracts, BTCPay records, emails, and
accountant-provided files.

Kassiber should reconcile those documents to BTC flows, preserve provenance,
and feed confirmed tax primitives to RP2 without becoming the merchant system of
record.

## In Scope

- BTCPay invoice/payment provenance ingest
- linking or storing externally created documents through the existing
  attachment store
- matching documents to BTC transactions
- confidence-scored suggestions with explanations
- user review and confirmation
- commercial annotations such as `sales_receipt`, `supplier_payment`, `refund`,
  `payroll`, or `owner_contribution`
- normalized RP2-facing primitives such as `income`, `buy`, `sell`, `move`, or
  `fee`
- accountant-facing BTC subledger exports with document references

## Out Of Scope

- invoice issuing
- AR/AP workflow, dunning, customer/vendor master data
- VAT, RKSV, sales-tax returns
- company P&L, statutory books, or full balance sheet
- generic document management unrelated to BTC flows

## Ownership Boundary

| Concern | Owner |
|---|---|
| BTC transactions, provenance, review, attachments | Kassiber |
| Document extraction and payment matching | Kassiber |
| Commercial annotations | Kassiber |
| Lot math, basis carry, country tax rules | RP2 / `bitcoinaustria/rp2` |
| Invoice issuing, VAT, COGS, company ledger | ERP/accounting system |

## Three Layers

Keep these separate:

1. **Source / provenance**: where the fact came from, including raw BTCPay IDs,
   payload snapshots, attachment IDs, invoice numbers, or payment hashes.
2. **Commercial match**: what business event the payment relates to.
3. **Tax normalization**: what primitive RP2 should compute.

Do not collapse these into one transaction flag. The same on-chain shape can be
a sale, refund, owner contribution, supplier payment, or treasury movement.

## Conservative Defaults

- BTCPay import/sync must not silently promote inbound BTC to `income`.
- Transport-level kinds such as `deposit` and `withdrawal` are safe defaults.
- A receipt becomes RP2 `INCOME` only after confirmed commercial context or an
  explicit user action.
- Matching is propose-only until reviewed.
- Suggestions must preserve the explanation, confidence, and generation method.

## Data Model Direction

Use the shipped `attachments` table/store for files and URLs. Do not create a
second blob store.

Likely future tables:

- `external_documents`: one invoice, receipt, contract, or related document
- `external_document_attachments`: joins documents to existing attachments
- `document_payment_links`: many-to-many allocations between documents and
  transactions
- `match_suggestions`: proposed links, confidence, explanation, method
- `commercial_annotations`: confirmed business meaning

Idempotency should prefer stable external IDs and fall back to content hashes
plus document/transaction identity.

## Matching Pipeline

Run cheap deterministic signals first:

1. payment hash, BIP21 address, exact amount, tight timestamp window,
   BTCPay invoice/payment IDs
2. amount tolerances, time windows, counterparty overlap, OCR text overlap,
   notes/tags
3. multi-leg allocation for partial/split payments
4. optional AI extraction or tie-breaking after deterministic matching narrows
   the field

AI is optional, review-gated, and belongs in Kassiber, not RP2. Prefer local
models. Any remote model use must be explicit opt-in.

## Implementation Order

1. Extend BTCPay provenance beyond confirmed wallet history with stable
   invoice/payment IDs and raw payload snapshots.
2. Add external document records that reuse shipped attachments.
3. Add deterministic matching and allocation tables.
4. Add review/confirmation workflow.
5. Feed confirmed annotations into `kassiber/core/tax_events.py`.
6. Add accountant-facing export.
7. Add optional local AI extraction/tie-breaking only after deterministic
   matching is solid.

## One-Line Restatement

Kassiber reconciles BTC flows to external evidence and normalizes reviewed facts
for RP2. Invoicing, VAT, and the company ledger stay outside.
