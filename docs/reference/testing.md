# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. Includes a baseline watch-only tape and an edge-case tape (multi-address wallet, immature vs. mature coinbase, dust, RBF-replaced conflict pair, same-wallet self-spend, mempool-pending receipt). |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses the regtest Compose stack (Bitcoin Core, Elements, Bitcoin Fulcrum, plus local mempool/esplora-compatible loopback endpoints), creates real wallets and transactions (including coinbase maturity and a watched receive), drives the Core RPC sync/pricing/journal/report/export smoke, and compares a real Fulcrum/Electrum address-wallet sync against Core RPC for receipt, spend, incremental, and no-op sync parity. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: sixteen Kassiber wallets including multi-address Bitcoin wallets, a Silent Payments wallet, rotation targets, a mining wallet, deterministic historical elementsregtest/LBTC import wallets, Boltz v2 metadata import wallets, and one descriptor-backed Liquid wallet synced from real `elementsd` transactions through the local Liquid Electrum endpoint; real regtest acquisitions/disposals/transfers, ownership-derived fan-out self-transfer matching, operating-expense disposals with deterministic amount/fee variation, deprecated rotated-out wallets, batched, consolidation, dust, RBF-replacement, and mempool-pending edge cases, local Bitcoin/Liquid Electrum and mempool-compatible backend rows, a multi-year stress ledger, CoinJoin- and PayJoin-shaped collaborative transactions, swap/peg bridge pairs, loan marks, bundled real historical BTC/EUR pricing, journals, reports, and transaction exports. |
| SILENT PAYMENTS | `./scripts/integration-harness.sh silent-payments` | yes | Starts the regtest Compose stack with the `silent-payments` profile, which builds/runs Sparrow Frigate against Bitcoin Core v30 and Fulcrum, waits until Frigate advertises `silent_payments: [0]` through `server.features`, then runs Kassiber's Silent Payments sync tests. Override the cold-start wait with `KASSIBER_REGTEST_FRIGATE_WAIT_SECONDS` if the local Frigate index is slow. |
| BOLTZ | `./scripts/integration-harness.sh boltz-liquid` | yes, upstream Boltz stack | Starts or reuses Boltz's official [`BoltzExchange/regtest`](https://github.com/BoltzExchange/regtest) Docker environment, probes the local Boltz API for Liquid-capable submarine, reverse, and BTC -> L-BTC chain-swap pairs, executes a Liquid on-chain payment plus an L-BTC -> BTC Lightning submarine swap, builds Kassiber import rows from the observed txids/hash/amounts, and verifies the swap pairs while the plain Liquid payment stays unpaired. Reverse, chain, and cooperative refund accounting are also covered with deterministic Boltz v2 provider-metadata JSON rows until the harness delegates client-side signing to Boltz's SDK/client. |
| LIGHTNING | `./scripts/integration-harness.sh lightning-business` | yes, Kassiber stack + CLN overlay | Starts the existing regtest Compose stack plus `dev/regtest/compose.lightning.yml` with four pinned Core Lightning nodes. A seeded sim-ln-inspired business plan drives mainchain top-ups/withdrawals, merchant invoices, supplier payments, routed forwarding activity, an expired quote, and an intentionally failed oversized payment, then verifies Kassiber through `wallets sync`, `ui.connections.node.snapshot`, `reports lightning-profitability`, and `export-lightning-profitability-csv`. |

The slow lane is opt-in with `KASSIBER_INTEGRATION=1`; normal unit gates do not
start Docker. To reuse an existing regtest node instead of Compose, set an
explicit Core URL and matching disposable RPC credentials:

```bash
export KASSIBER_REGTEST_CORE_URL=http://127.0.0.1:18443
export KASSIBER_REGTEST_RPC_USER=kassiber
export KASSIBER_REGTEST_RPC_PASSWORD=...
./scripts/integration-harness.sh bitcoin-core
# Or run only the Fulcrum/Electrum parity slice:
./scripts/integration-harness.sh bitcoin-electrum
```

