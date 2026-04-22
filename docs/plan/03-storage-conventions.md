# Storage Conventions

**Status note:** Current runtime behavior still uses the app-wide state root
described in `README.md` and `AGENTS.md`. This doc describes the **target**
storage direction after the planned project migration lands, so later work has
one clear end state instead of a mix of app-global and project-local data.

**Engine:** SQLite (stdlib `sqlite3`).
**Path:** `~/.kassiber/projects/<project>/kassiber.sqlite3`, with a small
global app config under `~/.kassiber/`.
**Mode:** WAL for concurrent CLI + UI access.
**ORM:** None. Plain SQL + dataclass returns through a small repository layer.

This doc pins the small part we actually need: what a project is, where it
lives, and how much machinery we are deliberately *not* standardizing yet.

## Why SQLite (brief)

Decided in a separate discussion. Summary:

- Embedded, in Python stdlib, zero shipped dependency
- ACID + WAL for concurrent reads during writes
- All query shapes in Kassiber are relational
- Scale fits comfortably
- Backup can be a snapshot of one file
- Security posture is well understood
- INTEGER is int64, so msat amounts fit with no float precision hazard

## Project boundary

- **One DB per project.** A project is the unit of storage, backup,
  import/export, and deletion.
- **Not one DB per wallet.** Kassiber's accounting and tax logic spans
  wallets.
- **Not one giant DB for the whole machine.** Separate projects should not
  silently share accounting state.
- **Minimal global app state.** `~/.kassiber/` outside `projects/` should only
  hold launcher/UI preferences, recent-project pointers, and other install-wide
  metadata.
- **Project-local first.** If something belongs to the user's bookkeeping, it
  should live in the project or be an explicit external reference recorded by
  the project.

## Connection opening — mandatory pragmas

Every connection opened by the canonical DB bootstrap runs:

```sql
PRAGMA journal_mode = WAL;           -- concurrent reads during writes
PRAGMA synchronous = NORMAL;         -- fsync on commit boundary only; WAL-safe
PRAGMA foreign_keys = ON;            -- SQLite disables FKs by default
PRAGMA busy_timeout = 5000;          -- wait up to 5s on a locked write
PRAGMA temp_store = MEMORY;          -- temp tables/sorts in RAM
```

Notes:

- `foreign_keys = ON` is **per-connection** in SQLite, not a DB-level flag.
- `journal_mode = WAL` is persistent once set, but setting it on every
  connection is cheap and self-healing.
- `synchronous = NORMAL` under WAL is the right default here.
- `busy_timeout = 5000` is enough for short UI writes and batched sync work.

## Concurrency model

