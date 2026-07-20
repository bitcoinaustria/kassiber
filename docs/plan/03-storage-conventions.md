# Storage Conventions

**Status:** Project-container implementation in progress. New default runtime
resolution uses `~/.kassiber/projects/<project>/...`; `--data-root` remains an
explicit escape hatch for tests, scripts, and manually chosen project data
roots.
**Current source of truth:** `kassiber/db.py`, `kassiber/core/runtime.py`,
README, and TODO.md.

## Product Direction

Move toward one project bundle per bookkeeping scope:

```text
~/.kassiber/
  config/projects.json       # non-secret project catalog
  projects/
    <project>/
      data/kassiber.sqlite3
      config/settings.json
      config/backends.env
      attachments/
      exports/
```

A project is the unit of storage, unlock, backup, import/restore, and deletion.
Books/profiles live inside a project; they are not separate cryptographic
boundaries.

Do not split one project across unrelated writable roots unless a later design
explicitly requires it.

## Handoff Boundaries

Use the project boundary for privacy isolation and the book boundary for scoped
accounting handoff:

- **Project export / backup** is the full local custody package for a related
  set of books. It may contain encrypted wallet configuration, descriptors,
  backend records, attachments, and every book in the project. It is not the
  default artifact for tax advisors.
- **Tax advisor report** is the default external handoff. It contains report
  outputs and supporting report tables, but it never includes wallet
  descriptors, xpubs, backend credentials, raw wallet config, AI settings, logs,
  or unrelated books.
- **Audit package** is a trusted/internal handoff for exactly one book or an
  explicit set of books. It may include transaction-level evidence, journals,
  reviewed source-of-funds state, selected attachments, and import provenance.
  It still excludes descriptors and xpub-like wallet material by default. Import
  should create a new project by default, not merge into an existing private
  project silently.
- **Current audit package export** writes a managed directory under
  `exports/reports/` with a deterministic `manifest.json` plus an
  `evidence/` folder when copied-file inclusion is enabled. The manifest ties
  included transactions to direct attachments, copied-file hashes, URL
  references, source-funds links, review state, journal context, copied-evidence
  provenance, and missing evidence warnings. URL/cloud documents remain
  references only; Kassiber does not fetch or mirror them. Kassiber derives an
  editable display label from the URL itself; the stored URL is unchanged.
  When evidence is reused between transactions, file attachments are duplicated
  under a new attachment id instead of sharing `stored_relpath`.
- **Technical wallet evidence** is a separate restricted action, not a normal
  export checkbox. If it is ever implemented, it must require explicit approval
  and should explain that descriptors or xpub-like material can reveal wallet
  history and future wallet activity.

Default UI and CLI export flows should prove the accounting without proving
wallet completeness. Wallet-completeness evidence is a different job with a
different sensitivity class.

## Current Layout

Current default state is:

```text
~/.kassiber/
  config/projects.json
  projects/default/
    data/kassiber.sqlite3
    config/backends.env
    config/settings.json
    exports/
    attachments/
```

The global catalog contains only non-secret metadata: project id, name, path,
encrypted status, and last-opened timestamp. It never stores passphrases,
verifiers, wrapped keys, descriptors, backend tokens, xpubs, accounting rows,
chat history, or AI provider secrets.

Backend definitions are canonical in each project's SQLite database; the
project-local dotenv remains a bootstrap and compatibility path. If
`backends.env` contains token/password/auth-header/user entries, those are
plaintext until `kassiber secrets migrate-credentials` lifts them into the
encrypted project DB.

## SQLite Rules

- SQLite remains the system of record.
- Use stdlib `sqlite3`; no ORM.
- Each initialized database has a durable random `database_instance_id` in its
  settings table. The operator broker binds admitted work to the id read from
  the opened connection and verifies it before migrations or command work in a
  child. A byte-for-byte backup/restore remains the same logical database
  instance; a newly initialized database receives a new id.
- BTC amounts are integer msat.
- Fiat columns are still `REAL` unless a future report-specific boundary
  deliberately uses integer cents.
- Prefer additive schema changes compatible with `CREATE TABLE IF NOT EXISTS`
  and lightweight compatibility migrations.

Project-mode connections run:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
```

Current `open_db()` guarantees schema bootstrap plus these connection pragmas.

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

## Legacy App-Wide Migration

On first default startup without a project catalog, Kassiber discovers legacy
databases in the old hidden-home and XDG locations (`~/.kassiber/data/`,
`~/.local/share/kassiber/`, `~/.local/share/satbooks/`) and stages a copy into
`~/.kassiber/projects/default/`. After the staged project is in place, the old
plaintext database, `config/backends.env`, `config/settings.json`,
`attachments/`, and `exports/` artifacts are moved aside under
`pre-project-migration-<timestamp>/` at the legacy source root. This preserves a
manual rollback package without leaving the old active plaintext path in place.
Logs are not migrated because Kassiber's normal log ring is RAM-only.

A legacy DB with multiple workspaces is migrated as one default project
container because a project/book-set may contain multiple books/profiles. A JSON
report under the legacy source root's `config/migration-reports/` records the
workspace count and the future split policy. An encrypted legacy DB whose
workspace count cannot be read before unlock is validated after the first
successful unlock and then clears the validation marker instead of blocking
startup. The split policy for any future explicit split is:

- workspace/profile-scoped tables filter by `workspace_id` / `profile_id`
- relationship tables follow their parent transaction/session/attachment rows
- `settings`, `backends`, rates caches, AI providers/secret refs, and graph
  caches are project-shared and copied to each future split project
- attachments are copied project-local and orphan cleanup may prune unused files
- exports are generated plaintext artifacts and are copied only as convenience,
  not treated as accounting source of truth

## Backup / Restore

`kassiber backup export` is project-scoped. It uses `Connection.backup()` to
take a hot SQLCipher copy of the selected project's DB while writers continue,
tars the staging tree (DB + attachments + `backends.env` + `settings.json`),
records that exports/logs are excluded, and pipes the tarball through `age`
(binary or `pyrage`) into a single `.kassiber` envelope. Recovery
without Kassiber is intentionally possible with stock `age` + `tar` +
`sqlcipher`.

Restore is staged through `kassiber backup import`: decrypt to a temp
tarball, run the strict tar-member validator, extract under a staging
directory, validate the manifest, and only on `--install` copy the staged
tree into the live data root after snapshotting any pre-existing files
into `pre-restore-<ts>/`. Successful installs remove the decrypted temp
workspace; stage-only imports leave the extracted staging tree for manual
inspection. There is no hot in-place restore.

## Secrets

Each project database at
`~/.kassiber/projects/<project>/data/kassiber.sqlite3` may be encrypted at rest
under that project's own SQLCipher passphrase (`kassiber secrets init` while
that project is selected, or `--data-root` for explicit roots). Stock SQLCipher
PRAGMA defaults
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

Normal daemon, desktop, and operator-broker logs are bounded and RAM-only; they
do not live under the project directory. Secret-floor redaction happens before
ring insertion, and broker records omit raw argv, paths, endpoint names, and
secrets. Only explicit user exports may write a redacted support artifact, as
specified in [the logging reference](../reference/logging.md). An always-on
project log remains a rejected design.
