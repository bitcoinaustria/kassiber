# Transaction Attachments

**Status:** New feature. Not yet implemented.
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
    tx_id           INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
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

Content-addressed directory under the data root:

```
~/.kassiber/data/attachments/
  ab/
    ab2341c9e7...f3.pdf
  cd/
    cd5e9a1b34...7f.jpg
  ...
```

Rules:
- Filename on disk is `<sha256>.<ext>`, where the extension is preserved from the upload when known
- The first two hex chars of the sha256 form the subdirectory, keeping any one directory from growing unbounded
- Identical files (same sha256) are stored once; the DB row captures the user-facing filename + note per attachment
- On attach: compute sha256 of the source file, copy (not move) into the store if not already present
- On detach: decrement a logical reference count. If no remaining `transaction_attachments` row points at this sha256, the file becomes eligible for GC. Do GC on `kassiber vacuum` or an explicit `kassiber attachments gc` command — not inline (simpler, safer).

Why content-addressed:
- Deduplication: same invoice attached to two related transactions costs one copy on disk
- Integrity: re-computing sha256 on read detects tampering
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
| User runs `kassiber backup create` | Archive DB + `attachments/` directory together in a tar |
| User restores a backup | Archive unpacks to a fresh `~/.kassiber/data/` atomically (via temp dir + rename) |
| Maximum attachment size | 50 MB per file for MVP; configurable later via `ui:max_attachment_mb` in settings. Hard limit 500 MB (reject with error). |
| Allowed MIME types | Not restricted. Detected via python-magic or `mimetypes` stdlib; stored but not gatekeeping. |

## CLI surface

```
kassiber tx attach <tx-id> --file <path> [--note "..."]
kassiber tx attach <tx-id> --url <url>   [--note "..."]
kassiber tx attachments <tx-id>            # list attachments
kassiber tx detach <attachment-id>
kassiber attachments gc                    # garbage-collect orphaned files
kassiber attachments verify                # verify sha256 of every stored file
```

Envelope shapes:

```json
// attach success
{"status": "ok", "data": {"attachment_id": 42, "tx_id": 17, "kind": "file", "sha256": "ab...", "filename": "rechnung.pdf", "size_bytes": 182344}}

// list
{"status": "ok", "data": {"tx_id": 17, "attachments": [ {...}, {...} ]}}

// gc
{"status": "ok", "data": {"freed_count": 3, "freed_bytes": 524288}}
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
2. Walk `attachments/` directory
3. For each file whose sha256 is not in the set, delete it
4. Report count and bytes freed

Runtime on a 10k-attachment store: <5 seconds. Acceptable as a manual action.

## Backup + restore

`kassiber backup create <path>` produces a `.kassiber.tar` containing:

```
kassiber.sqlite3           # via sqlite3 .backup to ensure WAL-safe copy
attachments/               # entire directory tree
config/                    # backends.env + settings.json
_backup_metadata.json      # version, created_at, hostname
```

Restore:
1. Validate archive structure and metadata
2. Refuse if schema_version in archive > schema_version the running code supports
3. Unpack into a temp directory
4. Atomic swap: rename current `~/.kassiber/data/` to a timestamped backup dir; rename temp to `~/.kassiber/data/`
5. Restart any UI worker threads with fresh connections
6. On any failure: leave original data intact, log error

## Implementation touchpoints

- `core/attachments.py` — new module: `attach_file(conn, tx_id, path, note="")`, `attach_url(conn, tx_id, url, note="")`, `list_attachments(conn, tx_id)`, `detach(conn, attachment_id)`, `gc(conn)`, `verify(conn)`
- `core/repo/attachments.py` — CRUD with typed `Attachment` dataclass
- `cli/commands/attach.py` — argparse wiring
- `ui/viewmodels/transaction_vm.py` — attachment list as Qt property; signals for attach/detach
- `ui/resources/qml/dialogs/TransactionDetail.qml` — drag-drop zone, URL form, chips
- Migration `core/migrations/002_add_transaction_attachments.sql`

## Non-goals

- **Versioning of attachments.** If the user attaches a replacement invoice, they detach the old and attach the new. No history.
- **Searching inside attachments.** No OCR, no full-text index.
- **Cloud backup integration.** The user can sync `~/.kassiber/` with their own tool.
- **Signing / timestamping attachments.** Future feature; keep the data model additive-compatible.
