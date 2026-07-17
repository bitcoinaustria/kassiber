# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Dev Environment

Use the locked `uv` environment for local tests and daemon-backed desktop
development:

```bash
./scripts/bootstrap-dev-env.sh
export KASSIBER_PYTHON="$PWD/.venv/bin/python"
uv run --locked python -m pytest tests/test_wallet_descriptors.py -q
```

The bootstrap script runs `uv sync --locked`, which fails if `uv.lock` no longer
matches `pyproject.toml` and never rewrites the lock, then
verifies imports for the packages that most often go missing in ad-hoc shells
(`embit` and `sqlcipher3`). On Debian/Ubuntu it fails early with the required
SQLCipher system package command if the development headers are not available.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. Includes a baseline watch-only tape and an edge-case tape (multi-address wallet, immature vs. mature coinbase, dust, RBF-replaced conflict pair, same-wallet self-spend, mempool-pending receipt). |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses the regtest Compose stack (Bitcoin Core, Elements, Bitcoin Fulcrum, plus local mempool/esplora-compatible loopback endpoints), creates real wallets and transactions (including coinbase maturity and a watched receive), drives the Core RPC sync/pricing/journal/report/export smoke, and compares a real Fulcrum/Electrum address-wallet sync against Core RPC for receipt, spend, incremental, and no-op sync parity. |
| CHAIN OBSERVERS | `./scripts/integration-harness.sh chain-observers [all\|bitcoin\|liquid]` | yes, unless reusing nodes | Extends the same disposable Compose stack with an independent Core/Elements truth oracle. It records bounded mode-0600 manifests for descriptor/address families, receive/change and gap ownership, mempool/confirmation/RBF/reorg/restart transitions, Bitcoin and Liquid UTXOs, policy/issued-asset values, and protocol endpoint heights. It also proves observer state commits and rollback/restart behavior inside a temporary SQLCipher book with no dependency sidecar store. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: thirteen Kassiber wallets including 8–12-address Bitcoin wallets, legacy/nested-SegWit/bech32/taproot rotation targets, a Silent Payments wallet, a mining wallet, and descriptor-backed Liquid wallets synced from real `elementsd` transactions through the local Liquid Electrum endpoint; monthly EUR-denominated receipts, payments, payroll, rent, cloud, and other expenses converted through the bundled Kraken BTC/EUR rate for each cycle; a deterministic jittered clock with quiet and doubled activity months; era-sensitive fees, rare heavy-tail receipts, 2–3 outbound payments in busy regimes, five external counterparty clusters, threshold-driven working-capital/merchant/cold-reserve transfers, near-full Bitcoin rotations with an explicit bounded legacy reserve where required, a real Liquid treasury migration with a delayed-receipt catch-up sweep, three policy-selected UTXO consolidation windows, and assertions that deprecated wallets retain at most one fee-reserve UTXO; 2021–22 pool payouts plus one solo coinbase, scheduled batched/dust/PayJoin/RBF edge cases, closed and still-open collateralized loans, ownership-derived fan-out self-transfer matching, a mempool-pending receipt, local Bitcoin/Liquid Electrum and mempool-compatible backend rows, swap/peg bridge pairs, journals, reports, and transaction exports. The persistent `demo-up` variant also starts the Core Lightning and BTCPay overlays by default, seeding merchant Lightning, BTCPay connections, paid BTCPay invoice/payment-request examples, and reviewed commercial reconciliation unless explicitly disabled. |
| SILENT PAYMENTS | `./scripts/integration-harness.sh silent-payments` | yes | Starts the regtest Compose stack with the `silent-payments` profile, which builds/runs Sparrow Frigate against Bitcoin Core v30 and Fulcrum, waits until Frigate advertises `silent_payments: [0]` through `server.features`, then runs Kassiber's Silent Payments sync tests. Override the cold-start wait with `KASSIBER_REGTEST_FRIGATE_WAIT_SECONDS` if the local Frigate index is slow. |
| BOLTZ | `./scripts/integration-harness.sh boltz-liquid` | yes, upstream Boltz stack | Starts or reuses Boltz's official [`BoltzExchange/regtest`](https://github.com/BoltzExchange/regtest) Docker environment, probes the local Boltz API for Liquid-capable submarine, reverse, and BTC -> L-BTC chain-swap pairs, executes a Liquid on-chain payment plus an L-BTC -> BTC Lightning submarine swap, persists the paid invoice through Kassiber's native LND adapter boundary, records the Liquid policy-asset id and Elements regtest scope, and explicitly reviews the resulting strong provider-hash candidate while the plain Liquid payment stays unpaired. An outbound submarine lockup has no owned claim witness, so provider metadata plus a native invoice is intentionally not called exact. Optional `KASSIBER_BOLTZ_V2_EVIDENCE=/path/to/evidence.json` adds real Boltz wallet/client/provider v2 chain/reverse/refund evidence rows. Exact `provider_swap_id` assertions require a unique 1:1 provider key, canonical send/receive route txids, explicit chain/network scope and Liquid consensus asset id, plus integer-msat principals covering both complete rows; incomplete evidence must stay strong/manual. |
| BTCPAY | `./scripts/integration-harness.sh btcpay` | yes | Starts the standard Bitcoin regtest stack plus a BTCPay overlay (Postgres, NBXplorer, BTCPay Server) wired to the same disposable `bitcoind`, creates a first admin user, creates or reuses a test store, generates a BTC on-chain store wallet, creates and pays a realistic invoice mix from the regtest Core wallet, confirms it, syncs BTCPay wallet history into a temporary Kassiber book, syncs invoice provenance, and writes a local seed JSON containing the store id, disposable API key, invoice ids, scenarios, and payment txids. |
| LIGHTNING | `./scripts/integration-harness.sh lightning-business` | yes, Kassiber stack + CLN/LND overlay | Starts the existing regtest Compose stack plus `dev/regtest/compose.lightning.yml` with four pinned Core Lightning nodes and one LND backup merchant node. A seeded sim-ln-inspired business plan drives mainchain top-ups/withdrawals, merchant invoices, supplier payments, routed forwarding activity, LND backup receipts/outbound payments, an expired quote, and an intentionally failed oversized payment, opens private CLN merchant -> LND backup and LND backup -> CLN router channels, then verifies Kassiber through `wallets sync`, `ui.connections.node.snapshot` for both node implementations, `reports lightning-profitability`, and `export-lightning-profitability-csv`. |

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

