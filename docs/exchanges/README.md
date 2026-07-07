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
- **How to add one:** run the intake interview in
  [skills/kassiber/references/add-exchange.md](../../skills/kassiber/references/add-exchange.md)
  (or invoke the `/add-exchange` command), which fills a copy of the template at
  `docs/exchanges/<slug>.md`.
- **Implemented importers** and their CLI/behavior live in
  [docs/reference/imports.md](../reference/imports.md).

Keep specs free of secrets and personal data. Sample exports under
`samples/<slug>/` must be scrubbed of account numbers, names, and balances, or
kept out of the repo entirely.
