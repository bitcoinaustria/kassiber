# Local general accounting and CLI

General accounting is an explicit, encrypted-book opt-in, separate from the
Bitcoin tax journal. Personal portfolio users need no chart, company profile,
accounting enrollment or new screen. Existing accounts and RP2 journal entries
are not repurposed as general-ledger accounts or postings.

This first accounting PR supplies the deterministic domain, local CLI and
internal daemon contracts. The following stacked PR adds scoped agent access
and exact approval in the existing Assistant. This PR exposes **no accounting
AI tools**, selected-disclosure chat mode or dedicated accounting UI.

## Supported workflow

- Enroll an already encrypted profile with `accounting configure`, create its
  chart and periods, then create balanced drafts and post their reviewed digest.
- Retain evidence bytes and extracted text inside SQLCipher. Local text/PDF/OCR
  workers are bounded and cancellable; unavailable extraction does not fall
  back to a remote provider. Reviewed manual transcription remains available.
- Import canonical bank CSV, reconcile separately evidenced totals, allocate
  receipts/payments, and retain open items and manually reviewed schedules.
- Capture immutable Bitcoin source and RP2 calculation artifacts, bind economic
  roles, and project ordinary ledger drafts without double recognition.
- Record historical openings, book valuations and partial valuation releases;
  cash-basis reports remain distinct from accrual financial statements.
- Inspect account ledgers, trial balance, P&L, balance sheet and close blockers.
  Reversals and reopen/correction workflows preserve historical records.
- Prepare Austrian 2025 K2 and supported annex workpapers through the versioned
  jurisdiction interface. Applicability, unknown facts, N/A and specialist
  review are explicit. Working papers are not a FinanzOnline submission.
- Use durable local tasks to select a source population, prepare, post, close,
  finalize or export one explicitly approved step at a time. Rules authorize
  draft preparation only. A saved task or receipt is not future approval.

## Command discovery and exact inputs

Use `kassiber accounting --help`, `kassiber accounting <action> --help`, or
`kassiber commands describe` to discover the command catalog. Every book
operation names `--workspace` and `--profile`; portable package verification
does not open a book. Machine responses use the standard JSON envelope.

Commands accept a JSON object via `--payload-stdin`, a bounded inherited
`--payload-fd`, or `--payload-file` paired with its exact lowercase
`--payload-sha256`. Inline `--payload` is supported but exposes values in shell
history. Keep passphrases, real evidence and financial payloads out of shell
history and public diagnostics. Non-interactive SQLCipher unlock uses the
existing `--db-passphrase-fd` path.

Monetary values are integer minor units, not floating-point major units. Use
canonical decimal strings for monetary and Bitcoin atomic quantities; large
JSON numbers are rejected before rounding can hide a change. Draft digests,
task revisions and idempotency keys bind the specific operation.

For example, feed the following explicit configuration to
`accounting configure --workspace <workspace> --profile <profile> --payload-stdin`
after creating an encrypted profile:

```json
{"currency":"EUR","timezone":"Europe/Vienna","entity_kind":"association","accounting_regime":"accrual"}
```

The operator must confirm the organization, currency, regime and dates; fixture
defaults do not infer legal obligations. Executable full workflows live in
[`test_accounting_cli_tasks.py`](../../tests/test_accounting_cli_tasks.py),
[`test_accounting_integration.py`](../../tests/test_accounting_integration.py)
and [`test_accounting_tax_workpapers.py`](../../tests/test_accounting_tax_workpapers.py).

## Approval, retention and exports

The CLI is an explicit local operator surface, not a generic model dispatcher.
Posting requires the exact current draft; task application requires current
preview/revision/digest and `confirmed:true`. Export additionally requires
`confirm_plaintext:true`. Preparation, posting, close, tax-finalization and
export remain separate decisions.

`accounting export-close` and the `task-apply` `export_close` step return package
data. Save the complete response deliberately, then pass it to
`accounting verify-package --payload-stdin`. The independent verifier checks
the recorded snapshot digest and arithmetic, not missing-source completeness
or legal correctness. Saved exports are plaintext financial information.

Posted entries, retained artifacts, close snapshots and receipts cannot be
removed through ordinary book reset/workspace deletion. Accounting tables are
excluded from replication. Existing encrypted backups preserve the local
record; tests exercise actual archive/restore under both journaling policies.

## Delivery limits

No EBICS, payment initiation, payroll engine, FinanzOnline transmission,
automatic tax advice or universal compliance certification is included.
Schedules are reviewed records, not specialist calculators. The organization's
fiscal year, tax route, VAT facts, openings and complete source population need
confirmation. Synthetic scenarios prove only their specified contracts, not
the real organization's end-to-end pilot. RP2 dependency review and full-stack
acceptance remain separate merge gates.
