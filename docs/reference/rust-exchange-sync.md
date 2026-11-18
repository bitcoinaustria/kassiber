# Rust exchange API sync

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

This page inventories Python's exchange API syncs and records how the Rust
backend serves them. The Python reference lives in
`kassiber/core/exchange_imports.py` (fetch and normalize) and
`kassiber/cli/handlers.py::import_exchange_api` (wallet and import
coordination). The Rust port is `crates/wallets/src/file_import/exchange_api.rs`
with one module per exchange. The user-facing behavior is described under
[DALI/RP2-inspired exchange imports](imports.md#dalirp2-inspired-exchange-imports).
Every exchange now follows the shared [exchange contract](exchange-contract.md).

## Where exchange sync runs

Python runs exchange API sync from three CLI commands only:
`wallets sync-kraken`, `wallets sync-coinbase` and `wallets sync-binance`.
Each command takes a required `--backend` and an optional `--wallet`. No daemon
kind, desktop action or AI tool calls an exchange API:

- The freshness planner has no exchange job type. An exchange API wallet
  (`kind` kraken/coinbase/binance, `config_json` `{"backend": name}`) yields no
  freshness source.
- `ui.wallets.sync` and the AI wallet-sync tool then take Python's direct
  `sync_wallet` path, which classifies the wallet as having no source and
  answers `{"status": "skipped", "reason": "no file source, descriptor, or
  backend addresses configured"}` without I/O.

The Rust backend keeps these boundaries. The three CLI commands run natively.
Three more, `wallets sync-bitpanda`, `wallets sync-bitvavo` and
`wallets sync-bitstamp`, exist only in Rust and follow the same framework;
[Bitpanda](bitpanda.md), [Bitvavo](bitvavo.md) and [Bitstamp](bitstamp.md)
record their APIs, mappings and refusals.
`ui.wallets.sync` and the AI tool now answer the direct path natively when
every selected wallet is sourceless (exchange API wallets and empty manual
wallets). They still return `native_fallback_required` for any other direct-path
selection, such as a paused source or a wallet with a file source.

The desktop sets up and syncs exchange connections through four Rust-only
kinds, `ui.connections.exchange.{status,test,create,sync}`, documented in
[exchange connections in the desktop](exchange-desktop.md). Their sync runs
the CLI command for the wallet's own backend, so its rows are the CLI's.
`ui.wallets.sync`, freshness and the assistant still leave exchange wallets
sourceless.

## Echoed credentials and shared helpers

Every exchange read goes through `HttpClient::authenticated_json` or
`authenticated_response` (`StatusPolicy::Exchange`), which keeps every
credential each request carried (key, secret, computed signature, URL and
proxy userinfo) at any length (`HttpClient::display_credentials`).

- A successful body is data and is never rewritten. If it carries a request
  credential of at least 16 characters, exactly or behind JSON escapes, the
  client notes it (`HttpClient::echoed`) and `execute` refuses the sync with
  `credential_echo` before anything is stored. Shorter values are ordinary
  data and never match.
- Error bodies (accepted 401/403 or raised) are display text: every request
  credential is withheld at any length on their decoded JSON strings, so an
  escape cannot hide one; numbers stay, so error codes survive.
- Adapters show a provider's error code only from their own table
  (`known_code`, `known_number`), never the provider's text.
- Every error a sync or desktop key check returns is withheld once more with
  every request credential (`scrub_error`); a numeric detail equal to one is
  dropped.

`exchange_api.rs` also holds the helpers the adapters share:
`exact_msat`/`btc_decimal` (whole msat by the amount's own digits, never
rounded), `strict_decimal`, `canonical`, `asset_code`, the `malformed` and
`refusal` error builders, and `tag_same_second` (tags rows whose order
within one second across imports is uncertain). See
[credential echoes](credential-echo.md).

## Request sequences

All three share these rules:

- The backend is resolved by name (`backends.resolve_backend`). The key comes
  from the first non-blank of `token`, `api_key` or `username`, and the secret
  from `auth_header`, `api_secret` or `password`. Both are stripped. The base
  URL is `url` or the provider default, without a trailing `/`. `timeout` is a
  decimal integer (default 30). Only `tor_proxy` routes the request. Chain
  trust overrides (`insecure`, `certificate`) and `batch_size` do not apply.
- The wallet is resolved, or created and committed, before credentials are
  checked or any request is made. A failed import therefore leaves a new empty
  provider wallet in both implementations.
- Each request goes through the shared host limiter (8 concurrent requests per
  host). 429 and 503 are retried up to three attempts, honoring `Retry-After`
  or exponential backoff with jitter, with at most 20 s of total wait. When
  retries are exhausted the error is `rate_limited` (retryable) with
  `details.retry_after_seconds`. Other non-2xx statuses fail with
  `HTTP <status> from backend for <scheme>://<redacted-host>: <redacted body
  prefix>`.
- Bodies are decoded as UTF-8 and parsed as JSON. Decode and parse failures
  are `internal_error` with Python's exception text.
- All requests happen first, then normalization, then a single import through
  the shared wallet record coordinator (`import_records_into_wallet`, source
  `<kind>:api:<backend>`, `report_updates=True`).

| Exchange | Requests | Paging | Signature |
| --- | --- | --- | --- |
| Kraken | `POST /0/private/TradesHistory`, then `POST /0/private/Ledgers`, form body `nonce=<ns>&ofs=<offset>` | `ofs` advances by the page's entry count until a page is empty or the merged row count reaches `result.count`; a non-integer `count` falls back to the row count | `API-Key`, `API-Sign` = base64 HMAC-SHA512(base64-decoded secret, path ‖ SHA-256(nonce ‖ body)) |
| Coinbase | `GET /v2/accounts`, then `GET /v2/accounts/<quoted id>/transactions` for each crypto account (BTC/LBTC and, in either asset scope, every other crypto asset; Python read only BTC accounts) | follows `pagination.next_uri` until absent | `CB-ACCESS-KEY`, `CB-ACCESS-SIGN` = hex HMAC-SHA256(secret, timestamp ‖ `GET` ‖ path), `CB-ACCESS-TIMESTAMP`, `CB-VERSION: 2017-11-27` |
| Binance | `GET /sapi/v1/fiat/payments` (`transactionType=0`) from `1498867200000` to now; `/sapi/v1/capital/deposit/hisrec` and `/sapi/v1/capital/withdraw/history` (`coin=BTC`), `/sapi/v1/asset/assetDividend` (`asset=BTC`), `/sapi/v1/convert/tradeFlow`, each in time windows from `1498867200000` to now; then `/api/v3/account` (Python sent one request each for the first four, and no Convert or balance read) | deposits and withdrawals by `offset`; dividends and converts by halving a full window ([imports](imports.md#binance-api-and-supplemental-csv)) | `X-MBX-APIKEY`, query `...&timestamp=<ms>&signature=` hex HMAC-SHA256(secret, query) |
| Bitvavo (Rust only) | `GET /v2/account/history?fromDate=0&toDate=<ms>&page=<n>&maxItems=100`, then `GET /v2/balance?symbol=BTC` for the reported balance | `page` from 1 to the first page's `totalPages`; a page that is not the one asked for, or a moving count, fails closed | `Bitvavo-Access-Key`, `Bitvavo-Access-Signature` = hex HMAC-SHA256(secret, timestamp ‖ `GET` ‖ path with `/v2` and query), `Bitvavo-Access-Timestamp` (ms), `Bitvavo-Access-Window: 10000`; re-signed on each retry |
| Bitstamp (Rust only) | `POST /api/v2/user_transactions/` with `limit=1000&since_id=<n>&sort=asc`, then `POST /api/v2/account_balances/btc/` for the reported balance | `since_id` from 1, then one past the last id, until a page holds fewer than 1,000 rows; ids out of cursor order fail closed | `X-Auth: BITSTAMP <key>`, `X-Auth-Signature` = hex HMAC-SHA256(secret, `BITSTAMP <key>` ‖ method ‖ host ‖ path ‖ content type ‖ nonce ‖ timestamp ‖ `v2` ‖ body), `X-Auth-Nonce` (random UUID per attempt), `X-Auth-Timestamp`, `X-Auth-Version: v2` |

Kraken API errors (`error` truthy) fail as `auth_error` when any entry mentions
`permission`, otherwise `protocol_error`, with `details.kraken_errors`.

The table is the `bitcoin` asset scope, the default and Python's behaviour.
A wallet whose `asset_scope` is `crypto` ([plan 24](../plan/24-crypto-assets.md))
changes the following, which Python never had:

- **Kraken** sends the same requests in both scopes and maps the ledger
  grouped by `refid`; the scope only filters which events are kept
  ([mapping](imports.md#kraken-api)). Its BTC rows keep the reading above.

In both scopes the envelope gains `assets` (`scope`, `skipped` and `excluded`
by ticker, and `next_step`) and `swap_pairs`. A Kraken sync sets `skipped` to
`{"fiat_only_entries", "other_asset_entries", "internal_moves"}` counts.

## Stored rows

Each exchange record is a Python import dict: `txid`, `occurred_at`,
`direction`, `asset` (BTC or LBTC only in `bitcoin` scope; any crypto asset,
stored exactly to 11 decimals, in `crypto` scope), absolute `amount` and `fee`, the ten
pricing fields (exact `exchange_execution` pricing for fiat-quoted trades and
Binance fiat buys, otherwise null), `kind`, `description`, `counterparty` and
`raw_json` (the provider row, sorted keys). Before a record is built, its
amount and fee in an asset other than Bitcoin are checked as the provider
gave them: finer than 11 decimals or too large to store refuses the sync,
naming the transaction, before any rounding. The coordinator writes the same
`transactions` rows, import runs, journal invalidation and update reports as
a JSON file import with those dicts. Re-syncs deduplicate by external id and
fingerprint. Changed pricing updates the existing row. Reviewed exclusions and
tags survive.

The Coinbase transaction `network` object is payment metadata on a `coinbase`
wallet only. Python drops it from network evidence for that wallet kind, and
the Rust import guard now does too. On any other wallet kind it fails the
network guard in both implementations.

## Credentials and privacy

- Credentials are read only from the resolved backend (the book's `backends`
  row, merged with seeded configuration). They are never logged and never
  appear in the output. The envelope reports `backend_url` through
  `redact_backend_url`.
- No request starts without a consent the CLI mints for this exact command.
  The grant binds the book identity, profile, wallet, exchange kind and the
  transport snapshot (URL, timeout, proxy). Every request and the publication
  savepoint recheck the gate, so offline mode or revocation stops the import
  without writing transactions.
- A server that echoes the API key or secret never gets it into an error.
  Both implementations withhold each literal credential, and its percent- and
  JSON-escaped spellings, as `<redacted>`: in `kraken_errors` (a Kraken error
  string is one item) and in every non-2xx body before it is cut to 200
  characters (`withhold_credentials`). Python orders equal-length spellings by
  set iteration; Rust breaks those ties by text.
- Deliberate divergences (fail closed):
  - Coinbase `next_uri` must continue the same collection on the same origin,
    with exactly the path that was signed. Python follows any `next_uri`.
  - Paging stops at 10,000 pages or 1,000,000 rows per command
    (`protocol_error`), and one command has an hour's budget. Python has no
    bound.
  - A non-positive `timeout` is refused before I/O.
  - Redirects are refused by the shared transport (`protocol_error`) rather
    than reported as `HTTP 302 from backend`. Neither implementation follows
    them.
  - Connection failures carry the shared transport's message, not urllib's
    `Failed to reach backend`, as for every Rust backend client.

## Shared seams this port corrected

- **Non-finite amounts.** Python keeps a `NaN` or `Infinity` amount as a
  Decimal until the row's fingerprint converts it to msat. That is after the
  network guard, and before the row's lookups and price-currency check. The
  shared Rust record coordinator raised the error while normalizing. It now
  defers the error to the same point, for file imports as well.
- **Coinbase network objects** (see stored rows). The shared import guard now
  receives the wallet kind.
- **Freshness book results** follow Python's key order (`profile`, `results`,
  `force_full`, ..., snapshot), with a wallet sync's journal step last.
- **`abs()` of a quiet NaN** keeps its sign in the exchange normalizers, as
  Decimal does. The River importer's shared helper still drops it. That path
  is unverified and is left as is.

## Python reference findings

Two reference bugs this port found are fixed in Python (round 8) and matched
here; `tests/rust_port/repro_exchange_reference.py` checks they stay fixed:

- Exchange error details repeated echoed credentials. Both now withhold them
  (above).
- A sell without a quoted price got a negative implicit `fiat_rate`
  (`fiat_value / signed amount`). The rate now divides a positive value by
  the absolute amount; a value that is not positive prices nothing (the row
  stays `missing_spot_price`). A Kraken trade without `price` takes cost /
  volume.

## Verification

- `tests/test_rust_exchange_api.py` compares Python and Rust over signed
  loopback fakes (`tests/rust_port/exchange_api.py`). It checks the exact
  envelope, the stable request sequence (path, cursor, parameters), and every
  table named by the schema manifest, on plaintext and SQLCipher books:
  - first import with a throttled first request, a re-sync over reviewed
    state, and a pricing update;
  - failure and edge pages: throttling and outage exhaustion, server errors,
    malformed and non-UTF-8 bodies, a BOM, Kraken error shapes, counts and
    paging, Coinbase and Binance shape errors, and missing, wrong and fallback
    credentials;
  - secrets that `binascii` decodes leniently;
  - hostile field values (non-finite, underscored, non-ASCII digits, booleans,
    containers, huge and out-of-range timestamps);
  - redirect refusal and the custom-wallet network refusal;
  - echoed credentials in Kraken error strings and lists and in non-2xx bodies,
    at the truncation cut and in escaped forms;
  - the registered CLI through `hybrid_cli.py`, served by Rust with zero
    fallbacks.
- `crates/wallets/tests/vectors/python_exchange_api.json` (from
  `crates/wallets/scripts/generate_exchange_api_vectors.py`) pins each
  normalizer against Python over 4,718 payloads. The Rust unit tests in
  `exchange_api/tests.rs` replay them and pin the codecs.
- `crates/wallets/scripts/exchange_mutations.json` lists the mutation entries.
  `exchange_mutation_check.py` checks that each one is killed by its named
  Rust tests.
- `tests/test_rust_freshness_product.py` covers native `ui.wallets.sync` for
  sourceless selections (exchange API and empty wallets, all and single wallet,
  with and without journals) and the remaining direct-path refusals.

## Old branches

Two interrupted workers started this port on an older base:

- `t3code/r5-exchange-sync-a` committed a Kraken port (feecccc85) and left a
  Coinbase draft. This port keeps their consent-binding design, the loopback
  fake and the CLI wiring. It replaces their normalizers, which worked on
  lossy JSON at precision 28 instead of the process context's 32 and skipped
  the coordinator's metadata bindings.
- `t3code/r5-exchange-sync-b` drafted a Binance importer (Python-value based)
  plus unrelated Lightning/probe edits. Its row semantics informed the Binance
  module. The Lightning, probe and chain edits are out of scope and were not
  taken.
