# Exchange spec: Coinbase

- **Display name:** Coinbase
- **Slug:** coinbase
- **Spec status:** implemented, v2 accounts and transactions; Bitcoin by
  default, other crypto assets with the wallet's `crypto` asset scope
  ([plan 24](../plan/24-crypto-assets.md))
- **Integration shape:** API import

## Custodial model

Coinbase is a custodial exchange/wallet. Kassiber imports account rows into a
Coinbase provider wallet and keeps self-custody sync separate.

## Accounts and asset scope

`GET /v2/accounts` names each account's currency and Coinbase's own type for
it. Kassiber classifies an account from that type: `crypto` (BTC and LBTC are
always Bitcoin), `fiat`, or, for any other type, excluded (never imported). A
currency without a type falls back to its code.

Every sync reads every crypto account's transactions, in either scope; fiat
accounts never are, and an excluded account is counted once in
`assets.excluded` without reading it. The scope is a filter over the rows
([exchange contract](../reference/exchange-contract.md)):

- **`bitcoin` scope (default):** BTC rows are imported, and every other
  crypto asset's rows are counted in `assets.skipped`. An account listed
  without a currency is skipped.
- **`crypto` scope:** every crypto asset's rows are imported. An account
  listed without a currency is refused.

An account in another crypto asset without an id is refused in either scope.

## Implemented rows

- `buy`, `sell`, `trade` and `advanced_trade_fill` against fiat: exact
  `exchange_execution` pricing from Coinbase's fiat `native_amount`; a buy's
  `commission` is added to its cost.
- Crypto-to-crypto trades (`crypto` scope): two `swap` legs with a shared
  `swap_ref`, each valued at its fiat `native_amount` when usable, paired by
  the import.
  - A convert (`trade`) pairs its legs by Coinbase's `trade.id`. A convert
    with one leg among the crypto accounts has its other leg in a fiat
    account and stays a buy or sell.
  - An Advanced Trade fill on a crypto-quoted product (`advanced_trade_fill.product_id`,
    such as `ETH-BTC`) pairs by `order_id`. An order with several fills
    pairs each fill's legs by their shared time and `fill_price`.
  - The two amounts are what each account moved. A convert's spread and a
    fill's commission (in the quote asset) are inside them.
- `send`, `receive` and exchange transfer rows: deposits and withdrawals
  without pricing. A send with a `network.hash` keys on that hash.
- `interest` (kind `interest`), `staking_reward` and `inflation_reward` (kind
  `staking`): inbound income without pricing.
- A buy, sell or convert whose `fee` Coinbase states in a crypto asset that
  is neither of the trade's assets: its own outbound `fee` row of that asset
  (`crypto` scope), once per trade.

## Safety decisions

- Unknown row types (`earn_payout`, `staking_transfer`, `tx`, and the
  others Coinbase documents) are refused, naming the type and row.
- Rows with fiat amounts below one cent are rejected for exact trade import.
- A crypto-to-crypto trade must have exactly one leg out and one leg in, in
  two different assets; a convert needs its `trade.id` and a fill its
  `product_id` and `order_id`.
- A `receive` with a negative amount is refused.
- Amounts of assets other than Bitcoin finer than 11 decimals, or too large
  to store, are refused, naming the transaction, never rounded.
- In the `bitcoin` scope a BTC trade whose `fee` is in another crypto asset
  is refused rather than dropped.
- In the `bitcoin` scope the BTC leg of a convert (`trade`) or of a fill on a
  crypto-quoted product is refused, naming the row, the other asset and the
  command that sets the `crypto` scope. Booked alone, the BTC leg of a swap
  would be a fiat-priced sale or purchase (in an Austrian book, a realized
  gain on a tax-neutral swap). The legs are paired in either scope, from
  the BTC rows and the rows that share their trade or order, so another
  coin's trades never stop a Bitcoin wallet; a convert whose other leg is in
  a fiat account stays a buy or sell in both.

## Known gaps

- **Authentication.** Kassiber signs with Coinbase's legacy HMAC API keys.
  Coinbase expired those in February 2025 and accepts only CDP API keys,
  which sign a short-lived ES256 or Ed25519 JWT per request. Until Kassiber
  supports CDP keys, a sync against Coinbase's API fails authentication.
- A send's `network.transaction_fee` is not booked.
- A wallet synced before the `bitcoin` scope refused converts may hold a
  convert's BTC leg as a `sell`. Turning on the `crypto` scope pairs that
  stored row with the new swap leg; the stored row keeps its kind.
