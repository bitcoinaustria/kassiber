# Transaction Attachments

**Status:** Shipped behavior plus project-storage guidance.
**Current source of truth:** `kassiber/core/attachments.py`, `kassiber/db.py`,
README, AGENTS.md, and TODO.md.

## Current Behavior

Kassiber already supports transaction attachments:

- copied local files stored under the managed attachments root
- URL attachments stored as literal strings
- add/list/remove/verify/gc CLI commands
- transaction-detail desktop controls for adding files, adding URL references,
  opening managed files/URLs, removing attachment rows, and manually reusing
  selected evidence from another transaction
- daemon/export evidence readiness summaries that combine direct attachments
  with reviewed source-funds link/root evidence and persisted journal/pricing
  warnings
- Reports audit package export that can include selected copied attachment
  files plus URL references in a manifest
- no URL fetching, indexing, OCR, preview generation, or health checking

## Product Boundary

For the project-bundle MVP, optimize for external document links first. Links
are enough for many invoice/contract/accountant-reference workflows and keep
backup simple because metadata lives in SQLite.

Copied-file attachments can remain supported, but do not expand them into a
document-management system unless a concrete offline/self-contained evidence
workflow needs it.

## Rules

- A transaction can have zero or more attachments.
- URL attachments are references only; Kassiber does not fetch or mirror them.
- File attachments are copied into managed local storage and tracked by hash.
- Deleting a transaction deletes attachment rows via FK behavior.
- Backup must account for the DB plus any managed copied files.
- Logs should avoid full secret-bearing URLs.

## Schema Direction

The shipped `attachments` table is the default primitive for files and URLs.
Source-of-funds evidence reuses this store through
`source_funds_source_attachments` and `source_funds_link_attachments`; it does
not introduce another blob store.

Do **not** add a separate `transaction_links` table just because an older sketch
mentioned it. Add one only if a future project-bundle migration deliberately
splits link references from managed file attachments.

If copied project-local files become central to backups, keep them under the
project directory, for example:

```text
~/.kassiber/projects/<project>/blobs/attachments/
```

## UI Direction

Transaction detail exposes:

- Add URL
- Add file
- list existing attachments
- open URL/file through the OS handler
- remove attachment
- reuse selected evidence from another transaction
- keep the in-detail evidence surface focused on direct attachments

Reused URL evidence creates a new persisted URL attachment row on the target
transaction. Reused file evidence duplicates the managed file under a new
attachment id; attachment rows must not share `stored_relpath` unless a future
shared-blob/refcount model lands. Reused evidence rows carry
`copied_from_attachment_id` and `copied_from_transaction_id` provenance when
available.

`attachments verify` remains a CLI-level integrity check. Audit readiness
summaries and audit package exports can flag copied-file rows whose managed
file is missing, but the transaction detail sheet does not hash every file on
open.

## Non-Goals

- mirroring cloud documents
- Drive/Dropbox/Nextcloud API sync
- OCR/indexing/preview generation
- photo understanding or invoice auto-extraction
- background broken-link monitoring
- a second blob store for external-document reconciliation
