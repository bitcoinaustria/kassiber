# Exchange spec: Kraken

- **Display name:** Kraken
- **Slug:** kraken
- **Spec status:** implemented, API import; other crypto assets per wallet
  scope
- **Integration shape:** API import

## Custodial model

Kraken is a custodial exchange. Kassiber imports ledger rows into a Kraken
provider wallet while self-custody wallet sync remains separate. The wallet's
`asset_scope` decides which assets: `bitcoin` (the default) imports BTC/LBTC
and counts every other asset it leaves out; `crypto` imports every crypto
asset ([plan 23](../plan/23-crypto-assets.md)).

## Implemented rows

- Private `Ledgers` deposit/withdrawal rows: wallet movement.
- Private `Ledgers` trade rows paired to `TradesHistory`: fiat-quoted buys and
  sells with exact `exchange_execution` pricing.
- `crypto` scope only: crypto-to-crypto trades as two `swap` rows sharing the
  trade's `refid`; Buy Crypto (`spend`/`receive`) as priced buys and sells;
  `KFEE` and other third-asset fee rows as `fee` rows; staking and earn
  rewards as `income`; moves between spot, staking and earn balances skipped.

The [imports reference](../reference/imports.md#kraken-api) lists the full
mapping and Kraken's asset codes (`XXBT`, `ZEUR`, staking suffixes).

## Safety decisions

Trade rows without matching trade history, a single crypto leg without a fiat
quote, unknown ledger types or subtypes, asset codes that cannot be mapped
exactly, swaps that also move fiat, and amounts finer than 11 decimals fail
validation and import nothing. Kraken Futures transfers, margin, adjustments
and other types need review through the generic ledger.
