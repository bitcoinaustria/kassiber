# Transaction Attachments

**Status:** Current code already supports copied local files and URL attachments.
For the project-storage MVP, this plan intentionally optimizes for the simpler
case we actually seem to need most often: a few external links per transaction.
Treat `README.md`, `AGENTS.md`, and `kassiber/core/attachments.py` as the
source of truth for current shipped behavior.

**Scope:** Link-first MVP sketch for project-local storage planning.
**Purpose:** Let users attach one or more Drive, Dropbox, Nextcloud, or other
document links to a transaction without forcing Kassiber to become a blob-store
product first.

## MVP direction

A transaction can have zero or more external links.

- The link itself lives in the project DB.
- Kassiber does not fetch, cache, or mirror the target file.
- Backup stays simple because the relevant metadata is already in SQLite.
- If copied local files become a real product need later, add them as a separate
  layer instead of pre-designing that machinery now.

## User stories

1. As a self-employed user, I want to attach the Google Drive link for an invoice
   to the BTC transaction that paid it.
2. As a business user, I want a few document links on a transaction: invoice,
   contract, and email thread.
3. As a tax filer, I want my accountant to see the references behind a
   transaction without me re-explaining where each document lives.

## Data model sketch

### New table: `transaction_links`

```sql
CREATE TABLE transaction_links (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    tx_id       TEXT    NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    url         TEXT    NOT NULL,
    title       TEXT    NOT NULL DEFAULT '',
    note        TEXT    NOT NULL DEFAULT '',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL
);

CREATE INDEX idx_transaction_links_tx
    ON transaction_links(tx_id, sort_order, id);
```

Why a dedicated table:

- Multiple links per transaction stay simple.
- Ordering is explicit.
- We avoid baking file/blob assumptions into the schema.
- A later copied-file feature can be additive instead of distorting the link
  model.

## Behavior rules

| Situation | Behavior |
|---|---|
| User adds a link | Insert a row in `transaction_links` |
| User adds the same link twice | Allowed; Kassiber does not silently deduplicate user intent |
| User deletes a link | Delete the row |
| Transaction is deleted | `ON DELETE CASCADE` removes its links |
| User opens a link | Pass it to the OS/browser via the normal URL handler |
| User backs up a project | A SQLite snapshot already includes all transaction links |

Rules:

- No fetching.
- No link validation beyond basic UI sanity checks.
- No background health checks.
- No Drive API integration in the storage MVP.

## CLI / UI sketch

### CLI

```text
kassiber transaction-links add    --transaction <tx-id> --url <url> [--title "..."] [--note "..."]
kassiber transaction-links list   --transaction <tx-id>
kassiber transaction-links remove --link <id>
```

### UI

Transaction detail view:

- "Add link" button
- Small form: URL, optional title, optional note
- Flat list of links under the transaction
- Click opens the URL in the system handler

## Backup and portability

- The portable unit is still the project DB.
- Because the links are stored in SQLite, they automatically ride along with a
  DB snapshot.
- The linked files themselves stay where the user already keeps them. Kassiber
  records references, not mirrored copies.

## Future copied-file option

If offline/self-contained evidence becomes a concrete requirement later, add a
separate copied-file feature under the project directory, for example:

```text
~/.kassiber/projects/<project>/blobs/attachments/
```

That should be a later, additive feature with its own real product case behind
it. The link-first MVP should not pre-commit Kassiber to GC logic, manifests,
hash trees, restore journals, or blob-verification protocols.

## Privacy and security

- Kassiber does not fetch linked URLs.
- Link metadata lives in the local project DB like other transaction metadata.
- The security of the underlying document stays with the external system that
  hosts it.
- Logging should avoid dumping full secret-bearing URLs if a service uses
  embedded access tokens in the query string.

## Non-goals

- Mirroring external documents locally
- OCR, indexing, or preview generation
- Drive API sync
- Background "broken link" monitoring
- Designing a full copied-file attachment subsystem in this PR
