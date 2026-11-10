# Rust Liquid sync

Native Liquid wallet sync (plan 21) uses `lwk_wollet` 0.18.0, the same LWK
release the Python `lwk` 0.18.0 binding ships. LWK owns gap discovery,
unblinding and the wollet state. Kassiber owns every connection, the opaque
store, the accounting projection and publication. This page lists what Python
does, which part LWK performs on each side, the egress sequence, and the
remaining differences.

Code: `crates/wallets/src/lwk.rs` (execution, buffered store, consent),
`lwk/backend.rs` (LWK's `BlockchainBackend` over Kassiber's Electrum client),
`lwk/facts.rs` (records, outputs, coverage, checkpoint), `lwk/descriptor.rs`
(descriptor translation and route preflight), the Liquid branch of
`fetch.rs`, and the values store in `observer_state.rs`, `observer_apply.rs`
and `kassiber-store`'s `observer.rs`.

## Python's Liquid sync, step by step

1. **Sync state** (`sync_backends.resolve_wallet_sync_targets`). The wallet
   needs a descriptor plan whose blinding material can unblind. Otherwise:
   "Liquid descriptor wallets require private blinding keys for full sync and
   fee accounting". It must name a backend (no public Liquid default), and
   `validate_backend_for_wallet` checks the backend's chain and network and
   allows only Esplora or Electrum. Targets come from
   `_offline_descriptor_targets`; no request is made. The policy asset is
   `wallet_policy_asset_id` (a symbolic `LBTC` resolves to the network's id).
2. **Route preflight** (`lwk_compatibility_reason`), in order: Esplora
   insecure TLS, Esplora custom CA, operator CA override with HTTPS Esplora,
   backend kind, proxy or `.onion` endpoint, Electrum custom CA, Electrum
   insecure TLS, Electrum timeout other than 30, multipath shape, separate
   change descriptors that are not structurally equivalent, a descriptor LWK
   rejects, and finally every Esplora route (`http_route_policy`). Any reason
   selects the named compatibility observer before a connection.
3. **Source overlap**: the local targets must equal the descriptor's own
   targets, or the route becomes `source_overlap_partial_descriptor`.
4. **Identities** (`identities_for_wallet(..., observer_kind="lwk")`): one
   `descriptor:default` identity (branch keys from the plan's labels), or one
   per `xpub:<script type>`. Each LWK observer receives the whole plan.
5. **Opaque values** (`load_observer_values`): skipped for a forced refresh.
   A namespace version other than 1 raises `observer_state_rebuild_required`.
6. **Prepare** (`LwkObserver.prepare`):
   - Stored state with `schema_version` other than 1 must be rebuilt, unless
     the refresh is forced, which discards it in memory only.
   - The retraction baseline is the stored `canonical_txids`. A forced
     rebuild without stored state uses the identity checkpoint's list.
   - Build the CT descriptor (`lwk_descriptor_for_plan`: canonical `<0;1>`
     multipath, `ct(slip77(<hex>),...)` or `ct(<view key>,...)`, outer
     wrapper only `elwpkh`/`elwsh`/`eltr`/`elsh`) and the network
     (`liquidv1`, `liquidtestnet`, or regtest with the policy asset).
   - Construct the wollet with the request-local `ForeignStore`. This replays
     the stored updates, so a corrupt value fails here, before any request.
   - Connect (`ElectrumClient(endpoint, tls, validate_domain)`). The no-egress
     switch is checked and one `lwk.electrum.connect` ledger row is recorded.
   - Emit `backend_fetch` progress.
   - Forced rebuild with a checkpoint tip: `backend_tip_behind` when the
     remote tip is lower.
   - Scan horizon (`_lwk_scan_to_index`): the gap limit minus one, or every
     stored or checkpoint `highest_used` plus the gap, whichever is larger.
   - `full_scan_to_index`, `apply_update` when it returned an update, `tip()`.
   - Project the facts. Any non-Kassiber exception becomes
     `backend_sync_failed` with redacted LWK text.
7. **Facts** (`_records`, `_facts`): one record per asset in the
   transaction's LWK balance plus the explicit owned deltas LWK omits
   (`_wallet_tx_balance`), sorted by asset id text. The `vin`/`vout` graph
   marks owned (LWK-unblinded), fee and explicit external outputs. Fee
   handling: a sole-owner spend books the explicit fee output, and a
   mixed-input spend keeps the exact delta with `amount_includes_fee`. The
   attested HTLC claim or refund, `payment_hash` and swap-refund fields are
   filled when exactly one claim input or one refund outpoint exists.
   Outputs are `wollet.utxos()` plus explicit TXOs no wallet transaction
   spends (`confidential: false`), in outpoint order; coverage uses the
   exclusive request bound, and the checkpoint carries the tip, `highest_used` and the
   canonical txids.
8. **Negative-balance repair** (`_prepare_negative_balance_repairs`, in the
   CLI prefetch): stored rows overlaid with the prepared records, keyed by
   (txid, asset). A prospective deficit triggers a forced repair fetch with
   gap 500 before the savepoint.
9. **Apply** (one wallet savepoint): JSON state and coverage, then the opaque
   values (`persist_observer_values`, its own clock read), policy coverage,
   composition, retractions, insertion with provenance, output inventory,
   derivation coverage and freshness. Any failure rolls back everything.

Cancellation in Python is checked before and after each SDK step; one
`full_scan_to_index` cannot be interrupted.

## What LWK does, and what Kassiber does

| Concern | Python | Native |
| --- | --- | --- |
| Gap discovery, script batches (20 per chain batch) | LWK | LWK, same crate |
| Unblinding, explicit/confidential secrets, balances, UTXOs | LWK | LWK |
| Update encoding and the store keys (`000000000000`, ...) | LWK (unencrypted, `with_updates_store`) | LWK, same builder; values interchangeable |
| Electrum transport | rust-electrum-client 0.21 inside the binding | `kassiber_chain::ElectrumClient` over `kassiber-net` |
| Consent, offline, no-egress, ledger | before connecting only | before connecting, around every call, and after the scan |
| Descriptor translation, route preflight, facts | Python | Rust port (`lwk/descriptor.rs`, `lwk/facts.rs`) |
| Opaque store durability | SQLCipher rows at apply | same rows, same savepoint position |

## Egress sequence

Python, after consent: connect, `blockchain.headers.subscribe` (client
construction). For a forced rebuild with a checkpoint tip, one more
`headers.subscribe`. Then `scripthash.get_history` in batches of 20 per chain
until a batch comes back empty past the horizon, one `headers.subscribe`
(LWK's `tip()`), `transaction.get` for txids not yet stored,
`block.header` for heights not yet stored plus the tip, and a final
`headers.subscribe`. Requests go as JSON-array batches.

Native: identical method sequence, preceded by Kassiber's
`server.version` negotiation, with batches as newline requests chunked by
the backend's `batch_size`. Kassiber's client never queues header
notifications, so every LWK `tip()` re-subscribes, which is the branch the
binding takes on a quiet connection. No request is made before consent, while
offline, after cancellation, or for a refused route. Launch, status and idle
paths do not reach this code.

## Native refusals

Every reason above, a descriptor another wallet of the profile partly owns
(`source_overlap_partial_descriptor`) and an owned script LWK discovers past
the horizon that a better wallet owns (`source_overlap_dependency_discovery`)
take Python's Liquid compatibility observer instead of LWK, natively since
[round 2](rust-native-wallet-gaps.md#c5-liquid-compatibility-observers).
Shared earlier refusals (`compatibility_transport`, `backend_selection`,
`wallet_route`) still apply.

After preparation, a real deficit in Python's prospective overlay (step 8)
runs the widened repair fetch natively, as for Bitcoin
([wallet-sync gaps](rust-native-wallet-gaps.md#negative-balance-repair-r1-r3)).
Multi-asset Liquid transactions no longer trigger it.

A forced rebuild keeps each observer's persisted `tip_height` and canonical
txids through `full_scan_checkpoint`, so a backend behind the book refuses
with `backend_tip_behind` (Liquid and Bitcoin) instead of retracting newer
transactions.

## Differences from Python

- Hostile-server hardening: a `transaction.get` answer whose txid differs
  from the request, or a `block.header` for a different height, fails as
  `backend_sync_failed`. Python's LWK client accepts both. `broadcast` is
  refused.
- `backend_sync_failed.details.error` carries Kassiber's redacted message
  instead of the binding's exception text.
- Kassiber's cancellation, consent revocation, offline and no-egress errors
  pass through unchanged from inside a scan; Python can only observe them
  between SDK steps.

## Python reference findings, fixed in Python r7-7

Found by this port and fixed on `t3code/python-reference-fixes-r7-7`
(merged at `eabbe6606`); the native route matches each fix.

1. **Explicit outputs to wallet scripts were never booked** (`56f928ace`).
   LWK 0.18.0 excludes secrets with zero blinding factors from balances and
   UTXOs. Both observers now add the explicit owned amounts to the per-asset
   delta and hold unspent explicit outputs. Pinned by
   `test_explicit_wallet_outputs_are_booked_and_held_in_both_observers`.
2. **Output order depended on the process** (`fe3f30079`). Both observers
   publish outputs in outpoint order; the oracles compare raw order
   (`test_outputs_are_in_outpoint_order_in_both_observers`).
3. **Spurious repair rescans for multi-asset transactions** (`005d5dfd6`).
   The prospective overlay is keyed by (txid, asset) on both sides. Pinned by
   `test_multi_asset_history_syncs_natively_without_a_repair_scan` and
   `prospective_overlay_is_keyed_by_txid_and_asset`.
4. **The forced-rebuild lag guard never fired on the CLI path**
   (`d35dbb6d3`). `full_scan_checkpoint` now keeps each observer's tip
   height and canonical txids. Pinned by
   `test_liquid_force_full_rebuild_matches_python`,
   `test_bitcoin_force_full_rebuild_refuses_a_lagging_backend_like_python`
   and `esplora_forced_scan_discards_fetch_caches_but_preserves_versioned_memberships`.

## Dependency

`lwk_wollet = "=0.18.0"` and `lwk_common = "=0.18.1"`, both with default
features off (`crates/wallets/Cargo.toml`). The one approved online fetch
resolved `lwk_common` 0.18.1. The Python wheel's SBOM lists 0.18.0, and no
behavioural difference is visible: values cross between runtimes in both
directions in the oracle. New lockfile packages: `lwk_wollet`, `lwk_common`,
`elements` 0.26.2 (LWK's `elements26`), `aes-gcm-siv`, `bip39`, `fxhash`,
`qr_code`, `bmp-monochrome`, `regex-lite`, `tokio-macros`, and two
wasm-only crates. `cargo tree -e features` shows no `lwk_wollet` feature
enabled, `lwk_common` with only `amp0`/`default`, and `tokio` with
`rt`, `rt-multi-thread`, `macros`, `sync`, `time` (no `net`). No Electrum,
Esplora, Waterfalls, HTTP, RPC, WebSocket or socket crate is present.
`lwk_wollet_is_pinned_without_clients_or_network_features`
(`crates/net/tests/dependencies.rs`) enforces this, and both crates are in
`reviewed-dependencies.txt`.

## Size

Release `kassiber` (Linux x86-64) grows by 3,779,008 bytes (+2.5%), from
150,536,832 to 154,315,840.

## Tests

- `crates/wallets/src/lwk/tests.rs`: projection, fee folding, HTLC
  evidence, coverage, horizon, restart, reorg, forced rebuild, lag guard,
  fences, transport errors, substituted responses, corrupt store, descriptor
  translation and every refusal reason, over an in-memory RPC.
- `tests/test_rust_lwk_execution.py`: prepared updates against Python LWK
  0.18.0 over a loopback Electrum fake, with stores crossing between runtimes.
- `tests/test_rust_lwk_sync_service.py`: Python's `wallets sync` handler
  against the native sync service, every table, plaintext and SQLCipher,
  restart, reorg, forced rebuild, a Python-written store, the refusals, and
  profile and workspace freshness runs (envelopes and progress events).
- `crates/wallets/scripts/lwk_mutations.json`: 46 mutations, each killed by
  its named Rust test (`lwk_mutation_check.py`).
- Vectors: `crates/wallets/tests/vectors/lwk_chain.json`, from the
  `lwk_vectors` example (deterministic, regenerate with the command in its
  header).
