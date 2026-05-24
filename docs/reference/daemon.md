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

The first line is always a lifecycle envelope. The `supported_kinds` array
below is representative; use the live `daemon.ready` payload from the running
daemon for the exact current allowlist:

```json
{
  "kind": "daemon.ready",
  "schema_version": 1,
  "data": {
    "version": "...",
    "supported_kinds": [
      "status",
      "ui.overview.snapshot",
      "ui.transactions.list",
      "ui.transactions.metadata.update",
      "ui.wallets.list",
      "ui.backends.list",
      "ui.backends.options",
      "ui.reports.capital_gains",
      "ui.reports.export_pdf",
      "ui.reports.export_summary_pdf",
      "ui.reports.export_csv",
      "ui.reports.export_xlsx",
      "ui.reports.export_capital_gains_csv",
      "ui.reports.export_austrian_e1kv_pdf",
      "ui.reports.export_austrian_e1kv_xlsx",
      "ui.reports.export_austrian_e1kv_csv",
      "ui.journals.snapshot",
      "ui.journals.events.list",
      "ui.journals.quarantine",
      "ui.journals.transfers.list",
      "ui.journals.process",
      "ui.profiles.snapshot",
      "ui.profiles.create",
      "ui.profiles.switch",
      "ui.rates.summary",
      "ui.rates.kraken_csv.import",
      "ui.rates.rebuild",
      "ui.workspace.health",
      "ui.workspace.create",
      "ui.workspace.delete",
      "ui.profiles.reset_data",
      "ui.secrets.init",
      "ui.secrets.change_passphrase",
      "ui.next_actions",
      "ui.wallets.create",
      "ui.connections.btcpay.create",
      "ui.connections.btcpay.discover",
      "ui.connections.btcpay.test",
      "ui.connections.node.snapshot",
      "ui.reports.lightning_profitability",
      "ui.metadata.bip329.import",
      "ui.wallets.update",
      "ui.wallets.delete",
      "ui.wallets.sync",
      "daemon.lock",
      "daemon.unlock",
      "ai.providers.list",
      "ai.providers.get",
      "ai.providers.create",
      "ai.providers.update",
      "ai.providers.set_api_key",
      "ai.providers.delete",
      "ai.providers.set_default",
      "ai.providers.clear_default",
      "ai.providers.acknowledge",
      "ai.list_models",
      "ai.test_connection",
      "ai.chat",
      "ai.chat.cancel",
      "ai.tool_call.consent",
      "wallets.reveal_descriptor",
      "backends.reveal_token",
      "daemon.shutdown"
    ]
  }
}
```

`ui.reports.export_summary_pdf` writes a managed stakeholder summary PDF. It
accepts optional `start` / `end` RFC3339 timestamps, optional `wallets` as an
array of wallet ids or labels, and `include_snapshot` to prepend an as-of-now
cover snapshot. The export response includes `data_integrity` (now with an
`internal_transfers` count + volume), period-end `wallet_holdings` /
`holdings_totals`, current `snapshot_wallets` / `snapshot_totals` when
requested, and `balance_history` so callers can reconcile the generated PDF
against the portfolio-summary and balance-history report surfaces. The
response also surfaces `metrics` (including `unrealized_pnl`, `end_cost_basis`,
and BTC stack start/end), a `benchmark` block with BTC spot performance over
the period, plus `top_movements`, `top_disposals`, and `holding_age` so
callers can re-render the same narrative outside the PDF. The export is
portfolio-shaped and deliberately omits tax tables; Austrian tax PDFs remain
the authoritative tax handoff.

`supported_kinds` is the public UI allowlist the Tauri supervisor mirrors;
treat this list (not the docs) as the source of truth for what the supervisor
will pass through. Reveal kinds (see below) are included in the list but still
require their own passphrase round-trip before the daemon returns raw secret
material.

