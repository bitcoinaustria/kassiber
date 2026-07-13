# Chain observers

This document is the current-truth migration map for Kassiber's Bitcoin and
Liquid chain observers. `TODO.md` remains the executable checklist. This file
defines the supported capability boundary, the code being replaced, and the
persistence and privacy contracts that the replacement must preserve.

The migration started from commit
`5d232097506c6cab904bd02c5b5e2e94404c5ed4` on
`codex/dependency-chain-observers`. It is one deliberately broad change set with
phase checkpoint commits; it is not a shadow-observer rollout.

## Target boundary

`bdkpython` (`bdk_wallet`) will own supported Bitcoin descriptor-wallet chain
state for Esplora and Electrum. `lwk` (`lwk_wollet`) will own supported Liquid
descriptor-wallet chain state. Kassiber continues to own:

- wallet/source configuration and the SQLCipher security boundary;
- normalized accounting transactions, graph evidence, retractions and review;
- custody components and legal or beneficial ownership;
- UTXO presentation, source-overlap policy and derivation ownership;
- transfer, swap, loan, quarantine and tax projection;
- freshness scheduling, progress envelopes and backend privacy policy.

The wrappers are observation-only. They do not expose address issuance,
transaction construction, coin selection, PSBT/PSET creation, signing or
broadcasting. Dependency state is serialized into the Kassiber SQLCipher
database. No dependency-created wallet file, side database, cache or state
directory is permitted.

Fetch and scanning finish before a SQLite write transaction begins. Applying a
successful refresh is one transaction containing observer state, normalized
transactions, retractions/replacements, output inventory, derivation coverage,
and the freshness checkpoint. A failed or cancelled apply rolls back all of
them.

## Initial capability matrix

This is a routing policy, not a claim that migration is already complete.
Every dependency-backed cell must be proven by executable contract and regtest
tests before its route is enabled. Capability selection occurs before network
access; a dependency failure is never retried silently through compatibility
code.

| Chain/source | Configuration | Planned observer | Initial status |
| --- | --- | --- | --- |
| Bitcoin Esplora | supported watch-only descriptor, normal platform trust | BDK | planned; dependency contract and parity tests required |
| Bitcoin Electrum | supported watch-only descriptor over TCP or normal TLS | BDK | planned; TCP/TLS and proxy contract tests required |
| Bitcoin Esplora/Electrum | SOCKS proxy supported by the tested binding | BDK | conditional; must preserve per-backend proxy and `KASSIBER_NO_EGRESS` |
| Bitcoin Esplora/Electrum | custom CA unsupported by the tested binding | named compatibility observer | permitted only after executable capability preflight |
| Bitcoin Core RPC | descriptor, xpub or address watch source | existing `bitcoinrpc` adapter | explicit compatibility route; not migrated to BDK |
| Bitcoin Silent Payments | BIP352/BIP392 material | dedicated Silent Payments path | explicit compatibility route |
| Bitcoin | spending-private descriptor/key material | none | always rejected before network access |
| Liquid Electrum/Esplora | LWK-supported watch-only descriptor with private view/blinding material | LWK | planned; exact descriptor/backend support must be contract-tested |
| Liquid | fixed descriptors | LWK when executable test passes | expected supported; do not route until proven |
| Liquid | Taproot descriptor | LWK when executable test passes | expected supported despite stale upstream error variants |
| Liquid | general pre-SegWit descriptor | named compatibility observer | expected unsupported by LWK; preflight required |
| Liquid | noncanonical multipath descriptor | named compatibility observer | expected unsupported by LWK; preflight required |
| Liquid | proxy or custom-CA backend unsupported by the binding | named compatibility observer | preflight required; never downgrade after a failed LWK call |
| Liquid | spending-private descriptor/key material | none | always rejected; private blinding/view material remains allowed and sensitive |

`embit` remains available for compatibility parsing, intentionally unsupported
descriptor forms, Liquid transaction decoding, HTLC parsing and specialized
primitives not replaced by BDK/LWK.

## Manual observer inventory

### Descriptor parsing, derivation and branch management

