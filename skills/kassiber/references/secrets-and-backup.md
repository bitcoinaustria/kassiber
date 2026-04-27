# Secrets and Backup

Use this reference for SQLCipher database encryption (`kassiber secrets ...`),
`tar | age` backups (`kassiber backup ...`), passphrase entry through the
global `--db-passphrase-fd` flag, and the `--*-stdin` / `--*-fd` channels for
secret-bearing CLI input.

These features were added in V4.1. Before then the local SQLite store was
plaintext on disk and credentials were collected via argv flags.

## Passphrase entry

The SQLCipher passphrase has no argv form — by design. Three channels:

- Interactive `getpass()` against the controlling TTY when one is attached.
- `--db-passphrase-fd <FD>` global flag (any non-negative integer) reads raw
  UTF-8 bytes from an already-open file descriptor. Strips one trailing
  newline so shell redirects work without trimming.
- The desktop supervisor (eventually) will hand a passphrase to a child
  process via fd inheritance.

```bash
# interactive
kassiber status

# script-friendly: the parent shell opens fd 3 from a file
kassiber --db-passphrase-fd 3 status 3< /tmp/passphrase

# pipe-friendly: useful from password managers
gopass show kassiber/db-pass | kassiber --db-passphrase-fd 0 status
```

Behavior to remember:

- A wrong passphrase surfaces as the structured `unlock_failed` envelope, not
  a generic SQLite error.
- A missing passphrase against an encrypted DB produces `passphrase_required`.
- The plaintext code path is preserved: when the on-disk file looks like a
  vanilla SQLite database, no passphrase is asked for and the legacy behavior
  is unchanged.
- The passphrase is **not** stored in the OS keychain; the passphrase is the
  perimeter and there is no recovery path if it is lost.

## First-time encryption

`kassiber secrets init` migrates an existing plaintext database in place
using the SQLCipher-recommended `sqlcipher_export()` recipe. It preserves
`user_version` and `auto_vacuum`, scans the encrypted output for credential
markers as a sanity check, and runs `cipher_integrity_check` when the bundled
SQLCipher build supports it.

```bash
# interactive prompt + confirm
kassiber secrets init

# fd-based prompt (parent shell opens fd 4 with the new passphrase)
kassiber secrets init --new-passphrase-fd 4 4< /tmp/new

# inspect what is on disk before deciding
kassiber secrets status
```

After a successful migration:

- `~/.kassiber/data/kassiber.sqlite3` is the encrypted file.
- `~/.kassiber/data/kassiber.pre-encryption.sqlite3.bak` is the original
  plaintext file, preserved so a manual rollback is one `mv` away.
- Advise the user to run `kassiber secrets verify` and then delete the `.bak`
  file once they trust the encrypted DB. Kassiber does not auto-delete it.

If a previous migration failed mid-flight, `kassiber secrets init-resume`
describes the state and points at the leftover `*.encrypted.sqlite3` file.

## Rotating the passphrase

```bash
kassiber secrets change-passphrase
kassiber secrets change-passphrase --db-passphrase-fd 3 --new-passphrase-fd 4 \
  3< /tmp/old 4< /tmp/new
```

Rotation runs `PRAGMA rekey` and then re-opens the file with the new
passphrase to verify. The old passphrase will not unlock the file after this
returns successfully.

## What stays plaintext on disk

The SQLCipher boundary covers `kassiber.sqlite3` only. Everything else under
the data root remains plaintext on disk:

- `~/.kassiber/config/backends.env` — backend bootstrap dotenv. URLs,
  `KIND`, chain, network, batch sizes, and other addressing metadata
  may stay here in plaintext. Tokens, passwords, auth headers, and
  basic-auth usernames must NOT — those are secret-shaped fields that
  belong in the encrypted DB. Seed new credentials with `--token-stdin`
  / `--token-fd FD` so they go straight into the DB, and run
  `kassiber secrets migrate-credentials` once to lift any pre-existing
  secret entries out of the file (see "Migrating credentials" below).
- `~/.kassiber/config/settings.json` — path layout manifest. Not secret, but
  it discloses where the rest of the local state lives.
- `~/.kassiber/attachments/` — copied attachment files for transactions.
  Treat the directory as user data.
- `~/.kassiber/exports/` — generated reports (PDF/CSV/XLSX) and saved
  diagnostics. Reports contain full transaction detail; diagnostics are
  designed to be public-safe.
- `kassiber.pre-encryption.sqlite3.bak` — pre-migration plaintext snapshot.
  Tell the user to remove it after verifying the encrypted DB.

When a user asks "is my data encrypted now," this list is the honest answer.

## Migrating credentials out of `backends.env`

`kassiber secrets migrate-credentials` moves the secret-shaped dotenv
entries (`*_TOKEN`, `*_PASSWORD`, `*_AUTH_HEADER`, `*_USERNAME`,
`*_RPCPASSWORD`, `*_RPCUSER`) into the encrypted `backends` table. URL,
kind, chain, network, and other addressing fields stay where they are.

```bash
# preview what would migrate, without touching the file
kassiber secrets migrate-credentials --dry-run

# perform the migration (requires the DB passphrase)
kassiber secrets migrate-credentials
kassiber secrets migrate-credentials --db-passphrase-fd 3 3< /tmp/pass
```

Behavior:

- Each secret-shaped entry is written to the matching `backends` row.
  If the row does not exist yet, that entry is reported in `skipped`
  with `reason="backend_not_in_db"` — create the backend first with
  `kassiber backends create`, then re-run.