`ai.providers.set_api_key` accepts
`{"name":"provider","api_key":"..."}` or `{"name":"provider","api_key":null}`.
It is the desktop API-key rotate/re-enter path. The terminal envelope is
redacted and contains `has_api_key` plus `secret_ref.{store_id,state}`, never
the raw key. `ai.providers.move_api_key` accepts
`{"name":"provider","store_id":"sqlcipher_inline|macos_keychain|windows_dpapi|linux_secret_service"}`
and moves an existing provider key through the desktop-only native-store bridge.
OS-backed refs that cannot be read return `secret_ref_unavailable` with
`details.refs` and a Settings repair path.
`ai.providers.create`, `ai.providers.update`, and `ai.test_connection` reject
caller-supplied `api_key`; desktop callers must set or rotate keys through
`ai.providers.set_api_key` and then test the stored provider.

`ui.profiles.switch` accepts `{"profile_id":"..."}` and updates the active
books set / book (`context_workspace` / `context_profile` internally) after the
database is already unlocked. It does not create a per-books/profile passphrase
boundary; SQLCipher encryption is database-level.

`ui.profiles.create` accepts `{"workspace_id":"...","label":"..."}` and creates
a new book in that books set. `workspace` and `profile` remain the daemon/API
names for books set and book. It inherits fiat currency, tax country, long-term
period, and gains algorithm from the active book in that set when available;
otherwise it uses the first book in the set, then generic EUR/FIFO defaults for
empty sets. It can also accept `source_profile_id` to copy those settings from a
specific book in the same set. Wallets, accounts/buckets, and transactions are
not copied. The new book becomes active.

`ui.workspace.create` accepts `{"label":"..."}` and creates an empty books set.
`workspace` remains the daemon/API name. The daemon makes the new books set
current and clears the active book/profile until the user creates or switches to
books inside that set.

`ui.workspace.delete` accepts
`{"confirm":"DELETE","confirm_workspace":"..."}` for the current books set. Like
wallet deletes, encrypted databases require `args.auth_response.passphrase_secret`
and plaintext databases require `DELETE LOCAL DATA`.

`ui.profiles.reset_data` accepts
`{"confirm":"RESET","confirm_profile":"...","clear_shared_rates":false}` for
the current book. It keeps the current books set, book/profile, bucket/account
rows, wallet connection rows, and configured backends, then clears
imported/synced transactions, journals, quarantines, swap pairs/dismissals/rules,
saved views, BIP329 labels, transaction tags, attachment metadata/files, and
source-of-funds review state so testing can redo sync, journal processing, and
swap review from the preserved wallet connections. The global local fiat-rate
cache is shared across books in the data root and is only cleared when
`clear_shared_rates` is the boolean value `true`; string values such as
`"false"` are rejected. Like other sensitive local-data changes, encrypted
databases require `args.auth_response.passphrase_secret` and plaintext databases
require `DELETE LOCAL DATA`.

`ui.backends.options` returns safe backend setup choices for desktop forms. It
lists configured backend names, kinds, chain/network metadata, presence flags,
and default state, but does not expose exact endpoint URLs or tokens.

`ui.wallets.create` is the desktop connection setup path for local/imported
wallet sources. It accepts `label`, `kind`, and the same wallet config fields
the CLI stores (`backend`, `chain`, `network`, `descriptor`,
`change_descriptor`, `source_file`, `source_format`, `store_id`,
etc.) and returns the redacted wallet row. Desktop callers can pass
`wallet_material` instead of separate descriptor fields; the daemon recognizes
common descriptor export shapes and stores receive/change descriptors when the
material contains both.

`ui.wallets.preview_descriptor` is a read-only helper for the connection
setup form. It accepts `wallet_material` (or explicit `descriptor` /
`change_descriptor`), optional `chain`/`network`/`count` (1–20, default 5),
and returns the first N derived receive addresses plus the first change
address when present. The daemon does not persist anything; callers use the
preview to confirm a descriptor produces the expected wallet before
committing.

