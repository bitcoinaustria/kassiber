# Storage Conventions

**Status note:** Current runtime behavior still uses the app-wide state root
described in `README.md` and `AGENTS.md`. This doc describes the **target**
storage direction after the planned project-bundle migration lands, so later
storage work has one clear end state instead of a mix of app-global and
project-local data.

**Engine:** SQLite (stdlib `sqlite3`).
**Path:** `~/.kassiber/projects/<project>/kassiber.sqlite3`, with a small
global app config under `~/.kassiber/`.
**Mode:** WAL for concurrent CLI + UI access.
**ORM:** None. Plain SQL + dataclass returns through a small repository layer.

This doc codifies how we use SQLite so CLI and UI can share one database without stepping on each other, and so future sessions (or another contributor) don't reinvent the discipline.

## Why SQLite (brief)

Decided in a separate discussion. Summary:

- Embedded, in Python stdlib, zero shipped dependency
- ACID + WAL for concurrent reads during writes — exactly what CLI + UI coexistence needs
- All query shapes in kassiber are relational (joins, date ranges, account rollups) — SQL is the right language
- Scale fits comfortably (realistic max ~100k transactions over a decade; SQLite handles millions)
- Backup = file copy (matches the simplified local export behavior)
- One of the most security-audited pieces of software on the planet
- INTEGER is int64 → msat amounts fit with no float precision hazard

## Project bundle boundary

- **One DB per project bundle.** A project is the unit of portability,
  backup/restore, deletion, and archival.
- **Not one DB per wallet.** Kassiber's tax and accounting logic spans
  wallets, so the bundle needs to hold the whole reporting unit.
- **Not one giant DB for the whole machine.** Separate projects should not
  silently share accounting state, backend config, or attachments.
- **Project-local first.** If workspaces and profiles remain in the domain
  model, they live inside one project bundle and never span bundle
  boundaries.
- **Minimal global app state.** `~/.kassiber/` outside `projects/` should
  only hold launcher/UI preferences, recent-project pointers, and keychain
  references — not active accounting state.

## Connection opening — mandatory pragmas

Every connection opened by the canonical DB bootstrap (`db.py::open_db()` during Phase 0, later possibly re-exported as `core.db.open_db()`) runs:

```sql
PRAGMA journal_mode = WAL;           -- concurrent reads during writes
PRAGMA synchronous = NORMAL;         -- fsync on commit boundary only; WAL-safe
PRAGMA foreign_keys = ON;            -- SQLite disables FKs by default; this enables
PRAGMA busy_timeout = 5000;          -- 5s wait on a locked write before ETIMEDOUT
PRAGMA temp_store = MEMORY;          -- temp tables/sorts in RAM, not /tmp
```

Notes:

- `foreign_keys = ON` is **per-connection** in SQLite, not a DB-level flag. This is the #1 footgun with SQLite. Every connection must set it.
- `journal_mode = WAL` is persistent once set, but setting it on every connection is cheap and self-healing if someone opens the file with a tool that reverts it.
- `synchronous = NORMAL` under WAL is safe and fast. `FULL` is overkill; `OFF` risks corruption on power loss.
- `busy_timeout = 5000` means a writer waits up to 5 seconds for another writer to finish. For CLI sync (one transaction per batch) + UI (mostly reads + occasional small writes) this is plenty.

## Concurrency model

