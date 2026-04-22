# Transaction Attachments

**Status:** Historical design note. Attachments have since shipped, but the implementation landed with a simpler CLI and storage layout than this draft describes. Treat `README.md`, `AGENTS.md`, and `kassiber/core/attachments.py` as the source of truth for current behavior until this plan doc is fully reconciled. Long-term attachment layout should follow the project-bundle storage target in `03-storage-conventions.md`.
**Scope:** CLI in Phase 0.5, UI drag-drop in Phase 3.
**Purpose:** Let users tag a receipt PDF (or other file) or a drive link to any transaction. Supports audit trails, personal bookkeeping, and Finanzamt backup.

A transaction can have zero or more attachments. Each attachment is either a **local file** (copied into a content-addressed store) or an **external URL** (just a string).

## User stories

1. As a self-employed user, I paid an invoice in BTC. I want to attach the PDF invoice to the outgoing transaction so that during an audit I can show "this BTC went here, and here's the invoice."
2. As a business accepting BTC payments, I receive a transaction and want to attach a link to the Google Drive folder where the counterparty's contract lives.
3. As a tax filer, I want my Steuerberater to see the receipt behind every significant transaction without me having to email files separately.
4. As an importer of CSV data, some rows need proof of external context (hash of a terms sheet, scan of a wire receipt). I attach them per row.
5. As someone who uses `kassiber backup` to archive my state, I want the attached files to travel with the database so restoring is complete.

## Data model

### New table: `transaction_attachments`

```sql
CREATE TABLE transaction_attachments (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    tx_id           TEXT    NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind            TEXT    NOT NULL CHECK (kind IN ('file', 'url')),
    -- for kind='file':
    sha256          TEXT,                -- 64-hex lowercase; NULL for URL kind
    filename        TEXT,                -- original user-facing filename
    mime            TEXT,                -- detected at attach time, e.g. 'application/pdf'
    size_bytes      INTEGER,
    -- for kind='url':
    url             TEXT,                -- the literal link; NULL for file kind
    -- shared:
    note            TEXT NOT NULL DEFAULT '',  -- short user-entered description
    created_at      TEXT NOT NULL,       -- ISO8601 UTC
    CONSTRAINT file_fields  CHECK (kind != 'file' OR (sha256 IS NOT NULL AND filename IS NOT NULL)),
    CONSTRAINT url_fields   CHECK (kind != 'url'  OR (url IS NOT NULL))
);

CREATE INDEX idx_attachments_tx ON transaction_attachments(tx_id);
CREATE INDEX idx_attachments_sha ON transaction_attachments(sha256);
```

Migration file: `core/migrations/002_add_transaction_attachments.sql` (or whatever the next version is).

### File storage layout

Content-addressed directory under the project bundle:

```
~/.kassiber/projects/<project>/blobs/attachments/
  ab/
    ab2341c9e7f3...        # on-disk name is the sha256, no extension
  cd/
    cd5e9a1b347f...
  ...
```

Rules:
- **Filename on disk is `<sha256>`** — no extension, no decoration. The
  sha256 is the full on-disk identity. User-facing filenames (and their
  extensions) are preserved only in the `transaction_attachments` DB
  row; on-disk blob naming is purely content-addressed.
- Why extensionless: if the same bytes are attached twice under
  different filenames (`invoice.pdf` once, `invoice.bin` once), both
  attachments share the single on-disk blob. Any scheme that bakes an
  extension into the on-disk filename either picks a winner (lossy) or
  stores the blob twice (breaks dedup). Backup and restore also rely
  on being able to locate a blob by its sha256 alone — putting the
  extension in the DB row keeps blob identity decoupled from user
  metadata.
- The first two hex chars of the sha256 form the subdirectory, keeping
  any one directory from growing unbounded.
- Identical files (same sha256) are stored once; the DB row captures
  the user-facing filename + note per attachment.
- On attach: compute sha256 of the source file, copy (not move) into
  the store if not already present. Write the blob first, `fsync`, then
  commit the DB row — content-first ordering that backup relies on
  (see `03-storage-conventions.md`).
- On detach: delete the DB row; the on-disk blob is not removed
  inline. GC happens on an explicit `kassiber attachments gc`
  command — simpler, safer, and backup excludes un-GC'd orphan blobs
  from archives so the "keep the blob for later" choice does not
  leak deleted documents into future backups.

Why content-addressed:
- Deduplication: same invoice attached to two related transactions costs one copy on disk
- Integrity: re-computing sha256 on read detects tampering; trivial verification because the on-disk filename IS the expected hash
- Rename-safe: the user-facing filename is metadata, not the storage key

### URL attachments

Just a string. No fetching, no caching, no link-checking. Opening the URL is the user's OS default browser handler's problem.

## Behavior rules