`ui.wallets.import_file` accepts `wallet`, `source_file`, and `source_format`
for wallet-scoped CSV/JSON imports. Wallet CSV import results include
`inserted_records`, `updated_records`, and `unchanged` so the desktop can show
what changed after an exchange CSV import. For
`source_format="bullbitcoin_csv"` or `source_format="coinfinity_csv"`, `wallet`
is optional. The default `mode="relevant"` treats the export as book-wide
exchange evidence and enriches only unique matching transactions in the active
profile. `mode="full"` imports all normalized provider rows into the selected
or default provider wallet as excluded evidence, then flags each row as
`matched`, `unmatched`, or `ambiguous` against this book's wallet transactions.
Coinfinity imports return `coinfinity_rows`. For
`source_format="21bitcoin_csv"`, the default `mode="full"` imports active
custodial ledger rows into the selected or default `21bitcoin` wallet; explicit
`mode="relevant"` keeps the evidence-only matching behavior for L1 withdrawal
rows. `source_format="pocketbitcoin_csv"` follows the Bull Bitcoin mode
contract, uses a default `Pocket Bitcoin` wallet in full mode, and returns
`pocketbitcoin_rows`. Because Pocket's CSV does not expose the blockchain txid,
relevant-mode matching uses the net BTC amount, direction, asset, and nearby
timestamp. `source_format="strike_csv"` imports active Strike platform ledger
rows into the selected or default `Strike` wallet, including exchange buy/sell
rows plus Lightning and on-chain wallet activity. It keeps Lightning payment
hashes when exported, skips fiat-only platform rows, and returns `strike_rows`.
The result also includes `matched`, `skipped_unmatched`, and
`skipped_ambiguous` in relevant mode, or `matched`, `unmatched`, `ambiguous`,
`excluded`, and `reconciliation_records` in Bull Bitcoin/Coinfinity/Pocket full
mode.

`ui.connections.sources` returns the daemon's authoritative catalog of
supported wallet kinds (with summary/config-fields metadata) and the
recognized import `source_formats`. The desktop catalog stays the source of
truth for icons, copy, and ordering, but uses this list to verify it isn't
advertising a "ready" connection backed by a wallet kind or import format
the daemon does not implement.

`ui.connections.btcpay.create` configures a BTCPay store in one of two modes.
The default `wallet_sources` mode creates wallets configured for confirmed
Greenfield wallet-history sync from a BTCPay instance, so a BTCPay-only setup is
enough when BTCPay is the source of the wallet history. The `existing_wallets`
mode maps selected BTCPay payment methods onto already configured settlement
wallets and stores BTCPay provenance routes there; those wallets keep their
normal descriptor/file sync source while BTCPay comments and labels enrich
matching transactions. Both modes accept either a saved `backend` or inline
instance credentials (`backend_label`, `server_url`, `api_key`) plus `label`,
`store_id`, and either optional `payment_method_id` (default `BTC-CHAIN`) or
`payment_method_ids` for bulk setup. In `wallet_sources`, bulk setup creates one
Kassiber wallet per selected payment method and suffixes labels with the
payment method id. In `existing_wallets`, callers pass `routes` containing
`wallet` and `payment_method_id`. Inline credentials create a local `btcpay`
backend row first, then store only the redacted backend reference on the
wallet. Use one Kassiber wallet per real underlying BTCPay-backed wallet
balance; stores that share the same BTCPay wallet should not be duplicated as
separate Kassiber wallets.

`ui.connections.btcpay.discover` accepts the same saved-backend or inline
instance credential shape as `create`, performs read-only Greenfield discovery,
and returns safe store ids/names plus enabled payment method ids. It does not
persist anything and does not request payment-method config bodies, because
those may contain wallet material. Desktop setup should default to selecting all
sync-supported payment methods for the chosen store and leave unsupported
methods for future source-specific adapters.