- The original dotenv is copied to
  `backends.env.pre-credentials-migration-<timestamp>.bak` before any
  rewrite. Inspect and delete the backup once you trust the new state.
- Non-secret rows (URLs, `KIND`, `KASSIBER_DEFAULT_BACKEND`, comments,
  blank lines) survive the rewrite untouched.

Whenever the database is encrypted but `backends.env` still contains
secret-shaped entries, every Kassiber command writes a one-line warning
to stderr pointing at this command. `kassiber secrets status` lists the
exact entries that would migrate.

## `--*-stdin` and `--*-fd` for secret-bearing fields

Every secret-bearing CLI value now has matching `--<name>-stdin` and
`--<name>-fd FD` variants. The argv form (e.g. `--token <value>`) still works
but emits a deprecation warning and leaks to shell history. Prefer the safe
forms.

Covered fields include `--token`, `--password`, `--username`, `--auth-header`,
`--descriptor`, and `--change-descriptor`. Pattern:

| Argv form (warns)                     | Safe replacement                              |
|---------------------------------------|-----------------------------------------------|
| `--token <value>`                     | `--token-stdin` or `--token-fd FD`            |
| `--password <value>`                  | `--password-stdin` or `--password-fd FD`      |
| `--descriptor <value>`                | `--descriptor-stdin` or `--descriptor-fd FD`  |
| `--change-descriptor <value>`         | `--change-descriptor-stdin` or `--change-descriptor-fd FD` |

Constraints:

- Only one `--*-stdin` option may be active per invocation (it consumes
  the regular `stdin` channel). Use `--*-fd` for additional inputs.
- Multiple `--*-fd` options may coexist; each reads from the indicated
  fd and closes it.
- Empty values and NUL bytes are rejected up front.

Examples:

```bash
# create a BTCPay backend without putting the token in shell history
printf %s "$BTCPAY_TOKEN" | kassiber backends create btcpay-prod \
  --kind btcpay --url https://btcpay.example.com --token-stdin

# create a descriptor wallet, reading both descriptors from fds
kassiber wallets create \
  --label vault --kind descriptor --account treasury --backend mempool \
  --descriptor-fd 3 --change-descriptor-fd 4 \
  3< /path/to/receive.desc 4< /path/to/change.desc
```

When you hand the user a paste-ready local template, prefer the `--*-stdin`
form with a `printf %s "$VAR" | kassiber ...` pipe rather than collecting
the secret in chat.

## Reveal: pulling a secret back out of the DB

The daemon refuses to return raw descriptor or token material without a
fresh passphrase round-trip even if the DB is already unlocked in the
running process. Each reveal request requires an `auth_response` carrying
the passphrase; a wrong passphrase produces `local_auth_denied`.

CLI surfaces:

```bash
kassiber backends reveal-token <name>
kassiber wallets reveal-descriptor <wallet-label> --workspace personal --profile main
```

These exist for legitimate recovery and rotation workflows. Do not pipe the
output into chat — it is the raw secret.

## Backup: `tar | age` with a SQLCipher inner DB

`kassiber backup export` writes a single file containing:

- `manifest.json` — schema version, kassiber version, paths, entry counts.
- `kassiber.sqlite3` — a SQLCipher copy of the live DB (still encrypted with
  the same DB passphrase).
- `attachments/` — mirror of the live attachments tree.
- `config/backends.env` — mirror of the live env file when present.

The whole tar stream is then encrypted by `age` with an outer passphrase
(or a recipient public key, if you pass `--recipient`). The two passphrases
are independent: the outer age passphrase only protects the bundle in
transit / at rest off-box; the inner DB passphrase still gates the data
inside it.

```bash
# export with a fresh outer passphrase (interactive prompt)
kassiber backup export --file /tmp/snap.kassiber

# export with the outer passphrase from fd 4
kassiber backup export --file /tmp/snap.kassiber --backup-passphrase-fd 4 4< /tmp/outer

# export to an age recipient (public-key mode, streams through binary `age`)
kassiber backup export --file /tmp/snap.kassiber --recipient "age1..."
```

`kassiber backup import` reverses the process. It decrypts to a temp tarball,
runs the strict tar member validator (rejects symlinks, hardlinks, device
nodes, FIFOs, traversal, duplicates, oversized members), extracts into a
staging directory, validates the manifest against the staged files, and
optionally moves the staged tree into place with `--install`.

```bash
# stage only — leave the tree under a temp directory
kassiber backup import /tmp/snap.kassiber --backup-passphrase-fd 4 4< /tmp/outer

# install over the active data root (creates a pre-restore snapshot first)
kassiber backup import /tmp/snap.kassiber --backup-passphrase-fd 4 \
  --install --target-data-root ~/.kassiber/data 4< /tmp/outer
```

`--install` moves any pre-existing live data into a sibling
`pre-restore-<timestamp>/` directory before overwriting, so an accidental
restore over a populated data root is recoverable.

A `.kassiber` file does **not** include or recover the inner DB passphrase.
You still need it to read the database after restore.

## age backend selection

Kassiber prefers the stand-alone `age` (or `rage`) binary on `PATH` for
recipient-mode backups because they stream arbitrary archive sizes. For
passphrase-mode flows it prefers the in-process `pyrage` library, which is
shipped as a dependency. If `pyrage` is missing and only the binary is
present, passphrase-mode backups will fail with a clear error.

Recovery without Kassiber: any stranded user can decrypt a `.kassiber` file
with stock `age` + `tar` + `sqlcipher` and recover everything that was inside
the bundle, given both passphrases. The format is intentionally boring.
