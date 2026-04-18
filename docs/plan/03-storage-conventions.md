# Storage Conventions

**Engine:** SQLite (stdlib `sqlite3`).
**Path:** `~/.kassiber/data/kassiber.sqlite3` (overridable via `KASSIBER_DATA_DIR` and the settings manifest).
**Mode:** WAL for concurrent CLI + UI access.
**ORM:** None. Plain SQL + dataclass returns through a small repository layer.

This doc codifies how we use SQLite so CLI and UI can share one database without stepping on each other, and so future sessions (or another contributor) don't reinvent the discipline.

## Why SQLite (brief)

Decided in a separate discussion. Summary:

- Embedded, in Python stdlib, zero shipped dependency
- ACID + WAL for concurrent reads during writes — exactly what CLI + UI coexistence needs
- All query shapes in kassiber are relational (joins, date ranges, account rollups) — SQL is the right language
- Scale fits comfortably (realistic max ~100k transactions over a decade; SQLite handles millions)
- Backup = file copy (matches Clams' Backup Data button semantics)
- One of the most security-audited pieces of software on the planet
- INTEGER is int64 → msat amounts fit with no float precision hazard

## Connection opening — mandatory pragmas

Every connection opened by `core.db.open_conn()` runs:

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
- Every CLI command that writes calls `apply_pending_migrations` first. UI calls it on app start.
- First-run bootstrap: if `schema_version` table doesn't exist, create it, then treat every file as pending.

### Migration file rules

- **Filename is the version.** `001_`, `002_`, ... zero-padded for sort stability.
- **One change per file.** Easier to bisect when something goes wrong.
- **Idempotent-if-possible**, but not required. The runner prevents double-apply; we don't also need `CREATE TABLE IF NOT EXISTS` everywhere.
- **Never edit an applied migration.** Write a new one that fixes it.
- **No data migrations that can't be re-run.** If data fix is inherently one-shot, put it in a commented section with explicit guidance.

### Example

`003_add_wallet_altbestand.sql`:

```sql
-- Austrian tax regime: mark pre-2021-03-01 wallet contents as Altvermögen
-- See docs/plan/06-austrian-tax-engine.md

ALTER TABLE wallets
    ADD COLUMN altbestand INTEGER NOT NULL DEFAULT 0;
    -- 0 = false, 1 = true (SQLite has no native bool)

-- existing rows default to 0 (Neuvermögen-treated)
-- user marks Altvermögen wallets explicitly via
-- `kassiber wallet set <id> --altbestand`
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
    id: int
    account_id: int
    kind: str        # 'descriptor' | 'xpub' | 'address' | 'coreln' | ...
    name: str
    altbestand: bool
    created_at: str  # ISO8601

def list_wallets(conn: Connection, *, account_id: int | None = None) -> list[Wallet]:
    sql = """SELECT id, account_id, kind, name, altbestand, created_at
             FROM wallets
             WHERE (? IS NULL OR account_id = ?)
             ORDER BY name"""
    rows = conn.execute(sql, (account_id, account_id)).fetchall()
    return [Wallet(
        id=r[0], account_id=r[1], kind=r[2], name=r[3],
        altbestand=bool(r[4]), created_at=r[5],
    ) for r in rows]

def get_wallet(conn: Connection, wallet_id: int) -> Wallet | None: ...
def insert_wallet(conn: Connection, *, account_id: int, kind: str, name: str, altbestand: bool = False) -> Wallet: ...
def update_wallet(conn: Connection, wallet_id: int, **fields) -> Wallet: ...
def delete_wallet(conn: Connection, wallet_id: int) -> None: ...
```

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

- **Backup**: `python -c "import sqlite3; sqlite3.connect(src).backup(sqlite3.connect(dst))"` or `sqlite3 <src> ".backup <dst>"`. This is the **only** safe way to copy a WAL database — a raw file copy can miss checkpoints.
- **Attachments** (see `05-attachments.md`) are bundled alongside the `.sqlite3` in a tar archive: `kassiber backup create /path/to/archive.kassiber.tar`.
- **Restore**: stop the UI, replace the DB file and attachments directory from the archive, restart. CLI commands should refuse to run if the schema_version is newer than the code knows about.

## Encryption

Not in scope for MVP. Options considered for later:

| Option | Effort | Trade-off |
|---|---|---|
| OS disk encryption (FileVault, LUKS, BitLocker) | zero | Free, effective against laptop theft; nothing the app does |
| SQLCipher | medium | Drop-in API for Python via `pysqlcipher3`; adds native build dep; encrypts DB only, not attachments |
| Encrypt whole `~/.kassiber/data/` at app level | high | Password prompt on launch; kassiber owns keys; complex key-rotation story |

For now: rely on OS-level disk encryption. Revisit if we ship to users other than the project owner.

## What not to do

- **No ORM.** SQLAlchemy's ergonomics don't win back the cost of magic + vocabulary. Plain SQL is readable by any Python dev in one look.
- **No `detect_types=sqlite3.PARSE_DECLTYPES`.** It silently rewrites values (e.g., TIMESTAMP strings become `datetime` objects) and surprises readers. We do explicit conversions in the repo.
- **No `row_factory` global changes outside `open_conn`.** If a command needs `sqlite3.Row` temporarily, set it on that local cursor.
- **No auto-commit in the middle of a domain operation.** Use `with conn:` blocks to scope transactions around domain functions.
- **No connection pooling.** Each CLI invocation opens one connection; the UI keeps one long-lived connection on the main thread plus per-worker connections in QThreads.
- **No `PRAGMA journal_mode = MEMORY` or `OFF`.** Corruption risk. Smoke-test speed is fine under WAL.
- **No schemas in Python code.** All DDL lives in migration SQL files. The runtime never calls `CREATE TABLE`.

## Storage layout summary

```
~/.kassiber/
  config/
    settings.json          # schema_version, paths manifest (already exists)
    backends.env           # sync backend definitions (already exists)
  data/
    kassiber.sqlite3       # primary DB
    kassiber.sqlite3-wal   # WAL file (transient)
    kassiber.sqlite3-shm   # shared-memory file (transient)
    attachments/
      <sha256[:2]>/
        <sha256>.<ext>     # content-addressed, see 05-attachments.md
  exports/                  # reports, PDFs (user-facing outputs)
  logs/                     # (new) rotated logs for Download logs button
```

## Observability

- Every CLI command logs to `~/.kassiber/logs/cli-<date>.jsonl` (one line per command invocation: command name, args, exit code, duration, AppError kind if any)
- UI logs to `~/.kassiber/logs/ui-<date>.jsonl`
- **Never** log secret values (xpubs, macaroons, descriptors with private keys). The logger has a blacklist.
- The Settings → Download logs button zips the last 14 days of logs for the user to share.

## References

- [SQLite WAL docs](https://www.sqlite.org/wal.html)
- [PRAGMA foreign_keys](https://www.sqlite.org/foreignkeys.html#fk_enable)
- [sqlite3 .backup API](https://docs.python.org/3/library/sqlite3.html#sqlite3.Connection.backup)