The Compose lane generates disposable RPC credentials per run unless you set
them explicitly, passes only the `rpcauth` hash to bitcoind, publishes RPC on
host loopback, and uses Bitcoin Core v30 by default. It also publishes Core's
ZMQ `sequence` feed inside the Compose network for Frigate. The lane uses a
per-worktree Compose project name so parallel runs do
not share containers or volumes. It uses regtest only, no mainnet funds, no user
wallet files, and no production descriptors. The Compose stack publishes only
loopback ports: Core RPC, Elements RPC, the optional Frigate Electrum endpoint,
and the four protocol endpoints used by the UI/backend health and graph paths:

- `core-regtest` -> Bitcoin Core RPC, authoritative sync backend explicitly
  assigned to ordinary Bitcoin wallets
- Elements Core runs as `elementsd` on `elementsregtest` to provision a local
  Liquid daemon. The demo includes a descriptor-backed Liquid wallet with
  private blinding material stored inside the disposable test book; it receives
  and spends real elementsregtest LBTC, then syncs through the local Liquid
  Electrum service backed by `elementsd`.
- `bitcoin-electrum-regtest` -> the `fulcrum` container's Electrum TCP port
- `bitcoin-frigate-regtest` -> Sparrow Frigate's Electrum TCP port when
  `KASSIBER_REGTEST_COMPOSE_PROFILES=silent-payments` or the
  `silent-payments` lane is used; Frigate proxies ordinary Electrum calls to
  Fulcrum and handles Silent Payments discovery natively
- `bitcoin-mempool-regtest` -> local mempool/esplora-compatible HTTP API and
  the stored default backend, so transaction graph lookups and mempool-backed
  market-price probes use the local graph-capable endpoint instead of public
  infrastructure
- `liquid-electrum-regtest` -> local Liquid Electrum-compatible
  health/scripthash endpoint for the elementsregtest demo rail
- `liquid-mempool-regtest` -> local Liquid mempool/esplora-compatible HTTP
  API for graph lookups on deterministic LBTC demo txids

The HTTP explorer endpoints are host-loopback services and include regtest-only
CORS headers so a host browser can inspect them from a web explorer or local
tooling page. Use the manifest or `demo-up` output for the exact ports; by
default they are:

```text
Bitcoin explorer API: http://127.0.0.1:18544/api
Liquid explorer API:  http://127.0.0.1:18546/api
```

Set `KASSIBER_REGTEST_EXPLORER_CORS_ORIGIN` to a specific origin to narrow the
default `*`, or to an empty value to disable these CORS headers. The services
remain bound to host loopback and must stay regtest-only.

The Compose stack also maps `host.docker.internal` to the Docker host. A
containerized Kassiber process can use a host Ollama instance by seeding the
default local provider with:

```bash
KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1
```

This only affects first-time AI provider seeding; existing books should update
their `ollama` provider row to the host alias explicitly.

Set `KASSIBER_REGTEST_KEEP=1` to keep the full Docker Compose project running
for debugging (containers, bound ports, and volumes); otherwise the stack is
removed on exit.
Fresh Compose runs use the scenario manifest's historical timestamp sequence,
starting in January 2019 and covering activity into spring 2026. Reused Core
nodes can only move forward from their existing regtest chain tip, so their
calendar dates may drift while preserving the same relative spacing and row
shape.

## Silent Payments Regtest

The `silent-payments` lane exercises the infrastructure boundary for
Sparrow Frigate, not a generic HTTP scanner. Frigate speaks Electrum protocol
extensions (`blockchain.silentpayments.*`) and advertises support through
`server.features`, so the harness starts it in a dedicated Compose profile and
probes that Electrum capability before running Kassiber's SP sync coverage. On
a fresh regtest volume the lane mines a disposable readiness block first, because
the genesis-only tip still looks like initial block download to Fulcrum and keeps
Frigate waiting for sync.

The full demo book also includes a `silent-payment` wallet. Because Kassiber's
current server-assisted SP adapter expects HTTP JSON while Frigate exposes
Electrum JSON-RPC, the demo wallet syncs through the existing `local-index`
mode: the builder creates a real regtest Taproot output, writes a private
scanner JSON file bound to the wallet's descriptor fingerprint, and stores that
path on the Frigate-marked backend. This keeps the accounting fixture honest
without pretending the Frigate Electrum adapter exists yet.

