# Chain observers

This document is the current-truth migration map for Kassiber's Bitcoin and
Liquid chain observers. `TODO.md` remains the executable checklist. This file
defines the supported capability boundary, the code being replaced, and the
persistence and privacy contracts that the replacement must preserve.

The migration started from commit
`5d232097506c6cab904bd02c5b5e2e94404c5ed4` on
`codex/dependency-chain-observers`. It is one deliberately broad change set with
phase checkpoint commits; it is not a shadow-observer rollout.

This implemented decision supersedes earlier planning guidance that avoided
BDK/LWK adoption. The replacement is accepted because dependency state now
stays inside SQLCipher through custom persistence callbacks, supported routes
have live restart/reorg/replacement parity oracles, and Core RPC plus Silent
Payments remain separate first-class observers rather than being forced
through descriptor discovery.

## Target boundary

Pinned `bdkpython` 3.0.0 (`bdk_wallet`) owns supported Bitcoin
descriptor-wallet chain state for Esplora and Electrum on CPython 3.10–3.13
macOS, Linux x86-64, and Windows AMD64, where the release ships native wheels.
Other runtimes use the named Bitcoin compatibility observer. Pinned `lwk` 0.18.0
(`lwk_wollet`) owns supported Liquid descriptor-wallet chain state for
Esplora and Electrum on platforms for which it ships a wheel; macOS Intel uses
the named Liquid compatibility observer. Kassiber continues to own:

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

Authority is scoped by connection model, not by chain alone:

| Configuration | Authoritative observer |
| --- | --- |
| Bitcoin descriptor + Esplora/Electrum | BDK where the pinned binding ships; named compatibility observer elsewhere |
| Liquid descriptor + Esplora/Electrum | LWK where the pinned binding ships; named compatibility observer on macOS Intel |
| Bitcoin Core backend | Bitcoin Core RPC adapter |
| Silent Payments wallet | BIP352/BIP392 Silent Payments scanner |

Core RPC and Silent Payments are independent first-class observers. They are
not compatibility fallbacks, and ordinary BDK descriptor discovery must not be
used as a substitute for Core-specific import/rescan behavior or BIP352 chain
scanning.

Fetch and scanning finish before a SQLite write transaction begins. Applying a
successful refresh is one transaction containing observer state, normalized
transactions, retractions/replacements, output inventory, derivation coverage,
and the freshness checkpoint. A failed or cancelled apply rolls back all of
them.

## Transactional observer contract

`kassiber/core/chain_observer/` is the dependency boundary below the CLI and
daemon layers. A request-scoped observer loads prior state and performs backend
preparation only while SQLite has no active transaction. It returns an explicit
JSON-safe `PreparedObserverUpdate`; dependency wallet, builder, signer, PSBT or
PSET objects never cross the boundary. Application is accepted only inside the
coordinator-owned wallet savepoint. It persists the new state and returns
normalized transaction, retraction, output, coverage and freshness facts for
Kassiber's existing projection stages.

Observer JSON state uses two private main-database tables:

- `chain_observer_instances` stores representation-versioned JSON state for a
  stable logical-wallet/source identity;
- `chain_observer_coverage` stores versioned per-branch scan coverage.

LWK additionally uses `chain_observer_values`, a versioned string-keyed byte
store implementing its `ForeignStore` callback. Values are opaque to Kassiber,
loaded only for the matching observer identity, and replaced only inside the
coordinator-owned apply savepoint. LWK never receives a filesystem path. A
rollback discards the request-local wollet and its buffered values.

Identities are derived from structural, non-secret source keys. A multi-script
xpub gets one instance per script family; receive/change branches remain
distinct within the instance. Samourai child sources retain their concrete
source-wallet identity while all map to the parent logical wallet. Descriptor
or xpub text is never hashed into the identity.

The store accepts JSON primitives only: no pickle, Python object serialization,
or dependency sidecar database. Unknown or mismatched state/coverage versions
raise `observer_state_rebuild_required` without returning the stored payload on
an ordinary refresh. A forced refresh treats incompatible state as absent in
memory, retains canonical txids from the freshness checkpoint for retractions,
and replaces encrypted observer rows only inside the successful wallet
savepoint. Fetch or apply failure leaves the old rows intact. Writes never
commit independently. A failed apply discards its request-local dependency
object. Wallet material changes,
wallet deletion and confirmed book reset remove observer rows; clearing this
derived state does not touch transactions, notes, attachments, edit history or
custody components.