- `kassiber/wallet_setup.py`: `normalize_wallet_material`,
  `parse_bsms_descriptor_record`, `_descriptors_from_json`,
  `_descriptors_from_text`, `_descriptors_from_slip132`, and bare-xpub
  rendering normalize ingress material.
- `kassiber/wallet_descriptors.py`: `load_descriptor_plan`,
  `enabled_script_branches`, `_promote_receive_only_to_multipath`,
  `branch_limits`, `derive_descriptor_target`, `derive_descriptor_targets`,
  `liquid_plan_can_unblind` and `liquid_blinding_secret` implement the current
  descriptor model and derivation geometry.
- `kassiber/core/sync_backends.py`: `sync_target_from_derived`,
  `resolve_wallet_sync_targets` and `detect_active_script_types` turn that
  geometry into backend scan targets.

Compatibility parsing may remain, but supported BDK/LWK descriptors must be
validated as watch-only and converted once at the wrapper boundary. Kassiber's
manual derived-target list must not remain the production state engine for
those routes.

### Gap discovery and derivation coverage

- `scan_descriptor_targets` performs branch-by-branch gap walking and reuses a
  manual `highest_used` map.
- `discover_descriptor_targets` probes Esplora stats or Electrum subscription
  status, while Bitcoin Core builds targets from imported range ends.
- `_highest_used_branch_index`, `_merge_highest_used`,
  `_bitcoinrpc_descriptor_end`,
  `_bitcoinrpc_descriptor_targets_for_checkpoint`, and
  `_bitcoinrpc_highest_used_from_details` maintain manual coverage state.
- `core.sync._negative_balance_rescan_gap_limit` and
  `_wallet_with_temporary_gap_limit` implement a second repair scan policy.

BDK/LWK replace this machinery for supported routes. Kassiber persists a
redacted coverage projection for inventory/ownership UI, not a competing scan
checkpoint.

### Esplora and Electrum history checkpoints

- Esplora uses `esplora_scripthash_stats`, `esplora_stats_fingerprint`,
  `fetch_esplora_history`, `esplora_records_for_wallet` and the
  `esplora_scripthashes` checkpoint map.
- Electrum uses the local `ElectrumClient`, `electrum_call_many`, subscription
  statuses, history/header caches, `electrum_records_for_wallet`, and the
  `electrum_scripthash_statuses` checkpoint map.
- `core.freshness` stores those backend-specific maps in
  `freshness_source_states.checkpoint_json`.

The dependency observer blob becomes the authoritative supported-route chain
state. The freshness checkpoint may retain scheduling metadata, but not a
parallel transaction/UTXO state machine.

### Transaction graph, confirmation and replacement state

- Bitcoin Esplora normalization lives in `record_from_bitcoin_esplora_tx`.
- Bitcoin Core replacement/retraction handling spans
  `fetch_bitcoinrpc_wallet_transactions`, `_bitcoinrpc_retracted_txids`,
  `_bitcoinrpc_normalized_graph`, `bitcoinrpc_records_for_wallet` and
  `bitcoinrpc_sync_adapter`.
- Electrum graph reconstruction spans `decode_raw_transaction`,
  `_normalize_electrum_bitcoin_graph_for_storage`,
  `record_from_electrum_tx` and `electrum_records_for_wallet`.
- `transaction_graph_cache`, `core.imports.retract_wallet_records`, and
  transaction upserts persist the accounting projection.

For supported BDK/LWK routes, dependency canonical transactions and chain
positions drive confirmation, reorg and replacement deltas. Kassiber still
normalizes the resulting accounting rows and graph evidence.

### UTXO lifecycle

- `esplora_utxos_for_wallet`, `electrum_utxos_for_wallet` and
  `bitcoinrpc_utxos_for_wallet_name` construct current-output snapshots.
- `core.output_inventory.update_wallet_output_inventory` normalizes them,
  upserts `wallet_utxos`, marks missing outputs spent per source, and updates
  `wallet_utxo_refreshes`.
- `clear_wallet_output_inventory` and `clear_backend_output_inventory` remove
  derived state after material/backend changes.

The tables and safe presentation remain Kassiber-owned. Supported observers
must provide one complete output projection inside the atomic refresh result.

### Derivation ownership and source overlap

