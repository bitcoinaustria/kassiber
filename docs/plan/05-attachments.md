# Transaction Attachments

**Status:** Shipped behavior plus project-storage guidance.
**Current source of truth:** `kassiber/core/attachments.py`, `kassiber/db.py`,
README, AGENTS.md, and TODO.md.

## Current Behavior

Kassiber already supports transaction attachments:

- copied local files stored under the managed attachments root
- URL attachments stored as literal strings
- add/list/remove/verify/gc CLI commands
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

Do **not** add a separate `transaction_links` table just because an older sketch
mentioned it. Add one only if a future project-bundle migration deliberately
splits link references from managed file attachments.

If copied project-local files become central to backups, keep them under the
project directory, for example:

```text
~/.kassiber/projects/<project>/blobs/attachments/
```

## UI Direction

Transaction detail should eventually expose:

- Add URL
- Add file, if retained in desktop MVP
- list existing attachments
- open URL/file through the OS handler
- remove attachment
- verify copied files where useful

## Non-Goals

- mirroring cloud documents
- Drive/Dropbox/Nextcloud API sync
- OCR/indexing/preview generation
- background broken-link monitoring
- a second blob store for external-document reconciliation
