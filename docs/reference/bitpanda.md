# Bitpanda

Kassiber reads Bitpanda history through Bitpanda's public REST API, read-only.
This page records the API as documented, how each event maps to Kassiber
records, what is skipped or refused and why, and how to set up a key. The
importer is `crates/wallets/src/file_import/exchange_api/bitpanda.rs`, on the
shared [exchange API framework](rust-exchange-sync.md). The Python backend
never had a Bitpanda importer.

## Sources

Retrieved 2026-11-08. Bitpanda's owner link (`share.google/7ypaq8z8GJRQYFZLp`)
resolves to the developer docs at <https://docs.public.bitpanda.com/>; the
pages used:

| Page | URL |
| --- | --- |
| Overview | <https://docs.public.bitpanda.com/overview> |
| API key generation (scopes) | <https://docs.public.bitpanda.com/api-key-generation> |
| Migrating from the legacy API | <https://docs.public.bitpanda.com/migrating-from-the-legacy-api> |
| API and MCP rate limits | <https://docs.public.bitpanda.com/api-and-mcp-rate-limits> |
| List operations (`GET /v1/operations`) | <https://docs.public.bitpanda.com/list-operations-4375770e0> |
| Operation schema | <https://docs.public.bitpanda.com/operationresponse-877253d0> |
| List assets (`GET /v1/assets`) | <https://docs.public.bitpanda.com/list-available-assets-4375772e0> |
| List currencies (`GET /v1/currencies`) | <https://docs.public.bitpanda.com/list-available-currencies-4375771e0> |
| Trading limits (`GET /v1/trading-limits`) | <https://docs.public.bitpanda.com/get-current-trading-limits-and-remaining-budget-4461012e0> |
| MCP tools (scope per endpoint) | <https://docs.public.bitpanda.com/mcp-tools> |
| Machine index | <https://docs.public.bitpanda.com/llms.txt> |

Bitpanda Pro, now One Trading, is a separate exchange with a different API
and is not covered here.

## The API

- **Base URL** `https://api.public.bitpanda.com`, paths under `/v1`. The
  legacy API (`https://api.bitpanda.com/v1`: `/wallets`, `/fiatwallets`,
  `/asset-wallets`, `/trades`, `/wallets/transactions`,
  `/fiatwallets/transactions`, `/assets/transactions/commodity`) is deprecated
  and is decommissioned on 31 January 2027. Kassiber does not use it. Its
  history endpoints are all replaced by `GET /v1/operations`.