| Situation | Behavior |
|---|---|
| User attaches a file | Copy into store, insert row, return the attachment record. Original file untouched. |
| User attaches same file twice (by content) | Second attach creates another row but no extra disk copy (dedup by sha256) |
| User attaches same file to same transaction twice | Allowed. It's the user's call. No de-dup at row level. (Alternative: enforce UNIQUE(tx_id, sha256) — rejected for flexibility.) |
| User detaches an attachment | Delete the row. GC happens later. |
| Transaction is deleted | `ON DELETE CASCADE` removes attachment rows. File stays on disk until GC. |
| User runs `kassiber attachments gc` | Walk the store; for each file, check if any row references the sha256; if none, delete the file. Print summary. |
| User runs `kassiber backup create` | MVP scope: produce an archive containing DB + only the attachment blobs the DB still references (orphans excluded) + exports + manifest, via the canonical Backup flow in `03-storage-conventions.md`. Attach-plan does not restate the archive format here; see that doc for the single source of truth. |
| User restores a backup | Not in MVP. Both in-place Restore and Install-bundle-as-new-project are deferred until an authenticated bundle format lands — see `MVP does not ship in-place restore` in `03-storage-conventions.md`. Restoring an archive today is not supported inside Kassiber. |
| Maximum attachment size | 50 MB per file for MVP; configurable later via `ui:max_attachment_mb` in settings. Hard limit 500 MB (reject with error). |
| Allowed MIME types | Not restricted. Detected via python-magic or `mimetypes` stdlib; stored but not gatekeeping. |

## CLI surface

```
kassiber metadata records attachment add-file --transaction <tx-id> --file <path> [--note "..."]
kassiber metadata records attachment add-url  --transaction <tx-id> --url <url>   [--note "..."]
kassiber metadata records attachment list     --transaction <tx-id>
kassiber metadata records attachment remove   --attachment <attachment-id>
kassiber attachments gc
kassiber attachments verify
```

Envelope shapes:

```json
// attach success
{"kind": "metadata.records.attachment.add-file", "schema_version": 1, "data": {"attachment_id": 42, "tx_id": "tx_123", "kind": "file", "sha256": "ab...", "filename": "rechnung.pdf", "size_bytes": 182344}}

// list
{"kind": "metadata.records.attachment.list", "schema_version": 1, "data": {"tx_id": "tx_123", "attachments": [ {...}, {...} ]}}

// gc
{"kind": "attachments.gc", "schema_version": 1, "data": {"freed_count": 3, "freed_bytes": 524288}}
```

## UI surface (Phase 3)

### Transaction Detail dialog

An "Attachments" section near the bottom:

- **Drag-drop zone**: "Drop a file here to attach" outline, accepts PDF/image/text via Qt's drag-drop API
- **Add URL** button: opens a small inline form (URL + optional note) → Save button adds the row
- **Attachment list**: each row renders as a chip with:
  - Icon indicating kind (file-type icon or link icon)
  - User-facing filename (or a truncated URL)
  - Note (if present) in smaller mono text
  - Hover: size and created_at tooltip
  - Click: opens in system default handler (`QDesktopServices.openUrl(QUrl.fromLocalFile(path))` or the URL directly)
  - Remove button (X) with confirmation

### Future UI touches (Phase 5+)

- Thumbnails for images
- Inline PDF preview for the first page
- "Drag a transaction onto an attachment file in Finder" sorcery — skip this, too clever

## Privacy and security

- **No auto-fetching of URL attachments.** Kassiber never makes an HTTP request for a user-pasted link.
- **No indexing of attachment content.** We don't OCR PDFs or extract text. MVP stores blobs, serves them back.
- **Disk encryption** is OS-level (same as the DB). If encrypted-at-rest becomes a priority, attachments get the same treatment as the DB.
- **Log hygiene**: attachment filenames are logged (useful for debugging); content hashes are logged; note text is logged with potential PII truncation. Absolute paths of user home directory are logged as-is — acceptable because logs are local.
- **Opening attachments** delegates to the OS default handler. A maliciously crafted PDF is the user's problem, not kassiber's — same threat model as opening any attachment from email.
- **URL attachments** are strings. No validation; we don't parse them. Clicking passes to `QDesktopServices` which routes to browser. Standard web-URL handling risk applies.

## Capacity and performance

- A typical user attaching one receipt per significant transaction over a decade: ~1000 files, ~500 MB aggregate. Fine.
- A heavy user with image-heavy documentation: ~10k files, ~20 GB. Still fine on SQLite + filesystem.
- Attachment-intensive use isn't a kassiber strength; if someone wants to store full photo libraries, they're in the wrong tool.

## GC strategy

