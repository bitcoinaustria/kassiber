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
- `blobs/attachments/` — content-addressed attachment store
  (see `05-attachments.md` for the internal layout)
- `blobs/imports/` — managed copies of import sources, when present
- `exports/` — project-local reports and PDFs that should travel with
  the bookkeeping state
- `logs/` — project-scoped CLI and UI logs
- `_bundle_manifest.json` — archive version, created_at, source hostname,
  DB `schema_version`, and the list of keychain key ids the DB references
  so a cross-machine restore can enumerate what needs re-pairing. The
  manifest is an archive-only artifact; the live project directory does
  not carry a copy.

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

### Portability scope

- **Same-machine, same user account:** the bundle alone is sufficient.
  Keychain references resolve against the existing OS keychain; backends
  and wallets light up transparently after restore. Today, with no
  keychain integration yet, this path is effectively "restore a plaintext
  bundle on the same machine."
- **Cross-machine or cross-account (end state):** the bundle restores the
  accounting state, but any row that depends on a keychain-backed secret
  re-opens in a locked state. The user must re-pair each via the same
  flow used at initial setup. The UI enumerates the expected keys from
  `_bundle_manifest.json` so re-pairing is deterministic and no wallet
  is silently missed. This flow cannot be built until keychain
  integration lands.
- A backup is **not** an offsite key escrow. Users who want their secrets
  to travel with the bundle must export them through an explicit secret
  export flow (not in MVP).

### Restore flow

1. Validate archive structure and `_bundle_manifest.json`.
2. Refuse if the bundle's `schema_version` is newer than the running
   code supports.
3. Unpack into a sibling temp directory under `~/.kassiber/projects/`.
4. Stop any UI workers or CLI processes holding an open connection to
   the active project DB.
5. Atomic swap: rename the current project directory to a timestamped
   backup dir, then rename the temp directory into place.
6. Restart workers with fresh connections. Resolve keychain references;
   for any that fail, surface a locked state to the UI so the user can
   re-pair.
7. On any failure, leave the original project directory intact and log
   the error.

### MVP vs later

- MVP: the bundle format above, same-machine restore, and the
  schema_version gate.
- Later (post-keychain-integration): the per-wallet locked state and the
  rebind wizard described in Portability scope.

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
  app.json                  # global UI prefs + recent projects only
  projects/
    project-satoshi/
      kassiber.sqlite3      # primary DB
      kassiber.sqlite3-wal  # WAL file (transient)
      kassiber.sqlite3-shm  # shared-memory file (transient)
      blobs/
        attachments/
          <sha256[:2]>/
            <sha256>.<ext>  # content-addressed, see 05-attachments.md
        imports/
          <sha256[:2]>/
            <sha256>.<ext>  # optional managed copies of import sources
      exports/              # project-local reports and PDFs
      logs/                 # project-local logs and diagnostics
      tmp/                  # safe scratch space for restore/export work
```

## Observability

- Every project-scoped CLI command logs to
  `~/.kassiber/projects/<project>/logs/cli-<date>.jsonl` (one line per
  command invocation: command name, args, exit code, duration, AppError kind
  if any)
- Project UI logs live beside the project DB under
  `~/.kassiber/projects/<project>/logs/ui-<date>.jsonl`
- **Never** log secret values (xpubs, macaroons, descriptors with private keys). The logger has a blacklist.
- The Settings → Download logs button zips the last 14 days of project logs
  for the user to share.

## References

- [SQLite WAL docs](https://www.sqlite.org/wal.html)
- [PRAGMA foreign_keys](https://www.sqlite.org/foreignkeys.html#fk_enable)
- [sqlite3 .backup API](https://docs.python.org/3/library/sqlite3.html#sqlite3.Connection.backup)
