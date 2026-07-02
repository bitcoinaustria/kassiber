# TODO: Lightning in the regtest harness

Plan for adding Lightning Network coverage to the integration harness and the
demo book, modeled on BTCPayServer's test stack
(`BTCPayServer.Tests/docker-compose.yml` + `docker-lightning-channel-setup.sh`
in <https://github.com/btcpayserver/btcpayserver>). Status: not started; the
notes below capture what was learned from BTCPayServer's setup and from
Kassiber's existing Core Lightning adapter so the work can start without
re-research.

## What Kassiber already has (integration points)

- Backend kind `coreln` (`normalize_backend_kind` maps `core-ln`/
  `core-lightning`); the adapter shells out to `lightning-cli --json --raw`
  via `subprocess.run` (`kassiber/core/lightning/cln.py`), in either local
  RPC-socket mode (`lightning_dir`/`rpc_file`) or remote commando-rune mode.
- The CLI binary is configurable per backend (`config.lightning_cli`,
  `cln.py` `_resolve_lightning_cli`) — this is the hook that makes a
  **dockerized** CLN node syncable: point `lightning_cli` at a wrapper script
  that runs `docker compose exec -T <node> lightning-cli "$@"`.
- Read-only allowlist enforced at the call boundary (`CLN_ALLOWED_METHODS`:
  getinfo, listfunds, listpeerchannels, listforwards, listpays, listinvoices,
  bkpr-listincome, bkpr-listbalances). Assertions must never need pay/open/
  close through Kassiber.
- Sync imports received invoices as `cln_invoice` wallet rows; forwards are
  **daily-aggregated per channel** (never itemized rows); opsec discards
  (preimages, payment_secrets, routes, erring nodes) happen at the adapter
  boundary. LND (`lnd`), NWC (`nwc`) have adapters; Phoenix is CSV-import
  only.
- `kassiber/core/htlc_parser.py` recovers payment hashes from Boltz-style
  HTLCs for submarine-swap pairing between LN and on-chain legs.

## What to copy from BTCPayServer (and what to skip)

- **Two CLN nodes minimum** (`btcpayserver/lightning:v25.05` images, pinned)
  with dev flags that make regtest Lightning fast and reliable:
  `--developer --funding-confirms=1 --dev-fast-gossip --dev-bitcoind-poll=1`.
  BTCPay runs 4 nodes (CLN+LND × merchant/customer); start with 2 CLN and add
  an LND pair only when the LND adapter gets live-sync coverage.
- **Channel bootstrap as an idempotent "ensure" step**, not a one-shot:
  BTCPay's `ServerTester.EnsureChannelsSetup()` calls `ConnectChannels.
  ConnectAll` (from the separate BTCPayServer.Lightning library) at the start
  of every Lightning test; it reuses existing channels when capacity is
  already there. Model: connect peers → fund node wallets from the faucet →
  `fundchannel` → mine confirmations → wait for `CHANNELD_NORMAL`.
  BTCPay's shell variant is `docker-lightning-channel-setup.sh` (fund 0.615
  BTC per node, 5M-sat channels with ~2.45M sats pushed to the far side so
  both directions can pay immediately, mine ~10 blocks, verify with a test
  payment).
- **Wrapper CLIs** per node (their `docker-customer-lightning-cli.sh`
  pattern) — we already do this for bitcoind (`dev/regtest/bitcoin-cli.sh`);
  add `lightning-cli-1.sh` / `lightning-cli-2.sh`. The same wrapper doubles
  as the `lightning_cli` binary for the Kassiber backend.
- **Skip**: NBXplorer (Kassiber talks to Core directly), postgres perf
  tricks, Tor, MailPit, the shared `bitcoin_datadir` volume trick (their LN
  nodes read bitcoind's dir directly; ours can use rpcauth credentials
  passed as env, consistent with the existing compose file).

## Concrete TODOs

1. **Compose overlay** `dev/regtest/compose.lightning.yml`: services
   `lightningd_1`, `lightningd_2` (pinned `btcpayserver/lightning` tag),
   regtest + dev flags above, `--bitcoin-rpcconnect=bitcoind` with the same
   rpcauth credentials, per-node named volumes, loopback-only published P2P
   ports. Harness gains `--with-lightning` (or `KASSIBER_REGTEST_LIGHTNING=1`)
   so `demo-up`/`bitcoin-core` lanes can add the overlay with
   `-f compose.bitcoin.yml -f compose.lightning.yml`.
2. **Channel bootstrap** in the demo runner (or `tests/integration/
   lightning_regtest.py`): idempotent ensure-channel step + activity
   generation — a handful of invoices paid node2→node1 (become `cln_invoice`
   rows), a few node1→node2 pays, and **forwards**: needs a third hop or a
   circular route; simplest is faucet-CLN as a third node later — start with
   direct pays and mark forwards coverage as phase 2.
3. **Scenario manifest v2**: optional `lightning` section (nodes, channels,
   invoice/pay schedules with msat amounts and deterministic descriptions);
   validation fails closed like the existing sections.
4. **Kassiber wiring in the demo book**: `backends create <name> --kind
   core-ln` with `lightning_cli` pointing at the wrapper script +
   `lightning_dir`/`rpc_file` for the socket inside the container; wallet
   kind `coreln` in a new `lightning` account; `wallets sync` then must
   produce `cln_invoice` rows, node snapshots, and zero quarantines.
5. **Fast-lane tape**: a `ClnCliTape` twin of `BitcoinRpcTape` that fakes
   `call_core_lightning` (method+args → recorded JSON) so invoice import,
   daily forward aggregation, and opsec discards replay in the gate without
   Docker; fixture `tests/fixtures/regtest_tapes/cln_node_baseline.json` with
   the same provenance/fail-closed rules.
6. **Assertions to pin** (from the adapter's actual semantics):
   - received invoices → `cln_invoice` inbound rows, priced, 0 quarantines;
   - forwards aggregate per-channel-per-day in `lightning_node_*` tables and
     never become wallet rows;
   - snapshots carry no preimage/payment_secret/route/erring-node fields;
     private channels surface `peer_pubkey=None`;
   - a Boltz-shaped submarine swap staged on-chain pairs with the LN leg via
     `htlc_parser` (cross-asset swap coverage, extends the existing
     `swap_bridges`). The upstream-Docker guard now lives in
     `./scripts/integration-harness.sh boltz-liquid`: it starts or reuses
     `BoltzExchange/regtest`, verifies Liquid-capable Boltz pair metadata
     against the demo's Boltz-marked BTC -> L-BTC bridge, executes an
     L-BTC -> BTC Lightning submarine swap, imports Liquid/LN rows generated
     from the observed txids/hash/amounts into Kassiber, and asserts the swap
     is paired while a separate Liquid payment remains unpaired.
   - remaining Boltz live work: execute reverse and chain swaps through Boltz's
     official client/SDK so Kassiber does not reimplement claim/refund signing
     and recovery state machines in the test harness.
7. **macOS caveat to verify early**: bind-mounted unix sockets do not work
   through Docker Desktop's file sharing — the local `rpc_file` mode
   probably cannot cross the container boundary on macOS. The `docker
   compose exec` wrapper (or commando-rune mode over the P2P port) is the
   portable path; decide after testing both on Linux + macOS.
8. **Demo book realism**: recurring Lightning income (invoice per stress
   cycle), occasional channel open/close (on-chain fee rows), one submarine
   swap bridging the spending wallet to the LN node.
