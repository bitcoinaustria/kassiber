# Bitstamp

Rust only. The signed API v2 (`wallets sync-bitstamp`, the user's own
read-only key) and the RFC 4180 transaction CSV (`wallets import-bitstamp`)
share one mapping (`file_import/exchange_api/bitstamp.rs`, CSV reader
`file_import/bitstamp.rs`) and one id, `bitstamp:<id>` with the id
zero-padded to 20 digits, so both in one wallet keep one record per
transaction. Transport rules are the [framework's](rust-exchange-sync.md).
Sources (2026-11-12): the official [API page](https://www.bitstamp.net/api/)
(its embedded OpenAPI document: authentication, `GetUserTransactions`,
`GetAccountBalancesForCurrency`, error codes, limits). Bitstamp does not
document the CSV export; its columns come from third-party parsers
([BittyTax](https://github.com/BittyTax/BittyTax/blob/master/src/bittytax/conv/parsers/bitstamp.py),
[raccoin](https://github.com/bjorn/raccoin/blob/main/src/bitstamp.rs)), and
[rotki](https://github.com/rotki/rotki/blob/develop/rotkehlchen/exchanges/bitstamp.py)
confirms the `since_id` cursor and the fee behaviour.

## Key setup

In Bitstamp: Settings → API access
(<https://www.bitstamp.net/settings/access-control/api/>) → new key. Turn on
only **Account balance** and **User transactions**; leave every order,
withdrawal, deposit and transfer permission off. Optionally restrict the key
to this computer's IP address, confirm with 2FA, copy the key and the secret
(shown once) and activate the key from Bitstamp's email. No read reveals a
key's other permissions, so Kassiber cannot refuse a key that could trade or
withdraw; it only calls the two read endpoints below. A key bound to a sub
account reads that sub account.

```bash
kassiber backends create bitstamp-main --kind bitstamp \
  --url https://www.bitstamp.net --token-stdin --auth-header-stdin
kassiber wallets sync-bitstamp --backend bitstamp-main
kassiber wallets import-bitstamp --file bitstamp-transactions.csv
```

## API sync

Signed POSTs only: `X-Auth: BITSTAMP <key>`, `X-Auth-Nonce` (a fresh random
UUID v4 per attempt, retries included), `X-Auth-Timestamp` (ms),
`X-Auth-Version: v2`, `X-Auth-Signature` = hex HMAC-SHA256(secret,
`BITSTAMP <key>` ‖ `POST` ‖ host ‖ path ‖ content type ‖ nonce ‖ timestamp ‖
`v2` ‖ body). A request without a body sends no `Content-Type` and signs none.

1. `/api/v2/user_transactions/` with `limit=1000&since_id=<n>&sort=asc`,
   from `since_id=1`, then one past the last id read, until a page holds
   fewer than 1,000 rows. A page whose ids do not start at the cursor and
   strictly rise fails closed.
2. `/api/v2/account_balances/btc/`: the envelope's `reported_balance`, scope
   `account_available_plus_reserved` (`total`, which must equal `available`
   + `reserved`, each whole msat by its own digits).

Responses are read from their exact JSON lexemes (integers as integers,
other numbers as decimal text, never through `f64`). A 400/401/403 or a 200
`{"status": "error"}` fails with `details.http_status` and `bitstamp_code`:
a documented code (`API0001`–`API0021`, `400.002`, `400.067`, `400.068`) or
`"unknown"`, never Bitstamp's text. Key codes are `auth_error` with a hint
(`API0002` IP, `API0003` permission, `API0004`/`API0017`/`API0018` clock,
`API0005` secret, `API0006` frozen account); the rate-limit codes are
`rate_limited` (retryable). The desktop key check reads the balance and one
transaction (`limit=1&offset=0&sort=asc`) and reports `read_only: null`.

Malformed pages fail `protocol_error` (not JSON, not a list, a row without
an id) or `validation` naming the row: no numeric type, a time that is not
`YYYY-MM-DD HH:MM:SS[.ffffff]`, a negative or non-decimal fee, or any
currency amount or pair rate, of any asset, that is null or not a plain
decimal. Times have no documented zone and are read as UTC.

## Mapping

The wallet's `asset_scope` ([other crypto assets](../plan/24-crypto-assets.md))
decides which assets are kept: BTC in `bitcoin` scope (the default), every
crypto asset in `crypto` scope. A transaction is read when it moves a kept
asset or charges a fee in one. Others are skipped: both sources report each
other asset a skipped transaction moves (an unknown CSV type too), counted
once per transaction, in the envelope's `assets` (`skipped` by ticker, with
the command that turns the `crypto` scope on in `next_step`) and their swaps
in `swap_pairs`; the sync also keeps `skipped.fiat_only_transactions` and
`skipped.non_btc_transactions`.

| Type | Record |
| --- | --- |
| 0 deposit (positive amount, no fee) | `deposit`, unpriced |
| 1 withdrawal (negative amount) | `withdrawal`, the `fee` on top in the same asset |
| 14 sub account, 35 inter account transfer | `deposit` or `withdrawal` by sign |
| 2 market trade, 36 simple buy: a kept asset against one fiat currency | `buy`/`sell`, exact `exchange_execution` pricing, `pricing_method` `bitstamp`: the fiat amount plus the fee for a buy, minus it for a sell |
| 2, 36: two crypto assets (`crypto` scope, e.g. `eth_btc`) | two unpriced `swap` legs, `bitstamp:<id>:out` and `bitstamp:<id>:in`, sharing `swap_ref` `bitstamp:<id>` |
| 32 referral reward (positive amount, no fee) | `income`, unpriced |

A trade's quote amount must equal its base amount × the `<base>_<quote>`
rate within one unit of each one's last written digit, so it excludes the
fee; a fee inside that tolerance fits both readings and is refused. The API
leaves the fee's currency implied (read as the trade's quote currency, the
asset a withdrawal moves); the withdrawal fee is assumed to be charged on
top (rotki's matching supports this; the reported balance lets a checkpoint
catch a different reading). A swap's quote leg carries the fee as a fiat
trade's cash does: a buy's outgoing leg pays it on top (amount and fee
apart), a sale's incoming leg is net of it. A fee the CSV states in a third
kept asset is its own fee-only row `bitstamp:<id>:fee`. Other assets are
stored to 11 decimals, never rounded.

Rows are stored oldest first by microsecond time, then id. A row a later
import adds into a second that already holds an order-sensitive row (the
other direction, or the same direction at another unit cost or fee per
unit) from an earlier import (either route) is tagged
`same-second-order-uncertain`,
as for [Bitvavo](bitvavo.md). Kassiber never reads `crypto-transactions`, so
records carry no txid; pairing with on-chain wallets uses amount and time.

Bitstamp rate-limits per key: two syncs running at once with the same key
(two books, or the desktop and the CLI) may fail `rate_limited`; sync again
once the other has finished.

## Refusals

One refused transaction stops the whole import, from either source,
naming the transaction id and type; record it through the generic ledger
with its fiat value. Refused: in `bitcoin` scope, BTC traded against an
altcoin or stablecoin (`eth_btc`, USDT, USDC…) naming the coin and the
scope command, or a fee in another crypto asset; a fee in another fiat
currency or a traded asset (CSV); a fee in a kept asset on a row without
one kept amount, or two kept assets outside a trade; staking, conversions,
settlements and derivatives (25–27, 33, 53, 55, 58–61); **every
undocumented type** (API codes outside the table, unknown CSV types); a fee
that is not stated (API `fee` absent or null) or charged without any
amount; a trade without a rate between its two assets, with both moving
the same way, off its rate, with a fee too small to place or exceeding its
proceeds, or with more than 28 written digits; a deposit or withdrawal
moving the wrong way, with a second currency, or with a fee Kassiber cannot
place; a referral that is not a plain credit; an unsigned CSV transfer of
a kept asset; a BTC amount or fee finer than a millisatoshi, or another
asset's finer than 11 decimals.
