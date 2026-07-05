# Lightning Business Regtest Lane

Status: implemented as an opt-in live lane.

Run:

```bash
./scripts/integration-harness.sh lightning-business
```

The lane extends the existing `dev/regtest/compose.bitcoin.yml` stack with
`dev/regtest/compose.lightning.yml`. It keeps Bitcoin Core regtest as the
funding/mining source and adds four pinned Core Lightning containers plus one
LND backup node:

- `cln_merchant` — the operational Core Lightning merchant node Kassiber syncs
- `cln_customer` — pays merchant invoices and routed supplier invoices
- `cln_supplier` — receives merchant expense payments
- `cln_router` — creates/receives routed flow so forwarding fees are real
- `lnd_merchant_backup` — live LND backup node with funded private channels to
  `cln_merchant` and `cln_router`

The default image is `elementsproject/lightningd:v25.05`; override with
`KASSIBER_REGTEST_CLN_IMAGE` when intentionally testing a different CLN build.
The LND image defaults to `lightninglabs/lnd:v0.18.4-beta`; override with
`KASSIBER_REGTEST_LND_IMAGE`.

## Files

- `compose.lightning.yml` adds the CLN/LND services and named volumes.
- `lightning-business-plan.py` generates the seeded business workload that the
  shell scenario executes. It is inspired by sim-ln's capacity-multiplier and
  defined-activity model, but remains deterministic and assertion-friendly.
- `lightning-business-bootstrap.sh` idempotently funds nodes, connects peers,
  opens channels, mines confirmations, and waits for `CHANNELD_NORMAL`.
- `lightning-business-scenario.sh` creates deterministic business activity:
  customer-paid merchant invoices, merchant-paid supplier invoices routed via
  the router, third-party routed payments crossing the merchant, one
  intentionally expired merchant quote, one intentionally failed oversized
  payment, LND backup receipts/outbound payments, and real Bitcoin regtest
  mainchain top-ups/withdrawals around the merchant CLN wallet.
- `lightning-cli-merchant.sh` is the only `lightning-cli` wrapper stored in
  the Kassiber book. It always executes against `cln_merchant`.
- `tests/integration/lightning_business_regtest.py` builds/syncs the Kassiber
  book, drives the daemon snapshot surface, exports profitability CSV, and
  asserts the SQLite/report output.

## Topology

```text
cln_customer -- cln_merchant -- cln_router -- cln_supplier
                   |             |
             lnd_merchant_backup
```

Channels are opened with balanced push amounts so both directions can pay on
the first run. The LND backup edges are private/unannounced; the CLN path
remains the public routing topology. Re-running the bootstrap reuses existing channels and only
funds nodes whose on-chain wallet balance falls below the configured threshold.
The scenario's business plan lives at
`${KASSIBER_LIGHTNING_BUSINESS_PLAN:-$KASSIBER_LIGHTNING_BUSINESS_HOME/business-plan.json}`.
Set `KASSIBER_REGTEST_LIGHTNING_SEED` for stable alternative traffic and
`KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER` to scale the plan against the
configured channel capacity. `KASSIBER_REGTEST_LIGHTNING_EXPECTED_PAYMENT_MSAT`
and `KASSIBER_REGTEST_LIGHTNING_CHANNEL_CAPACITY_SAT` also affect the generated
plan hash.

## Kassiber Invariants

The live assertion module verifies:

- Kassiber has two Lightning wallet/backend rows: `cln_merchant` (`coreln`) and
  `lnd_merchant_backup` (`lnd`).
- Customer, supplier, and router never become Kassiber wallets or connections.
- `wallets sync --wallet cln_merchant` imports merchant invoice rows through
  the CLN sync path and persists aggregate Lightning node records.
- `ui.connections.node.snapshot` returns CLN merchant alias/pubkey, balances,
  on-chain balance, channels, invoice/payment counts, expired/failed cases,
  routing summary, and forward rows, and returns an LND backup snapshot with
  the CLN-LND and router-LND channels plus LND invoice/payment activity.
- `reports lightning-profitability` and
  `reports export-lightning-profitability-csv` include liquidity, routing
  revenue, payment costs, forwarding counts, and open-cost coverage fields.
- Persisted Lightning records keep `raw_json = '{}'`.
- AI-safe snapshot/profitability payloads omit peer pubkeys, funding outpoints,
  short channel ids, route-hop identifiers, invoice secrets, preimages, bolt11
  strings, and failure-source nodes.

## Keep/Reuse

By default the lane removes Docker volumes and the throwaway book on success or
failure. Set `KASSIBER_REGTEST_KEEP=1` to preserve the full Compose project
(containers, ports, and volumes) plus the throwaway home for inspection.

Set `KASSIBER_REGTEST_LIGHTNING_REUSE=1` to reuse an already-running compose
project. The bootstrap and scenario scripts are safe to run repeatedly when the
business plan hash is unchanged. Changing the seed, multiplier, expected
payment, or channel capacity while reusing preserved state requires a fresh
`KASSIBER_LIGHTNING_BUSINESS_HOME` or manual cleanup of the preserved state and
volumes. The Kassiber assertion book is rebuilt by default; set
`KASSIBER_LIGHTNING_BUSINESS_REUSE_BOOK=1` only when intentionally debugging an
existing preserved book.

Useful paths/commands while preserved:

```bash
KASSIBER_REGTEST_KEEP=1 ./scripts/integration-harness.sh lightning-business
./dev/regtest/lightning-cli-merchant.sh getinfo
uv run python -m tests.integration.lightning_business_regtest
```

The assertion module writes its book under
`${KASSIBER_LIGHTNING_BUSINESS_HOME:-/tmp/kassiber-lightning-business-<project>}`.
