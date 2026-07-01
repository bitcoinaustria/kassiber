# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds an
opt-in real-node lane for proving wallet sync against disposable regtest
infrastructure.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses a Bitcoin Core regtest node, creates real wallets and transactions, then drives Kassiber sync, pricing, journal, report, and export. |

The slow lane is opt-in with `KASSIBER_INTEGRATION=1`; normal unit gates do not
start Docker. To reuse an existing regtest node instead of Compose:

```bash
export KASSIBER_REGTEST_REUSE_CORE=1
export KASSIBER_REGTEST_CORE_URL=http://127.0.0.1:18443
export KASSIBER_REGTEST_RPC_USER=kassiber
export KASSIBER_REGTEST_RPC_PASSWORD=...
./scripts/integration-harness.sh bitcoin-core
```

The Compose lane generates disposable RPC credentials per run unless you set
them explicitly. It uses regtest only, no mainnet funds, no user wallet files,
and no production descriptors. Set `KASSIBER_REGTEST_KEEP=1` to keep the Docker
volume for debugging; otherwise it is removed on exit.

## Guardrails

- `KASSIBER_NO_EGRESS=1` blocks non-loopback `socket.connect` calls inside fast
  harness tests so replay fixtures cannot accidentally reach live exchanges or
  public backends.
- Tapes must include provenance (`backend_kind`, network, regtest anchor, and
  issue number) and fail closed: an adapter request absent from the tape raises
  `TapeMiss`.
- Export assertions are content-level. XLSX files are inspected for expected
  sheets and self-verification content rather than byte-compared.
- Docker infrastructure is contributor test tooling only. It must not add an
  app-facing shell/filesystem escape hatch or relax desktop daemon allowlists.

## Growth Path

The current checked-in slow lane covers Bitcoin Core RPC. The harness is shaped
so Fulcrum/Electrum, explorer HTTP, Liquid, and optional BTCPay modules can add
new tapes and live tests without changing the contributor entrypoint.
