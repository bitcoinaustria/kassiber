# Exchange spec: Ledger Live

- **Display name:** Ledger Live
- **Slug:** ledgerlive
- **Spec status:** implemented
- **Integration shape:** wallet movement CSV

## Custodial model

Ledger Live is non-custodial wallet software. Kassiber imports BTC/LBTC
`IN` and `OUT` operation-history rows as wallet movement, and other crypto
assets' rows when the wallet's `asset_scope` is `crypto`. This is reconciliation
evidence, not exchange/order evidence.

## Pricing

Ledger's export labels countervalues informational, so Kassiber does not import
them as accounting price evidence. Rows enter without `exchange_execution`
pricing and can be priced by the normal rates/review flow.

## Safety decisions

- Account xpub columns are redacted from stored `raw_json`.
- Other assets' rows are counted in `assets.skipped` in the `bitcoin` scope
  and imported as deposits and withdrawals of that asset in the `crypto`
  scope. A send of another asset with a fee is refused, naming the row: the
  export does not say which asset pays it (a token's fee is paid in its
  chain's coin). So is another asset's row with no amount but a fee (gas for
  a token send, a failed transaction, a contract call); a BTC row like it is
  skipped, as before.
- Unsupported operation types raise `AppError` instead of being guessed.
