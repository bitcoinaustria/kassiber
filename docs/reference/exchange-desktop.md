# Exchange API connections in the desktop

The desktop connects Kraken, Coinbase, Binance and Bitpanda through a
read-only API key and syncs their history on request. Each setup and sync runs
the [exchange API framework](rust-exchange-sync.md) the CLI's
`wallets sync-<kind>` commands use ([Bitpanda](bitpanda.md) has its own
page).

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
   into it. When that wallet is the same exchange's CSV wallet without a key,
   the refusal names it (`attachable_wallet_id`, `attachable_wallet`) and the
   dialog offers to add the key to it instead (`wallet` argument): the wallet
   gains `backend` in its config and becomes a hybrid source whose API keeps
   recent activity current while its CSV runs cover older history
   ([sources](desktop.md#sources-live-sync-and-file-snapshots)). Rows
   deduplicate within that one wallet by external ID and fingerprint, so CSV
   and API rows only match where both carry the same transaction ID scheme. The key lives in the book's `backends` row, inside SQLCipher when
   the book is encrypted, and no kind returns it.
6. **Sync now** (`ui.connections.exchange.sync`), either right after creation
   when the user ticks **Import history right after adding the connection**
   (off by default) or from the wallet page. It runs `wallets sync-<kind>
   --backend <the wallet's backend> --wallet <wallet>`.

## Wallet page

An exchange wallet's page shows an exchange card instead of the chain refresh:
whether its key is stored, its last sync (time, new and read rows, skipped
Bitpanda operations) or its error, and **Sync now**. The header
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

Every sync, CLI or desktop, stores its outcome on the wallet it ran for, also
when no `--wallet` named it (`config_json.exchange_sync`: `status`, `at`, the
counts, and for a failure only its error class: `auth`, `rate_limited`,
`offline`, `network`, `protocol`, `validation`, `config`, or `other`). Provider
text (messages, hints, response bodies) is never stored: the status read
reports a stored failure as the class's error code and Kassiber's own
explanation. Older records that held text are rewritten to their class when
the book opens, with `secure_delete` on, and the WAL is checkpointed and
truncated; until that truncate completes (a reader can hold old pages) the
cleanup stays pending and later opens retry it
([compatibility](database-compatibility.md)). A
success also stores `last_synced_at`. `ui.connections.exchange.status` reads it, so it survives a
restart, and source coverage and completeness use it. API syncs record no
import run ([imports](imports.md#import-runs-and-rolling-one-back)).

## Daemon kinds

All four are Rust-only daemon kinds, in the Tauri and browser-bridge
allowlists, classified like their BTCPay counterparts (`status` read, the
rest operator), and refuse unknown arguments before any work.

| Kind | Arguments | Network | Answer |
| --- | --- | --- | --- |
| `ui.connections.exchange.status` | `wallet?` | none | `connections`: per exchange wallet of the active profile, `wallet_id`, `wallet`, `exchange`, `provider`, `backend`, `backend_configured`, `has_api_key`, `has_api_secret` (null for Bitpanda) and `last_sync` (`status`, `at`, `imported`, `fetched`, `skipped`, or the error envelope's `error`) |
| `ui.connections.exchange.test` | `backend`, or `exchange`, `api_key`, `api_secret`, `url?` | the check above, once | `exchange`, `ok`, `read_only` (true when the exchange confirmed it, null when it does not say) |
| `ui.connections.exchange.create` | `exchange`, `api_key`, `api_secret` (not for Bitpanda), `label?`, `url?`, `wallet?` (attach to that exchange's CSV wallet) | none | `exchange`, `attached`, `wallet` (`id`, `label`, `kind`), `backend` (the redacted backend row) |
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
  neither reaches the reply, the RAM logs, the stored last-sync record (which
  keeps only an error class) or
  the book. Request arguments named `api_key` and `api_secret` are sensitive
  to the shared redaction.
- `ui.wallets.sync`, `ui.freshness.run` and background freshness still treat
  exchange wallets as sourceless, and no assistant or MCP tool calls these
  kinds: exchange syncs stay out of the assistant's surface, as before. Only
  the user's click on Sync now, or on the title-bar refresh (Cmd/Ctrl+R),
  contacts the exchange; the title-bar refresh runs `ui.connections.exchange.sync`
  once per exchange wallet with a key after the book refresh. Each of those
  requests carries the originating book's `expected_scope`, and the sequence
  stops when the book or daemon session changes. Another title-bar refresh while the
  sequence still lists or syncs leaves it running rather than starting over, and
  opening an exchange form does not end it. Each of those syncs shows like Sync
  now in the notification list (a running connection sync, then a result, or a
  failed-sync condition on that connection) and on an open wallet's exchange
  card, whose status is re-read. Startup, timers and menu-driven book refreshes
  do not.

## Verification

- `crates/wallets/src/file_import/exchange_api/tests/desktop_api.rs`: each
  exchange's check over the signed loopback fakes (requests, read-only
  verdicts, Binance's refused flags including unknown ones, Bitpanda's
  trading-key refusal), consent, offline and missing-key refusals before any
  request, echoed credentials withheld, and a new connection's wallet plus
  sync leaving exactly the CLI sync's rows.
- The recorded `test_rust_exchange_desktop` sessions
  (`scripts/replay-sessions.sh --only test_rust_exchange_desktop`): the
  daemon's create + sync against the CLI's `backends create` + `wallets
  sync-<kind>` for all four exchanges (both SQLCipher and plaintext for Kraken
  and Bitpanda): every table, the sync data and the request sequence are
  equal. It also pins that only the explicit test and sync contact the fake,
  that offline mode stops both, the Binance trading-key refusal, the key
  scrub in output, tables and status, and the argument refusals.
- `ui-tauri/src/components/kb/exchange/*.dom.test.tsx`: the setup panel
  (guides, exact test arguments, retest after a change, explained errors,
  German), the dialog flow (no request before Test connection, create without
  sync unless opted in) and the wallet card.
- `crates/wallets/scripts/exchange_mutations.json`: the desktop entries, each
  killed by named tests.
