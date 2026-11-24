# Exchange spec: Kraken

- **Display name:** Kraken
- **Slug:** kraken
- **Spec status:** implemented, API import; other crypto assets per wallet
  scope
- **Integration shape:** API import

## Custodial model

Kraken is a custodial exchange. Kassiber imports ledger rows into a Kraken
provider wallet while self-custody wallet sync remains separate. The wallet's
`asset_scope` filters the same events: `bitcoin` (the default) imports every
event that moves BTC/LBTC and counts every other asset it leaves out; `crypto`
imports every crypto asset ([plan 24](../plan/24-crypto-assets.md)).

## Implemented rows

- Private `Ledgers` deposit/withdrawal rows: wallet movement.
- Private `Ledgers` trade rows paired to `TradesHistory`: fiat-quoted buys and
  sells with exact `exchange_execution` pricing.
- Buy Crypto (`spend`/`receive`) as priced buys and sells; staking and earn
  rewards (BTC in `XBT.M` too) as `income`; moves between spot, staking and
  earn balances skipped.
- `crypto` scope only: crypto-to-crypto trades as two `swap` rows sharing the
  trade's `refid`; `KFEE` and other third-asset fee rows as `fee` rows.
  An incoming crypto leg whose ledger fee is in the same asset is stored net
  of that fee (100 DOT in, 0.1 DOT fee: 99.9 DOT); BTC rows are unchanged.

The [imports reference](../reference/imports.md#kraken-api) lists the full
mapping and Kraken's asset codes (`XXBT`, `ZEUR`, staking suffixes).

## Safety decisions

Trade rows without matching trade history, a single crypto leg without a fiat
quote, unknown ledger types or subtypes, asset codes that cannot be mapped
exactly, swaps that also move fiat, and amounts finer than 11 decimals fail
validation and import nothing. In the `bitcoin` scope a BTC trade against
another crypto asset is refused with the command that sets the `crypto`
scope, which imports both legs. Kraken Futures transfers, margin,
adjustments and other types need review: record them yourself with the
generic CSV/JSON import (a swap as two rows sharing a `swap_ref`), since the
generic ledger refuses another asset's leg.
