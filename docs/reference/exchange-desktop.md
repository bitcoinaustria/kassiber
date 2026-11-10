# Exchange API connections in the desktop

The desktop connects Kraken, Coinbase, Binance and Bitpanda through a
read-only API key and syncs their history on request. Each setup and sync runs
the [exchange API framework](rust-exchange-sync.md) the CLI's
`wallets sync-<kind>` commands use ([Bitpanda](bitpanda.md) has its own
page). This is a Rust backend feature: the Python daemon refuses its kinds
(see [Python](#python)).

## Flow

1. **Pick the exchange.** The Add connection dialog lists Kraken, Coinbase,
   Bitpanda and Binance API under Exchanges (setup kind `exchange-api`). The
   Binance supplemental CSV import stays a separate card.
2. **Create a read-only key.** The setup shows where to create the key and
   which of the exchange's permissions to turn on and leave off. The scopes
   follow what each sync reads:

   | Exchange | Turn on only | Kassiber checks |
   | --- | --- | --- |
   | Kraken | Query closed orders & trades, Query ledger entries | nothing more: Kraken does not report a key's other permissions |
   | Coinbase | `wallet:accounts:read`, `wallet:transactions:read` (an API key and secret; Coinbase Developer Platform key files are not supported) | nothing more: the v2 API does not report them |
   | Binance | Enable Reading | `GET /sapi/v1/account/apiRestrictions`: reading must be on and every other `enable*`/`permits*` flag off, including flags Binance adds later |
   | Bitpanda | Transaction, Trade (Read) | `GET /v1/trading-limits`: a key that can trade is refused |

3. **Paste the key** (and secret; Bitpanda has none). The fields are password
   inputs; the values stay in the form until it saves or closes.
4. **Test connection** (`ui.connections.exchange.test`). This is the first
   request to the exchange. It signs the fewest reads that prove the key reads
   what the sync needs: Kraken's first `TradesHistory` and `Ledgers` pages,
   Coinbase's first accounts page, Binance's API restrictions, and Bitpanda's
   write-scope check, currencies and a one-row operations page. It writes
   nothing. The dialog saves only credentials that passed the test as they
   are; changing the key or secret asks for a new test.
5. **Create connection** (`ui.connections.exchange.create`). No request. It
   stores the backend `backends create <kind> --kind <kind> --url <API>
   --token-stdin [--auth-header-stdin]` stores (named after the exchange, then
   `<kind>-2`, ...), and the provider wallet `wallets sync-<kind>` creates on
   its first run (label, kind, `{"backend": name}`). A wallet that already
   has the label is refused (`conflict`), because the CLI sync would import
   into it. The key lives in the book's `backends` row, inside SQLCipher when
   the book is encrypted, and no kind returns it.
6. **Sync now** (`ui.connections.exchange.sync`), either right after creation
   when the user ticks **Import history right after adding the connection**
   (off by default) or from the wallet page. It runs `wallets sync-<kind>
   --backend <the wallet's backend> --wallet <wallet>`.

## Wallet page

An exchange wallet's page shows an exchange card instead of the chain refresh:
whether its key is stored, the last sync of this session (time, new and read
rows, skipped Bitpanda operations) or its error, and **Sync now**. The header
Refresh runs the same sync; Full rescan is hidden. Errors are explained by
code, then the daemon's own message and hint:

| Code | Shown as |
| --- | --- |
| `auth_error` | the key was refused (wrong, expired, IP-restricted, missing scope, or able to trade); Binance's refusal lists the permissions still on |
| `rate_limited` | the exchange is limiting requests |
| `network_egress_disabled` | offline mode stopped the request |
| `protocol_error` | an answer Kassiber cannot read safely; nothing imported |
| `validation` | an entry Kassiber cannot book safely, such as Bitpanda's unrecognised types; nothing imported, please report it |
| `config_error` | the saved key is incomplete |
| `backend_timeout`, `network_error`, `app_error` | unreachable, slow, or an HTTP error |
| `daemon_unavailable`, `unsupported_kind` | the Python backend; see [Python](#python) |

The last sync result lives in the open book's memory only
(`ui.connections.exchange.status`), so it resets when Kassiber restarts or the
book closes. The CLI's sync stores no sync record, and the desktop's book
must hold the CLI's rows, so the book has nothing to read it from.

## Daemon kinds

All four are Rust-only daemon kinds, in the Tauri and browser-bridge
allowlists, classified like their BTCPay counterparts (`status` read, the
rest operator), and refuse unknown arguments before any work.

| Kind | Arguments | Network | Answer |
| --- | --- | --- | --- |
| `ui.connections.exchange.status` | `wallet?` | none | `connections`: per exchange wallet of the active profile, `wallet_id`, `wallet`, `exchange`, `provider`, `backend`, `backend_configured`, `has_api_key`, `has_api_secret` (null for Bitpanda) and `last_sync` (`status`, `at`, `imported`, `fetched`, `skipped`, or the error envelope's `error`) |
| `ui.connections.exchange.test` | `backend`, or `exchange`, `api_key`, `api_secret`, `url?` | the check above, once | `exchange`, `ok`, `read_only` (true when the exchange confirmed it, null when it does not say) |
| `ui.connections.exchange.create` | `exchange`, `api_key`, `api_secret` (not for Bitpanda), `label?`, `url?` | none | `exchange`, `wallet` (`id`, `label`, `kind`), `backend` (the redacted backend row) |
| `ui.connections.exchange.sync` | `wallet` | the CLI sync's reads | the CLI's `wallets.sync-<kind>` data |

`url` defaults to the exchange's public API and exists for tests and
proxies; the desktop never sends it.

## Consent and privacy

- Launch, route mounts, the wallet page, `status`, `create`, freshness and
  idle paths send nothing. A saved exchange backend is configuration, not
  consent.
- `test` and `sync` each mint one explicit-trigger consent for their exact
  route: `test` binds the kind, book, exchange and transport; `sync` binds
  what the CLI command binds (book, profile, wallet, exchange, transport).
  `test` revokes its grant when it returns. Every request rechecks the gate,
  so offline mode or `KASSIBER_NO_EGRESS` stops either before its first
  request, and the sync's publication savepoint is rechecked too.
- An echoed key or secret is withheld from every error, as for the CLI, so
  neither reaches the reply, the RAM logs, the session's last-sync record or
  the book. Request arguments named `api_key` and `api_secret` are sensitive
  to the shared redaction.
- `ui.wallets.sync`, Refresh all and background freshness still treat
  exchange wallets as sourceless, and no assistant or MCP tool calls these
  kinds: exchange syncs stay out of the assistant's surface, as before. Only
  the user's click on Sync now contacts the exchange.

## Python

Python has no handler for these kinds and does not advertise them. Its daemon
answers each with an error envelope (`daemon_unavailable`, details `{kind}`)
without writing or contacting anything. The desktop maps that answer, and
`unsupported_kind`, to "exchange API connections need the Rust backend" and
disables Sync now. A connection the Rust backend created still lists in
Python, and `wallets sync` skips it as sourceless.

## Verification

- `crates/wallets/src/file_import/exchange_api/tests/desktop_api.rs`: each
  exchange's check over the signed loopback fakes (requests, read-only
  verdicts, Binance's refused flags including unknown ones, Bitpanda's
  trading-key refusal), consent, offline and missing-key refusals before any
  request, echoed credentials withheld, and a new connection's wallet plus
  sync leaving exactly the CLI sync's rows.
- `tests/test_rust_exchange_desktop.py` (needs `KASSIBER_RUST_BIN`): the
  daemon's create + sync against the CLI's `backends create` + `wallets
  sync-<kind>` for all four exchanges (both SQLCipher and plaintext for Kraken
  and Bitpanda): every table, the sync data and the request sequence are
  equal. It also pins that only the explicit test and sync contact the fake,
  that offline mode stops both, the Binance trading-key refusal, the key
  scrub in output, tables and status, and the argument refusals.
- `tests/test_python_rust_only_exchange_desktop.py`: Python's refusals.
- `ui-tauri/src/components/kb/exchange/*.dom.test.tsx`: the setup panel
  (guides, exact test arguments, retest after a change, explained errors,
  German), the dialog flow (no request before Test connection, create without
  sync unless opted in) and the wallet card.
- `crates/wallets/scripts/exchange_mutations.json`: the desktop entries, each
  killed by named tests.
