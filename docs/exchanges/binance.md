# Exchange spec: Binance

- **Display name:** Binance
- **Slug:** binance
- **Spec status:** implemented, BTC-focused subset
- **Integration shape:** API import plus supplemental CSV

## Custodial model

Binance is a custodial exchange. Kassiber keeps Binance exchange/order evidence
in a Binance provider wallet and leaves later self-custody descriptor sync as
separate reconciliation evidence.

## Implemented rows

- API fiat-payment BTC buys: exact `exchange_execution` pricing.
- API BTC deposits/withdrawals: wallet movement, no exact execution pricing.
- API BTC dividend/income rows: inbound income/mining, no exact execution
  pricing.
- Supplemental CSV autoinvest BTC rows funded by fiat: exact
  `exchange_execution` pricing.
- Supplemental CSV BTC dividend/mining rows: inbound income/mining, no exact
  execution pricing.

## Deferred

Spot pair crawling, crypto-funded autoinvest/cross-asset trades, BNB dust
conversions, altcoin staking/locked-savings principal bookkeeping, and mining
subaccounts that require an extra username are deferred. They are altcoin-heavy
or need explicit provider controls before Kassiber can fail safe.