These tables are absent from the positive replication allowlist and every
public snapshot, AI, diagnostics and audit query. Backup includes them only as
pages in the encrypted main SQLCipher database. Dependency and compatibility
facts cannot be applied together: the coordinator raises
`observer_projection_conflict` instead of running a shadow observer or falling
back after dependency application begins.

## Capability matrix

Capability selection occurs before network access; a dependency failure is
never retried silently through compatibility code.

| Chain/source | Configuration | Observer | Status |
| --- | --- | --- | --- |
| Bitcoin Esplora | supported watch-only descriptor, normal platform trust | BDK | enabled; BDK owns scan, canonical transaction, chain-position and output state |
| Bitcoin Electrum | supported watch-only descriptor over TCP or normal TLS | BDK | enabled; live descriptor restart/no-op coverage runs in the regtest observer lane |
| Bitcoin Esplora/Electrum | SOCKS proxy configured or `.onion` endpoint | named compatibility observer | BDK accepts only `socks5://`; Kassiber does not downgrade `socks5h://` and leak DNS outside Tor |
| Bitcoin Electrum | custom CA unsupported by BDK | Bitcoin script-protocol observer | the explicit Electrum client loads the configured CA; selected before connect |
| Bitcoin Esplora | custom CA unsupported by BDK and the compatibility HTTP client | none | fails before egress with `observer_capability_unsupported`; remove only after a dependency client can load the configured trust root |
| Bitcoin Esplora | custom HTTP authorization unsupported by the binding | named compatibility observer | selected before connect; never a runtime fallback |
| Bitcoin Esplora | non-default caller timeout unsupported by the binding | named compatibility observer | selected before connect so the configured timeout remains enforceable |
| Bitcoin Esplora/Electrum | finite source-overlap exclusion would require a partial descriptor scan | named compatibility observer | selected after local overlap policy and before connect |
| Bitcoin Esplora/Electrum | address-list source | Bitcoin script observer | BDK cannot reconstruct key semantics from an address; prefer descriptor migration when more wallet material exists |
| Bitcoin Core RPC | descriptor, xpub or address watch source | Bitcoin Core RPC observer | first-class Core route; BDK Python has no Bitcoin RPC chain source |
| Bitcoin Silent Payments | BIP352/BIP392 material | Silent Payments observer | first-class BIP352/BIP392 discovery route |
| Bitcoin `mempool` backend alias | supported descriptor | BDK Esplora | normalized once for capability selection, client construction, remote-tip checks and initial/incremental scans; the live restart oracle uses this alias |
| Bitcoin | spending-private descriptor/key material | none | always rejected before network access |
| Liquid Electrum/Esplora | watch-only confidential SegWit v0 (including nested `elsh(wpkh(...))`), Taproot, executable legacy P2SH, fixed or canonical `<0;1>` ranged descriptor; private view/blinding material with public spend keys | LWK 0.18.0 | enabled; only the outer Elements script wrapper is translated, while nested miniscript keeps Bitcoin spelling |
| Liquid Electrum/Esplora | macOS x86_64 package | named compatibility observer | LWK 0.18.0 publishes no Intel Mac wheel; universal packaging retains the existing observer for that architecture |
| Liquid address-list source | any | none | rejected locally because confidential outputs require descriptor-backed private view/blinding material |
| Liquid | unsupported general pre-SegWit descriptor | named compatibility observer | selected when executable LWK descriptor construction rejects the form; remove as upstream support becomes executable |
| Liquid | structurally equivalent separate `/0/*` receive + `/1/*` change descriptors | LWK 0.18.0 | canonicalized to `<0;1>` only when every ranged key has the same blinding policy, script, origins, keys, order and wildcard geometry |
| Liquid | genuinely different change policy or noncanonical multipath | named compatibility observer | retained because accepting a constructed descriptor is not proof of equivalent ownership |
| Liquid Esplora/Electrum | SOCKS proxy configured or `.onion` endpoint | named compatibility observer | LWK 0.18.0 Python transport cannot carry Kassiber's proxy policy; compatibility preserves it until the binding exposes proxy configuration |
| Liquid Electrum | custom CA unsupported by LWK | named script-protocol observer | the explicit Electrum client loads the configured CA; selected before connect |
| Liquid Esplora | custom CA unsupported by LWK and the compatibility HTTP client | none | fails before egress with `observer_capability_unsupported` rather than ignoring the requested trust root |
| Liquid Esplora | bearer header or static token | LWK 0.18.0 | passed through `EsploraClientBuilder.headers` / `token_provider`; credentials remain inside SQLCipher and redacted errors |
| Liquid Electrum | platform-trusted TLS | LWK 0.18.0 | uses the explicit TLS/domain-validation constructor with validation enabled |
| Liquid Electrum | explicit `insecure` TLS opt-in | named script-protocol observer | retained because LWK 0.18.0 pins rust-electrum-client 0.21.0, whose Rustls no-verification implementation advertises no signature schemes and fails a real TLS handshake; remove after the packaged binding includes the upstream verifier fix and the local dependency-direct probe passes |
| Liquid Electrum | non-default timeout | named compatibility observer | selected before connect so the configured bound remains enforceable; remove when the binding exposes a timeout |
| Liquid Esplora/Electrum | finite source-overlap exclusion would require a partial descriptor scan | named compatibility observer | selected after local overlap policy and before connect; remove when partial descriptor ownership can be represented safely |
| Liquid | backend other than Esplora/Electrum | none | rejected by wallet/backend validation |
| Liquid | spending-private descriptor/key material | none | always rejected; private blinding/view material remains allowed and sensitive |