Orphan cleanup is **not automatic** because:
- It requires a table scan; cheap but still I/O on every detach
- Accidental deletion is worse than wasted disk
- The user has a dedicated `kassiber attachments gc` command they can run whenever

What GC does:
1. Build set of referenced sha256s from `transaction_attachments WHERE kind='file'`
2. Walk `blobs/attachments/` directory
3. For each file whose sha256 is not in the set, delete it
4. Report count and bytes freed

Runtime on a 10k-attachment store: <5 seconds. Acceptable as a manual action.

## Backup (MVP); install-bundle and in-place restore (deferred)

Attachments travel inside the project bundle. The canonical bundle
manifest, the schema gate, and every flow that reads or writes the
bundle live in `03-storage-conventions.md` — this doc no longer
restates them so the two cannot drift. In MVP the only active
archive flow is **Backup**. Both archive consumers — Install bundle
as new project, and the in-place Restore — are documented there but
deferred until an authenticated bundle format lands.

Attachment-specific notes:

- `blobs/attachments/` is part of the bundle; the content-addressed
  layout described above is exactly what gets archived.
- **Content-first ordering.** Attach-file must write the blob to
  disk (under its final extensionless `<sha256>` path — see File
  storage layout above) and fsync both the file and its containing
  subdirectory before it commits the DB row that references it.
  This guarantees that any attachment row visible in a consistent
  DB snapshot has its blob durably on disk by the time a concurrent
  backup reads `blobs/attachments/<xx>/<sha256>`. Without the
  subdirectory fsync, a crash between write and commit could leave
  a ghost directory entry whose contents were never persisted.
- **Project-wide locking.** `attachments gc` and Backup take the
  project-wide exclusive lock defined under
  `Interprocess coordination` in `03-storage-conventions.md`, and
  mutually exclude each other. Attach-file, detach, and transaction
  delete take the shared lock — they may run alongside readers but
  cannot run while Backup or GC holds exclusive. There is no
  concurrent blob deletion during Backup, because GC is the only
  path that removes blobs and GC cannot coexist with Backup. (When
  in-place Restore and Install-bundle eventually ship, they join
  this set as additional exclusive-mode ops — Install-bundle
  against its own global `~/.kassiber/.import.lock`, in-place
  Restore against the project lock.)
- **Manifest-verified consistency, hashed not just listed.** The
  archive's `_bundle_manifest.json` enumerates every blob the
  snapshot DB references as `{sha256, path}`. When the deferred
  archive-consumption flows ship, they verify by recomputing the
  sha256 of each unpacked blob and refusing the archive if any
  hash does not match its manifest entry. Presence at the right
  path is necessary but not sufficient; corrupted or bit-rotted
  bytes at the right filename still reject. This is a corruption
  guard, not a tamper guard — the manifest lives inside the
  archive, so a forger can rewrite both the blobs and the manifest.
  That is exactly why neither Install-bundle nor in-place Restore
  ships in MVP.
- **Orphan blobs are excluded from backups.** Backup copies only the
  blobs the DB snapshot still references, not the full
  `blobs/attachments/` tree. An attachment the user detached — but
  that `kassiber attachments gc` has not yet reclaimed from live disk —
  does not ride along into the archive. That closes an otherwise
  silent data-retention leak: a document the user thought they removed
  would otherwise follow every subsequent backup to their accountant,
  another machine, or off-site storage. Live disk may still hold the
  orphan blob until the next GC, which is a user-visible housekeeping
  choice, not a data-retention failure.
- **Broken-blob surfacing.** If a live bundle ever does reference a blob
  that is missing (recovered-by-hand archive, external tampering,
  filesystem damage), the UI surfaces a broken-attachment warning on
  that row rather than silently degrading it.

## Implementation touchpoints

- `core/attachments.py` — new module: `attach_file(conn, tx_id, path, note="")`, `attach_url(conn, tx_id, url, note="")`, `list_attachments(conn, tx_id)`, `detach(conn, attachment_id)`, `gc(conn)`, `verify(conn)`
- `core/repo/attachments.py` — CRUD with typed `Attachment` dataclass
- `cli/commands/metadata.py` — transaction-bound attachment subcommands
- `cli/commands/attachments.py` — maintenance commands (`gc`, `verify`)
- `ui/viewmodels/transaction_vm.py` — attachment list as Qt property; signals for attach/detach
- `ui/resources/qml/dialogs/TransactionDetail.qml` — drag-drop zone, URL form, chips
- Migration `core/migrations/002_add_transaction_attachments.sql`

## Non-goals

- **Versioning of attachments.** If the user attaches a replacement invoice, they detach the old and attach the new. No history.
- **Searching inside attachments.** No OCR, no full-text index.
- **Cloud backup integration.** The user can sync the project bundle with their own tool.
- **Signing / timestamping attachments.** Future feature; keep the data model additive-compatible.