## Boltz Liquid Regtest

The `boltz-liquid` lane is a narrow bridge between Kassiber's demo accounting
path and Boltz's upstream Docker development setup. Boltz documents normal
submarine swaps as chain -> Lightning, reverse swaps as Lightning -> chain,
and chain swaps as chain -> chain, with "chain" including Bitcoin mainchain
and Liquid. Their regtest repository publishes a local API on
`http://127.0.0.1:9001`, WebSocket on `ws://127.0.0.1:9004`, Bitcoin Esplora
on `http://127.0.0.1:4002`, and Elements/Liquid Esplora on
`http://127.0.0.1:4003`.

Run it with an existing checkout:

```bash
git clone https://github.com/BoltzExchange/regtest ~/.cache/kassiber/boltz-regtest
./scripts/integration-harness.sh boltz-liquid
```

Or let the lane clone the upstream repo into the same cache path:

```bash
KASSIBER_BOLTZ_REGTEST_AUTO_CLONE=1 ./scripts/integration-harness.sh boltz-liquid
```

Set `KASSIBER_BOLTZ_REGTEST_REUSE=1` when the Boltz stack is already running,
or `KASSIBER_BOLTZ_REGTEST_KEEP=1` to leave it running after the test. The
lane uses `docker` directly when available and falls back to passwordless
`sudo docker`, matching the other regtest lanes. It uses Boltz's documented v2
API for the low-signing happy path that Kassiber needs to account for today:

- `POST /v2/swap/submarine` with `from=L-BTC`, `to=BTC`, a real LND invoice,
  and a refund pubkey.
- `elements-cli-sim-client sendtoaddress` to fund the returned Liquid lockup
  address, then local Elements mining.
- a separate `elements-cli-sim-client sendtoaddress` payment to prove ordinary
  Liquid outflows stay payments and are not matched as swaps.
- a temporary Kassiber book with a Liquid ledger import and Lightning import
  generated from the observed txids, payment hash, and amounts, followed by
  `transfers suggest` and `transfers bulk-pair` assertions.
- deterministic JSON imports that mimic a redacted Boltz v2 SDK/client export
  for BTC -> L-BTC chain swap, BTC -> L-BTC reverse swap, and same-asset failed
  refund. These rows carry `provider=boltz`, a provider-scoped swap id, route
  txids, and Taproot/cooperative metadata, and assert exact `provider_swap_id`
  matching through the same Kassiber accounting book.

The lane still only live-executes the L-BTC -> BTC submarine path because
reverse, BTC -> L-BTC chain, and refund execution require client-side
claim/refund transaction construction. Their current coverage is accounting
metadata coverage, not proof that Kassiber can produce or sign those Boltz
transactions. They should move from metadata fixtures to executed fixtures once
the harness delegates that signing/recovery state machine to Boltz's official
client or SDK.

Boltz's upstream Compose file binds bitcoind RPC to host port `18443`, the same
default used by Kassiber's own regtest lane. If that port is already occupied,
the harness writes a temporary Compose file with the host-only binding changed
to `19443 -> 18443`; internal Boltz services still use `bitcoind:18443`, and
the upstream checkout stays untouched. Set `KASSIBER_BOLTZ_BITCOIN_RPC_PORT=<port>`
to choose a different host binding.

## Lightning Business Regtest

The `lightning-business` lane is Kassiber's live Core Lightning merchant-node
test. It layers `dev/regtest/compose.lightning.yml` onto the existing Bitcoin
regtest compose file and uses Bitcoin Core regtest as the funding/mining
source. The CLN overlay defaults to the pinned
`elementsproject/lightningd:v25.05` image; set `KASSIBER_REGTEST_CLN_IMAGE` to
test a different CLN build intentionally.

Run:

```bash
./scripts/integration-harness.sh lightning-business
```

The lane creates these Docker-only actors:

- `cln_merchant` — the only node Kassiber connects to
- `cln_customer` — pays merchant invoices and routed supplier invoices
- `cln_supplier` — receives merchant expense payments
- `cln_router` — provides the extra hop for routed payments and fee rows

Kassiber stores exactly one Lightning backend/wallet, both for
`cln_merchant`. The backend's `lightning_cli` points at
`dev/regtest/lightning-cli-merchant.sh`; customer, supplier, and router are
never created as Kassiber wallets or connections.

The bootstrap script is idempotent: it waits for CLN, creates/reuses the
Bitcoin faucet wallet, funds CLN wallets only below the threshold, opens the
`customer -- merchant -- router -- supplier` channels if absent, mines
confirmations, and waits for normal channels. The scenario then generates a
seeded business plan at `$KASSIBER_LIGHTNING_BUSINESS_PLAN` (default:
`$KASSIBER_LIGHTNING_BUSINESS_HOME/business-plan.json`) and executes the
stable-label activity from that plan:

- customer-paid merchant invoices (`merchant-pos-sale-*`) with varied amounts
- merchant-paid supplier invoices routed through the router
- customer/router third-party payments that cross the merchant as forwards
- one expired/unpaid merchant quote
- one intentionally oversized failed payment to exercise failed-payment rows
- Bitcoin Core mainchain actor wallets that send top-ups into the merchant CLN
  wallet and receive merchant CLN withdrawals, with regtest blocks mined between
  batches so the merchant node's L1 balance and bookkeeper rows are non-trivial

The plan borrows sim-ln's useful modeling knobs without making sim-ln a hard
test dependency: defined activity remains deterministic for assertions, while
`KASSIBER_REGTEST_LIGHTNING_SEED`,
`KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER`, and
`KASSIBER_REGTEST_LIGHTNING_EXPECTED_PAYMENT_MSAT` control variation; the
channel-capacity input comes from
`KASSIBER_REGTEST_LIGHTNING_CHANNEL_CAPACITY_SAT`. The default multiplier is
intentionally modest so the route directions stay liquid on the default
5,000,000 sat channels.

Assertions happen in `tests/integration/lightning_business_regtest.py` and are
against Kassiber output/state, not only `lightning-cli`: daemon snapshot,
profitability report, CSV export, synced DB records, and the one-merchant-only
wallet/backend invariant. The lane also checks that the merchant snapshot has
on-chain balance evidence plus paid, expired, failed, outgoing-payment, channel,
and forwarding activity. Persisted Lightning records do not store raw RPC JSON,
and AI-safe Lightning payloads omit sensitive route, peer, preimage,
payment-secret, bolt11, funding-outpoint, and failure-source fields.

Set `KASSIBER_REGTEST_KEEP=1` to leave the full Docker Compose project running
for inspection (containers, bound ports, and volumes), or
`KASSIBER_REGTEST_LIGHTNING_REUSE=1` to reuse an already-running project. The
book lives under
`${KASSIBER_LIGHTNING_BUSINESS_HOME:-/tmp/kassiber-lightning-business-<project>}`.
The assertion book is rebuilt by default so stale DB rows cannot satisfy a
fresh run; set `KASSIBER_LIGHTNING_BUSINESS_REUSE_BOOK=1` only when debugging a
preserved book intentionally. The scenario state records the generated plan
hash, so changing seed, multiplier, expected-payment, or channel-capacity knobs
while reusing preserved Lightning state requires a fresh
`KASSIBER_LIGHTNING_BUSINESS_HOME` or manual cleanup of the preserved state and
volumes.

## Full Accounting Demo

`demo-full` is the replacement substrate for fake one-click accounting data. It
does not inject synthetic transaction rows into SQLite. Instead, it creates real
Bitcoin Core regtest wallets, broadcasts real transactions, syncs Kassiber from
the Core RPC backend, then verifies Kassiber behavior through the public CLI:

- address-wallet creation and Bitcoin Core watch-only sync, with every
  operational wallet watching several rotating addresses (fresh receive and
  change addresses per payment, funding spread across the address set, and
  greedy multi-UTXO coin selection) so the book looks like real wallet usage
