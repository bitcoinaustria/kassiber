# Testing

Kassiber's tests are split into three layers that correspond to what each one
actually catches:

1. **Fixture layer** (every PR, no infra): CLI-driven fake-wallet demos plus
   engine-level snapshot regressions. Fast, deterministic, zero containers.
2. **Live regtest layer** (opt-in, nightly in CI): session-scoped Bitcoin Core
   and Liquid (Elements + electrs-liquid) regtest stacks that exercise the
   real sync protocol paths end-to-end.
3. **Manual compatibility checks** (ad-hoc, developer-run): pointing Kassiber
   at a self-hosted signet / non-mainnet instance to verify external wallet
   interop. Not automated — public signet/testnet Electrum servers leak
   watched scripts to whoever runs them, so that path stays off the
   automated gate.

All three layers stay on loopback-only infrastructure. The automated layers
never contact a public chain or indexer.

## Baseline Gate (layer 1)

Run the baseline gate before pushing code or docs:

```bash
./scripts/quality-gate.sh
```

The gate compiles the Python modules, runs the CLI smoke / regression /
fake-wallet / Boltz-swap suites, the sync-backend unit suites, and imports
the live-sync test modules with live tests disabled. It does not contact any
external service.

Two fake-wallet demos drive the fixture layer:

- `scripts/seed-fake-wallets.sh` seeds a BTC + LBTC workspace with a
  self-transfer, a Liquid federation peg-in, and a matching peg-out. Use this
  when you want a deterministic accounting demo or UI dataset.
- `scripts/seed-boltz-swaps.sh` seeds a BTC + LBTC workspace with a full
  Boltz chain-swap round trip (forward BTC -> LBTC and reverse LBTC -> BTC)
  with the service-fee spread baked into the amounts and both legs paired
  with `--kind chain-swap --policy taxable`. Use this when you need to
  exercise the cross-asset pairing path without running live chains.

Both scripts create a fresh `--data-root` by default and emit the final
`reports summary` envelope to stdout on success.

## Live Regtest Layer (layer 2)

Layer 2 is opt-in. Enable it with `KASSIBER_LIVE_SYNC_TESTS=1` and run:

```bash
scripts/live-sync-tests.sh                     # both Bitcoin and Liquid
scripts/live-sync-tests.sh --suite bitcoin     # Bitcoin only
scripts/live-sync-tests.sh --suite liquid      # Liquid only
```

Each suite brings up its own Docker containers, shares them across the
module's test methods via `setUpModule` / `tearDownModule`, and gives every
test a fresh Kassiber `--data-root`. Session-scoped startup keeps a full
live-sync run to roughly one Docker pull + one daemon start per chain, not
per test.

### Privacy posture (important)

- All Docker port binds use `127.0.0.1::<container-port>` so the daemons are
  reachable only over loopback on the host.
- `rpcallowip` is scoped to the Docker bridge range
  (`172.16.0.0/12`) rather than `0.0.0.0/0`.
- RPC credentials are randomized per session (not hardcoded).
- Descriptor material is generated inside the regtest container and written
  to files under the test's temporary data root.
- The Liquid test resolves the elementsregtest policy asset id from the
  running daemon at runtime instead of hardcoding one that only matches a
  specific genesis config.

### Bitcoin regtest

`tests/test_live_sync_bitcoin.py` drives:

1. Start one `bitcoin/bitcoin:28.1` container on an ephemeral loopback port.
2. Create a miner wallet, mine 101 blocks, mint a watch address.
3. `kassiber wallets create --kind address --backend bitcoinrpc`, then
   `kassiber wallets sync`.
4. Assert the synced transaction matches what we broadcast, then sync again
   and assert idempotency (`imported=0`, `skipped=1`).

Override the image when you want a different Core version:

```bash
KASSIBER_BITCOIND_IMAGE=bitcoin/bitcoin:27.1 \
  scripts/live-sync-tests.sh --suite bitcoin --pull-images
```

### Liquid regtest

`tests/test_live_sync_liquid.py` drives a two-container stack on a dedicated
Docker bridge network:

1. `ghcr.io/vulpemventures/elements` running `-chain=elementsregtest`.
2. `ghcr.io/vulpemventures/electrs-liquid` providing an Electrum TCP
   endpoint on ephemeral loopback.

The test creates a descriptor wallet inside elementsd, exports the
`ct(slip77(...), elwpkh(...))` external + internal descriptors (splitting
the unified `<0;1>` form when present, with fresh `getdescriptorinfo`
checksums), writes them to files, and hands them to
`kassiber wallets create --kind descriptor`. It then funds the first
derived address, mines one block, and asserts Kassiber's Liquid Electrum
sync unblinds the amount and reports the transaction as `LBTC`.

Override the images when you want different builds:

```bash
KASSIBER_ELEMENTSD_IMAGE=ghcr.io/vulpemventures/elements:23.2.1 \
KASSIBER_ELECTRS_LIQUID_IMAGE=ghcr.io/vulpemventures/electrs-liquid:latest \
  scripts/live-sync-tests.sh --suite liquid --pull-images
```

Extra daemon args can be appended with
`KASSIBER_ELEMENTSD_EXTRA_ARGS` and `KASSIBER_ELECTRS_LIQUID_EXTRA_ARGS`.

### Skip vs. fail

By default, "Docker unreachable" and "image not present" are reported as
skipped tests. Turn them into hard failures when you want to prove the path
really ran:

```bash
scripts/live-sync-tests.sh --suite bitcoin --pull-images --require-bitcoin-regtest
scripts/live-sync-tests.sh --suite liquid  --pull-images --require-liquid-regtest
```

### Log capture on failure

The live test base classes dump the recent `docker logs` output from every
container that was part of the stack when a test fails. That output goes to
stderr along with the test id so a CI failure is debuggable without
re-running.

## CI Topology

- **`ci` workflow (fast lane)**: runs `scripts/quality-gate.sh` on every
  pull request and on pushes to `main`. Layer 1 only.
- **`live-sync` workflow (slow lane)**: daily at 05:00 UTC on `main`, plus
  manual dispatch with a suite selector. Layer 2 only. Each chain runs as
  its own job so Bitcoin and Liquid failures stay independent.

The daily cadence keeps layer 2 out of the per-PR loop (Docker pulls alone
are a few minutes per job) while still catching regressions within a day.
Drop to weekly later if the signal-to-noise ratio holds.

## Chain-Swap Demo (Boltz)

Boltz offers BTC <-> LBTC **chain swaps** (onchain atomic swaps between the
two chains) in addition to their Lightning submarine swaps. From Kassiber's
point of view, a chain swap is two independent transactions on two different
chains with a direction inversion plus the Boltz service-fee spread. There
is nothing Boltz-specific for the sync path to learn, so testing focuses on
the accounting pipeline:

```bash
scripts/seed-boltz-swaps.sh --data-root /tmp/kassiber-boltz/data
python3 -m kassiber --data-root /tmp/kassiber-boltz/data journals transfers list
```

The fixture pairs each leg with `--kind chain-swap --policy taxable`, which
is the correct posture for generic (non-Austrian) profiles today — Austrian
profiles can use `--policy carrying-value` to carry basis across the swap.

## Regtest, Signet, And Privacy

Regtest is the default for automated live tests because it is local,
deterministic, fast, and does not reveal wallet structure or timing to
public infrastructure.

Signet and mutinynet both look tempting for compatibility checks, but both
leak watched scripts to any Electrum/Esplora server you query that you do
not run yourself. If you need a signet run, point Kassiber at infrastructure
you control and keep the descriptor and backend credentials local.