## Pull-request CI

The required PR workflow is fail-fast without reducing the test inventory:

1. `Preflight` compiles Python, validates the test-shard manifest, runs the
   dependency/workflow/catalog/report drift contracts, and gates only the
   Python shards. Frontend, CLI smoke, and observer jobs start independently.
2. Pytest runs every `tests/**/test_*.py` module in exactly one lane. The
   `core-accounting`, `wallets-sync`, `daemon-cli`,
   `security-replication`, and `reports-contracts` lanes use two xdist workers
   with class/module scope. Isolated `serial-network`, `serial-daemon`,
   `serial-regressions`, and `serial-integration` jobs own
   socket-, listener-, broad-regression-, OS-process-, and opt-in
   integration-sensitive modules.
3. The CLI smoke lane checks the broad argparse help surface in-process and
   keeps only status/health/next-actions, command discovery, and daemon EOF as
   real subprocess probes. TypeScript, ESLint, and Vitest run together in an
   independent frontend job.
4. Specialized jobs can remain independent roots and join `ci/required`
   without blocking unrelated setup. Platform credential jobs retain their
   separate credential/packaging path filter, and ordinary pull requests do
   not build release packages.
5. Each pytest/Vitest lane uploads a JUnit artifact even after failure. Python
   lanes print `--durations=50` output in their job summary. The stable
   `ci/required` aggregate is successful only when every required lane passes.