- **Authentication**: the API key in the `x-api-key` header. No secret, no
  request signature. Keys are created under Account settings → API Keys
  (<https://app.bitpanda.com/my-account/apikey>), are shown once, expire after
  at most one year, and can be restricted to an IP allow list.
- **Scopes**: `Trade (Read)` (market data, assets, currencies), `Trade
  (Write)` (create and accept quotes, set trading limits), `Earn (Read)`,
  `Earn (Write)` (stake, unstake), `Balances` (portfolio), `Transaction`
  (operation history). The MCP page names the scope each endpoint needs:
  `/v1/assets` and `/v1/currencies` need `trading` (Trade Read),
  `/v1/operations` needs `transaction`, `/v1/trading-limits` needs
  `trading-write`.
- **Rate limits**: per user and group, in one-second windows. The `read`
  group (assets, currencies, portfolio, operations, tickers, earn configs)
  allows 50 requests per second; `trading_write` (quotes, trading limits)
  and `earn_write` allow 5. An exceeded limit answers `429 Too Many
  Requests` with `Retry-After` (seconds), `X-RateLimit-Limit`,
  `X-RateLimit-Remaining` and `X-RateLimit-Category`.
- **Paging**: cursor based. A page is `{"data": [...], "self_cursor",
  "next_cursor", "has_next_page"}`; the next page is requested with
  `cursor=<next_cursor>`. `page_size` defaults to 25 and allows 1 to 100.
  `/v1/currencies` is a single unpaged `{"data": [...]}`.
- **Errors**: documented as `{"error": {"code": "..."}}` with 400 (invalid
  query), 401 ("missing or invalid API key, or API key lacks the ... scope"),
  404 and 500.
- **Time**: `credited_at` is an ISO 8601 `date-time`; the examples use UTC
  (`...Z`).
- **Amounts**: decimal strings in `{"value", "asset_id", "currency_id"}`
  objects. Assets and fiat currencies are UUIDs; `/v1/assets` and
  `/v1/currencies` resolve them to symbols.

### `GET /v1/operations`

Query: `cursor`, `page_size`, `from`, `to` (date-time), `asset_id[]`,
`currency_id[]` (UUIDs). Each operation is `{operation_id, operation_type,
transactions: [...]}`; each transaction is:

| Field | Meaning (as documented, or as Kassiber reads it) |
| --- | --- |
| `transaction_id` | UUID; Kassiber's external id is `bitpanda:<transaction_id>` |
| `asset_id`, `currency_id` | the asset or fiat currency moved |
| `wallet_id`, `wallet_owner` | the Bitpanda wallet |
| `asset_amount` | the amount moved (`value` string with its asset or currency id) |
| `fee_amount` | a fee on this transaction, same shape |
| `transaction_type` | undocumented string |
| `flow` | `INCOMING` or `OUTGOING` |
| `order_id` | |
| `credited_at` | date-time |
| `asset_balance_after` | the wallet's balance after this transaction |
| `compensates`, `compensates_info` | the transaction this one reverses, and its types |
| `index_asset_id` | the crypto index a leg belongs to |
| `trade` | `{trade_id, fee, fee_percentage, rate, rate_with_fee, to_eur_rate}` for trade legs |

**What the docs do not say.** The values of `operation_type` and
`transaction_type` are not documented anywhere (every schema page shows
`string`); third-party integrations report the same gap. Nor do the docs say
whether `asset_amount` includes `fee_amount`, or how `trade.fee` relates to
the cash leg. Kassiber therefore maps by structure where the structure
decides, uses Bitpanda's export vocabulary (`deposit`, `withdrawal`,
`reward`) only where a type name is needed, and refuses everything else
(below). Real responses will show whether the vocabulary matches; until then
a refusal names the operation and its types so the user can report them.

## Request sequence

One `wallets sync-bitpanda` command, after the wallet is resolved and the
consent for this exact route is minted, sends only these GETs, each with
`Accept: application/json` and `x-api-key`:

1. `/v1/trading-limits`: the write-scope check. 200 and its documented 404
   ("no trading limits configured for this API key") both mean the key holds
   `Trade (Write)`: the sync refuses with `auth_error` before reading any
   history. 401 and 403 mean it does not. Any other status fails.
2. `/v1/assets?symbol=BTC&page_size=100` (paged): BTC's asset id. Several
   assets may share the ticker (an ETF or stock), so several matches narrow to
   the one whose `type` is `crypto` or `group` is `cryptocoin`; anything but
   exactly one id fails `protocol_error`.
3. `/v1/currencies`: fiat currency ids to codes.
4. `/v1/operations?page_size=100[&cursor=...]` until `has_next_page` is
   false, over the whole history (no `from`/`to`).

The shared transport rechecks the gate for each request, retries 429 and 503
up to three attempts within 20 s (honouring `Retry-After`) and then fails
`rate_limited` (retryable). A 401 or 403 on a read fails `auth_error`
("Bitpanda refused the API key (HTTP 401)") with `details.http_status` and the
server's `error.code` (credentials withheld, at most 100 characters). Other
non-2xx statuses fail with the shared transport's message, whose body prefix
is withheld before it is cut. Malformed pages fail closed (`protocol_error`):
a body that is not JSON or not an object, `data` that is not a list,
`has_next_page` that is not a boolean, a next page without a cursor, a
repeated cursor, an operation without an id, or the same operation twice with
different contents. Paging stops at the framework's bounds (10,000 pages,
1,000,000 rows, one hour).

Kassiber never calls the quote, trading-limit-setting or earn-action
endpoints, never sends anything but GET, and never sends a body.

## Event mapping

