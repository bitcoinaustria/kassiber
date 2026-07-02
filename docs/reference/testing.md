# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. Includes a baseline watch-only tape and an edge-case tape (multi-address wallet, immature vs. mature coinbase, dust, RBF-replaced conflict pair, same-wallet self-spend, mempool-pending receipt). |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses a Bitcoin Core regtest node, creates real wallets and transactions (including coinbase maturity and a watched receive), then drives Kassiber sync, pricing, journal, report, and export. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: eleven Kassiber wallets including multi-address Bitcoin wallets, rotation targets, a mining wallet, and deterministic Liquid/LBTC import wallets; real regtest acquisitions/disposals/transfers, operating-expense disposals with deterministic amount/fee variation, deprecated rotated-out wallets, batched, consolidation, dust, RBF-replacement, and mempool-pending edge cases, a multi-year stress ledger, CoinJoin- and PayJoin-shaped collaborative transactions, swap/peg bridge pairs, loan marks, bundled real historical BTC/EUR pricing, journals, reports, and transaction exports. |

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

- address-wallet creation and Bitcoin Core watch-only sync, with every
  operational wallet watching several rotating addresses (fresh receive and
  change addresses per payment, funding spread across the address set, and
  greedy multi-UTXO coin selection) so the book looks like real wallet usage
- file-source Liquid wallet creation and generic-ledger LBTC import through the
  same `wallets sync --all` path
- acquisition and disposal rows across Treasury, Cold Storage, Spending, and
  Merchant wallets, plus empty Bitcoin and Liquid rotation-target wallets that
  become active after security upgrades
- large single-source custody receipt into cold storage
- batched treasury payout to multiple external recipients in one transaction
- same-block merchant point-of-sale receipt burst with sat-level ragged
  amounts, followed by a many-input consolidation that imports as a fee-only
  wallet row, and an unsolicited dust deposit that lingers in the UTXO
  inventory
- an RBF fee-bumped payment: the conflicted original must be skipped by sync
  while only the confirmed replacement is booked
- solo-mining block rewards into a dedicated mining wallet at two points in
  history (visibly smaller after regtest halvings); immature coinbases must
  never import
- a customer payment that is still unconfirmed in the mempool when the book is
  synced, imported with an empty confirmed-at and a mempool UTXO state
- a deterministic historical stress lane: 132 cycles spaced 20 days apart, with
  batched inbound funding into operational wallets, rotating outbound payments,
  and regular fiat-expense disposals for payroll, rent, software, tax prep,
  contractors, and equipment; amounts and fees vary per cycle through a
  deterministic jitter (`stress.variation_bp`) so the ledger is volatile but
  reproducible; the demo still adds several hundred synced/imported wallet rows
  across seven years
- deterministic economic regimes (`stress.economic_regimes`) that scale
  inflows down and outflows up during downturns and vice-versa in booms, so
  operational balances genuinely rise **and** draw down (e.g. the treasury
  account falls through a 2020 shock and a 2022 bear market rather than
  climbing monotonically); regimes stay within each wallet's running balance
  so RP2's per-account balance gate never trips
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
- real historical BTC/EUR pricing from Kassiber's bundled Kraken daily cache,
  with LBTC rows priced through Kassiber's LBTC-to-BTC rate-pair alias; set
  `KASSIBER_REGTEST_DEMO_LIVE_RATES=coinbase-exchange` (or another supported
  live source) to opt into live provider backfill during local demo runs
- journal processing, summary reporting, PDF/CSV/XLSX report export, and
  CSV/XLSX transaction export

The scenario manifest lives at
`dev/regtest/scenarios/full_accounting.json`; the runner lives at
`tests/integration/regtest_demo.py`. The command prints a JSON summary with the
generated `data_root` and `export_dir`, for example:

```bash
./scripts/integration-harness.sh demo-full
```

## Developer Demo Environment (replaces mock data)