- **Multiple readers, single writer** at a time (SQLite's WAL invariant).
- CLI and UI can both run simultaneously. UI reads are cheap and don't block a concurrent CLI sync. If both try to write at the same moment, one waits up to 5s.
- In practice, the UI writes tiny (add a wallet, attach a receipt, set a tag) and CLI sync writes in short batched transactions. Contention is imperceptible.
- For long-running work (a full esplora sync of a large wallet), `core.sync` breaks the work into transactions of a few hundred inserts each and yields between batches. This keeps the writer-lock window short.
- SQLite's WAL covers DB-level consistency. Bundle-level operations
  (backup, restore, reset, `attachments gc`) also touch the blob store
  and therefore need a project-wide advisory lock above SQLite's own
  locking — see `Interprocess coordination` below.

## Interprocess coordination

SQLite's WAL is not enough by itself: backup, restore, reset, and blob
GC touch the DB and the blob store as one logical unit, so they need a
lock that spans both. This section defines that lock so every CLI command
and UI worker can cooperate across processes.

### Lockfile

- Location: `~/.kassiber/projects/<project>.lock` — deliberately **outside**
  the bundle directory so it survives a restore swap.
- Mechanism: an OS advisory lock (`fcntl.flock` on POSIX, `LockFileEx` on
  Windows). A portable shim (`portalocker` or `filelock`) is acceptable if
  stdlib ergonomics hurt; either way the lock is advisory, so every
  bundle-touching caller must participate.
- The lockfile is created on demand by the first accessor. Its presence
  on disk carries no meaning on its own — only the held flock state does.

### Lock protocol

| Actor | Mode | Held for |
|---|---|---|
| Any CLI command or UI worker performing a read/write operation on the DB or blob store | Shared | **One logical unit of work** — a single CLI command's full execution, a single view-model refresh, one editor save, one report build, one batched sync transaction — **not** one SQL statement, and **not** the full connection lifetime |
| `backup_worker` producing an archive | Exclusive | DB snapshot + blob/export copy + manifest write |
| Restore / Reset / project delete | Exclusive | Staging + swap + generation bump |
| `kassiber attachments gc` | Exclusive | Full scan + delete pass |

Rules:

- The shared lock is **per-logical-operation, not per-connection and
  not per-SQL-statement**. "Operation" here is one logical unit of
  user-visible work: a CLI command's full execution, a view-model's
  complete refresh, one editor save, one report build, one batched
  sync transaction. Shared is held across every DB query and blob
  read that makes up that unit, so a Reset or Restore cannot slot
  in between two repo calls inside the same refresh and produce a
  mixed-generation snapshot. A long-lived connection (the desktop
  window's bootstrap connection; a CLI session) stays open between
  operations but does not pin shared for its entire lifetime: each
  new operation re-acquires shared, does its work, and releases.
  This is what lets backup, restore, and reset slot *between* logical
  operations instead of requiring the UI to shut down first — while
  still preventing them from interleaving *within* one.
- Exclusive-mode holders try `LOCK_EX | LOCK_NB` with a short
  spin-and-retry (default 5 s, configurable). If the exclusive lock
  cannot be acquired in time, the operation aborts with a clear
  "another process is using the project" error rather than waiting
  indefinitely. A genuinely long operation (a multi-minute sync) can
  force a restore or backup to time out; that is intentional, and
  preferable to forcibly killing the sync.
- Readers and writers coexist under shared — SQLite's own WAL locking
  continues to serialize writes inside the DB. The bundle-level lock
  exists only to coordinate against exclusive operations.
- The OS releases advisory flocks on process exit, including unclean
  exit. There is no lockfile cleanup step on startup.

### Connection lifetime

Connections are session-scoped; the bundle-level lock is
per-logical-operation. These two lifetimes must not be confused.

- Opening a connection does **not** take the bundle-level shared
  lock. The DB file handle is kept open for as long as the session
  runs (the whole UI lifetime for the desktop dashboard; a single
  CLI invocation for one-shot commands).
- Every logical operation on that connection takes shared at the
  start of the unit of work (command dispatch, view-model refresh
  start, editor save begin, report build begin), checks the
  generation (see below), performs the unit's full body — which
  may be many DB queries and blob reads — then releases shared.
  Shared is **not** released between intermediate queries that
  belong to the same logical unit.
- POSIX `rename()` does not invalidate open file descriptors:
  without a generation check, a UI connection opened before a
  restore swap would silently read and write the displaced old
  bundle through its stale fd. The generation check below is what
  makes a session-scoped connection safe across restore.

### Generation token

- `<project>/.generation` is a monotonically-increasing integer bumped
  by restore, reset, and any future destructive bundle op before the
  exclusive lock is released.
- Accessor protocol on every operation:
  1. Acquire shared lock.
  2. Read `<project>/.generation` from the canonical path.
  3. If it differs from the value cached when the current connection
     was opened, run the **in-memory invalidation** contract below
     (do **not** silently swap in a fresh connection beneath stale UI
     state), then close the stale connection, reopen against the
     canonical path, and re-cache the generation.
  4. Perform the operation.
  5. Release shared.
- One-shot CLI commands skip the recheck beyond their initial open;
  their "generation at open" is implicitly also the generation at
  commit, because the shared lock held across their single operation
  excludes a concurrent restore.
- **Batched long-running CLI work treats every batch boundary as an
  operation.** `core.sync.sync_all`, `core.import.*`, and any other
  CLI flow that intentionally releases the shared lock between
  transactions to let other writers (or a restore) slot in MUST
  re-run the generation check every time it re-acquires shared.
  The contract:
  1. Release the shared lock at a batch boundary (already the rule).
  2. Re-acquire the shared lock for the next batch.
  3. Read `<project>/.generation` from the canonical path.
  4. If the generation changed, abort the whole command with a
     clear "the project was restored while this command was
     running; rerun to continue" error. Do **not** try to resume
     across the boundary: batch-level progress state is tied to the
     pre-restore bundle's IDs/rows and cannot be safely applied to
     the new bundle.
  5. If the generation is unchanged, proceed with the next batch.
  This is a requirement on `core.sync`, `core.import.*`, and every
  other multi-transaction CLI flow that exists today — not a
  concession to hypothetical future REPLs. Without it, a batched
  sync can cross a restore boundary and write against displaced or
  mixed-generation state.

### In-memory invalidation on generation change

A reopened connection is necessary but **not sufficient**. View-model
caches, open detail dialogs, and half-filled editors populated before
the swap still hold IDs and field values from the old bundle. Letting
them persist risks the obvious corruption path: user opens a
transaction editor, restore replaces the bundle with a different
snapshot, user hits Save and the editor commits stale IDs or values
against the new DB.

On every detected generation change, callers must:

1. Discard all view-model caches and derived state sourced from the
   previous bundle. Nothing that was materialized before the bump may
   survive into post-bump operations.
2. Close or force-reload any open detail dialog or editor. An editor
   with unsaved changes surfaces a data-loss warning naming the
   restore event; it does not silently commit.
3. Refuse any write whose originating view-model's cached generation
   does not match the current value. Fail the save with a clear
   "project was restored from backup; review and retry from the
   refreshed dashboard" error rather than writing against ghost state.
4. Surface a persistent banner that tells the user the project was
   replaced and the dashboard has been refreshed.

CLI sessions at MVP scope are single-command and do not need invalidation
beyond the connection reopen. Future long-running CLI or REPL tooling
must implement the same contract.

## Schema migrations

**Tool:** plain numbered SQL files, runner in `core/migrations/runner.py`. No Alembic, no yoyo. Vibecoding with Claude should prefer tools that are one-page-of-code simple.

```
kassiber/core/migrations/
  runner.py
  001_initial.sql
  002_add_transaction_attachments.sql
  003_add_wallet_altbestand.sql
  ...
```

### Runner contract

```python
def apply_pending_migrations(conn: sqlite3.Connection) -> list[int]:
    """Applies any migrations whose version > schema_version table max.
    Returns applied versions in order."""
```

- Connection is already opened with standard pragmas.
- Each migration runs in its own transaction; partial application is impossible.
- After success, the runner inserts into `schema_version (version, applied_at)`.
- `open_db()` remains the canonical entrypoint and is responsible for leaving the DB usable for both reads and writes on every invocation.
- During the transition away from embedded schema DDL, `open_db()` may still call today's compatibility helpers (`SCHEMA`, `ensure_schema_compat`, msat migration) before or after the SQL-file runner. The rule is behavioral compatibility, not a flag day.
- The migration runner is invoked from the canonical bootstrap path, not only from write commands, so older databases never fail on read-only commands.
- First-run bootstrap: if `schema_version` table doesn't exist, create it, then treat every file as pending.

### Migration file rules

- **Filename is the version.** `001_`, `002_`, ... zero-padded for sort stability.
- **One change per file.** Easier to bisect when something goes wrong.
- **Idempotent-if-possible**, but not required. The runner prevents double-apply; we don't also need `CREATE TABLE IF NOT EXISTS` everywhere.
- **Never edit an applied migration.** Write a new one that fixes it.
- **No data migrations that can't be re-run.** If data fix is inherently one-shot, put it in a commented section with explicit guidance.

### Example

`002_add_transaction_attachments.sql`:

```sql
CREATE TABLE transaction_attachments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    tx_id       TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('file', 'url')),
    sha256      TEXT,
    filename    TEXT,
    mime        TEXT,
    size_bytes  INTEGER,
    url         TEXT,
    note        TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_attachments_tx ON transaction_attachments(tx_id);
```

## Repository pattern

**What it is:** a thin module per domain with functions that translate SQL rows into typed Python values. Not an ORM; just boundaries.

**What it is not:** active-record objects with `save()` methods, lazy relationships, or query builders.

### Example: `core/repo/wallets.py`

```python
from dataclasses import dataclass
from sqlite3 import Connection

@dataclass(frozen=True)
class Wallet:
    id: str
    workspace_id: str
    profile_id: str
    account_id: str | None
    label: str
    kind: str
    config_json: str
    created_at: str

def list_wallets(conn: Connection, *, profile_id: str, account_id: str | None = None) -> list[Wallet]:
    sql = """SELECT id, workspace_id, profile_id, account_id, label, kind, config_json, created_at
             FROM wallets
             WHERE profile_id = ?
               AND (? IS NULL OR account_id = ?)
             ORDER BY label"""
    rows = conn.execute(sql, (profile_id, account_id, account_id)).fetchall()
    return [Wallet(
        id=r[0], workspace_id=r[1], profile_id=r[2], account_id=r[3],
        label=r[4], kind=r[5], config_json=r[6], created_at=r[7],
    ) for r in rows]

def get_wallet(conn: Connection, wallet_id: str) -> Wallet | None: ...
def insert_wallet(conn: Connection, *, workspace_id: str, profile_id: str, label: str, kind: str, config_json: str = "{}") -> Wallet: ...
def update_wallet(conn: Connection, wallet_id: str, **fields) -> Wallet: ...
def delete_wallet(conn: Connection, wallet_id: str) -> None: ...
```

Domain helpers can project additional convenience fields from `config_json` (for example wallet-level tax provenance) without pretending the raw schema is different from what Kassiber stores today.

### Principles

1. **Plain SQL, never `sqlite3.Row` leaks.** Translate in the repo.
2. **Frozen dataclasses** for query results. Cheap, immutable, easy to serialize.
3. **Functions, not classes.** No `WalletRepository` object. Just `repo.wallets.list_wallets(conn, ...)`.
4. **One repo module per domain table (or tight cluster).** `repo.wallets`, `repo.accounts`, `repo.transactions`, `repo.attachments`, etc.
5. **Write paths return the created/updated object** (or None for deletes). Saves callers a second fetch.
6. **Keep complex joins in domain modules**, not in repos. The repo is CRUD-shaped; domain modules orchestrate.

### Trade-off accepted

Writing typed wrappers is more code than `conn.execute(sql).fetchall()`. That's deliberate:

- The UI calls `repo.wallets.list_wallets(...)` and gets a typed `list[Wallet]` it can bind to a QML ListView
- Claude writes against clean interfaces rather than hunting through SQL strings in app.py
- Tests stub the repo if needed (rare — SQLite in-memory is usually fine)
- Swapping storage later (unlikely) touches repo modules only

## Backup and restore

This section is the canonical bundle manifest. Other docs (notably
`05-attachments.md` and `04-desktop-ui.md`) must reference this section
rather than restate a narrower archive layout — partial restatements are
how manifests drift.

Implementation status: the project-bundle migration and the OS keychain
integration described below are both unimplemented. See `TODO.md` and
`SECURITY.md` for current runtime behavior. This section describes the
end-state contract so later work has one target to hit.

### Bundle contents (inside the archive)

- `kassiber.sqlite3` — copied via `sqlite3 .backup` or
  `Connection.backup()` so the WAL and SHM files are consistent. A raw
  file copy of a WAL database can miss checkpoints and is not allowed.
- `blobs/attachments/` — content-addressed attachment store (see
  `05-attachments.md` for the internal layout). Verified in the
  manifest by sha256.
- `blobs/imports/` — managed copies of import sources, when present.
  Content-addressed; verified in the manifest by sha256.
- `exports/` — project-local reports and PDFs that should travel
  with the bookkeeping state. Verified in the manifest by sha256 +
  size per file. Exports are accountant-facing artifacts (the
  actual PDFs the user may have filed); a corrupted export that
  imports silently is a real deliverable-integrity failure, so
  presence-only verification is not enough. Exports are not
  regenerated from the DB on restore — the archive's copy is
  authoritative, so the archive must prove the bytes are intact.
- `_bundle_manifest.json` — archive-only metadata listing: archive
  version, created_at, source hostname, DB `schema_version`, the
  expected-blob sha256 sets for attachments and imports, the expected
  file list under `exports/`, and the list of keychain key ids the
  snapshot references. The manifest is an archive-only artifact; the
  live project directory does not carry a copy.

### Outside the archive (by design)

- **OS keychain entries** referenced by wallet/backend rows in the DB.
  The bundle carries the *references*, not the secret material. Until OS
  keychain integration lands (see `SECURITY.md` and `TODO.md`), sensitive
  fields still live in the DB / `backends.env` and travel with the bundle
  as plaintext — i.e. today's bundle is effectively a secret-bearing
  archive and should be treated as such.
- **Global app state** under `~/.kassiber/` — launcher preferences,
  recent-project pointers, and global keychain references. That state
  belongs to the install, not to any one project.
- **Project logs** (`<project>/logs/`) — live in the project directory
  during day-to-day use, but do **not** travel inside the backup archive.
  Logs are diagnostic artifacts, not accounting state; bundling them
  into every backup inflates archive size without helping recovery. The
  Settings → Download logs flow in `04-desktop-ui.md` is the supported
  path for sharing logs; anyone who needs forensic log continuity
  across a restore should copy `logs/` manually before or after.
- **Transient project state** — `<project>/tmp/` (scratch space) and the
  `<project>/.generation` token. Restore writes a fresh `.generation`
  on install; `tmp/` is never archived.
- **The lockfile** at `~/.kassiber/projects/<project>.lock` — lives
  outside the bundle on purpose and is not in the archive.

### Archive authentication

The pre-swap verification above catches **corruption**: archive damage
from bit-rot, incomplete writes, partial downloads, or bugs in the
producing tool. It does **not** catch a deliberately forged archive.

The manifest (`_bundle_manifest.json`) lives inside the archive. An
attacker who can rewrite the ledger DB or a blob can also rewrite the
manifest to match, and every documented verification step still
passes on that forgery. Hash-matching is integrity within a single
archive, not trust that the archive came from the user.

This is a real caveat for MVP. A user restoring from cloud storage,
email, a friend's USB stick, or any other channel that could be
tampered with in transit has no way to reject a forged bundle through
the restore path alone.

Out of MVP scope (future work):

- **Detached HMAC** over the manifest using a passphrase or
  user-derived key. Requires a passphrase-prompt on backup and
  restore, plus key-management UI. Simplest path if passphrase UX is
  already being introduced for other reasons.
- **Detached signature file** delivered through a separate trusted
  channel. Orthogonal to the archive format.
- **Hardware-key attestation** tying a bundle to the producing
  machine's keychain identity. Would compose with the keychain
  integration roadmap.

Until one of these lands:

- Restore refuses corrupted archives, not tampered ones.
- Users who move bundles across trust boundaries should continue to
  use out-of-band verification (known-good sha256 of the archive
  published separately, signed email, etc.).
- `SECURITY.md` mirrors this caveat so the restore contract is not
  read as a trust claim it does not make.

### MVP does not ship in-place restore

A "local filesystem only" picker is not a trust boundary: downloaded
files, emailed archives, USB copies, and mounted cloud shares all
arrive as local paths. Without authentication, any local-path
archive can silently replace the ledger, attachments, and reports
with attacker-chosen bytes the first time the user clicks Restore.
That is not a feature Kassiber should ship.

MVP therefore does **not** expose an in-place Restore that overwrites
the canonical project bundle. The in-place staged-swap / journal /
recovery design documented in `Restore flow` below is preserved in
this doc as the target design for after the archive format is
authenticated. Until that lands, the Restore flow is a specification
pinned for later — not a shippable feature.

What MVP ships instead:

- `Backup flow` above — produces archives so the user has a
  disaster-recovery copy of their project on disk.
- Reset and Purge — they destroy the user's own project state,
  which does not involve trusting external input.
- **No archive-consumption path.** MVP intentionally ships without
  install-bundle or Restore. A produced archive is for future use;
  its consumers (install-bundle, in-place Restore) wait for an
  authenticated bundle format.

The authenticated bundle format is out of scope here; any of
detached HMAC with a user-held key, detached signature delivered
through a separate trusted channel, or hardware-key attestation
tying a bundle to the producing machine's keychain identity would
qualify. None of those is in MVP.

Why defer install-bundle too: re-locking secret-bearing rows
protects credentials, but it does not protect the accounting state
itself. A forged archive can carry attacker-chosen transactions,
attachments, and exports; the only MVP gate before adoption would
be manual review, and subtle ledger tampering is exactly what a
user is least likely to catch before filing or sharing
accountant-facing output. The plan doc therefore keeps
install-bundle documented below as the target import design — and
ships it only once the origin of a bundle can be cryptographically
verified.

### Install bundle as new project (deferred — not MVP)

**This flow is deferred until an authenticated bundle format
lands**, alongside in-place Restore. See `MVP does not ship
in-place restore` (and the "Why defer install-bundle too"
paragraph) for the rationale. The design below is pinned so the
eventual implementation has a crash-safe, trust-conscious
specification to work from.

When authentication lands and this flow ships, it will be the
import path that cannot overwrite an existing project; its
keychain-rebinding story (see `Imported-project trust`) ensures an
imported archive cannot hijack the user's existing local secrets
either.

The flow stages into a hidden directory, journals its progress,
and only renames to the final project name on success. That makes
install-bundle crash-recoverable in the same style as Reset and
the deferred Restore: a kill or power loss during copy or verify
never exposes a half-populated project under a canonical name.

Reminder: none of what follows runs in MVP. Treat the numbered
steps as the contract the eventual implementation must satisfy.

1. Acquire a **global** import lock at
   `~/.kassiber/.import.lock` (advisory, exclusive) to serialize
   concurrent imports against each other. This is not the
   per-project exclusive lock; install-bundle never touches the
   canonical path of an existing project.
2. Read and parse `_bundle_manifest.json` (archive listing — no
   unpack yet). Validate the same required fields as the Restore
   flow step 2.
3. Refuse if `schema_version` is newer than the running code
   supports.
4. Validate archive members against the manifest-derived allowlist
   exactly as in Restore step 4 (reject links, non-regular files,
   absolute paths, `.`/`..` components, paths outside the
   allowlist). This step is as load-bearing here as in Restore: an
   untrusted archive can still try to escape the target directory.
5. Decide on the target project name. Default to the manifest's
   `project` field, sanitized (single path segment, no separators,
   no reserved names). If that name is already an existing project
   directory under `~/.kassiber/projects/`, append `-imported-N`
   until the name is free. The UI may let the user override the
   suggested name; the CLI `project install` subcommand takes an
   explicit `--as <name>` argument.
6. **Persist an install journal** outside the staging directory at
   `~/.kassiber/projects/.install-<uuid>.journal.json`, fsync the
   file and fsync `~/.kassiber/projects/`:

   ```json
   {
     "source_archive_path":   "<absolute path>",
     "source_archive_sha256": "<64-hex>",
     "staging_path":          "<target-name>.installing.<uuid>",
     "target_name":           "<target-name>",
     "state":                 "preparing"
   }
   ```

   This is the only durable marker that an install is in flight.
7. Create the staging directory
   `~/.kassiber/projects/<target-name>.installing.<uuid>/` with
   mode 0700; fsync `~/.kassiber/projects/`.
8. Stream-copy approved archive members into the staging
   directory. `fsync` every file, `fsync` every subdirectory of
   staging that gained entries, `fsync` `~/.kassiber/projects/`.
9. Verify the staging directory against the manifest: sha256-check
   every blob, sha256+size-check every `directory_members.exports`
   entry (presence-only would silently accept a corrupted
   accountant-facing report), sha256/size-check and `PRAGMA
   integrity_check` on `kassiber.sqlite3`. When authenticated
   bundles land this step also verifies the detached HMAC or
   signature against a trusted key. On any failure: `rm -r`
   staging, `fsync` `~/.kassiber/projects/`, delete the journal,
   `fsync` again, release the import lock, surface the error,
   abort.
10. Write `<staging>/.generation = 1`, `fsync` the file, `fsync`
    the staging directory.
11. Mark the staged project as an imported, not-yet-approved
    bundle by writing `<staging>/.imported` (empty sentinel file).
    This file is what `Imported-project trust` below uses to gate
    keychain-backed rows. `fsync` the file and the staging
    directory.
12. Update the journal's `state` to `"verified"`, same
    temp-file-plus-rename pattern as Restore, `fsync` the journal
    and `~/.kassiber/projects/`.
13. `rename(staging, target)`, `fsync` `~/.kassiber/projects/`.
14. Update the journal's `state` to `"completed"`, fsync, fsync.
15. Log the import (source path, source sha256, target name) into
    the target project's log directory for audit.
16. Delete the journal, `fsync` `~/.kassiber/projects/`.
17. Release the import lock.
18. The UI surfaces the new project in the project picker with an
    "imported bundle, not yet approved" badge. Adoption is a
    separate explicit user action.

**Startup recovery for interrupted installs.** Recovery runs on
every CLI and UI bootstrap. It must **acquire
`~/.kassiber/.import.lock` in exclusive mode first**, before
scanning journals or touching staging/canonical paths. Without that
lock, a second process launched while another import is mid-flight
could mistake an active import's journal for an orphan, delete its
`preparing` staging tree, or publish a `verified` staging tree
under the wrong authority. With the lock held, either the running
import holds it (and recovery blocks briefly, then finds no orphans
when it does run) or it does not hold it (and the journal really is
orphaned). Recovery releases the lock when it finishes.

With the lock held, reconcile every orphan
`~/.kassiber/projects/.install-<uuid>.journal.json` against on-disk
state:

| Journal `state` | Staging `<target>.installing.<uuid>/` | Canonical `<target>/` | Action |
|---|---|---|---|
| preparing | exists or absent | absent | `rm -r` staging if present, delete the journal, `fsync` `~/.kassiber/projects/`. Treat the import as never-happened. |
| verified | exists | absent | Complete the import: `rename(staging, target)`, `fsync` `~/.kassiber/projects/`, then take the `completed` action below. |
| completed | absent | exists | Delete the journal, `fsync` `~/.kassiber/projects/`. |
| any other combination | — | — | Refuse auto-recovery; surface journal + filesystem state and block the import from completing until the operator reconciles manually. |

Recovery also sweeps `<target>.installing.*` directories whose
journals are missing: if no orphan journal mentions them, they are
leftovers from a crashed `preparing`-state import whose journal was
lost before fsync. Delete them with a `fsync` of the parent. This
sweep runs under the same exclusive `.import.lock` — an active
import holds the lock and its `.installing` dir is never orphaned
from recovery's perspective.

### Imported-project trust

An imported bundle must not be allowed to automatically reuse
secrets just because it happens to land on the machine that owns
the credentials it names. Until the bundle format is authenticated,
Kassiber cannot distinguish a genuine backup the user made from a
forgery that drops in matching backend URLs, wallet descriptors,
or keychain key ids — so the safe default is: an imported project
cannot exercise any credentialed operation until the user has
explicitly re-entered the relevant secret material.

Contract (while `<project>/.imported` exists):

- The runtime treats **every row that carries or references a
  secret** as locked, regardless of where the secret is stored.
  That covers:
  - Wallet rows whose descriptor or xpub includes a private
    component (SLIP77 blinding keys, descriptors with private
    keys).
  - Backend rows whose auth material lives in the OS keychain
    (once keychain integration lands).
  - Backend rows whose auth material currently lives in the DB or
    `config/backends.env` as plaintext — **this is most backend
    rows in MVP today**. The `.imported` lock applies regardless
    of whether the secret is already usable on disk: that it is
    usable on disk is precisely the reason Kassiber must refuse
    to use it without the user's confirmation that this project
    is theirs.
- Locked rows cannot sync, sign, hit a backend URL using bundled
  credentials, or otherwise exercise a secret. The UI surfaces the
  locked state on every affected row and drives the user through a
  re-entry flow — typed again through the same form as initial
  setup — before that row is usable.
- The rule applies regardless of whether the bundle's manifest
  hostname matches the local host. Same-hostname does not imply
  same-origin for an unauthenticated archive.
- Removing `.imported` is the explicit adoption step. The user
  confirms that every secret-bearing row has been re-entered (or
  deliberately left locked) and the sentinel is deleted with an
  `fsync` of the project directory. From that point the project
  behaves like any other.
- CLI parity: `kassiber wallets sync`, every `kassiber backend …`
  command that would reach a network service using project
  credentials, and any other credential-exercising CLI refuses to
  run against an `.imported` project with a clear "re-enter
  secrets via the UI or `kassiber project adopt` before using"
  error.

This contract closes the trust-boundary hole where a forged archive
with matching credentials could silently drive sync traffic through
the user's real accounts. It intentionally over-gates today's
plaintext-secret reality rather than wait for keychain integration:
during the plaintext-secrets MVP window, any imported bundle
already carries the credentials in recoverable form, so the only
safe default is to refuse to use any of them without explicit user
re-entry.

### Preconditions before bundle backup / install-bundle can ship

The bundle contract in this doc assumes the single-source-of-truth
storage model (`Backends, descriptors, and secrets` below): backend
definitions and wallet descriptors live in the project DB, not in
`config/backends.env`. That migration is tracked in `TODO.md`.

Bundle backup and install-bundle MUST NOT ship until that migration
lands, because:

- The bundle contract archives the DB and blobs but does not
  archive `config/backends.env`. Shipping backup/install today
  would silently drop every backend definition on a round-trip —
  the DB half of the project restores, the dotenv half does not,
  and the user's backend list is gone.
- The alternative — including `config/backends.env` in every
  archive and hash-verifying it like a blob — leaks plaintext
  secrets into every backup in an undocumented side channel. That
  is worse than the drop, and it contradicts the "secrets migrate
  toward keychain references" direction the same doc commits to.

The precondition is therefore:

- [ ] `TODO.md`'s "Move backend definitions and default-backend
  selection into the project DB" lands *before* this doc's bundle
  flows are implemented.

Until then, this section is a design specification, not an
implementation instruction. The MVP surface in `00-overview.md`
still lists backup + install-bundle, but conditional on this
precondition being met by the time Phase 4 tries to ship them.

### Portability scope

Portability claims depend on both *how* a bundle entered the
machine and *what* the runtime can prove about its origin. The
matrix below is deliberately conservative: "transparent secret
resolution" is a real trust claim, and Kassiber only makes it when
the flow produced the bundle itself or verified it authenticated.

- **Install-bundle (deferred, authenticated-only):** when this flow
  ships, the imported project carries `.imported` and every
  secret-bearing row is locked until the user explicitly re-enters
  each binding, regardless of whether the hostname in the manifest
  matches the local host. See `Imported-project trust`.
  Install-bundle never claims same-machine portability. This flow
  is **not** an MVP feature — it waits for the same authenticated
  bundle format as in-place Restore, because `.imported` gating
  protects credentials but not the accounting state itself.
- **In-place Restore (deferred, authenticated-only):** once
  authenticated bundles ship, Restore on the same machine and user
  account resolves keychain references against the existing OS
  keychain transparently — because authentication proves the
  bundle came from this install, the local secret bindings the
  bundle names are the user's own. Cross-machine Restore still
  locks keychain-backed rows and drives the rebind flow described
  below.
- **Cross-machine or cross-account Restore (future):** the bundle
  restores accounting state, but any row that depends on a
  keychain-backed secret re-opens in a locked state. The user
  re-pairs each via the same flow used at initial setup. The UI
  enumerates the expected keys from `_bundle_manifest.json` so
  re-pairing is deterministic and no wallet is silently missed.
  Cannot be built until keychain integration lands.
- **All paths:** a backup is not an offsite key escrow. Users who
  want their secrets to travel with the bundle must export them
  through an explicit secret-export flow (not in MVP).

### Backup flow

Backup writes to a sibling temp path on the destination filesystem
and only renames the finished archive into place at the very end.
Without that, a process kill, power loss, or full disk mid-write
leaves a truncated archive at the user-visible destination — and if
the user reused an existing path, the previous known-good backup
was already overwritten in place. Both failure modes are
unacceptable for a feature whose entire job is producing trustworthy
recovery copies.

1. Acquire the project-wide exclusive lock (see Interprocess
   coordination). Fail fast on timeout.
2. Open a sibling temp archive path on the same filesystem as the
   user's chosen destination — for example
   `<destination>.partial-<uuid>`. Build the archive by writing to
   this temp path; the user-visible destination is not touched until
   step 8.
3. `sqlite3 .backup` the DB into the temp archive's
   `kassiber.sqlite3`. Record the sha256 and byte size of the
   produced file; these go into the manifest so restore can verify
   the DB before touching the live bundle.
4. From the snapshot, enumerate **unique** blob sha256s the DB
   references (attachments today; future blob-bearing domains as
   they land). Dedupe by sha256 — not by `{sha256, filename}` —
   because blob identity on disk is the sha256 alone (see
   `05-attachments.md`). Two attachments of the same bytes under
   different user-facing filenames share one entry here.
5. Copy only the blobs referenced by step 4 from live disk into the
   temp archive. For each unique sha256, read
   `<project>/blobs/<set>/<xx>/<sha256>` on live disk (where `<xx>`
   is the first two hex chars of the sha256 — on-disk names are
   extensionless, since extensions would conflict with dedup) and
   write the same relative path inside the archive. **Do not
   bulk-copy the whole `blobs/attachments/` tree** — orphan blobs
   on disk (attachments the user detached but GC has not reclaimed)
   must not ride along into future backups.
6. Copy the full contents of `exports/` into the temp archive (one
   tree, no filtering). For each file, record its relative path,
   sha256, and byte size for the manifest.
7. Write `_bundle_manifest.json` into the temp archive:

   ```json
   {
     "version": 1,
     "created_at": "<UTC ISO8601>",
     "source_hostname": "<hostname>",
     "schema_version": <integer from the snapshot>,
     "database": {
       "sha256":     "<64-hex of the staged kassiber.sqlite3>",
       "size_bytes": <integer>
     },
     "blob_sets": {
       "attachments": [
         {"sha256": "<64-hex>", "path": "blobs/attachments/<xx>/<sha256>"},
         "..."
       ],
       "imports": [
         {"sha256": "<64-hex>", "path": "blobs/imports/<xx>/<sha256>"},
         "..."
       ]
     },
     "directory_members": {
       "exports": [
         {"path": "<relative path>", "sha256": "<64-hex>", "size_bytes": <int>},
         "..."
       ]
     },
     "keychain_keys": ["<key id>", "..."]
   }
   ```

   Blob entries carry the full relative archive path next to the
   sha256 so restore can locate and hash-verify each blob without
   re-deriving any filename rules.

8. Finalize the temp archive: close the writer so all archive-format
   metadata is flushed, then `fsync` the temp archive file. Until
   this fsync returns, the archive bytes are not durable.
9. **Atomic publish.** `rename` the temp archive into the
   user-visible destination path. On POSIX, `rename` over an
   existing file replaces it atomically; either readers see the old
   archive or the new one, never a partial mix. After the rename,
   `fsync` the destination's parent directory so the new directory
   entry is durable.
10. Release the exclusive lock.

The exclusive lock held across steps 3–7 is what makes the snapshot
a consistent slice: it serializes backup against restore, reset,
GC, and every DB/blob writer. The content-first ordering required
of attach-file (see `05-attachments.md`) closes the remaining
window — every row in the snapshot has its blob on disk by the
time step 5 reads it. Steps 8–9 are what make the *publish*
crash-safe: a kill or power loss before step 9 leaves only the temp
archive on disk, never a corrupted bundle at the user's chosen
filename, and never destroys a previously-good archive that the
user reused as the destination.

### Restore flow (future — not MVP)

**This flow is deferred until an authenticated bundle format lands.**
See `MVP does not ship in-place restore` above for why.

The design is pinned here now so that when in-place Restore does
ship, its concurrency, durability, and recovery semantics are
already understood and reviewed. Every numbered step below is a
contract the eventual implementation must satisfy; none of this
runs in MVP code.

Restore is **crash-safe**, not just synchronous-error-safe. Every
on-disk step is sequenced so that a process kill, SIGKILL, or host
panic between any two steps leaves the filesystem in a state that
the startup recovery pass (see below) can either complete forward
or roll back on the next launch. Advisory locks are released on
process exit, so recovery acquires the lock fresh and uses the
on-disk journal instead of a still-held lock to know a restore was
in flight.

**Durability on POSIX** is not free: `write()` makes content visible
to reads but not necessarily durable, and `rename()` is atomic but
not durably committed across power loss until the containing
directory is fsynced. The steps below call out every `fsync` barrier
required for the recovery table to hold. Skipping a barrier turns
crash-safety back into "works unless the host actually crashes."

1. Acquire the project-wide exclusive lock. Fail fast on timeout.
   Once held, no accessor can enter a new operation because every
   operation takes shared first.
2. Read and parse `_bundle_manifest.json` from the archive (tar/zip
   listing — no unpack yet). Validate required fields and that every
   blob entry carries a `sha256` and a `path` matching the
   `blobs/<set>/<xx>/<sha256>` shape (extensionless, see
   `05-attachments.md`).
3. Refuse if the bundle's `schema_version` is newer than the running
   code supports.
4. **Validate archive members before any filesystem write.** Enumerate
   the archive's member list and refuse the whole archive if any
   member matches any of:
   - a link entry (symbolic or hard). Restore never creates links, so
     link entries are a strict signal of a crafted archive.
   - a non-regular-file entry (device, FIFO, socket, block/char).
     Only plain directories and regular files are allowed.
   - a path that is absolute (leading `/`, leading drive letter,
     leading UNC prefix).
   - a path whose components include `.` or `..`.
   - a path not present in the allowlist derived from the manifest:
     `kassiber.sqlite3`, `_bundle_manifest.json`, every `path` under
     `blob_sets.*`, every `exports/<file>` listed in
     `directory_members.exports`, and the parent directories those
     paths strictly imply (`blobs/`, `blobs/attachments/`,
     `blobs/attachments/<xx>/`, `blobs/imports/`,
     `blobs/imports/<xx>/`, `exports/`). No other directory may
     appear.

   This step runs **before** any filesystem write. A crafted archive
   whose members would escape the staging directory or smuggle a link
   that later writes redirect through is rejected now, not during or
   after extraction.
5. **Stream-copy only the approved members** into the staging
   directory `~/.kassiber/projects/<project>.restoring.<uuid>/`.
   "Stream-copy" means the implementation iterates the archive's
   member list, resolves each member against the manifest-derived
   allowlist a second time on read, and writes content through a
   path the allowlist explicitly permits — never through the archive
   library's generic extract-by-header path, which can re-interpret
   link fields or flag bits. After copying:
   - `fsync` every file written into staging.
   - `fsync` every subdirectory of staging that gained new entries.
   - `fsync` `~/.kassiber/projects/` to durably commit the staging
     directory entry itself.

   Without these fsyncs, a post-copy crash can leave the staging
   tree present in the directory listing but with unpersisted file
   contents, which would pass the verification pass below in the
   process that survived the crash but be lost after reboot.
6. **Pre-swap corruption check** against the staged (and now-durable)
   directory. This pass catches archive corruption and internal
   self-consistency failures. It does **not** detect a deliberately
   forged archive; see `Archive authentication` below. Any failure
   refuses the archive, deletes staging, releases exclusive, and
   aborts before touching the live project.
   a. For each entry in `blob_sets.attachments` and
      `blob_sets.imports`, open the unpacked file at the declared
      `path`, recompute its sha256, and confirm it matches the
      manifest's `sha256`. File presence alone does not pass — a
      corrupted or bit-rotted file at the right path still rejects.
   b. For each entry under `directory_members.exports`, open the
      unpacked file at `exports/<path>`, recompute its sha256 and
      byte size, and confirm they match the manifest. Exports now
      get the same content-integrity guarantee as blobs — a
      corrupted PDF or CSV at the right path rejects.
   c. **Database integrity.** Recompute the sha256 and byte size of
      staged `kassiber.sqlite3` and confirm they match
      `database.sha256` / `database.size_bytes` in the manifest.
      Then open the staged DB in a temporary read-only connection
      and run `PRAGMA integrity_check`. Any result other than `ok`
      refuses the archive. This closes the "archive passed blob
      verification but carries a truncated or corrupted SQLite
      file" path, which would otherwise only surface after swap as
      a broken live bundle.
7. **Bump generation into staging, not into the live bundle after
   swap.** Read the current `<project>/.generation` into
   `source_generation` (default 0 if absent). Set
   `target_generation = max(source_generation, 0) + 1`. Write
   `target_generation` into `<staging>/.generation`, `fsync` the
   file, and `fsync` the staging directory so the entry is durable.
   After the subsequent swap, the new canonical bundle already
   carries the bumped generation — there is no later "bump after
   swap" step to lose to a crash.
8. **Persist the restore journal** before touching the live bundle.
   The journal lives outside the bundle at
   `~/.kassiber/projects/<project>.restore-journal.json` (so a
   rename of the bundle directory can never move it):

   ```json
   {
     "project":            "<name>",
     "source_generation":  <int>,
     "target_generation":  <int>,
     "staging_path":       "<project>.restoring.<uuid>",
     "backup_path":        "<project>.restore-backup-<UTC-timestamp>",
     "state":              "prepared"
   }
   ```

   Write it to a temp file (e.g. `<journal>.tmp`), `fsync` the temp
   file, rename into place, then `fsync` `~/.kassiber/projects/` so
   the journal's directory entry is durable. This file is the only
   durable signal that a restore is in flight; a crash before it is
   durable means no restore ever happened, which is the correct
   interpretation.
9. Staged replacement, each rename followed by a directory fsync so
   the rename is durable, not just atomic:
   a. `rename` the live bundle to the journal's `backup_path`, then
      `fsync` `~/.kassiber/projects/`.
   b. `rename` the staging directory to the canonical project path,
      then `fsync` `~/.kassiber/projects/` again.
   c. If (b) fails: rename (a) back to the canonical path, `fsync`
      `~/.kassiber/projects/`, then delete the staging directory and
      the journal (each with a following parent-directory fsync),
      release exclusive, and abort the restore surfacing the
      original error. If the rename-back itself fails — only
      plausible on a cross-device move or a catastrophic filesystem
      error — leave both directories on disk, log both paths, leave
      the journal in `prepared` state, and surface a recovery
      message naming them. The next startup recovery pass (see
      below) will retry the rollback from the same persisted state.
10. Update the journal's `state` to `completed`. Write via the same
    temp-file-plus-rename pattern, `fsync` the journal, and `fsync`
    `~/.kassiber/projects/`. This step commits the restore: a crash
    before this fsync leaves the journal as `prepared` and recovery
    re-runs the table below.
11. Release the exclusive lock.
12. Long-lived accessors (the UI's bootstrap connection, a CLI
    session holding its socket) do **not** need to be stopped by
    restore itself. Their shared lock was released between
    operations, so none was held across the swap. Their open file
    descriptors stay attached to the displaced-backup inode; on
    their next operation the generation check reads the bumped
    `.generation` from the canonical path, triggers the `In-memory
    invalidation on generation change` contract, and reopens the
    connection against the new bundle. Keychain references are
    resolved at that reopen; any that fail surface a locked state so
    the user can re-pair.
13. Delete the journal file, then `fsync` `~/.kassiber/projects/`.
    If deletion or the directory fsync fails, the next startup
    recovery treats a `completed` journal as a no-op cleanup
    target — the leak is self-healing.
14. The `.restore-backup-<timestamp>/` directory is retained until
    the user explicitly clears it via a later command. Restore
    itself never deletes it.

Rollback invariants:

- Before step 9a succeeds, the canonical path still points at the
  live bundle. Aborting anywhere in steps 1–8 (including failure to
  write the journal) is a no-op for the user and leaves the original
  project intact.
- Between 9a and 9b the canonical path is briefly absent. The
  exclusive lock plus the persisted journal mean any concurrent
  accessor either waits on shared or finds the journal and runs
  recovery rather than seeing a missing path.
- After 9b the canonical path points at the new bundle, which
  already carries `target_generation` from step 7 — so even a crash
  before step 10 leaves long-lived accessors with the right value to
  detect on their next generation check.
- A crash between 9b and 13 is recoverable by the startup recovery
  table below; the user never has to reason about which directory
  is current.

### Startup recovery for interrupted restores / resets

Every CLI and UI bootstrap runs a recovery pass **before** opening
the project. It looks for an orphaned journal at
`~/.kassiber/projects/<project>.restore-journal.json`. Its presence
means a journaled staged-swap operation did not cleanly finish.
Recovery acquires the exclusive lock fresh (the crashed process's
advisory lock was released on exit) and reconciles disk state
against the journal.

In MVP, this table applies to **Reset journals**: Reset uses the
same staged-swap pattern as the deferred in-place Restore, with the
same journal schema and recovery semantics. When in-place Restore
lands, the same table applies to Restore journals too. The
`<project>.purge-journal.json` has its own recovery rules described
under `Purge flow`.

Every row below corresponds to a specific crash point in the restore
flow above. No row describes a state the documented flow cannot
produce. Implementations MUST cover every row with a table-driven
test that injects a crash at the named post-step point and asserts
the action produces a valid final state.

| Row | Journal `state` | Canonical `<project>/` | `backup_path/` | `staging_path/` | Crash point | Action |
|---|---|---|---|---|---|---|
| R1 | prepared | exists, `.generation` == source | absent | exists | After step 8 (journal persisted), before step 9a (backup rename). Also the state after a successful 9c rollback, if the process died before the post-rollback cleanup. | Delete `staging_path/`, `fsync` `~/.kassiber/projects/`. Delete the journal, `fsync` `~/.kassiber/projects/`. Live bundle at canonical is the source bundle; no change needed. |
| R2 | prepared | absent | exists, `.generation` == source | exists | After step 9a (backup rename), before step 9b (staging rename). Also reachable if 9b and the 9c rollback both failed and the process died. | Attempt to complete forward: `rename(staging_path, canonical)`, `fsync` `~/.kassiber/projects/`. On success, set journal to `completed` and take row R4's action. If the rename still fails, attempt rollback: `rename(backup_path, canonical)`, `fsync` `~/.kassiber/projects/`, delete `staging_path/`, delete the journal, `fsync` `~/.kassiber/projects/` after each. If the rollback rename also fails, refuse auto-recovery and surface a manual-recovery error naming all four locations. |
| R3 | prepared | exists, `.generation` == target | exists | absent | After step 9b (staging renamed into canonical), before step 10 (journal flipped to completed). | Rewrite the journal to `state: completed` via the temp-file-plus-rename pattern, `fsync` the journal, `fsync` `~/.kassiber/projects/`. Then take row R4's action. |
| R4 | completed | exists, `.generation` == target | exists | absent | After step 10 (journal completed), before step 13 (journal deleted). | Delete the journal, `fsync` `~/.kassiber/projects/`. Keep `backup_path/` for user-initiated cleanup. |
| R5 | prepared | exists, `.generation` == source | absent | absent | After a successful 9c rollback deleted `staging_path/` but the process died before the final journal delete. The live bundle is already back to source generation. | Delete the journal, `fsync` `~/.kassiber/projects/`. Treat as a cleanly-rolled-back failed restore. |
| default | any other combination of journal / canonical `.generation` / `backup_path` / `staging_path` | | | | Not produced by the documented flow. Something external (a manual move, a separate tool, filesystem corruption) is involved. | Refuse auto-recovery. Surface a human-readable error naming canonical, `staging_path`, `backup_path`, and the journal, and block the project from opening until the operator reconciles manually. |

Notes:

- Recovery never deletes a `.restore-backup-*` directory on its own.
  It is the user's last-resort rollback; only the user decides when
  to clear it.
- R2's forward-or-rollback order is deliberate: step 6 verified the
  staging bundle, so completing forward recovers to the user's
  intended post-restore state. Rolling back is the fallback only if
  forward cannot be made durable now.
- Crash between any sub-step and the immediate following `fsync` is
  recoverable under the same row, because on reboot the
  unpersisted rename simply "did not happen" — the row that names
  the earlier persisted state applies instead.

### Reset follows the same pattern

Reset is "restore to an empty bundle": it writes a reset journal,
builds a fresh empty staging bundle with `target_generation`, performs
the same staged rename + rollback, and is reconciled by the same
startup recovery table. A crash mid-reset either leaves the live
bundle intact or completes into a clean empty bundle on recovery.

### Purge flow (also crash-safe)

Purge is the irreversible counterpart to Reset: Reset first (so the
staged-swap machinery handles the live bundle atomically), then
delete the now-empty canonical project bundle **and** every
`~/.kassiber/projects/<project>.restore-backup-*/` directory for
this project so no recovery copy and no empty-shell bundle remain
on disk. Every post-confirmation crash must result in the purge
completing on the next launch — a Purge interrupted in a window
where the intent is not yet durable is a privacy failure, because
the user chose irreversibility and expected the project to be gone.

Purge must delete the canonical bundle too, not just the recovery
copies. Reset leaves a fresh empty bundle at
`~/.kassiber/projects/<project>/`; if Purge only removed the
`.restore-backup-*` copies, the project name would stay occupied
under an empty bundle, contradicting the Settings copy that
promises irreversible deletion.

The purge journal is therefore written **before** the Reset phase
runs, not after. Its on-disk presence is what binds subsequent
startup recovery to finish the deletion even if the running process
never reaches that point.

1. Acquire the project-wide exclusive lock.
2. Read `<project>/.generation` into `source_generation`; set
   `target_generation = source_generation + 1`. Enumerate every
   existing `<project>.restore-backup-*/` directory for this
   project. Choose the Reset phase's `reset_backup_path` name
   (timestamp-based, unique) now, up front — the list of
   directories Purge intends to delete is
   `existing backups ∪ { reset_backup_path } ∪ { canonical project path }`.
   The canonical path is listed last so it is deleted only after
   all recovery copies are gone.
3. **Persist the purge journal before anything else touches disk.**
   Write
   `~/.kassiber/projects/<project>.purge-journal.json` via the
   temp-file-plus-rename pattern, `fsync` the file, `fsync`
   `~/.kassiber/projects/`:

   ```json
   {
     "project":              "<name>",
     "source_generation":    <int>,
     "target_generation":    <int>,
     "reset_backup_path":    "<project>.restore-backup-<UTC-timestamp>",
     "reset_staging_path":   "<project>.purge-reset.<uuid>",
     "pending_purge_targets": [
       "<existing .restore-backup-*>",
       "...",
       "<reset_backup_path>",
       "<project>"
     ],
     "state":                "reset-pending"
   }
   ```

   `pending_purge_targets` always ends with the canonical project
   path (`"<project>"`, resolved relative to
   `~/.kassiber/projects/`). From this point on, the user's purge
   intent is durable. Any crash after this fsync is recovered by
   the Purge-journal recovery rules below, not by the Reset-journal
   rules.
4. Run the Reset phase. Reset uses its own separate Reset journal
   as documented under `Reset follows the same pattern`, but with
   `reset_backup_path` and `reset_staging_path` set to the names
   already reserved in the purge journal (so purge-journal and
   reset-journal agree on where files go).
5. When Reset commits (its journal reaches `completed` and is
   deleted), update the purge journal's `state` to `"purging"`,
   `fsync` the journal, `fsync` `~/.kassiber/projects/`.
6. For each directory in `pending_purge_targets`, in order:
   a. `rm -r <target>` (the canonical project path is deleted last
      because it appears last in the list; every `.restore-backup-*`
      copy goes first). `fsync` `~/.kassiber/projects/` after the
      removal.
   b. Rewrite the journal with that entry removed from
      `pending_purge_targets` via the temp-file-plus-rename
      pattern, `fsync` the journal, `fsync`
      `~/.kassiber/projects/`. The journal shrinks monotonically;
      on any crash the remaining entries are exactly the
      directories still on disk.
   When this loop finishes, both the canonical project path and
   every recovery copy are gone. The project name is free for reuse.
7. When `pending_purge_targets` is empty, set
   `state: "completed"`, fsync the journal, fsync
   `~/.kassiber/projects/`.
8. Delete the journal, `fsync` `~/.kassiber/projects/`.
9. Release the exclusive lock.

**Startup recovery for Purge.** On every CLI and UI bootstrap, the
recovery pass looks for an orphan
`~/.kassiber/projects/<project>.purge-journal.json`. Its presence
overrides any Reset-journal state: the user asked for purge, so
recovery will always complete the purge, not stop at "Reset
finished."

| Journal `state` | Canonical `<project>/` | `reset_backup_path` / `reset_staging_path` | Action |
|---|---|---|---|
| reset-pending | exists, `.generation` == source | neither exists yet | Reset phase never started. Acquire exclusive and run the full Reset flow now using `reset_backup_path` / `reset_staging_path` from the journal, flip the journal to `purging`, then proceed with step 6. |
| reset-pending | exists, `.generation` == source | reset_staging_path exists, reset_backup_path absent | Reset was mid-stage when the process died. Drive Reset forward from whichever step matches its own reset-journal (if present) or start Reset fresh (if not). Flip purge journal to `purging` once Reset commits. |
| reset-pending | exists, `.generation` == source | reset_backup_path exists (Reset's rollback ran) | Reset aborted before committing. Complete the Reset now from scratch (same `reset_backup_path` may or may not be reusable — if it exists, pick a new timestamp suffix and update the purge journal's `reset_backup_path` + `pending_purge_targets` accordingly). |
| reset-pending | exists, `.generation` == target | reset_backup_path exists, reset_staging_path absent | Reset committed; the state transition to `purging` was lost. Flip the purge journal to `purging`, continue with step 6. |
| purging | exists, `.generation` == target, OR absent (already deleted in step 6) | reset_backup_path may or may not still be listed | Continue step 6 over the remaining `pending_purge_targets`. For each target not present on disk, drop it from the journal. The canonical path may already be gone if the last step 6a ran; that is fine — drop it from the list like any other missing target. |
| completed | absent | any | Delete the journal, `fsync` `~/.kassiber/projects/`. |
| any other combination | — | — | Refuse auto-recovery; surface the journal contents plus observed filesystem state; block the project from opening until the operator reconciles manually. |

The purge journal is the only durable authority on purge intent.
Recovery never looks at the Reset journal to decide whether purge
should finish — it always finishes a purge that has a durable
`purge-journal.json`, regardless of what the Reset journal says.

Purge does not promise forensic-grade erasure: it uses ordinary
recursive delete plus directory fsync. On-disk blocks may persist
until the filesystem reuses them. The job is to remove the
Kassiber-visible recovery path, not to defeat disk forensics.
Secure-wipe integration (per-file shred on supported filesystems)
is a later item.

### MVP vs later

- MVP: the bundle format, the interprocess lock, the per-operation
  shared-lock protocol (including generation-check on every batched
  CLI boundary), the in-memory invalidation contract, Backup flow
  (with temp-file-plus-rename publication so an interrupted backup
  cannot leave a corrupted archive at the destination), Reset
  (journaled staged-swap), and Purge (journaled deletion of
  `.restore-backup-*` copies).
- Deferred until an authenticated bundle format lands: **both**
  archive consumers — Install bundle as new project, and the
  in-place Restore flow with its startup recovery table. Both are
  documented in full here so implementation can start from a
  reviewed specification, but neither ships in MVP. MVP is
  intentionally backup-only; without authentication, there is no
  way to consume an archive whose origin Kassiber can't verify.
- Later (post-keychain-integration): the per-wallet locked state
  and the rebind wizard described in Portability scope.

## Backends, descriptors, and secrets

- **Backend definitions belong in the project DB.** URLs, names, chain,
  network, timeout, batch size, and default backend selection are
  project-local state.
- **Wallet descriptors belong in the project DB.** The descriptor is part of
  the wallet definition, not an external sidecar file once imported.
- **Secrets do not belong in JSON settings files.** Backend tokens,
  auth headers, RPC credentials, and any sensitive descriptor material
  should migrate toward OS keychain references. Secrets therefore live
  outside the project bundle; see the Portability scope under
  `Backup and restore` for how cross-machine restores rebind them.
- **Optional env files are bootstrap-only.** Keep dotenv overrides for
  development or operator-managed installs, but do not treat them as the
  canonical long-term storage story.
- **One source of truth per concern.** No active top-level `wallets/` tree,
  no second DB beside the canonical project DB, and no hidden config split
  between JSON and dotenv without an explicit precedence story.

## Encryption

Not in scope for MVP. Options considered for later:

| Option | Effort | Trade-off |
|---|---|---|
| OS disk encryption (FileVault, LUKS, BitLocker) | zero | Free, effective against laptop theft; nothing the app does |
| SQLCipher | medium | Drop-in API for Python via `pysqlcipher3`; adds native build dep; encrypts DB only, not attachments |
| Encrypt whole project bundle at app level | high | Password prompt on launch; kassiber owns keys; complex key-rotation story |

For now: rely on OS-level disk encryption. Revisit if we ship to users other than the project owner.

## What not to do

- **No ORM.** SQLAlchemy's ergonomics don't win back the cost of magic + vocabulary. Plain SQL is readable by any Python dev in one look.
- **No `detect_types=sqlite3.PARSE_DECLTYPES`.** It silently rewrites values (e.g., TIMESTAMP strings become `datetime` objects) and surprises readers. We do explicit conversions in the repo.
- **No `row_factory` global changes outside `open_conn`.** If a command needs `sqlite3.Row` temporarily, set it on that local cursor.
- **No auto-commit in the middle of a domain operation.** Use `with conn:` blocks to scope transactions around domain functions.
- **No connection pooling.** Each CLI invocation opens one connection; the UI keeps one long-lived connection on the main thread plus per-worker connections in QThreads.
- **No `PRAGMA journal_mode = MEMORY` or `OFF`.** Corruption risk. Smoke-test speed is fine under WAL.
- **No stray DDL in random call sites.** During the migration transition, `db.py` remains the canonical place that can still contain bootstrap DDL/compatibility logic. Once the runner fully replaces it, new DDL belongs in migration SQL files, not scattered around the codebase.

## Storage layout summary

```
~/.kassiber/
  app.json                        # global UI prefs + recent projects only
  projects/
    <project>.lock                # advisory lockfile, outside the bundle
    <project>.restore-journal.json# present only while a restore is in flight
    <project>/                    # the project bundle
      .generation                 # monotonic integer bumped by restore/reset
      kassiber.sqlite3            # primary DB
      kassiber.sqlite3-wal        # WAL file (transient)
      kassiber.sqlite3-shm        # shared-memory file (transient)
      blobs/
        attachments/
          <sha256[:2]>/
            <sha256>              # extensionless; see 05-attachments.md
        imports/
          <sha256[:2]>/
            <sha256>              # optional managed copies of import sources
      exports/                    # project-local reports and PDFs
      logs/                       # project-local logs and diagnostics
      tmp/                        # safe scratch space for restore/export work
    <project>.restore-backup-<UTC-timestamp>/   # staged-replacement leftover; user decides when to clear
    <project>.restoring.<uuid>/                 # transient staging dir during an in-flight restore
```

## Observability

- Every project-scoped CLI command logs one structured event to
  `~/.kassiber/projects/<project>/logs/cli-<date>.jsonl`. The event
  schema is an **allowlist of safe fields** — never raw argv:
  ```json
  {
    "ts":         "<UTC ISO8601>",
    "command":    "<dotted command path, e.g. wallets.sync>",
    "flags":      {"<flag-name>": "<safe value>", "...": "..."},
    "exit_code":  <int>,
    "duration_ms": <int>,
    "error_kind": "<AppError subclass or null>"
  }
  ```
  Rules:
  - `flags` only includes keys the command has explicitly declared
    loggable. Each command definition lists its safe flags; secrets
    are not on that list. A command that has not declared a flag
    loggable does not write it — the default is "redact."
  - `--token`, `--auth-header`, `--password`, `--username`,
    `--cookiefile`, `--env-file`, raw descriptors, and every other
    argv path documented as secret-bearing in `SECURITY.md` are
    never loggable. They are missing from every command's loggable
    allowlist.
  - Positional and JSON arguments are recorded only if the command
    explicitly declares which sub-fields are safe. Blindly
    serializing an argv array into the log is forbidden.
  - This is a structural guarantee, not a blacklist: a new
    secret-bearing flag added later is redacted by default, because
    the allowlist does not mention it. Adding the flag to the
    allowlist requires an explicit code change and a review of
    whether the value is actually safe to log.
- Project UI logs live beside the project DB under
  `~/.kassiber/projects/<project>/logs/ui-<date>.jsonl` and follow
  the same structured-allowlist schema.
- **Never** log secret values (xpubs, macaroons, descriptors with
  private keys, backend tokens, auth headers, passwords). The
  allowlist enforces this structurally; any accidental leak via a
  new flag reaching logs before being explicitly approved is a
  security bug.
- The Settings → Download logs button zips the last 14 days of
  project logs for the user to share. Because logs are allowlist-
  structured, the share flow is safe by construction — there is no
  "redact before export" step to skip.

## References

- [SQLite WAL docs](https://www.sqlite.org/wal.html)
- [PRAGMA foreign_keys](https://www.sqlite.org/foreignkeys.html#fk_enable)
- [sqlite3 .backup API](https://docs.python.org/3/library/sqlite3.html#sqlite3.Connection.backup)
