# Exchange spec: Binance

- **Display name:** Binance
- **Slug:** binance
- **Spec status:** implemented, subset without spot trades; other crypto
  assets per wallet scope
- **Integration shape:** API import plus supplemental CSV

## Custodial model

Binance is a custodial exchange. Kassiber keeps Binance exchange/order evidence
in a Binance provider wallet and leaves later self-custody descriptor sync as
separate reconciliation evidence. The wallet's `asset_scope` decides which
assets: `bitcoin` (the default) reads and imports BTC and counts other coins
it sees; `crypto` reads and imports every coin
([plan 24](../plan/24-crypto-assets.md)).

## Implemented rows

- API fiat-payment (card) buys: exact `exchange_execution` pricing.
- API deposits/withdrawals: wallet movement, no exact execution pricing.
- API dividend/income rows: inbound income/mining, no exact execution
  pricing.
- `crypto` scope only: Convert orders, as priced buys and sells against fiat
  or as `swap` pairs between coins.
- Supplemental CSV autoinvest BTC rows funded by fiat: exact
  `exchange_execution` pricing.
- Supplemental CSV BTC dividend/mining rows: inbound income/mining, no exact
  execution pricing.
- Supplemental CSV rows of other coins: counted in `assets.skipped` in the
  `bitcoin` scope, imported the same way in the `crypto` scope. Refusals
  name the row.

The [imports reference](../reference/imports.md#binance-api-and-supplemental-csv)
lists the reads, their time windows and the mapping.

## Deferred

Spot trades: Binance serves them only per market (`myTrades?symbol=`) and
cannot list the markets an account traded, so a complete read needs the user
to name them. Also deferred: crypto-funded autoinvest, BNB dust conversions,
staking/locked-savings principal bookkeeping, and mining subaccounts that
require an extra username.