- **Multiple readers, single writer** at a time (SQLite's WAL invariant).
- CLI and UI can both run simultaneously.
- Normal reads and writes should rely on SQLite WAL, not a second,
  application-wide lock wrapped around every operation.
- For long-running sync/import work, write in short batches so the writer-lock
  window stays small.

## Project-level operations (MVP)

Keep whole-project coordination simpler than normal query lifecycle management.

- Ordinary reads and writes should not need an extra project-wide lock.
- Whole-project operations such as backup export, reset, or delete may use one
  coarse project lock or may simply require the project to be closed first.
- This doc does **not** pin hot in-place restore, generation tokens, session
  invalidation, or crash-recovery journals.
- "Import as a new project" is a simpler future direction than "replace the
  currently open project while the app stays alive."

## Schema migrations

**Tool:** plain numbered SQL files, runner in `core/migrations/runner.py`.
No Alembic, no yoyo.

```text
kassiber/core/migrations/
  runner.py
  001_initial.sql
  002_add_transaction_links.sql
  003_...
```

### Runner contract

```python
def apply_pending_migrations(conn: sqlite3.Connection) -> list[int]:
    """Apply migrations whose version is greater than the current max."""
```

- Connection is already opened with the standard pragmas.
- Each migration runs in its own transaction.
- After success, the runner inserts into `schema_version(version, applied_at)`.
- `open_db()` remains the canonical entrypoint.
- The migration runner is invoked from the canonical bootstrap path, not only
  from write commands.
- First-run bootstrap: if `schema_version` doesn't exist, create it and treat
  every file as pending.

### Migration file rules

- **Filename is the version.** `001_`, `002_`, ...
- **One change per file.**
- **Never edit an applied migration.** Write a new one that fixes it.
- **Keep data migrations boring.** Prefer straightforward SQL and small Python
  helpers over framework magic.

## Repository pattern

Repository functions should be small and typed, not generic data mappers.

```python
@dataclass
class WalletSummary:
    id: str
    name: str
    kind: str
    tx_count: int
    balance_sat: int


def list_wallet_summaries(conn: sqlite3.Connection, *, project_id: str) -> list[WalletSummary]:
    ...
```

Rules:

- SQL lives close to the function that uses it.
- Return dataclasses or small typed dicts, not raw tuples.
- Avoid "generic repository base classes."
- Prefer explicit joins and projections over hidden ORM behavior.

## Backup and project portability

This PR only needs to pin the storage unit, not a full archive protocol.

### MVP sketch

- One project = one SQLite DB at
  `~/.kassiber/projects/<project>/kassiber.sqlite3`
- Minimal global app state at `~/.kassiber/app.json`
- Per-transaction links live in the DB and therefore already travel with the
  project snapshot
- Project-local copied files are optional later work, not something this PR
  needs to standardize in detail

### Backup

- Back up a project by taking a SQLite snapshot via `Connection.backup()` or
  `sqlite3 .backup`.
- Because link/reference metadata lives in the DB, a DB snapshot already
  preserves the main user-facing attachment data for the simpler MVP.
- If later phases add project-local copied files, those can live beside the DB
  under `blobs/` and be included then.

### Restore / import

- Do **not** design hot in-place restore in this PR.
- A future restore/import flow may simply require the project to be closed
  first.
- "Import as new project" is a simpler and safer first step than "replace the
  currently-open project while the UI stays alive."
- No generation tokens, staged swap journals, manifest/authentication protocol,
  or crash-recovery matrix are required to choose the basic cross-platform
  layout.

## Backends, descriptors, and secrets

Keep the storage-layout decision separate from the final secret-sealing
mechanism.

- The project should become the unit of storage instead of splitting active
  state across the DB plus unrelated global side files.
- Moving backend definitions closer to the project is still the right
  direction.
- But this PR should **not** try to promise both effortless cross-platform
  portability and machine-bound OS-keychain rebinding in one shot.
- First make the project boundary clear; then pick the secret-storage strategy
  deliberately in a follow-up.

## Encryption

Today's runtime has **no** encryption at rest. The DB and related files are
plain files on disk.

If cross-platform portability is the primary product requirement, a portable
encrypted project/backup format is a better fit than machine-specific keychain
references. If passphrase-free local UX is the priority, OS-keychain
integration is a better fit. This PR does not need to settle that tradeoff.

## What not to do

- Do not split one logical project across multiple writable roots unless there
  is a very strong reason.
- Do not put active accounting state in global app config.
- Do not over-design live restore before the product even needs it.
- Do not make the attachment story more complex than the real use case.

## Storage layout summary

```text
~/.kassiber/
  app.json                        # global UI prefs + recent projects only
  projects/
    <project>/
      kassiber.sqlite3            # primary DB
      kassiber.sqlite3-wal        # WAL file (transient)
      kassiber.sqlite3-shm        # shared-memory file (transient)
      exports/                    # optional project-local reports
      logs/                       # optional project-local logs
      blobs/                      # only if later phases need copied local files
```

## Observability

- Project logs should live beside the project DB under
  `~/.kassiber/projects/<project>/logs/`.
- Keep the logging contract simple: structured events, no raw argv, and redact
  secret-bearing fields by default.
- The Settings → Download logs flow can zip recent project logs without
  becoming part of the storage-layout debate.

## References

- `README.md`
- `AGENTS.md`
- `docs/plan/00-overview.md`
- `docs/plan/04-desktop-ui.md`
- `docs/plan/05-attachments.md`
- `SECURITY.md`
- `TODO.md`
