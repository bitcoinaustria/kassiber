# Daemon Reference

Kassiber's desktop shell talks to the Python core through a local JSONL
daemon. The daemon is started by the Tauri supervisor, reads one JSON object
per line from stdin, and writes one JSON envelope per line to stdout.

Start it directly for development:

```bash
python -m kassiber --data-root /tmp/kassiber-demo/data daemon
```

The daemon bootstraps the normal Kassiber runtime: it creates the state
layout if needed, opens the SQLite database, and serves status from that
local store.

The Tauri supervisor starts the same command. In development it prefers the
repo-local `.venv/bin/python`, then `.venv\Scripts\python.exe` on Windows,
then `python3`. Set `KASSIBER_PYTHON=/path/to/python` to override the Python
executable, or `KASSIBER_REPO_ROOT=/path/to/checkout` when the development
binary should run against a different checkout.

Packaged prerelease desktop builds bundle a one-file PyInstaller CLI sidecar
and prefer that bundled executable before the development Python fallback when
starting the daemon. `KASSIBER_PYTHON` remains the highest-priority override
for daemon startup and installed-app CLI forwarding during debugging.

The first line is always a lifecycle envelope:

```json
{"kind":"daemon.ready","schema_version":1,"data":{"version":"...","supported_kinds":["status","ui.overview.snapshot","ui.transactions.list","ui.wallets.list","ui.backends.list","ui.reports.capital_gains","ui.reports.export_pdf","ui.reports.export_capital_gains_csv","ui.reports.export_austrian_e1kv_pdf","ui.reports.export_austrian_e1kv_xlsx","ui.journals.snapshot","ui.journals.quarantine","ui.journals.transfers.list","ui.profiles.snapshot","ui.rates.summary","ui.workspace.health","ui.next_actions","ui.wallets.sync","wallets.reveal_descriptor","backends.reveal_token","daemon.shutdown"]}}
```

`supported_kinds` is the public UI allowlist the Tauri supervisor mirrors;
treat this list (not the docs) as the source of truth for what the supervisor
will pass through. Reveal kinds (see below) are included in the list but still
require their own passphrase round-trip before the daemon returns raw secret
material.

Requests carry a caller-chosen `request_id`, a `kind`, and optional `args`:

```json
{"request_id":"status-1","kind":"status"}
```

Responses use the normal machine envelope plus the same `request_id`.
`schema_version` follows the CLI machine-output contract; bump it only when
consumers must change how they parse daemon envelopes.

```json
{"kind":"status","schema_version":1,"data":{},"request_id":"status-1"}
```

Errors use the standard error envelope shape and also echo `request_id` when
the request supplied one. Malformed JSON and non-object requests cannot carry
a caller request id, so they return `request_id: null`. `daemon.shutdown`
asks the daemon to write a final shutdown envelope and exit cleanly.

`status`, the `ui.*` snapshots, report export kinds, and `ui.wallets.sync`
are backed by real data today. Report export kinds write files under the
managed `exports/reports/` state directory and return the written path plus
metadata. UI kinds not yet wired return `daemon_unavailable` instead.

## Encrypted database

When `kassiber.sqlite3` is SQLCipher-encrypted, the daemon still bootstraps
through the normal runtime path: it accepts the global `--db-passphrase-fd
<FD>` and falls back to an interactive prompt only if a controlling TTY is
attached. The Tauri supervisor will eventually hand the passphrase via fd
inheritance (tracked in `TODO.md`).

## Reveal kinds (`auth_required` round-trip)

`wallets.reveal_descriptor` and `backends.reveal_token` return raw secret
material — descriptor bodies, blinding keys, BTCPay/RPC tokens. Even when
the daemon already has the database open with the user's passphrase, the
first reveal request returns:

```json
{"kind":"auth_required","schema_version":1,"data":{"scope":"reveal_token","label":"Re-enter database passphrase to reveal backend 'btcpay'"},"request_id":"reveal-1"}
```

The client then resends the same request with `args.auth_response =
{"passphrase_secret": "..."}`. The daemon verifies by opening a throwaway
SQLCipher connection against the on-disk file; a wrong passphrase returns the
structured `local_auth_denied` error envelope. This is a UX gate, not
cryptographic separation — once the daemon is running with an unlocked DB it
can read every credential. The auth round-trip exists so a compromised UI
process cannot silently siphon secrets without surfacing a re-prompt.

The supervisor and any client must redact `passphrase_secret`, `token`,
`descriptor`, `change_descriptor`, `blinding_key`, `auth_header`, and
`password` fields from any persisted log line.
