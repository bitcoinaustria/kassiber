# Storage Conventions

**Status:** Target-state design. Current runtime still uses the app-wide
`~/.kassiber/{data,config,exports,attachments}` layout described in README and
AGENTS.md.
**Current source of truth:** `kassiber/db.py`, `kassiber/core/runtime.py`,
README, and TODO.md.

## Product Direction

Move toward one project bundle per bookkeeping scope:

```text
~/.kassiber/
  app.json
  projects/
    <project>/
      kassiber.sqlite3
      exports/
      logs/
      blobs/          # only if copied project-local files remain needed
```

A project is the unit of storage, backup, import/export, and deletion.

Do not split one project across unrelated writable roots unless a later design
explicitly requires it.

## Current Layout

Current default state is:

```text
~/.kassiber/
  data/kassiber.sqlite3
  config/backends.env
  config/settings.json
  exports/
  attachments/
```

Backend definitions are canonical in SQLite; dotenv remains a bootstrap and
compatibility path.

## SQLite Rules

- SQLite remains the system of record.
- Use stdlib `sqlite3`; no ORM.
- BTC amounts are integer msat.
- Fiat columns are still `REAL` unless a future report-specific boundary
  deliberately uses integer cents.
- Prefer additive schema changes compatible with `CREATE TABLE IF NOT EXISTS`
  and lightweight compatibility migrations.

Future project-mode connections should run:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
```

Current `open_db()` already guarantees schema bootstrap and `foreign_keys = ON`;
WAL/busy-timeout rollout belongs with the project-bundle migration.

## Bootstrap Ownership

`open_db()` remains the canonical DB entrypoint. When numbered SQL migrations
land, their runner must be invoked from inside the canonical DB bootstrap path,
not from individual commands.

Target migration shape:

```text
kassiber/core/migrations/
  runner.py
  001_initial.sql
  002_...
```

Rules:

- one numbered file per migration
- never edit an applied migration
- each migration runs in its own transaction
- compatibility logic for older DBs stays reachable through `open_db()`

## Project Migration Gap

The app-wide layout to project-bundle layout is a real migration. Before it
lands, write a focused implementation plan covering:

- discovery of existing `~/.kassiber/data/kassiber.sqlite3`
- project naming / import-as-project behavior
- movement or rebinding of exports, attachments, backend records, and settings
- how `--data-root` continues to work
- rollback/error behavior
- CLI and desktop prompts

Until that plan lands, do not partially move active accounting state into
`projects/`.

## Backup / Restore

`kassiber backup export` now uses `Connection.backup()` to take a hot
SQLCipher copy of the live DB while writers continue, tars the staging
tree (DB + attachments + `backends.env`), and pipes the tarball through
`age` (binary or `pyrage`) into a single `.kassiber` envelope. Recovery
without Kassiber is intentionally possible with stock `age` + `tar` +
`sqlcipher`.

Restore is staged through `kassiber backup import`: decrypt to a temp
tarball, run the strict tar-member validator, extract under a staging
directory, validate the manifest, and only on `--install` move the
staged tree into the live data root after snapshotting any pre-existing
files into `pre-restore-<ts>/`. There is no hot in-place restore.

## Secrets

The local SQLite file at `~/.kassiber/data/kassiber.sqlite3` is now
optionally encrypted at rest under a user passphrase via SQLCipher 4
(`kassiber secrets init`). Stock SQLCipher PRAGMA defaults
(`kdf_iter = 256000`, `cipher_compatibility = 4`,
`cipher_page_size = 4096`) are deliberate so a stranded user can recover
with the upstream `sqlcipher` binary alone. The passphrase is the
perimeter; there is no recovery path if it is lost. See
[../../SECURITY.md](../../SECURITY.md) for the full at-rest boundary
including what stays plaintext (attachments, exports, the dotenv
addressing rows, the `*.pre-encryption.sqlite3.bak` rollback file).

Backend secrets (`token`, `password`, `auth_header`, `username`, plus
the RPC aliases `rpcuser` / `rpcpassword`) live in the encrypted
`backends` table, not in the dotenv. `kassiber secrets
migrate-credentials` lifts pre-existing entries out of `backends.env`
into the encrypted DB and rewrites the file with non-secret addressing
rows preserved; every command warns to stderr while the dotenv still
contains secret-shaped entries.

Future directions still on the backlog:

- secret-redacted success output stays safe for agents
- local-only enrollment flows avoid pasting secrets into prompts
- opt-in OS-keychain-backed refs as a convenience over the SQLCipher
  passphrase, never a cryptographic substitute
- typed project-local tables for descriptor / blinding-key material so
  they do not share the generic `wallets.config_json` blob

## Repository Pattern

Keep SQL close to the function that uses it. Return dataclasses or typed dicts
where helpful. Avoid generic repository base classes.

## Observability

Future project logs should live under the project directory. Logs must redact
secret-bearing fields and avoid raw argv by default.