`demo-full` is a test lane: it builds a throwaway book and tears the node
down. For day-to-day development there is a persistent variant that replaces
the browser mock fixtures with a real, synced book — the same model as
BTCPayServer's `docker-compose up dev` + launch-profile workflow:

```bash
./scripts/integration-harness.sh demo-up   # node + demo book, kept running
cd ui-tauri && pnpm dev:demo               # dev preview on that real book
```

Prerequisites on any machine: Docker (Desktop or engine), `uv`, and `pnpm`.
That is the whole setup — two commands from a fresh clone to a browser preview
backed by the real Python daemon reading a multi-year regtest book.

What `demo-up` does:

- starts (or reuses) the regtest node under the fixed Compose project
  `kassiber-regtest-demo`, separate from the per-worktree test projects, and
  leaves it running;
- builds the `full-accounting-v1` book once into
  `~/.kassiber/regtest-demo/data` (override with
  `KASSIBER_REGTEST_DEMO_HOME`) and reuses it on later runs while the
  scenario file is unchanged; set `KASSIBER_REGTEST_DEMO_REBUILD=1` to force
  a rebuild;
- persists the generated regtest RPC credentials in
  `~/.kassiber/regtest-demo/demo-manifest.json` (mode 600, regtest-only
  throwaway secrets) so restarts keep matching the book's stored backend and
  refresh/sync from the GUI keeps working;
- keeps the demo Core wallets loaded (`--keep-core-wallets`) so incremental
  syncs from the app keep seeing new activity.

`pnpm dev:demo` runs the Vite daemon bridge with
`KASSIBER_DEV_DATA_ROOT` pointed at the demo book; the desktop preview then
shows the regtest data mode instead of mock fixtures. `pnpm dev:browser`
(mock) stays available for pure component work, and the mock fixtures remain
the basis of UI unit tests — the demo book replaces them only as the
*interactive* dev dataset.

### Making resync do something (`demo-tick`)

A freshly built book sits at the chain tip, so an in-app refresh imports
nothing. To simulate ongoing business so the incremental sync path has real
work:

```bash
./scripts/integration-harness.sh demo-tick        # one batch of fresh activity
./scripts/integration-harness.sh demo-tick 5      # five batches, five blocks
```

Each tick broadcasts a randomized batch of receipts (external → wallet),
payments (wallet → external), and self-transfers across the active
(non-deprecated) wallets, then mines a block so it confirms. Refresh in the
app (or `wallets sync --all`) and the new rows import. Activity is
random by default (that is the point); pass `--tick-seed` to
`tests.integration.regtest_demo --tick` for a reproducible batch. `demo-full`
itself always ends with one built-in tick + resync and fails if that resync
imports nothing — a standing guard that "refresh" is never a dead button.

Poke the node like BTCPayServer's `docker-bitcoin-cli.sh`:

```bash
./dev/regtest/bitcoin-cli.sh getblockchaininfo
./dev/regtest/bitcoin-cli.sh -generate 1
uv run python -m kassiber --data-root ~/.kassiber/regtest-demo/data reports summary
```

Tear-down is explicit: `demo-down` stops the node but keeps the chain volume
and book (a later `demo-up` resumes both); `demo-down --purge` removes the
node, volume, and demo book.

Because the book is backdated with `setmocktime`, a resumed node mints new
blocks at wall-clock time — new activity lands "now", after the historical
span, which is exactly what a long-lived real book looks like.

Browser mock mode remains useful for component fixtures, but it should not be
treated as an accounting or sync proof.

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

Lightning is the next planned slice: the concrete plan — Core Lightning
regtest nodes in a Compose overlay, an idempotent channel-bootstrap step,
scenario-manifest extensions, a `lightning-cli` tape for the fast lane, and
the assertions worth pinning — is written down in
[`dev/regtest/LIGHTNING-TODO.md`](../../dev/regtest/LIGHTNING-TODO.md),
based on how BTCPayServer's test stack orchestrates its
merchant/customer Lightning nodes.
