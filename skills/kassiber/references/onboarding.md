# Onboarding

Use this reference when the user is setting up Kassiber state, creating the first ledger/books, or confirming the active paths and context. The CLI still calls these `workspace` and `profile`.

## Core idea

`kassiber init` is non-interactive. It creates the managed state tree and reports the active paths. It does not create a ledger/books pair.

## Fresh setup

```bash
kassiber init
kassiber status
```

For repo-local development where `kassiber` is not on `PATH`, use:

```bash
uv run kassiber init
uv run kassiber status
```

Common follow-up setup:

```bash
kassiber workspaces create personal
kassiber profiles create main \
  --workspace personal \
  --fiat-currency EUR \
  --tax-country at \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
kassiber context set --workspace personal --profile main
```

## Context and scope

Check scope before mutating data:

```bash
kassiber status
kassiber context show
kassiber context current
```

Use explicit scope flags when the current context is unclear:

```bash
kassiber profiles list --workspace personal
kassiber accounts list --workspace personal --profile main
```

## Books behavior

Books carry tax defaults through the internal `profile` row:

- `--fiat-currency`
- `--tax-country {generic,at}`
- `--tax-long-term-days`
- `--gains-algorithm {FIFO,LIFO,HIFO,LOFO}`

Creating books creates one default wallet/reporting bucket:

- `treasury`

Accounts are not a double-entry chart of accounts today. `fees` and `external`
are not automatic counterpart destinations.

## Paths

Default state root is usually `~/.kassiber`, but older machines may resolve to a legacy XDG path. Always verify with:

```bash
kassiber status
```

Override roots only when the user explicitly wants a custom location:

```bash
kassiber --data-root /custom/root/data status
kassiber --data-root /custom/root/data init
```

## Optional: encrypt the local database

After `kassiber init` and basic ledger/books setup, the user can opt
into SQLCipher at-rest encryption with:

```bash
kassiber secrets init        # interactive passphrase prompt + confirm
kassiber secrets verify      # confirm the encrypted DB opens cleanly
kassiber secrets status
```

After this runs, every later command needs the passphrase — interactively, or
via `--db-passphrase-fd <FD>`. The pre-encryption plaintext file is preserved
as `kassiber.pre-encryption.sqlite3.bak` so the user can roll back; advise
them to delete it once they trust the new encrypted DB. Kassiber refuses to
overwrite an existing rollback file at that path; inspect, move, or delete the
old file before retrying. There is no recovery path if the passphrase is lost.

If `backends.env` already had API tokens, RPC passwords, or auth headers
before the migration, lift them into the encrypted DB so they no longer sit
in plaintext on disk:

```bash
kassiber secrets migrate-credentials --dry-run
kassiber secrets migrate-credentials
```

URLs and other addressing fields stay in the dotenv; only the secret-shaped
entries move. See [references/secrets-and-backup.md](secrets-and-backup.md)
for the full flow, the `--*-stdin` / `--*-fd` channels for credential input,
and the `kassiber backup` round-trip.
