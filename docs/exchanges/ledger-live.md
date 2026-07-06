# Exchange spec: Ledger Live

- **Display name:** Ledger Live
- **Slug:** ledgerlive
- **Spec status:** implemented
- **Integration shape:** wallet movement CSV

## Custodial model

Ledger Live is non-custodial wallet software. Kassiber imports only BTC/LBTC
`IN` and `OUT` operation-history rows as wallet movement. This is reconciliation
evidence, not exchange/order evidence.

## Pricing

Ledger's export labels countervalues informational, so Kassiber does not import
them as accounting price evidence. Rows enter without `exchange_execution`
pricing and can be priced by the normal rates/review flow.

## Safety decisions

- Account xpub columns are redacted from stored `raw_json`.
- Non-BTC/LBTC assets are skipped.
- Unsupported operation types raise `AppError` instead of being guessed.