`ui.connections.btcpay.test` makes a single Greenfield request against
the saved-backend or inline instance credentials plus `store_id` (and optional
`payment_method_id`, defaulting to `BTC-CHAIN`) to confirm the credentials and
store reference resolve. It returns
`{backend, store_id, payment_method_id, ok: true}` on success, and otherwise
propagates the same structured error codes (`auth_error`, `not_found`,
`network_error`) the sync path uses. Nothing is persisted.

`ui.transactions.commercial_context` reads the reviewed or suggested
commercial provenance for one transaction. It joins wallet transactions to
BTCPay payments, their parent invoices, payment-request ids when present,
normalized origin hints such as POS/app/external-order, and linked external
documents. The payload is a redacted UI read model; it does not expose raw
BTCPay invoice JSON, rejected matches, payment hashes, destination addresses,
full origin URLs, payment-method configuration, descriptors, xpubs, or API
tokens.

`ui.metadata.bip329.import` accepts `file` and optional `wallet`, then imports
BIP329 JSONL labels into the active profile and bridges transaction labels to
matching local transactions.

`ui.transactions.metadata.update` accepts
`{"transaction":"...","note":"...","tags":["Reviewed"],"excluded":false}` for
the active books/profile. The daemon persists the note, replaces the
transaction tag set (creating missing tag rows), updates the exclusion flag, and
invalidates processed journals so reports are rebuilt before use.

`ui.wallets.update` accepts `{"wallet":"..."}` for the active books/profile and
edits at least one of `label`, the same wallet config fields the create
endpoint takes (`backend`, `chain`, `network`, `descriptor`,
`change_descriptor`, `wallet_material`, `source_file`, `source_format`,
`store_id`, `payment_method_id`, `gap_limit`, `addresses`, `policy_asset`),
or `clear` — a list of config field names to remove. `wallet_material`
overwrites the receive/change descriptors when present so users can paste a
fresh export to fix a typo. `ui.wallets.delete` accepts
`{"wallet":"...","confirm":"DELETE","confirm_wallet":"...","cascade":true|false}`.
Both kinds are sensitive local-state changes: encrypted databases require
`args.auth_response.passphrase_secret`, verified with the same throwaway
SQLCipher round-trip used by reveal requests; plaintext databases require an
explicit acknowledgement (`CHANGE LOCAL DATA` for updates, `DELETE LOCAL DATA`
for deletes).

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

`status`, the `ui.*` snapshots, report export kinds, `ui.wallets.sync`, and
`ui.journals.process` are backed by real data today. Report export kinds write
files under the managed `exports/reports/` state directory and return the
written path plus metadata. UI kinds not yet wired return `daemon_unavailable`
instead.

## Lightning node kinds

`ui.connections.node.snapshot` and `ui.reports.lightning_profitability`
route through the shared
[`kassiber.core.lightning`](../../kassiber/core/lightning/) scaffold. Each
request takes `args.connection` (a wallet id or label that resolves to a
Lightning-kind wallet) and optional `args.window_days` (default 30, max 365).
The daemon resolves the wallet, looks up the registered
[`LightningAdapter`](../../kassiber/core/lightning/adapter.py) for the wallet
kind (`lnd`, `coreln`, `nwc`), and dispatches the read. LND ships an
adapter ([`kassiber/core/lightning/lnd.py`](../../kassiber/core/lightning/lnd.py))
that the daemon imports at startup. Without a registered adapter the
daemon returns an `lightning_adapter_unavailable` error envelope so the
desktop can fall back to mock data.

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
`descriptor`, `change_descriptor`, `blinding_key`, `auth_header`, `password`,
and `api_key` fields from any persisted log line.
The daemon also redacts secret-shaped strings and sensitive detail keys at the
error-envelope boundary before responses cross into Tauri, the Vite bridge, or
UI state. Provider-controlled AI error bodies are treated as hostile and are
size-limited plus redacted before they become `error.details.body`.
