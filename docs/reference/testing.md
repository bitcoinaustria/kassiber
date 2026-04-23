# Testing

Kassiber has two testing layers:

- the normal quality gate, which is deterministic and does not need live chain
  services
- opt-in live sync tests, which use generated wallets and local regtest
  backends to exercise real wallet discovery and sync

## Baseline Gate

Run the baseline gate before pushing code or docs:

```bash
./scripts/quality-gate.sh
```

The gate compiles the Python modules, runs the CLI smoke/regression suites,
runs the sync backend unit suites, and imports the live-sync test module with
live tests disabled. It should not contact public Bitcoin, Liquid, OpenAI, or
other external services.

## Live Sync Tests

Prerequisites for the Bitcoin regtest live sync path:

- Docker installed
- Docker Desktop or the Docker daemon running
- the `docker` CLI reachable from the shell that runs the test script
- permission to pull `bitcoin/bitcoin:28.1`, or that image already present
  locally

Live sync tests are skipped unless explicitly enabled:

```bash
scripts/live-sync-tests.sh
```

The on-chain test starts a local Bitcoin Core regtest node in Docker, creates a
real Core wallet, mines private regtest blocks, sends funds to a generated
address, creates a Kassiber address wallet, and syncs through the real
`bitcoinrpc` backend. It then runs a second sync to prove idempotency.

By default the script will not pull Docker images. Pre-pull the image yourself
or allow a pull:

```bash
docker pull bitcoin/bitcoin:28.1
scripts/live-sync-tests.sh

# or
scripts/live-sync-tests.sh --pull-images
```

You can override the image:

```bash
KASSIBER_BITCOIND_IMAGE=bitcoin/bitcoin:28.1 scripts/live-sync-tests.sh
```

The generated wallet material, RPC credentials, blocks, and SQLite data live in
temporary local directories. The test talks to `127.0.0.1` only.

Docker must be reachable by the process that runs the script. If Docker is
installed but not running, or if a sandboxed tool cannot see the Docker daemon
socket, the live test is reported as skipped. To prove the Bitcoin regtest path
really ran, require it:

```bash
scripts/live-sync-tests.sh --pull-images --require-bitcoin-regtest
```

The same required Bitcoin regtest path is also available as a manual GitHub
Actions workflow named `live-sync`. Trigger it from the GitHub Actions tab when
you want CI to pull the Bitcoin Core image and prove the Docker-backed sync path
works without adding that cost to every PR run.

## Fake Wallet Demo

For deterministic manual and UI testing without chain services, seed a local
demo project:

```bash
scripts/seed-fake-wallets.sh
```

The script creates a fresh data root by default, imports fixture CSV files from
`tests/fixtures/fake_wallets/`, pairs one BTC -> LBTC peg-in and one LBTC -> BTC
peg-out, tags and annotates the review-relevant records, processes journals,
and emits the final `reports summary` machine envelope to stdout. The fixture
contains:

- a cold on-chain BTC wallet with an acquisition, a self-transfer, and a peg-in
- a hot on-chain BTC wallet with the matching self-transfer receive, a spend,
  and a peg-out receive
- a Liquid wallet with the peg-in receive, peg-out send, and a Liquid spend
- metadata tags for `swap`, `peg-in`, `peg-out`, `self-transfer`, and `spend`
  plus notes on the swap, spend, and self-transfer records

Use an explicit data root when you want to inspect it afterwards:

```bash
scripts/seed-fake-wallets.sh --data-root /tmp/kassiber-demo/data
python3 -m kassiber --data-root /tmp/kassiber-demo/data reports balance-sheet
python3 -m kassiber --data-root /tmp/kassiber-demo/data journals transfers list
python3 -m kassiber --data-root /tmp/kassiber-demo/data metadata records list --tag swap
```

This fixture is not a live-chain test. It is a local accounting demo that gives
the test suite and desktop work a stable BTC/LBTC swap scenario.

## Liquid Live Sync

Liquid live sync in Kassiber currently requires an Esplora-compatible or
Electrum backend so outputs can be fetched and unblinded from descriptor
context. Running a full Liquid regtest stack plus an indexer is more
environment-specific than Bitcoin Core, so the test is parameterized and
skipped until you point it at a local backend.

Required environment:

```bash
KASSIBER_LIVE_SYNC_TESTS=1
KASSIBER_LIVE_LIQUID_BACKEND_URL=http://127.0.0.1:3001
KASSIBER_LIVE_LIQUID_BACKEND_KIND=esplora
KASSIBER_LIVE_LIQUID_NETWORK=elementsregtest
KASSIBER_LIVE_LIQUID_DESCRIPTOR_FILE=/path/to/receive.desc
KASSIBER_LIVE_LIQUID_CHANGE_DESCRIPTOR_FILE=/path/to/change.desc
```

For a local Electrum-compatible backend, use:

```bash
KASSIBER_LIVE_LIQUID_BACKEND_URL=tcp://127.0.0.1:50001
KASSIBER_LIVE_LIQUID_BACKEND_KIND=electrum
KASSIBER_LIVE_LIQUID_BATCH_SIZE=10
```

The Liquid test refuses non-loopback backend URLs. That keeps accidental public
backend queries out of the live test path, where descriptor-derived scripts
would otherwise leak to the queried server. Descriptor files are read locally
and normal Kassiber command output redacts raw descriptor material.

Set `KASSIBER_LIVE_LIQUID_ALLOW_EMPTY=1` only when you want to verify
connectivity and descriptor discovery against an unfunded local wallet. Without
that flag, the test expects at least one imported or already-known record.

## Regtest, Signet, And Privacy

Regtest is the default for live tests because it is local, deterministic, fast,
and does not reveal wallet structure or timing to public infrastructure.

Signet is useful for manual compatibility checks against external software, but
it is not part of the automated live-sync gate: querying public signet
Electrum/Esplora servers leaks scripts to those servers. If you need a signet
run, point Kassiber at infrastructure you control and keep the descriptor and
backend credentials local.
