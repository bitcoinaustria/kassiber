# Exchange spec: Binance

- **Display name:** Binance
- **Slug:** binance
- **Spec status:** implemented: BTC through the API, the complete ledger
  through the Transaction History CSV
- **Integration shape:** BTC-only API sync, Transaction History CSV (every
  asset, through the [exchange contract](../reference/exchange-contract.md)),
  supplemental CSV

## Custodial model

Binance is a custodial exchange. Kassiber keeps Binance exchange/order evidence
in a Binance provider wallet and leaves later self-custody descriptor sync as
separate reconciliation evidence.

## Implemented rows

- API fiat-payment BTC buys: exact `exchange_execution` pricing.
- API BTC deposits/withdrawals (settled statuses only, per direction):
  wallet movement, no exact execution pricing.
- API BTC dividend/income rows: inbound income/mining, no exact execution
  pricing.
- API Convert orders between BTC and fiat: a buy or sell priced at the fiat
  side; between BTC and another coin: refused (the API never books the other
  coin). The API reads no spot trades; each sync says so and compares the
  spot BTC balance.
- Transaction History CSV (Orders → Transaction History → Export, all
  statements): Binance's complete ledger, spot trades included; trades per
  second and account, deposits, withdrawals, rewards, and matched moves
  between Binance wallets, filtered by the wallet's asset scope. Never in the
  same wallet as the API sync or the supplemental CSV, since the export has
  no ids. See
  [imports](../reference/imports.md#binance-transaction-history-csv).
- Supplemental CSV autoinvest BTC rows funded by fiat: exact
  `exchange_execution` pricing.
- Supplemental CSV BTC dividend/mining rows: inbound income/mining, no exact
  execution pricing.
- Supplemental CSV rows of other coins: counted in `assets.skipped` in the
  `bitcoin` scope, imported the same way in the `crypto` scope. Refusals
  name the row.

## Deferred

Spot trades through the API (pair crawling cannot be proven complete; the
Transaction History CSV covers them), altcoin staking/locked-savings principal
bookkeeping, and mining subaccounts that require an extra username are
deferred. They are altcoin-heavy
or need explicit provider controls before Kassiber can fail safe.