- file-source Liquid wallet creation on `elementsregtest` and generic-ledger
  LBTC import through the same `wallets sync --all` path; deterministic
  Liquid external IDs are txid-shaped so the local Liquid mempool-compatible
  graph endpoint can resolve them instead of linking to public infrastructure
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
- an ownership-derived Bitcoin fan-out transfer where one treasury transaction
  pays two owned operational wallets; before any manual `transaction_pairs`
  exist the demo runs `journals process` and `journals transfers list` to prove
  both MOVE legs are surfaced with `pairing_source=ownership_derived`
- Liquid/on-chain-style bridge events (`peg-in`, a Boltz-marked
  BTC -> L-BTC `chain-swap`, and `peg-out`) pair real Bitcoin Core txids with
  deterministic LBTC ledger external IDs so the generic-tax demo exercises
  taxable cross-asset swaps
- deterministic Boltz v2 provider-metadata JSON imports for cooperative
  key-path BTC -> L-BTC chain swap, BTC -> L-BTC reverse swap, and same-asset
  failed refund. The demo asserts they surface as exact `provider_swap_id`
  candidates before pairing, then bulk-pairs them into the reportable book with
  `chain-swap`, `reverse-submarine-swap`, and `swap-refund` kinds.
- local backend rows for Bitcoin Core RPC, Bitcoin Electrum/Fulcrum-compatible
  TCP, Bitcoin mempool-compatible HTTP, Liquid Electrum-compatible TCP, and
  Liquid mempool-compatible HTTP; the demo deletes public/default backends and
  fails if any non-regtest backend remains
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
  with LBTC rows priced through Kassiber's LBTC-to-BTC rate-pair alias; the
  docker preview also writes one current `mempool` latest-quote row from the
  local Bitcoin mempool-compatible endpoint so the overview's live BTC price
  is not a historical Kraken row. Set
  `KASSIBER_REGTEST_DEMO_LIVE_RATES=coinbase-exchange` or `coingecko` to
  override that provider, or `off` to keep only the historical cache.
- journal processing, summary reporting, PDF/CSV/XLSX report export, and
  CSV/XLSX transaction export
- a generated oracle artifact at `generated-truth.json` in the demo export
  directory for the report/export build point (before the optional post-sync
  business tick). The demo records expected transaction row identities,
  reviewed transfer pairs, and active Bitcoin Core UTXOs/balances while it
  creates the chain activity, then verifies Kassiber's synced rows, report
  metrics, UTXO inventory, and transaction export contents against that
  generated truth. Exact checks are used for the Core-controlled row/UTXO
  surface; report file bytes remain intentionally unchecked because PDF/XLSX
  metadata and writer ordering are not stable enough to be the source of truth.

The scenario manifest lives at
`dev/regtest/scenarios/full_accounting.json`; the runner lives at
`tests/integration/regtest_demo.py`. The command prints a JSON summary with the
generated `data_root` and `export_dir`, for example:

```bash
./scripts/integration-harness.sh demo-full
```

## Developer Regtest Demo Environment

`demo-full` is a test lane: it builds a throwaway book and tears the node
down. For day-to-day development there is a persistent variant that replaces
browser fixtures with a real, synced book — the same model as
BTCPayServer's `docker-compose up dev` + launch-profile workflow:

```bash
./scripts/integration-harness.sh demo-up   # node + demo book, kept running
cd ui-tauri && pnpm dev:demo               # dev preview on that real book
```

Prerequisites on any machine: Docker (Desktop or engine), `uv`, and `pnpm`.
From a fresh clone, install the project/runtime dependencies once:

```bash
uv sync
pnpm --dir ui-tauri install
```

After that, the two commands above take you to a browser preview backed by the
real Python daemon reading a multi-year regtest book. The harness checks for
core Python dependencies and fails with a setup hint instead of silently using
an unprepared system interpreter.

What `demo-up` does:

- starts (or reuses) the regtest Compose stack under the fixed Compose project
  `kassiber-regtest-demo`, separate from the per-worktree test projects, enables
  the `silent-payments` profile, and leaves Bitcoin Core, Elements, Fulcrum,
  Frigate, and the local protocol API services running;
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

