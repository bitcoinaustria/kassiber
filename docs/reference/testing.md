# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses a Bitcoin Core regtest node, creates real wallets and transactions, then drives Kassiber sync, pricing, journal, report, and export. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: four Kassiber wallets, real regtest acquisitions/disposals/transfers, CoinJoin- and PayJoin-shaped collaborative transactions, loan marks, deterministic manual pricing, journals, reports, and transaction exports. |

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

## Full Accounting Demo

`demo-full` is the replacement substrate for fake one-click accounting data. It
does not inject synthetic transaction rows into SQLite. Instead, it creates real
Bitcoin Core regtest wallets, broadcasts real transactions, syncs Kassiber from
the Core RPC backend, then verifies Kassiber behavior through the public CLI:

- address-wallet creation and Bitcoin Core watch-only sync
- acquisition and disposal rows across Treasury, Cold Storage, Spending, and
  Merchant wallets
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
- deterministic manual BTC/EUR pricing, journal processing, summary reporting,
  PDF/CSV/XLSX report export, and CSV/XLSX transaction export

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

The current checked-in slow lanes cover Bitcoin Core RPC and a full accounting
demo on Bitcoin regtest. The harness is shaped so Fulcrum/Electrum, explorer
HTTP, Liquid, and optional BTCPay modules can add new tapes, live tests, and
scenario manifests without changing the contributor entrypoint.