`embit` remains available for compatibility parsing, intentionally unsupported
descriptor forms, Liquid transaction decoding, HTLC parsing and specialized
primitives not replaced by BDK/LWK.

### BDK persistence representation

`kassiber/core/chain_observer/bdk_persistence.py` implements BDK's custom
`Persistence` callback without giving BDK a database path. Schema version 1
serializes every aggregate `ChangeSet` component explicitly: public
descriptors, network, local-chain changes, full transactions, floating txouts,
anchors, first/last-seen and eviction times, descriptor-index state, and locked
outpoints. Transaction and identifier bytes are hex; values and timestamps are
integers. Root, component and item fields are exact, so missing or future
fields require an explicit rebuild/migration rather than being silently
dropped. Pickle and opaque native blobs are prohibited.

The request-local callback merges staged BDK changes in memory. Only the
coordinator's apply savepoint writes its JSON payload to
`chain_observer_instances`, alongside normalized facts and branch coverage.
An immediate no-op therefore reloads the same aggregate from SQLCipher and
persists byte-for-byte equivalent canonical JSON without a BDK sidecar.

Initial import, forced rebuild and a disconnected-chain rebuild use BDK full
scan. Ordinary refreshes reveal a complete unused gap beyond every known used
index and use `start_sync_with_revealed_spks`; discovery widens and repeats the
incremental request only when a newly revealed address is used. This prevents
full descriptor rescans on every refresh without reintroducing a manual gap
engine.

The exact 3.0.0 Python binding bundles an Electrum client from before the
upstream stale-anchor rollback fix. Before an incremental Electrum sync,
Kassiber compares the persisted BDK tip with the remote header through BDK's
client API. A height rollback or disconnected tip rebuilds a fresh watch-only
BDK wallet from the same public descriptors and replaces the derived aggregate
atomically. It does not invoke the compatibility protocol client, retain a
second graph, or alter authored accounting evidence.

The live oracle waits for Electrum Merkle state before comparing transport
projections so backend indexing races are not mistaken for observer state.
The incremental request proves mempool-to-confirmed transitions without the
former second full scan.

### LWK persistence representation

`kassiber/core/chain_observer/lwk_persistence.py` implements LWK 0.18.0's
`ForeignStore` without a side database or data directory. The callback buffers
opaque string-keyed byte values in a request-local map during fetch and scan.
Only a successful observer apply copies that map into version-1
`chain_observer_values` rows under the same wallet savepoint as normalized
transactions, retractions, inventory, coverage and freshness. `put` and
`remove` never commit independently.