- `core.ownership.build_owned_index`, `_seed_from_inventory`,
  `_seed_from_transactions`, `_seed_transaction_outpoints`,
  `_derive_wallet_into_index`, `classify_txid` and `identify` build the local
  ownership graph.
- `core.source_overlap._descriptor_config_scripts`,
  `scripts_from_sync_state`, `detect_profile_source_overlaps`,
  `filter_sync_state_for_canonical_owner` and
  `apply_address_list_overlap_repairs` decide canonical source ownership.
- Wallet config retains bounded historic ownership material when a descriptor
  changes.

Kassiber keeps ownership policy. BDK/LWK replace manual supported-route
derivation and supply coverage/output facts; they do not decide beneficial
ownership or source precedence.

### Liquid decoding, unblinding and state

- `wallet_descriptors.normalize_descriptor_text`, `liquid_plan_can_unblind`,
  `liquid_blinding_secret` and `decode_liquid_transaction` provide current
  parsing and secrets.
- `_liquid_utxo_record_from_output`, `liquid_output_amount_asset_id`,
  `record_components_from_liquid_tx`, `esplora_records_for_wallet` and
  `electrum_records_for_wallet` manually fetch, decode, unblind and correlate
  Liquid inputs/outputs.
- `wallet_policy_asset_id`, `liquid_asset_code`, and the DB compatibility
  backfills map policy-asset identifiers to `LBTC`.

LWK becomes the authoritative supported-route wollet state and unblinding
engine. Private view/blinding material stays inside SQLCipher and must never
enter logs, diagnostics, audit packages, AI tools, daemon/event payloads or
replication.

## Descriptor ingress inventory

All ingress paths must pass one watch-only capability validator before storage
or network access:

- CLI `wallets create` and `wallets update` via
  `core.wallets.parse_wallet_config`, including `--descriptor-file`,
  `--change-descriptor-file`, stdin/fd material and config JSON/files.
- Desktop/daemon `ui.wallets.create`, `ui.wallets.update`,
  `ui.wallets.preview_descriptor` and `ui.wallets.detect_script_types` via
  `_wallet_config_from_ui_args`, `_apply_wallet_material_config`,
  `_preview_descriptor_payload` and `_detect_script_types_payload`.
- JSON/Core/Sparrow-style descriptor exports and free-form descriptor files via
  `wallet_setup.normalize_wallet_material`.
- BSMS descriptor records via `parse_bsms_descriptor_record`.
- Bare xpub/tpub plus explicit script types and SLIP-132 ypub/zpub/upub/vpub
  compatibility input.
- Samourai/Whirlpool public source sets via
  `core.samourai.import_samourai_wallet_group`, explicit descriptor sources,
  and account-xpub rendering.
- Bull Bitcoin wallet CSV connections are file import/enrichment routes; any
  descriptor material added by a future Bull Bitcoin export must use the same
  validator rather than bypassing wallet setup.
- Legacy/custom importers that populate `wallets.config_json` or call
  `create_wallet`/`update_wallet`, including compatibility source files.

Spending-private Bitcoin or Liquid descriptor material is rejected. Liquid
private blinding/view keys are allowed but classified as sensitive.

## Watch-only enforcement

Phase 2 centralizes the spending-key boundary in
`kassiber/wallet_security.py`. Wallet-export normalization performs a
best-effort parsed-key preflight without breaking placeholder-only legacy
exports; `load_descriptor_plan` performs the authoritative strict parse and
checks every spending key before derivation or network access. The check covers
primary and separate change descriptors, nested descriptors, multisig,
multipath, WIF and extended private keys. A value in the nominal `xpub` field
is parsed as untrusted key material even when the rest of that config is too
incomplete to form a plan.

Rejected material raises the stable
`wallet_spending_private_material` error with a secret-free remediation hint.
The error never includes the submitted value in its message, hint or details;
CLI and daemon envelopes preserve that code. Stored unsafe configurations are
not neutered or migrated: descriptor preview, wallet update and backend sync
fail closed. Liquid descriptor spending keys must be public, while private
SLIP-77 blinding/view material remains accepted inside SQLCipher. Silent
Payments retains its narrower exception for private scan material and rejects
private spend leaves, including leaves nested in spend-key expressions.