`scripts/python_test_shards.py` is the single file-to-lane mapping. Unknown new
test modules default to `core-accounting`, while `tests/test_ci_shards.py`
proves that every discovered module appears once and that sensitive modules do
not enter xdist. PR pushes use a concurrency group and cancel superseded CI;
test failures are never broadly retried. Locked uv and pnpm stores are cached;
cache misses still install only from the checked-in lockfiles.

The local `./scripts/quality-gate.sh` favors deterministic serial execution,
but runs the same complete Python inventory once before the frontend checks.

## Dependency chain-observer oracle

Run both independent node scenarios before changing a dependency adapter:

```bash
./scripts/integration-harness.sh chain-observers
```

The optional selector narrows local debugging without changing the shared
Compose stack:

```bash
./scripts/integration-harness.sh chain-observers bitcoin
./scripts/integration-harness.sh chain-observers liquid
```

The lane creates a temporary output root and removes it together with the
per-worktree Compose volumes by default. A normal run also removes any stale
project/volumes left by an earlier keep-mode run before starting, so node truth
always begins on a clean chain. `KASSIBER_REGTEST_KEEP=1` preserves both for
debugging; `KASSIBER_REGTEST_REUSE_CORE=1` reuses already-running loopback
nodes and the existing port/credential environment. Generated
`bitcoin-truth.json` and `liquid-truth.json` files are private run artifacts,
not fixtures: Bitcoin Core and Elements RPC define expected txids, outpoints,
heights, confirmation/replacement relationships, ownership indices, UTXOs and
asset values. Fulcrum/Electrum and the local Esplora-compatible endpoints must
report the same tip, but never define truth themselves.

Bitcoin transitions are full scan, no-op, gap payment/discovery, unconfirmed
and confirmed receipt, unconfirmed spend, RBF replacement and confirmation,
block invalidation, mempool resurrection, reconsideration, process restart,
incremental refresh and final no-op. Wallet-form metadata covers BIP44/49/84/86,
fixed 2-of-2, receive/change, canonical multipath, multi-script logical xpub,
and representative Samourai Deposit/Badbank/Premix/Postmix/Ricochet sources.

Liquid transitions cover ranged confidential receive/change discovery, LBTC
receive/spend, a real issued asset receive/spend, confirmation, invalidation,
resurrection, reconsideration and restart. The manifest records the consensus
policy/issued asset ids, confidential unblinding success and the controlled
wrong-key failure expectation. Capability rows state whether the pinned local
Elements build exposes Taproot or executable ranged multisig rather than
pretending unsupported forms ran.

Every transition is also referenced from versioned observer JSON in the main
temporary SQLCipher database. The runner injects and rolls back a failed state
write, reopens the encrypted project, and rejects BDK/LWK-looking sidecar files.
For Bitcoin/all selections, the lane then creates a real Core descriptor wallet,
funds both low and gap-edge receive indices, and refreshes its public descriptor
through Fulcrum using BDK. It asserts the BDK route, transaction/UTXO projection,
per-branch coverage, SQLCipher-only state, process restart, and byte-stable
immediate no-op state.
For Liquid/all selections, the lane creates a real Elements descriptor wallet
with private SLIP77 view material and public spending keys, then refreshes it
through both the local Electrum and Esplora-compatible services using LWK. It
asserts the LWK-only route, opaque `ForeignStore` bytes in the main database,
restart and immediate no-op stability, confidential LBTC receive/spend and fee
normalization, issued-asset history, transport parity, confirmation, block
invalidation and unconfirmed resurrection.
Only loopback RPC, Electrum and HTTP targets are accepted. Routing metadata
pins pre-connect compatibility selection for this phase, forbids runtime
fallback, and records that `.onion` endpoints may not connect directly.
The transport oracle also drives the pinned clients themselves: BDK crosses
plain Electrum, Esplora, insecure test TLS and SOCKS5h; LWK crosses plain
Electrum, Esplora, explicit insecure test TLS and an authentication-enforcing
Esplora reverse proxy. Custom-CA rows use Kassiber's manual Electrum client
because neither pinned dependency accepts a per-client trust root; configured
Esplora custom trust fails before egress rather than being silently ignored.

