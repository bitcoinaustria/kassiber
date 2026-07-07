# Exchange spec: Kraken

- **Display name:** Kraken
- **Slug:** kraken
- **Spec status:** implemented, BTC-focused API subset
- **Integration shape:** API import

## Custodial model

Kraken is a custodial exchange. Kassiber imports BTC/LBTC ledger rows into a
Kraken provider wallet while self-custody wallet sync remains separate.

## Implemented rows

- Private `Ledgers` deposit/withdrawal rows for BTC/LBTC: wallet movement.
- Private `Ledgers` trade rows paired to `TradesHistory`: fiat-quoted BTC/LBTC
  buys/sells with exact `exchange_execution` pricing.

## Safety decisions

Kraken BTC/LBTC trade rows without matching trade history or without a fiat
quote fail validation. Cross-asset trades should be entered through the generic
ledger with explicit user review.