The public JSON observer state stores only Kassiber's representation version
and canonical txids used for retraction. LWK's own bytes are not JSON-decoded,
logged, replicated, exported, exposed over desktop IPC, or returned to AI
tools. Unknown namespace versions fail with
`observer_state_rebuild_required`; clearing and rebuilding this derived state
does not alter authored transaction metadata.

LWK exposes one explicit fee output for the whole Elements transaction. When
every input belongs to the observed wollet, Kassiber can assign that fee to the
wallet row. For mixed-input transactions it instead records the exact wallet
delta with `amount_includes_fee=true`, keeps the whole transaction fee only as
graph evidence, and lets transfer/custody conservation require an explicit
component rather than charging the entire fee to one participant.

Elements consensus values are stored as integer 1e-8 base units for every
asset; Kassiber preserves that canonical unit even when an issued asset has
separate display metadata. The pinned binding declares
`WalletTxOut.wildcard_index() -> int`, so fixed-descriptor outputs do not expose
an optional-index state in this version. Forced rebuilds compare the backend
tip height with the persisted checkpoint before scanning, preventing a lagging
backend from retracting newer facts.

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

- The former `scan_descriptor_targets` network gap walker and the
  Esplora/Electrum branches of `discover_descriptor_targets` are deleted.
  Supported BDK/LWK observers now own gap discovery and chain position.
- `discover_bitcoinrpc_descriptor_targets` remains a local range calculation
  for the explicitly separate Bitcoin Core adapter; it performs no RPC call.
- `_highest_used_branch_index`, `_merge_highest_used`,
  `_bitcoinrpc_descriptor_end`,
  `_bitcoinrpc_descriptor_targets_for_checkpoint`, and
  `_bitcoinrpc_highest_used_from_details` maintain manual coverage state.
- `core.sync._negative_balance_rescan_gap_limit` and
  `_wallet_with_temporary_gap_limit` implement a second repair scan policy.

BDK/LWK replace this machinery for supported routes. Kassiber persists a
redacted coverage projection for inventory/ownership UI, not a competing scan
checkpoint.

The remaining manual history/UTXO helpers are reachable only through named
capability routes in the matrix above: Bitcoin address scripts, Electrum custom
CA, Bitcoin Esplora authorization/timeout, finite source-overlap filtering,
plus the explicitly labelled Liquid proxy, Electrum timeout/custom-CA/insecure
TLS and
genuinely different descriptor-policy limitations. Liquid Esplora auth is LWK
native; Esplora custom CA fails closed on both chains.
Dependency-contract tests fail if an ordinary supported BDK route calls one of
those adapters or retries through one after a BDK error.

### Esplora and Electrum history checkpoints

- Compatibility Esplora uses `esplora_scripthash_stats`,
  `esplora_stats_fingerprint`, `fetch_esplora_history`,
  `compatibility_esplora_records_for_wallet` and the
  `esplora_scripthashes` checkpoint map.
- Compatibility Electrum uses the local `ElectrumClient`, `electrum_call_many`,
  subscription statuses, history/header caches,
  `compatibility_electrum_records_for_wallet`, and the
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
  `record_from_electrum_tx` and
  `compatibility_electrum_records_for_wallet`.
- `transaction_graph_cache`, `core.imports.retract_wallet_records`, and
  transaction upserts persist the accounting projection.

For supported BDK/LWK routes, dependency canonical transactions and chain
positions drive confirmation, reorg and replacement deltas. Kassiber still
normalizes the resulting accounting rows and graph evidence.

### UTXO lifecycle

- `compatibility_esplora_utxos_for_wallet`,
  `compatibility_electrum_utxos_for_wallet` and
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

The dependency observers intentionally do not scan a complete descriptor and
then filter accounting rows after the fact. A mixed-input transaction can span
allowed and excluded scripts while carrying one indivisible network fee;
dependency-wide canonical txids also cannot directly drive filtered
retractions, and branch coverage cannot describe arbitrary excluded scripts.
Until those accounting-ownership semantics are explicit and adversarially
tested, finite overlap keeps the pre-connect compatibility route. Consolidating
duplicate wallet sources into one canonical wallet is preferable where
possible.