Pull requests and main-branch pushes expose a required
`Chain observers (Linux Docker)` job in `.github/workflows/ci.yml`. It runs the
lane when observer, descriptor, sync, persistence, dependency-lock, or regtest
backend paths change; unrelated changes retain the same successful check name
without starting the Docker stack.

The Compose lane generates disposable RPC credentials per run unless you set
them explicitly, passes only the `rpcauth` hash to bitcoind, publishes RPC on
host loopback, and uses Bitcoin Core v30 by default. It also publishes Core's
ZMQ `sequence` feed inside the Compose network for Frigate. The lane uses a
per-worktree Compose project name so parallel runs do
not share containers or volumes. It uses regtest only, no mainnet funds, no user
wallet files, and no production descriptors. The Compose stack publishes only
loopback ports: Core RPC, Elements RPC, the optional Frigate Electrum endpoint,
and the four protocol endpoints used by the UI/backend health and graph paths:

- `core-regtest` -> Bitcoin Core RPC, assigned to the ordinary Bitcoin wallets
  that do not opt into the Fulcrum coverage slice
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

## BTCPay Regtest

The dedicated BTCPay proof lane pulls a full BTCPay Server/NBXplorer/Postgres
overlay in addition to the normal Bitcoin regtest stack:

```bash
./scripts/integration-harness.sh btcpay
```

The overlay is defined in `dev/regtest/compose.btcpay.yml`. It reuses the
managed regtest `bitcoind` and generated RPC credentials, runs NBXplorer against
that node, and publishes BTCPay on host loopback. By default the ports are
derived from the regtest base port:

```text
BTCPay Server: http://127.0.0.1:18549
BTCPay NBXplorer: http://127.0.0.1:18550
```

Override them with `KASSIBER_REGTEST_BTCPAY_PORT` and
`KASSIBER_REGTEST_BTCPAY_NBXPLORER_PORT`. The BTCPay image is pinned to the
published stable `btcpayserver/btcpayserver:2.3.9` tag because Docker Hub does
not publish a `latest` tag. Override images with `KASSIBER_REGTEST_BTCPAY_IMAGE`,
`KASSIBER_REGTEST_BTCPAY_NBXPLORER_IMAGE`, and
`KASSIBER_REGTEST_BTCPAY_POSTGRES_IMAGE`.

The seed helper (`python -m dev.regtest.btcpay_seed`) uses the Greenfield API to
create a disposable admin user, store, BTC on-chain payment method, and scoped
API key. In the standalone `btcpay` lane it creates and pays a realistic
BTCPay-origin mix: a direct Greenfield invoice, a duplicate-order adjustment,
a point-of-sale sale, a two-transaction partial payment, a EUR-denominated
checkout invoice, a EUR-denominated payment-request invoice, and a crowdfund pledge. Each
invoice receives a fresh BTCPay on-chain address, is paid from the regtest Core
wallet, confirmed by a mined block, synced through BTCPay wallet history into a
temporary Kassiber book, and reconciled through invoice/payment provenance. The
standalone lane also creates a local commercial document keyed by the BTCPay
payment-request id, runs `btcpay provenance suggest`, reviews the combined
BTCPay-payment-to-wallet-transaction link as income, and checks that the
commercial subledger uses `btcpay_payment` pricing. The seed JSON records the
invoice ids, txids, scenarios, currencies, origin kinds, reviewed link id, and
applied pricing proof so failures can be replayed locally.
The persistent demo book starts the same BTCPay overlay by default:

```bash
./scripts/integration-harness.sh demo-up
```