Only operations that touch BTC (a leg, a transaction's asset or a fee in BTC)
are read further; within them every transaction must parse strictly: an id,
a plain non-negative decimal amount (`"0.01"`, no sign, exponent, NaN or
number type), a BTC amount that is positive and whole msat (at most 11
decimals), a flow of `INCOMING`/`OUTGOING`, a fee that is absent or a plain
decimal, and a `credited_at` with an explicit offset (stored as UTC, whole
seconds). One refused operation stops the whole sync; nothing is imported.

| Bitpanda event | Recognised by | Kassiber record |
| --- | --- | --- |
| BTC buy (including savings-plan buys) | one BTC leg with `trade`, `INCOMING`, against one fiat leg `OUTGOING` | `buy`; amount = BTC leg; exact `exchange_execution` pricing: `fiat_value` = the cash leg (what was paid), `fiat_currency` = its currency, `pricing_method = "bitpanda_api"`, `pricing_external_ref` = `trade_id` (else the operation id) |
| BTC sell | as above with the flows reversed | `sell`; `fiat_value` = the cash leg (what was received) |
| BTC deposit | one BTC leg, no trade, no cash, `operation_type` `deposit`, `INCOMING`, no fee | `deposit` (wallet movement, unpriced) |
| BTC withdrawal | one BTC leg, no trade, no cash, `operation_type` `withdrawal`, `OUTGOING` | `withdrawal`; a BTC fee is placed by the wallet's balances (below) |
| BTC reward (cashback, bonus, referral, staking) | one BTC leg, `operation_type` `reward`, `INCOMING`, no fee | `income`, unpriced (`journals process` holds it as `missing_spot_price` until priced) |
| Move between the user's own Bitpanda wallets | two or more BTC legs, no trade, no cash, no fees, incoming total = outgoing total | skipped (`internal_btc_moves`): the account's BTC does not change |
| Fiat deposit, withdrawal, card payment | every leg in a fiat currency | skipped (`fiat_only_operations`) |
| Other crypto, BEST, metals, stocks, ETFs, ETCs, indices without BTC | no BTC anywhere | skipped (`non_btc_operations`): Kassiber's exchange syncs import BTC only, like Kraken, Coinbase and Binance |

Type names compare case-insensitively after trimming. A buy or sell is
recognised by its shape, whatever its `operation_type` (so savings plans and
other buy flavours need no name), unless the type says `deposit`,
`withdrawal` or `reward`, which contradicts trade details.

**Withdrawal fees.** Whether a withdrawal's `asset_amount` includes its BTC
`fee_amount` is not documented. Kassiber reads the wallet's
`asset_balance_after` from its previous BTC transaction (the latest earlier
`credited_at` in the same `wallet_id`, from the full history): `before -
amount - fee = after` stores the amount and the fee as given; `before -
amount = after` stores `amount - fee` sent plus the fee. With no earlier
transaction, a tie in time, a missing balance, or neither equation holding,
the withdrawal is refused.

**Trade fees.** Bitpanda quotes include the fee (`rate_with_fee`), so the
cash leg is the exact amount paid or received; that follows Kassiber's rule
that fiat fees raise a buy's cost and lower a sell's proceeds. `trade.fee`
stays in the raw evidence. A separate `fee_amount` on either leg (in cash, in
BTC, or in BEST) leaves it unknown whether the cash leg already includes it,
so the trade is refused.

### Refused (validation error, nothing imported)

Each names the operation and its types; the hint asks the user to record it
through the generic ledger after checking it in Bitpanda.

- any other `operation_type` on a lone BTC leg (staking moves, transfers to
  other users if typed otherwise, airdrops, unknown or missing types), or a
  type whose flow contradicts it;
- a `transaction_type` naming another event (`deposit`, `withdrawal`,
  `reward`, `buy`, `sell`) than the operation's movement type;
- reversals: any leg with `compensates`;
- BTC against another asset: crypto swaps, crypto index trades (several
  asset legs), BEST-paid fees, BTC that appears only as a fee;
- a trade that is not one BTC leg against one fiat leg, both legs flowing the
  same way, a cash leg of zero, or in a currency Kassiber does not price;
- a trade with any separate `fee_amount`;
- BTC moving against cash without trade details;
- several BTC legs that do not balance;
- a deposit or reward with a fee, a withdrawal fee in another asset, or a
  withdrawal BTC fee the balances cannot place;
- amounts that are negative, non-decimal, zero BTC or finer than a msat;
  times without an offset; a transaction's asset disagreeing with its
  amount's; a transaction without an id; an operation without a transaction
  list.

### Stored rows

Records go through the shared wallet coordinator exactly as the other
exchange syncs' do (source `bitpanda:api:<backend>`): deduplication by
external id and fingerprint, pricing updates, journal invalidation, import
runs. `raw_json` is `{operation_id, operation_type, transaction}` plus
`cash_transaction` for a trade, sorted. The envelope adds the framework's
`backend`, `backend_kind`, `backend_url` (redacted), `wallet`, `fetched` and
`input_format: "bitpanda_api"`, and `skipped`:
`{"fiat_only_operations", "non_btc_operations", "internal_btc_moves"}` counts.

## Setup

1. In Bitpanda, open Account settings → API Keys → New API key
   (<https://app.bitpanda.com/my-account/apikey>). Grant only
   **Transaction** and **Trade (Read)**. Do not grant Trade (Write), Earn
   (Write) or Balances: Kassiber does not need them, and it refuses a key
   that can trade. Optionally restrict the key to your IP address.
2. Store the key in a backend (the key goes to `token`; no secret is needed):

   ```bash
   kassiber backends create bitpanda-main --kind bitpanda \
     --url https://api.public.bitpanda.com --token-stdin
   ```
3. Sync:

   ```bash
   kassiber wallets sync-bitpanda --workspace W --profile P --backend bitpanda-main
   ```

   The command creates (or reuses) a wallet labelled `Bitpanda` of kind
   `bitpanda`, or imports into `--wallet`.

The write-scope check catches `Trade (Write)` only: no read endpoint reveals
`Earn (Write)`, so leaving that scope off is up to the user. Kassiber never
calls an Earn or trading endpoint either way.

## Privacy and consent

The sync is an explicit user command: no request starts before the CLI
mints a consent for this exact route (book, profile, wallet, kind, URL,
timeout, proxy), every request and the publication recheck the gate, and
offline mode stops it without writing. The key lives in the backend secret
boundary, is never logged or printed, and is withheld (with its percent- and
JSON-escaped spellings) from every error: the 401/403 `bitpanda_error`, the
shared transport's non-2xx body prefix, and server text quoted in a refusal.
Saved Bitpanda backends are configuration, not consent: launch, status,
desktop reads and freshness jobs never contact Bitpanda (exchange API wallets
stay sourceless for `wallets sync` and `ui.wallets.sync`). Only the desktop's
explicit Test connection and Sync now do, under their own consent.

## Desktop and assistant

The desktop sets up and syncs Bitpanda like Kraken, Coinbase and Binance:
a key guide, Test connection (the write-scope check, currencies and one
operations row), Create connection and Sync now; see
[exchange connections in the desktop](exchange-desktop.md). No exchange sync
is exposed to the assistant, so neither is Bitpanda: no assistant tool calls
the Bitpanda API, and `ui.wallets.request_input` keeps the source list of the
AI tool catalog (`crates/ai/src/data/catalog.json`, generated from Python's
catalog and now maintained by hand). Adding either is a separate
consent and product decision.

## CSV export

Not imported. Bitpanda's transaction-history CSV ("Transaction ID",
"Timestamp", "Transaction Type", "In/Out", "Amount Fiat", "Fiat", "Amount
Asset", "Asset", "Asset market price", ..., "Fee", "Fee asset", "Spread",
...) is not documented by Bitpanda (the support article refuses automated
reads), and third-party parsers disagree on how `Fee` relates to the amounts
(for example whether a deposit's amount is net of its fee). Importing it
would mean guessing fee semantics; a sample export with known balances is
needed first. Until then a Bitpanda CSV can go through the generic ledger
import with explicit columns.

## Verification

- `crates/wallets/src/file_import/exchange_api/tests/bitpanda_api.rs`: a
  loopback fake built from the documented shapes: every mapped event, the
  request sequence and escaping, idempotent re-sync, both withdrawal-fee
  readings, the write-scope refusal, auth errors and echoed keys, throttling
  and its exhaustion, malformed pages, the BTC asset lookup, each refusal
  above, consent, offline and missing-key refusals before any request.
- The recorded `test_rust_exchange_desktop` sessions
  (`scripts/replay-sessions.sh`) replay the desktop's Bitpanda setup.
- `crates/wallets/scripts/exchange_mutations.json`: the Bitpanda mutation
  entries, each killed by named tests (the `wallets-exchange` suite of
  `cargo run -p kassiber-mutation-check`).

The Python suites that drove the binary (`test_rust_bitpanda_api.py`: every
schema table on plaintext and SQLCipher books; `test_rust_bitpanda_cli.py`:
no egress from saving the backend or reading the book, offline mode, echoed
keys) were removed with Python under [plan 22](../plan/22-python-removal.md).
