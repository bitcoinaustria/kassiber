# Bitvavo

Rust only. The signed REST API (`wallets sync-bitvavo`, the user's own
View-only key) and the transaction-history CSV (`wallets import-bitvavo`)
share one mapping (`file_import/exchange_api/bitvavo.rs`, CSV reader
`file_import/bitvavo.rs`) and one id, `bitvavo:<transactionId>`, so both in
one wallet keep one record per transaction. Transport rules are the
[framework's](rust-exchange-sync.md). Sources (2026-11-12): [get started](https://docs.bitvavo.com/docs/get-started/),
[authentication](https://docs.bitvavo.com/docs/rest-api/introduction/),
[`GET /account/history`](https://docs.bitvavo.com/docs/rest-api/get-transaction-history/),
[`GET /balance`](https://docs.bitvavo.com/docs/rest-api/get-account-balance/),
[errors](https://docs.bitvavo.com/docs/errors/),
[CSV export](https://support.bitvavo.com/hc/en-us/articles/44936625205137-How-can-I-download-my-tax-report). The CSV columns are
undocumented; public sample exports pin the layout.

## Key setup

In Bitvavo: Settings → API → **Add new API key**
(<https://account.bitvavo.com/user/api>), optionally whitelist this
computer's IP address, and tick only **View access** (leave Trade digital
assets, Withdraw digital assets, Internal Transfer, Administrative and
Include all subaccounts off). Confirm with 2FA and copy the key and secret,
which Bitvavo shows once. No endpoint reveals a key's
permissions, so Kassiber cannot refuse a key that could trade or withdraw;
it never sends a write request. Store the key in `token` and the secret in
`auth_header`:

```bash
kassiber backends create bitvavo-main --kind bitvavo \
  --url https://api.bitvavo.com --token-stdin --auth-header-stdin
kassiber wallets sync-bitvavo --backend bitvavo-main
kassiber wallets import-bitvavo --file bitvavo-transactions.csv
```

## API sync

Signed GETs only (`Bitvavo-Access-Key`, `-Timestamp` in ms, `-Window: 10000`,
`-Signature` = hex HMAC-SHA256 of timestamp + `GET` + path with the base
URL's own path, `/v2` and the query; re-signed on every retry):

1. `/v2/account/history?fromDate=0&toDate=<now>&page=<n>&maxItems=100` for
   `n = 1 … totalPages`. Each page must be the one asked for, hold at most
   100 rows and repeat the first page's `totalPages`; otherwise
   `protocol_error`. A repeated transaction must be identical.
2. `/v2/balance?symbol=BTC`: the envelope's `reported_balance`, scope
   `spot_available_plus_in_order` (`available` + `inOrder`, each whole msat
   by its own digits). Staking and other products are not in it.

Amounts are read only from JSON strings (a number-typed amount is refused).
A 401/403 fails `auth_error` with `details.http_status` and
`bitvavo_error_code`: a [documented](https://docs.bitvavo.com/docs/errors/)
number (101–112, 200–240, 300–317, 400–430) or `"unknown"`, never Bitvavo's
text; 311 (no View access), 307 (IP whitelist) and 305/306 (inactive key)
get their own hint. The desktop key check reads the balance and one history
row and reports `read_only: null`.

## CSV export

Transaction history → Export. Comma-separated UTF-8, optional BOM, header
`Timezone,Date,Time,Type,Currency,Amount,Quote Currency,Quote Price,Received / Paid Currency,Received / Paid Amount,Fee currency,Fee amount,Status,Transaction ID,Address`
(all but `Address` required). An active custodial ledger, imported in
`full` mode like Strike.

- `Date` `YYYY-MM-DD` and `Time` `HH:MM:SS[.fff]` are wall time in
  `Timezone`: `Europe/Amsterdam` or `UTC`. Other zones, and wall times the
  summer-time change skips or repeats, are refused.
- `Amount` is signed (negative when the row's currency leaves), `Received /
  Paid Amount` is the trade's quote side, after the fee, rounded to cents.
  Exponents (`1e-05`) are read exactly.
- A withdrawal's `Amount` includes the fee; Kassiber stores `|Amount| − fee`
  (in integer msat) as sent and the fee apart, as the API states them.
- `Status` `Completed`/`Distributed` import, `Pending`/`Canceled` rows are
  left out, any other status refuses the file.

## Mapping

The wallet's `asset_scope` ([other crypto assets](../plan/24-crypto-assets.md))
decides which transactions are read. Both sources report what the scope
left out in the envelope's `assets` (`skipped` by ticker, with the command
that turns the `crypto` scope on in `next_step`) and their swaps in
`swap_pairs`; the sync also keeps `skipped.fiat_only_transactions` and
`skipped.non_btc_transactions` (transactions the scope left out).

- `bitcoin` scope (the default): transactions with BTC on a side or as the
  fee are read; one without BTC is skipped, each other asset it moves
  counted once. A BTC trade against another crypto asset, a Convert with
  BTC, or a BTC trade whose fee is in another crypto asset is refused, the
  hint naming the scope command.
- `crypto` scope: every transaction with a crypto asset is read. Fiat-only
  transactions (EUR deposits, EUR affiliate payouts) are skipped in both.

| Type | Record |
| --- | --- |
| `buy`, `sell` against fiat | `buy`/`sell` of the asset, exact `exchange_execution` pricing: cost including the fiat fee, proceeds net of it |
| `buy`, `sell` against a crypto asset (`crypto` scope) | two unpriced `swap` legs, `bitvavo:<id>:out` and `bitvavo:<id>:in`, sharing `swap_ref` `bitvavo:<id>` |
| `convert_buy`, `convert_sell` (`crypto` scope) | the same two `swap` legs |
| `deposit` | `deposit`, unpriced wallet movement (no txid in either source) |
| `withdrawal` | `withdrawal`: amount that left on chain, fee apart in the same asset |
| `staking`, `fixed_staking` | `staking`, unpriced |
| `affiliate` | `income`, unpriced |

A fee in a third crypto asset (`crypto` scope) is its own fee-only row
`bitvavo:<id>:fee`. A swap's fee in a traded asset is placed only from the
CSV export of a trade, which states the quote side after the fee: the paid
amount includes it (the `out` leg stores amount and fee apart), the
received amount is net of it. Other assets are stored to 11 decimals,
never rounded ([units](../plan/24-crypto-assets.md#decisions)).

Fiat trade values are checked against amount × price (one cent tolerance). The
CSV states the quote amount after the fee. The API does not document it:
the single reading that fits is used, and when both fit (a fee under two
cents) the trade imports unpriced ("fee treatment unclear"), so `journals
process` holds it until the user prices it. Rows are stored oldest first by
their millisecond time. A row an import adds into a second that holds an
opposite-direction row, from an earlier import or the same one, is tagged
`same-second-order-uncertain` for review with it, since stored times are whole
seconds and the journals cannot tell the milliseconds apart.

## Refusals

One refused transaction stops the whole import, from either source, naming
the transaction id and type; record it through the generic ledger with its
fiat value. Refused: in `bitcoin` scope, BTC traded against a stablecoin or
altcoin (USDC, EURC, ETH…), a BTC fee on an altcoin trade and a Convert
with BTC; margin and loan rows; **every type the table does not list**
(`rebate`, `distribution`, `withdrawal_cancelled`, transfers, corrections,
unknown types); an asset code that is not a plain ticker; a fiat trade's fee
in its traded asset or another fiat currency; a swap's fee in a traded asset
from the API or in a Convert (neither says whether the amounts include it;
record a Convert yourself); a Convert without two crypto assets; a
trade without a price or whose amounts fit no reading; a movement with a
second asset or a fee Kassiber cannot place; a BTC amount finer than a
millisatoshi or another asset's finer than 11 decimals; a withdrawal whose
fee is as large as its amount.

The sync reads the whole history on every run, so an API trade between two
crypto assets with a fee in a traded asset refuses every sync of that wallet:
such a wallet cannot be synced. Import the transaction-history CSV export
into it instead, which states the fee, and import a newer export for later
history.