That writes the BTCPay URL, store id, and disposable API key into
`demo-manifest.json` (mode `0600`) and configures the demo book with a
`btcpay-regtest` backend plus a `BTCPay Regtest Store` wallet using
`BTC-CHAIN` wallet-history sync. By default it also runs the paid invoice
exercise from the seed helper, so the long-lived demo book includes paid
direct invoices, a payment request, POS/crowdfund provenance, a multi-payment
invoice, duplicate commercial references, wallet-history import, a reviewed
commercial payment-request link, and visible transaction tags
(`btcpay`, `payment-request`, `commercial-income`) on the reconciled
membership-income transaction. Use the dedicated `btcpay` lane when the same
coverage should run in a temporary proof book instead of the persistent demo.
Disable the BTCPay overlay for a lighter persistent demo with:

```bash
KASSIBER_REGTEST_DEMO_BTCPAY=0 ./scripts/integration-harness.sh demo-up
```

Or keep the BTCPay store/UI in the persistent demo but skip paid invoice and
commercial-reconciliation seeding with:

```bash
KASSIBER_REGTEST_DEMO_BTCPAY_INVOICES=0 ./scripts/integration-harness.sh demo-up
```

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
- a temporary Kassiber book with Liquid identity rows carrying the observed
  txids, Elements regtest network, and policy-asset id; the paid invoice is
  normalized through the native LND adapter/import boundary. The shared hash is
  therefore a strong review candidate (native node + Boltz provider evidence),
  and the lane explicitly confirms it through `transfers bulk-pair`.

The lane still only live-executes the L-BTC -> BTC submarine path because
reverse, BTC -> L-BTC chain, and refund execution require client-side
claim/refund transaction construction. The default lane therefore does not claim
reverse, chain, or cooperative refund v2 execution coverage. Those cases become
covered only when the harness is given real wallet/client/provider evidence from
an executed swap.

If those v2 flows have been executed by an official Boltz client/SDK or another
wallet has the relevant facts, pass that evidence through
`KASSIBER_BOLTZ_V2_EVIDENCE` or `--v2-evidence` when running the `boltz-liquid`
lane. The evidence must contain real route identifiers and observed
amounts/timestamps, physical chain/network scope, and the consensus asset id for
every Liquid leg; the harness rejects obvious placeholder ids instead of
recreating the old metadata-only fixture path. If the user only has the chain
rows and no provider/client facts, Kassiber should still surface heuristic
swap-pair suggestions for review rather than claiming exact provider evidence.
Minimal shape:

```json
{
  "swaps": [
    {
      "provider": "boltz",
      "id": "provider-swap-id",
      "flow": "chain",
      "status": "completed",
      "version": "2",
      "taproot": true,
      "cooperative": true,
      "spend_path": "key",
      "out": {
        "txid": "real-send-txid-or-external-id",
        "occurred_at": "2026-07-02T11:00:00Z",
        "asset": "BTC",
        "chain": "bitcoin",
        "network": "regtest",
        "amount": "0.01000000",
        "fee": "0.00000500"
      },
      "in": {
        "txid": "real-receive-txid-or-external-id",
        "occurred_at": "2026-07-02T11:04:00Z",
        "asset": "LBTC",
        "asset_id": "real-32-byte-elements-policy-asset-id",
        "chain": "liquid",
        "network": "elementsregtest",
        "amount": "0.00990000"
      }
    }
  ]
}
```

Boltz's upstream Compose file binds bitcoind RPC to host port `18443`, the same
default used by Kassiber's own regtest lane. If that port is already occupied,
the harness writes a temporary Compose file with the host-only binding changed
to `19443 -> 18443`; internal Boltz services still use `bitcoind:18443`, and
the upstream checkout stays untouched. Set `KASSIBER_BOLTZ_BITCOIN_RPC_PORT=<port>`
to choose a different host binding.

## Lightning Business Regtest

The `lightning-business` lane is Kassiber's live Lightning merchant-node test.
It layers `dev/regtest/compose.lightning.yml` onto the existing Bitcoin regtest
compose file and uses Bitcoin Core regtest as the funding/mining source. The
CLN overlay defaults to the pinned `elementsproject/lightningd:v25.05` image;
set `KASSIBER_REGTEST_CLN_IMAGE` to test a different CLN build intentionally.
The LND backup node defaults to `lightninglabs/lnd:v0.18.4-beta`; set
`KASSIBER_REGTEST_LND_IMAGE` to test a different LND build intentionally.