The `fulcrum` container is provisioned and exposed as the
`bitcoin-electrum-regtest` backend row. The slow Bitcoin lane now includes a
dedicated Fulcrum/Electrum parity slice that syncs the same real address wallet
through Core RPC and Electrum, then compares the persisted transaction and UTXO
views after receipts, a spend, an incremental receipt, and a no-op sync. The
demo book itself still pins ordinary Bitcoin wallet sync to Core RPC
(`core-regtest`). The Liquid Electrum and Liquid mempool rows are local services
backed by `elementsd`; the demo's `liquid_live_sync` wallet uses the Electrum
row for real descriptor-backed LBTC sync, while the older Liquid
treasury/operations history remains deterministic `generic_ledger` fixture
data. That keeps the preview repeatable while avoiding accidental public
mainnet explorers in regtest mode.
The stored default backend is `bitcoin-mempool-regtest`: wallet configs still
pin their sync source (`core-regtest` for ordinary Bitcoin, `bitcoin-frigate-regtest`
for the Silent Payments wallet), while graph-capable UI paths prefer the local
HTTP mempool/esplora endpoint.
Remaining backend-parity work is demo-wallet sync through Bitcoin explorer HTTP,
plus broader Liquid Electrum/explorer/Elements comparisons across the historical
fixture set.

`pnpm dev:demo` runs the Vite daemon bridge with
`KASSIBER_DEV_DATA_ROOT` pointed at the demo book; the desktop preview then
shows the regtest data mode instead of static fixtures. `pnpm dev:browser` is an
alias for the same regtest-backed browser preview. Fixture responses remain for UI
unit tests only; they are no longer an interactive data mode.

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

Fixture transport remains useful for component tests, but it should not be
treated as an accounting or sync proof and is not exposed as an interactive data
mode.

## Guardrails

- `KASSIBER_NO_EGRESS=1` blocks non-loopback `socket.connect` calls inside fast
  harness tests so replay fixtures cannot accidentally reach live exchanges or
  public backends.
- Tapes must include provenance (`backend_kind`, network, regtest anchor, and
  issue number) and fail closed: an adapter request absent from the tape raises
  `TapeMiss`, while unused recorded interactions fail the replay test.
- Export assertions are content-level. XLSX files are inspected for expected
  sheets and self-verification content rather than byte-compared.
- The full demo's generated truth is the oracle for controlled regtest facts:
  transaction row identities, transfer pair identities, Core wallet UTXOs, and
  Core wallet balances must match exactly. Broader report/export checks stay
  targeted where formatting, cached market data, or third-party writers make
  byte-for-byte comparison noisy.
- Docker infrastructure is contributor test tooling only. It must not add an
  app-facing shell/filesystem escape hatch or relax desktop daemon allowlists.
- The demo runner prints regtest addresses and txids, but never prints RPC
  passwords. CLI backend credentials are passed through file descriptors.

## Growth Path

The current checked-in slow lanes exercise Bitcoin Core RPC end to end (sync,
pricing, journal, report, export), a Core-vs-Fulcrum/Electrum parity slice, and
a full accounting demo on Bitcoin regtest. They also provision Elements Core,
Bitcoin Fulcrum, and local mempool/esplora-compatible endpoints, create
deterministic file-source elementsregtest/LBTC demo wallets, and include one
real `elementsd`-backed descriptor Liquid wallet through the local
Electrum-compatible service. Remaining parity targets are broader cross-backend
comparisons: Bitcoin Core vs Fulcrum vs explorer HTTP, and Liquid Electrum vs
explorer HTTP/`elementsd` views across the larger historical fixture set. Those
can be added without changing the contributor entrypoint.

Lightning now has an opt-in live merchant lane through
`./scripts/integration-harness.sh lightning-business`. Remaining parity work is
to add a fast recorded `lightning-cli` tape lane, broaden backend parity beyond
Core Lightning/LND as new adapters land, and eventually compare live
Lightning-derived accounting across multiple node implementations without
changing the contributor entrypoint.