### Liquid decoding, unblinding and state

- `wallet_descriptors.normalize_descriptor_text`, `liquid_plan_can_unblind`,
  `liquid_blinding_secret` and `decode_liquid_transaction` provide current
  parsing and secrets.
- `_liquid_utxo_record_from_output`, `liquid_output_amount_asset_id`,
  `record_components_from_liquid_tx`,
  `compatibility_esplora_records_for_wallet` and
  `compatibility_electrum_records_for_wallet` manually fetch, decode, unblind
  and correlate Liquid inputs/outputs only on named compatibility routes.
- `wallet_policy_asset_id`, `liquid_asset_code`, and the DB compatibility
  backfills map policy-asset identifiers to `LBTC`.

LWK is the authoritative supported-route wollet state and unblinding
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

## Atomic chain refresh boundary

Phase 3 routes single-wallet refresh, `--all`, and daemon freshness through the
same `_apply_wallet_sync_atomically` primitive. Chain discovery, source-overlap
preflight, backend fetching, retry/backoff and any widened negative-balance
repair scan finish before the coordinator opens its per-wallet savepoint. The
coordinator supplies only `commit=False` hooks; it alone releases, rolls back
and commits the local application.

Within that savepoint the ordered apply stages are observer persistence,
authoritative retractions, normalized transaction insertion (including the
stored graph evidence), output inventory, derivation coverage, wallet sync
metadata and the freshness checkpoint. The observer and coverage hooks are
explicit seams for the Phase 4 store and do not expose dependency objects to
CLI or daemon layers. If any stage or cancellation check fails, every local
write rolls back and the request-scoped observer discard hook runs. `--all`
then records a redacted error for that wallet and continues without disturbing
already committed wallets.

Freshness progress callbacks use the same SQLite connection and historically
committed their progress rows. Network progress is emitted during preparation;
commit-capable progress callbacks are suppressed during local apply so they
cannot invalidate the wallet savepoint. Deterministic fault injection covers
all six state boundaries and compares the exact before/after database state.

The boundary in this phase is the on-chain backend refresh path that the
BDK/LWK migration will consume. File, BTCPay, Core Lightning and LND ingestion
retain their existing source-specific orchestration; they use `commit=False`
when invoked by the shared wallet coordinator, but their transport-specific
fetch/apply decomposition is outside the chain-observer contract.

## Cross-platform packaging checkpoint

Phase 1 retains `macos-15-intel`, both Apple sidecars, the universal Tauri
target and the universal Homebrew artifact. BDK 3.0.0 ships wheels for both Mac
architectures. LWK 0.18.0 ships only arm64 on macOS, so the dependency is
platform-marked out on Intel and capability selection chooses the named Liquid
compatibility observer before network access. Linux x86_64 and Windows x86_64
continue to bundle both dependencies.

## Production cleanup inventory

The migration removed the production manual Esplora/Electrum descriptor gap
engine: `scan_descriptor_targets` and the network branches of
`discover_descriptor_targets`, together with their test-only checkpoint/gap
fixtures. The surviving manual fetchers are named `compatibility_*` and can be
reached only after a capability reason is selected before connection. Address
lists report `observer_route=bitcoin_script`, Core reports
`observer_route=bitcoin_core_rpc`, and Silent Payments reports
`observer_route=silent_payments`; these are first-class responsibilities rather
than failed BDK attempts. Bitcoin Core range/import logic, Silent Payments,
protocol decoding, HTLC evidence,
normalization, and Kassiber's ownership/tax domain logic remain intentionally
separate.

The required Linux `chain-observers` CI job is path-aware for observer,
descriptor, sync, persistence, dependency, and regtest changes. Packaging
smokes import both pinned bindings in macOS ARM64, Linux x86_64, and Windows
x86_64 sidecars before desktop bundling.

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
  output-inventory refresh tests, sync prefetch/savepoint tests, observer-blob
  rollback/restart reconstruction, and forced rebuild of unknown versions.
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