Run:

```bash
./scripts/integration-harness.sh lightning-business
```

The lane creates these Docker-only actors:

- `cln_merchant` — the operational Core Lightning merchant node Kassiber syncs
- `cln_customer` — pays merchant invoices and routed supplier invoices
- `cln_supplier` — receives merchant expense payments
- `cln_router` — provides the extra hop for routed payments and fee rows
- `lnd_merchant_backup` — a live LND backup merchant node connected to
  `cln_merchant` and `cln_router` through funded private channels

Kassiber stores two Lightning backend/wallet rows: the operational
`cln_merchant` Core Lightning source and the `lnd_merchant_backup` LND source.
The CLN backend's `lightning_cli` points at
`dev/regtest/lightning-cli-merchant.sh`; customer, supplier, and router are
never created as Kassiber wallets or connections. The LND backend stores the
loopback REST URL plus the node's disposable read-only macaroon from the
regtest volume.

The bootstrap script is idempotent: it waits for CLN and LND, creates/reuses the
Bitcoin faucet wallet, funds CLN/LND wallets only below the threshold, opens the
private `merchant -- lnd_merchant_backup` and `lnd_merchant_backup -- router`
backup channels plus the public `customer -- merchant -- router -- supplier`
channels if absent, mines confirmations, and waits for normal/active channels. The scenario then generates a
seeded business plan at `$KASSIBER_LIGHTNING_BUSINESS_PLAN` (default:
`$KASSIBER_LIGHTNING_BUSINESS_HOME/business-plan.json`) and executes the
stable-label activity from that plan:

- customer-paid merchant invoices (`merchant-pos-sale-*`) with varied amounts
- merchant-paid supplier invoices routed through the router
- customer/router third-party payments that cross the merchant as forwards
- LND backup receipts and outbound LND payments so the backup-node dashboard
  has real invoice/payment activity
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
and forwarding activity, while the LND snapshot has two private channels plus
paid invoice and completed payment evidence. Persisted Lightning records do not store raw RPC JSON;
channel lifecycle rows retain only the node-observed Bitcoin chain/network
scope required for safe L1 identity matching. AI-safe Lightning payloads omit
sensitive route, peer, preimage, payment-secret, bolt11, funding-outpoint, and
failure-source fields.

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
- descriptor-backed Liquid wallet creation on `elementsregtest`, with LBTC
  receipts/spends and BTC<->LBTC bridge legs mined on the local `elementsd`
  chain before import through the same `wallets sync --all` path
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
- solo-mining block rewards swept into treasury at two points in history
  (visibly smaller after regtest halvings), while the dedicated mining wallet
  stays intentionally tiny; immature coinbases must never import
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
  BTC -> L-BTC `chain-swap`, and `peg-out`) are backed by real Bitcoin Core
  and elementsregtest transactions so the generic-tax demo exercises taxable
  cross-asset swaps without synthetic transaction ids
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

Prerequisites on any machine: Docker (Desktop or engine), Python 3, `uv`, and
`pnpm`.
From a fresh clone, install the project/runtime dependencies once:

```bash
./scripts/bootstrap-dev-env.sh
pnpm --dir ui-tauri install --frozen-lockfile
```

After that, the two commands above take you to a browser preview backed by the
real Python daemon reading a multi-year regtest book. The harness checks for
core Python dependencies and fails with a setup hint instead of silently using
an unprepared system interpreter.

What `demo-up` does:

- starts (or reuses) the regtest Compose stack under the fixed Compose project
  `kassiber-regtest-demo`, separate from the per-worktree test projects, enables
  the `silent-payments` profile, and leaves Bitcoin Core, Elements, Fulcrum,
  Frigate, the Core Lightning overlay, and the local protocol API services
  running;
