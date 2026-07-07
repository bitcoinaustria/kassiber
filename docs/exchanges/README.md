# Exchange specs

One spec per exchange/broker/custodial platform Kassiber can ingest. Each spec
is the intake artifact captured before an importer is built, and the durable
record of how that provider's rows are interpreted.

- **Template:** [TEMPLATE.md](TEMPLATE.md)
- **Worked example:** [strike.md](strike.md) — the shipped Strike importer
  written up as a filled spec (illustrative reference; the code is the source of
  truth).
- **Implemented specs:** [binance.md](binance.md),
  [coinbase.md](coinbase.md), [kraken.md](kraken.md), and
  [ledger-live.md](ledger-live.md).
- **How to add one:** copy [TEMPLATE.md](TEMPLATE.md) to
  `docs/exchanges/<slug>.md`, fill the intake sections, and use the closest
  implemented importer as the code reference.
- **Implemented importers** and their CLI/behavior live in
  [docs/reference/imports.md](../reference/imports.md).

Keep specs free of secrets and personal data. Sample exports under
`samples/<slug>/` must be scrubbed of account numbers, names, and balances, or
kept out of the repo entirely.