## Current refresh commit boundaries

The current single-wallet chain refresh is not atomic:

1. `cli.handlers.sync_wallet` calls `core.sync.sync_wallets` with
   `_wallet_sync_hooks(commit=True)`.
2. `insert_records` reaches `core.imports.import_records_into_wallet` and may
   call `conn.commit()`.
3. `update_output_inventory` reaches
   `core.output_inventory.update_wallet_output_inventory` and may call a
   second `conn.commit()`.
4. `_mark_wallet_synced_from_results` writes wallet/freshness state, followed
   by the caller's final `conn.commit()`.

The `--all` path already creates a per-wallet savepoint and supplies
`commit=False`, but that is an orchestration accident rather than a shared
contract. Other wallet-refresh dispatches can also commit through their hooks:

- file imports through `core.imports`;
- BTCPay sync and BTCPay/Bull Bitcoin enrichment;
- Core Lightning and LND sync adapters;
- output-inventory updates.

The observer migration removes independent commits from chain-refresh
sub-hooks. One apply coordinator owns `BEGIN`/savepoint, rollback and commit for
observer state, transaction insertion/retraction, graph cache, inventory,
coverage and freshness. Backend access, retries, sleeps and scans occur before
that transaction.

## Apple Silicon packaging checkpoint

Phase 1 removes `macos-15-intel`, the `x86_64-apple-darwin` sidecar,
`macos-universal`, `--target universal-apple-darwin`, the supervisor's Intel
sidecar dispatch, and universal Homebrew/release documentation. New desktop
and Homebrew artifacts are explicitly `macos-arm64`; the historical universal
release remains available for Intel users but is not rebuilt.

The supported release targets after the packaging phase are macOS ARM64,
Linux x86_64 and Windows x86_64.

## Existing test inventory

- Descriptor parsing/derivation: `tests/test_wallet_setup.py`,
  `tests/test_wallet_descriptors.py`, `tests/test_wallet_config_multi_script.py`,
  `tests/test_daemon_descriptor_preview.py`, and daemon smoke descriptor flows.
- Sync/discovery/checkpoints: `tests/test_sync_backends.py`,
  `tests/test_sync_backends_legs.py`, `tests/test_daemon_detect_script_types.py`,
  and `tests/test_source_overlap.py`.
- RBF, reorg and replacement: Bitcoin Core conflicted/retracted/checkpoint
  cases in `tests/test_sync_backends.py`, live RBF/reorg assertions in
  `tests/integration/test_live_bitcoin_core_regtest.py`, regtest tapes, and the
  `spending_rbf_replaced_payment` full-demo operation.
- Output lifecycle: `tests/test_output_inventory.py`, Esplora/Electrum/Core
  UTXO cases in `tests/test_sync_backends.py`, and live Core/demo assertions.
- Ownership and overlap: `tests/test_ownership.py`,
  `tests/test_ownership_transfers.py`, `tests/test_rp2_ownership_transfers.py`,
  and `tests/test_source_overlap.py`.
- Liquid: `tests/test_liquid_electrum_sync.py`, Liquid cases in
  `tests/test_wallet_descriptors.py`, `tests/integration/boltz_liquid_regtest.py`,
  and the Elements-backed `demo-full` lane.
- Persistence/atomicity: importer `commit=False` tests, freshness tests,
  output-inventory refresh tests, and sync prefetch/savepoint tests. New fault
  injection is required for observer-blob rollback and restart reconstruction.
- Packaging: `.github/workflows/prerelease-binaries.yml` packaged descriptor
  smokes, `tests/test_rp2_packaging.py`, Homebrew cask rendering tests, Tauri
  supervisor sidecar tests, and the quality-gate workflow validation.

## Required migration proof

Each new route needs dependency API contract tests, privacy/egress routing
tests, deterministic normalization fixtures, old-versus-new comparison only in
tests, regtest confirmation/RBF/reorg/restart coverage, fault injection around
every apply stage, and packaged-binary smoke on each supported release target.
Compatibility routes remain narrow, named and tested; unsupported
configurations are never described as migrated.