- builds the `full-accounting-v1` book once into
  `~/.kassiber/regtest-demo/data` (override with
  `KASSIBER_REGTEST_DEMO_HOME`) and reuses it on later runs while the
  scenario file is unchanged; set `KASSIBER_REGTEST_DEMO_REBUILD=1` to force
  a rebuild;
- seeds the Lightning business topology into that same demo book by default:
  `cln_merchant` appears as a `coreln` wallet/backend in
  `Regtest Demo / Full Accounting`, while `cln_customer`, `cln_supplier`, and
  `cln_router` remain Docker-only actors. Set
  `KASSIBER_REGTEST_DEMO_LIGHTNING=0` to opt out of the CLN overlay and
  Lightning seed when you need a lighter Bitcoin/Liquid-only demo;
- persists the generated regtest RPC credentials in
  `~/.kassiber/regtest-demo/demo-manifest.json` (mode 600, regtest-only
  throwaway secrets) so restarts keep matching the book's stored backend and
  refresh/sync from the GUI keeps working;
- keeps the demo Core wallets loaded (`--keep-core-wallets`) so incremental
  syncs from the app keep seeing new activity.

The `fulcrum` container is provisioned and exposed as the
`bitcoin-electrum-regtest` backend row. The full-accounting demo assigns the
active `treasury_2020`, `merchant_2022`, and `cold_2024` wallets to Fulcrum while
the remaining ordinary Bitcoin wallets stay on Core RPC. The slow Bitcoin lane
also includes a dedicated Fulcrum/Electrum parity slice that syncs the same real
address wallet
through Core RPC and Electrum, then compares the persisted transaction and UTXO
views after receipts, a spend, an incremental receipt, and a no-op sync. The
Liquid Electrum and Liquid mempool rows are local services
backed by `elementsd`; every Liquid wallet in the demo is descriptor-backed and
syncs real elementsregtest LBTC transactions. The preview stays repeatable
without inventing Liquid transaction ids or contacting public mainnet explorers.
The stored default backend is `bitcoin-mempool-regtest`: wallet configs still
pin their sync source (`core-regtest` or `bitcoin-electrum-regtest` for ordinary
Bitcoin, `bitcoin-frigate-regtest` for the Silent Payments wallet), while
graph-capable UI paths prefer the local
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

### Large-book sync benchmark

The July 2026 performance slice used the checked-in full-accounting scenario on
the same local Docker stack before and after the changes. The resulting book
contained 998 transaction rows across 13 wallets after the built-in activity
tick and final indexer catch-up. Three active Bitcoin wallets used Fulcrum, the remaining ordinary Bitcoin
wallets used Core RPC, and Liquid wallets continued to use their Electrum
backend. Each reported resync figure is the median of repeated runs against the
same chain and SQLite book:

| Path | Before | After | Change |
| --- | ---: | ---: | ---: |
| CLI `wallets sync --all`, unchanged chain (5 runs) | 12.156 s | 2.461 s | 79.8% faster |
| Desktop freshness sync, unchanged chain (5 runs) | 2.966 s | 1.878 s | 36.7% faster |
| Desktop forced full replay, 594 Fulcrum transaction fetches / 642 records (3 runs) | — | 5.265 s | reference point |

The unchanged-chain runs imported zero rows, wrote zero false updates, and
fetched zero Fulcrum transaction bodies. The forced replay also wrote zero
false updates. End-to-end `demo-full` setup remains roughly 16 minutes on the
benchmark host because broadcasting and mining about 1,000 real operations over
the historical 50-block workload dominates that lane. Treat setup generation
and wallet resync as separate measurements when investigating regressions.

Poke the node like BTCPayServer's `docker-bitcoin-cli.sh`:

```bash
./dev/regtest/bitcoin-cli.sh getblockchaininfo
./dev/regtest/bitcoin-cli.sh -generate 1
uv run --locked python -m kassiber --data-root ~/.kassiber/regtest-demo/data reports summary
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
Bitcoin Fulcrum, and local mempool/esplora-compatible endpoints, then sync
descriptor-backed elementsregtest/LBTC demo wallets through the local Liquid
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
