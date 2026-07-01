# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses a Bitcoin Core regtest node, creates real wallets and transactions, then drives Kassiber sync, pricing, journal, report, and export. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: ten Kassiber wallets including Bitcoin rotation targets and deterministic Liquid/LBTC import wallets, real regtest acquisitions/disposals/transfers, operating-expense disposals, deprecated rotated-out wallets, batched and consolidation edge cases, a larger multi-year stress ledger, CoinJoin- and PayJoin-shaped collaborative transactions, swap/peg bridge pairs, loan marks, volatile deterministic manual pricing, journals, reports, and transaction exports. |

The slow lane is opt-in with `KASSIBER_INTEGRATION=1`; normal unit gates do not
start Docker. To reuse an existing regtest node instead of Compose, set an
explicit Core URL and matching disposable RPC credentials:

```bash
export KASSIBER_REGTEST_CORE_URL=http://127.0.0.1:18443
export KASSIBER_REGTEST_RPC_USER=kassiber
export KASSIBER_REGTEST_RPC_PASSWORD=...
./scripts/integration-harness.sh bitcoin-core
```

The Compose lane generates disposable RPC credentials per run unless you set
them explicitly, passes only the `rpcauth` hash to bitcoind, publishes RPC on
host loopback, and uses a per-worktree Compose project name so parallel runs do
not share containers or volumes. It uses regtest only, no mainnet funds, no user
wallet files, and no production descriptors. Set `KASSIBER_REGTEST_KEEP=1` to
keep the Docker volume for debugging; otherwise it is removed on exit.
Fresh Compose runs use the scenario manifest's historical timestamp sequence,
starting in January 2019 and covering activity into spring 2026. Reused Core
nodes can only move forward from their existing regtest chain tip, so their
calendar dates may drift while preserving the same relative spacing and row
shape.

## Full Accounting Demo

`demo-full` is the replacement substrate for fake one-click accounting data. It
does not inject synthetic transaction rows into SQLite. Instead, it creates real
Bitcoin Core regtest wallets, broadcasts real transactions, syncs Kassiber from
the Core RPC backend, then verifies Kassiber behavior through the public CLI:

- address-wallet creation and Bitcoin Core watch-only sync
- file-source Liquid wallet creation and generic-ledger LBTC import through the
  same `wallets sync --all` path
- acquisition and disposal rows across Treasury, Cold Storage, Spending, and
  Merchant wallets, plus empty Bitcoin and Liquid rotation-target wallets that
  become active after security upgrades
- large single-source custody receipt into cold storage
- batched treasury payout to multiple external recipients in one transaction
- same-block merchant point-of-sale receipt burst followed by a many-input
  consolidation that imports as a fee-only wallet row
- a deterministic historical stress lane: 132 cycles spaced 20 days apart, with
  batched inbound funding into operational wallets, rotating outbound payments,
  and regular fiat-expense disposals for payroll, rent, software, tax prep,
  contractors, and equipment; the demo still adds several hundred synced/imported
  wallet rows across seven years
- wallet key-rotation events for treasury, merchant, cold storage, and Liquid
  treasury, reviewed as same-asset transfer pairs after sync; the old source
  wallets are then marked deprecated so their history remains visible while
  refresh-all/background freshness skips them
- Liquid/on-chain-style bridge events (`peg-in`, `submarine-swap`, `peg-out`)
  pair real Bitcoin Core txids with deterministic LBTC ledger external IDs so
  the generic-tax demo exercises taxable cross-asset swaps
- reviewed same-asset transfer pairs for wallet-to-wallet movements
- CoinJoin-shaped PSBT flow with two owned inputs, equal external/tracked
  outputs, and explicit watched change; the resulting rows are explicitly
  review-excluded before tax reporting because the generic tax engine correctly
  treats unresolved owned fanout as unsafe to classify automatically
- PayJoin-shaped PSBT flow with payer and merchant inputs in the same
  transaction; those rows are likewise review-excluded after sync so the demo
  book remains reportable without pretending to know provider-specific intent
- Bitcoin-backed-loan marks for collateral lock/release and BTC principal
  receive/repay, linked under one loan id
- deterministic but volatile manual BTC/EUR and LBTC/EUR pricing, journal
  processing, summary reporting, PDF/CSV/XLSX report export, and CSV/XLSX
  transaction export

The scenario manifest lives at
`dev/regtest/scenarios/full_accounting.json`; the runner lives at
`tests/integration/regtest_demo.py`. The command prints a JSON summary with the
generated `data_root` and `export_dir`, for example:

```bash
./scripts/integration-harness.sh demo-full
```

Open that generated data root from the desktop import/storage flow when you want
the GUI to inspect the same real demo book. Browser mock mode remains useful for
component fixtures, but it should not be treated as an accounting or sync proof.

## Guardrails

- `KASSIBER_NO_EGRESS=1` blocks non-loopback `socket.connect` calls inside fast
  harness tests so replay fixtures cannot accidentally reach live exchanges or
  public backends.
- Tapes must include provenance (`backend_kind`, network, regtest anchor, and
  issue number) and fail closed: an adapter request absent from the tape raises
  `TapeMiss`, while unused recorded interactions fail the replay test.
- Export assertions are content-level. XLSX files are inspected for expected
  sheets and self-verification content rather than byte-compared.
- Docker infrastructure is contributor test tooling only. It must not add an
  app-facing shell/filesystem escape hatch or relax desktop daemon allowlists.
- The demo runner prints regtest addresses and txids, but never prints RPC
  passwords. CLI backend credentials are passed through file descriptors.

## Growth Path

The current checked-in slow lanes cover Bitcoin Core RPC, deterministic
file-source Liquid/LBTC demo wallets, and a full accounting demo on Bitcoin
regtest. The harness is shaped so Fulcrum/Electrum, explorer HTTP, live Liquid,
and optional BTCPay modules can add new tapes, live tests, and scenario manifests
without changing the contributor entrypoint.
