# Rust wallet-sync gaps

Every wallet-sync or freshness path where the Rust backend still answers
`native_fallback_required` (or a `not_implemented` stand-in for it) instead of
doing what Python does. Production has no Python handoff
([rust-shell.md](rust-shell.md#bitcoin-wallet-sync-cli-registration-r5-partial)): each open entry below is an
error a user sees. Inventory taken on `t3code/rust-native-wallet-gaps`
(base `81e7bbbf6`), 2026-11-07; "Status" records what that branch closed.

`details.reason` names the refusal. "Status" says what this branch did.

## Publication shape (`publication/wallet.rs::require_supported`)

| # | Reason | Trigger | Python | Status |
| --- | --- | --- | --- | --- |
| P1 | `source_overlap` | The profile has any other wallet | `source_overlap.filter_sync_state_for_canonical_owner` drops targets a better canonical wallet owns, before any connection; an emptied wallet is skipped; a partial descriptor moves to the compatibility observer (`source_overlap_partial_descriptor`) | Native ([rust-shell.md](rust-shell.md#wallet-source-ownership-across-a-profile-wallet-sync-gaps-p1)); trimmed Electrum descriptors and dependency-discovered overlaps since E1 ([round 2](#native-chain-routes-round-2)); trimmed Liquid descriptors and LWK-discovered overlaps since C5 ([round 2](#c5-liquid-compatibility-observers)) |
| P2 | `wallet_route` | `sync_source`, `source_file`/`source_format`, `btcpay_provenance`, `bullbitcoin_wallet_exports`, `silent_payment(s)` in the config | File re-import, BTCPay wallet/provenance sync, Bull Bitcoin enrichment, silent-payment scan (`core/sync.py::sync_wallets`) | Native ([sources gaps](#native-sources-round-2) S3, S5, S6); routing follows `classify_wallet_sync`, so a stray key no longer refuses. Silent-payment wallets on Core RPC backends since [C11](#c11-silent-payments-on-core-rpc) |
| P3 | `wallet_route` | Wallet kind `coreln` or `lnd` with a backend | Core Lightning / LND node sync hooks | Native ([sources gaps](#native-sources-round-2) S7, S8) |
| P4 | `wallet_route` | Config `chain` other than Bitcoin or Liquid | Rejected by `normalize_chain_value` in discovery | Open (Python's error) |
| P5 | `wallet_route` | No descriptor and no xpub+script types, but `addresses` | Address-list compatibility observer (`bdk_compatibility_reason` → `address_list`); Esplora, Electrum or Core RPC | Native on Esplora ([rust-shell.md](rust-shell.md#address-list-wallets-wallet-sync-gaps-p5)), Electrum (E1) and Core RPC (C6) |

## Fetch preparation (`fetch.rs::Fetch::new`)

| # | Reason | Trigger | Python | Status |
| --- | --- | --- | --- | --- |
| F1 | `backend_kind` | Backend kind other than Electrum or Esplora (`bitcoinrpc`) | `discover_bitcoinrpc_descriptor_targets` and the Core RPC adapter (address import, rescan) | Native since round 2 ([C6](#c6-bitcoin-core-rpc-wallets), and the `mempool`/`liquid-esplora` aliases in [C9](#c9-backend-kind-aliases)); `custom` stays with Silent Payments |
| F2 | `compatibility_transport` | Bitcoin Electrum with a proxy, Tor proxy, `.onion` URL or custom certificate | Compatibility Electrum observer (`proxy_transport`, `custom_ca`) | Native (E1, [rust-shell.md](rust-shell.md#compatibility-electrum-observer-wallet-sync-gap-e1)) |
| F3 | `compatibility_transport` | Bitcoin Esplora with insecure TLS, a certificate, a token or auth header, a non-30 timeout, a proxy or `.onion`, or `SSL_CERT_FILE` with HTTPS | The same Esplora compatibility observer the native path already runs, with a more specific `observer_compatibility_reason` | Native: `kassiber-net` carries each option, and `esplora_reason` labels the route in Python's order (`system_ca_trust` follows Python's `KASSIBER_HOST_CA_BUNDLE=explicit`, not `SSL_CERT_FILE`) |
| F4 | `compatibility_transport` | A backend with a nested `config_json` | Read like a flat backend | Guard only: backend resolution (`backend_runtime.rs::resolve`) flattens `config_json`, so product calls never reach it |
| F5 | `backend_selection` | Wallet `backend` not a string, or a named backend whose resolved name differs | Resolves the backend by name | Open (shape guard) |
| F6 | `wallet_route` | Samourai config | A Samourai child syncs as a descriptor wallet (Electrum: BDK under its parent's Samourai identity) | Native ([sources gaps](#native-sources-round-2) S2) |
| F7 | `descriptor` | `descriptors::load` returns no plan | Address-list or no-source route | Native with P5 |
| F8 | `observer_branches`, `observer_identity` | An xpub source key with zero or more than two branches, or no BDK identity | `bdk_branches_for_identity` / `observer_identity_invalid` | Open (shape guard) |
| F9 | `scan_horizon` | `highest_used + gap` overflows `u32` | Python integers do not overflow; derivation fails later | Open (shape guard) |
| F10 | Liquid reasons | `insecure_tls`, `custom_ca`, `system_ca_trust`, `backend_kind`, `proxy_transport`, `custom_timeout`, `noncanonical_multipath`, `separate_change_descriptor`, `descriptor_unsupported`, `http_route_policy` | Liquid compatibility observers (`compatibility_electrum_records_for_wallet` / Esplora with `chain == "liquid"`): Liquid decoding, unblinding and Liquid UTXO rows in Python, not LWK | Open: see Liquid |
| F11 | `invalid_json`, `discovery_targets`, `output_inventory` | Internal shapes that cannot occur with a well-formed fetch | n/a | Internal guards |

## Publication repair (`publication/wallet.rs::apply`)

| # | Reason | Trigger | Python | Status |
| --- | --- | --- | --- | --- |
| R1 | `negative_balance_repair` | Any prepared fetch whose prospective overlay has a real deficit | `_prepare_negative_balance_repairs`: one forced refetch with the widened gap (`negative_balance_rescan_gap_limit`) after every wallet's fetch and before the savepoint; outcome gains `negative_balance_rescan` | Native (CLI service and freshness batches); see below |
| R2 | `negative_balance_repair` | Compatibility (Esplora) fetch that leaves a deficit after applying | `sync_wallet_from_backend` re-syncs inside the savepoint with the widened gap and a forced checkpoint | Native ([round 2](#c7-the-rescan-after-applying-r2)) |
| R3 | `negative_balance_repair` | A prepared fetch already carrying `_prepared_negative_balance_rescan` | Applies it and reports `resolved` | Native |
| R4 | `non_authoritative_route` | A fetch not marked authoritative | Silent-payment scans that did not complete | Native for silent-payment scans ([sources gaps](#native-sources-round-2) S6); no other native route is non-authoritative |

## Freshness planner and daemon routes

| # | Reason | Trigger | Python | Status |
| --- | --- | --- | --- | --- |
| Q1 | `freshness_source` | A planned job other than on-chain history or journals: `btcpay_wallet`, `btcpay_provenance` (wallet or account route), `market_rates` | BTCPay sync jobs and the market-rate coverage job | Native ([sources gaps](#native-sources-round-2) S4) |
| Q2 | `freshness_ambiguous_wallet` | `--wallet` reference matching more than one wallet | `_load_wallets_for_freshness` runs every match | Native: the plan runs every match and its direct sync resolves the id, as `resolve_wallet` does (`test_product_wallet_reference_matching_two_wallets_runs_both`) |
| Q3 | `wallet_route` | Wallet kind `coreln`, `lnd`, `nwc`, `phoenix` or `silent-payment` in a planned profile | No freshness source; only the direct sync reaches them | Native ([sources gaps](#native-sources-round-2) S5): the plan refuses only when its run can only sync directly and a selected wallet's route is not native |
| Q4 | `freshness_journal_refresh` | Journals requested but no native journal handler supplied | Local journal refresh | Internal (every product caller supplies one) |
| Q5 | `native_fallback_required` ("direct-sync reference path") | `ui.wallets.sync` whose plan has no source spec, or a paused source | `_wallets_sync_payload` falls back to `handlers.sync_wallet` when the freshness run produced nothing | Native: the freshness execution runs the plan's own sync service ([rust-shell.md](rust-shell.md#direct-wallet-sync-wallet-sync-gaps-q5-q6)) |
| Q6 | `not_implemented` | AI `ui.wallets.sync` on the same books as Q5 | Same as Q5 | Native (same path) |
| Q7 | `not_implemented` | Report-read refresh with no configured backend at all | Falls back to the explicit wallets-sync path | Native ([round 2](#c8-report-read-refresh-without-a-backend-q7)) |
| Q8 | `not_implemented` (wrapping Q1–Q3) | Report-read refresh of unsupported sources | Freshness run | Native with Q1 and Q3 |

Every `ui.freshness.*` invocation whose arguments the native parser refuses
also answers `native_fallback_required` (`daemon/src/freshness/product.rs`);
those are argument shapes, not sources.

## Negative-balance repair (R1, R3)

`fetch::with_repair` runs Python's prefetch repair: the prospective overlay
(stored rows, retractions, the prepared records keyed by txid and asset)
decides, and a deficit replaces the wallet's fetch with one forced fetch of
the same wallet at `max(500, 2 × max(gap, 100))`, capped at 5,000, under a
grant that shares the original's revocation. The marker crosses the fetch
contract; publication skips its own prospective guard for a marked fetch and
reports `negative_balance_rescan` (`resolved`, `remaining_negative_events`)
in Python's key order. The CLI service repairs after every wallet's fetch, a
freshness batch after every job's fetch, as Python's `_prefetch_chain_wallets`
does. Pinned for Bitcoin on both protocols
(`test_negative_balance_repair_matches_python`), for freshness batches and
for Liquid (`test_liquid_negative_balance_repair_matches_python`). R2, the
in-savepoint rescan when a deficit appears only after applying, is native
since [round 2](#c7-the-rescan-after-applying-r2).

## Typed refusal for unreadable wallet configs

Gap 3 of the brief is already native on the base (`488741ab4`): the planner,
`wallets sync --all` and the AI `ui.wallets.sync` answer
`invalid_wallet_config` naming the wallet, matching Python `7bca137d4`. Pinned
by `test_sync_cli_unreadable_wallet_config_matches_python`,
`test_product_unreadable_wallet_config_is_a_typed_refusal` and
`test_unreadable_wallet_config_refuses_wallet_sync_typed`; re-run on this
branch (21 passed). Python's explicit `--wallet` path still crashes
(`tests/rust_port/repro_cli_wallet_sync_bad_config_explicit.py`); the native
service now refuses that selection typed too, a deliberate difference
recorded in [rust-shell.md](rust-shell.md#unreadable-wallet-configuration-on-an-explicit-sync-deliberate-difference).

## Electrum compatibility (E1)

Native since round 2 ([native chain routes](#native-chain-routes-round-2),
[rust-shell.md](rust-shell.md#compatibility-electrum-observer-wallet-sync-gap-e1)):
address lists, proxies, `.onion` endpoints, custom certificates, trimmed
descriptors and dependency-discovered overlaps take Python's compatibility
Electrum observer, with its stored-graph cache rule, record order and UTXO
rows.

## Liquid

Native since round 2 ([C5](#c5-liquid-compatibility-observers)):
every F10 reason, a trimmed Liquid descriptor and an overlap LWK discovers
take Python's Liquid compatibility observers (Esplora and Electrum), with its
decoder's records and `liquid_*` UTXO rows, including private view-key
descriptors ([C10](#c10-liquid-view-keys)).

## Differential run

`tests/test_rust_wallet_sync_gaps.py` runs Python's CLI and the Rust binary
on the same books over the loopback chain (`tests/rust_sync_chain.py`). The
first run (commit `88e13f05b`) pinned an address-list wallet (P5), a custom
Esplora timeout (F3) and two descriptor wallets in one profile (P1) as Python
syncs / Rust refuses on both protocols. Each case became a parity test when
its gap closed; since E1 every case there is a parity test, and
`tests/test_rust_electrum_compatibility.py` adds the transport routes.
`tests/test_rust_liquid_compatibility.py` covers the Liquid routes and
`tests/test_rust_core_rpc_sync.py` the Core RPC adapter (F1);
`test_sync_cli_unsupported_*` now pin a `custom` backend's handoff.

## What remains

- Shape guards F5, F8, F9 and the internal guards F11, Q4: refusals for
  configurations the product does not produce.

## Native sources, round 2

Paths outside the chain observers that Rust now serves natively. Item ids
S2–S12 name them; tests cite them by id.

### Samourai children (S2)

`samourai::import_samourai_wallet_group` writes a parent (`kind`
`samourai`, `samourai.role` `parent`, no source) and one `descriptor` child
per section with `descriptor`, `change_descriptor` and the safe
`samourai` metadata. Python syncs a child as any descriptor wallet; only the
observer identity differs. `fetch.rs::bdk_identities` ports
`identities_for_wallet` for the BDK route: a child observes under
`samourai.parent_wallet_id` (falling back to its own id) with source key
`samourai:<section>:<script_type>:<root_path>` (defaults `source`,
`descriptor`, `account`) and branch keys `receive`, plus `change` only when
`change_descriptor` is set; a parent observes nothing and refuses
`observer_identity_invalid` as Python's dependency route does; any other
`samourai` value falls through to the xpub or descriptor identities.
Esplora takes the compatibility observer, which has no identity.

Python reference bug, kept for parity: a child with a multipath descriptor
and no `change_descriptor` fails on Electrum in both implementations
(`observer_state_invalid`, "Observer coverage names an unknown branch"),
because Python's identity keeps only `receive` while the BDK observer covers
both branches. Template imports write the change descriptor separately; an
explicit multipath descriptor source passes the import's validation and then
cannot sync on Electrum. The `wpkh-multipath` Electrum case of
`test_samourai_child_matches_python` is the repro.

Evidence: `test_samourai_child_matches_python` (two descriptor shapes, both
protocols, two rounds, envelopes and whole books) and
`test_samourai_parent_with_a_descriptor_matches_python`;
`bdk_identities_follow_python_source_keys` pins the identities. Mutation
entries `fetch-samourai-*`, `fetch-xpub-identities-need-script-types`
(`fetch_mutations.json`) and `routing-samourai-instance-logical`
(`routing_mutations.json`).

### BTCPay provenance on chain wallets (S3)

`publication/wallet.rs::publish` runs `_merge_wallet_enrichments` in Python's
order inside the wallet's savepoint: BTCPay provenance routes
(`sync::btcpay::enrichment`, each Greenfield read under a grant bound to the
wallet and store), then Bull Bitcoin export routes. `wallets sync` passes the
CLI's Greenfield context; the freshness on-chain job and the direct sync of a
`ui.wallets.sync` without jobs pass the daemon's, built from the book's
backends. An enrichment failure rolls the wallet back. A publication without
Greenfield access refuses a wallet with routes (`native_fallback_required`,
`btcpay_provenance`); no product caller does that.

As in Python, the on-chain job's enrichment runs under the on-chain source's
permission: a report-read refresh with on-chain sources enabled also reads
the wallet's BTCPay routes, whatever the `btcpay_provenance` source class
says. That is Python's behaviour, kept for parity and listed for the owner.

The enrichment reads `wallet["_freshness_checkpoint"]`: the checkpoint the
chain fetch started from (`sync_state.checkpoint`, or the original one a
negative-balance repair keeps aside as `wallet_checkpoint`). Its `routes` and
`invoice_routes` seed the Greenfield reads, and the enrichment's own
`freshness_checkpoint` is that checkpoint updated, so from the second sync on
it carries the chain checkpoint's keys too, as in Python. The BTCPay sync
services (`sync_wallet`, the payment ledger, `enrichment`,
`btcpay::provenance::sync`) take the previous checkpoint the same way;
`wallets sync` passes none for local sources, as Python does.

Evidence: `test_btcpay_provenance_enriches_a_descriptor_wallet_like_python`
(one wallet and `--all`, two rounds, over the loopback Esplora chain and
Greenfield fakes: envelopes as JSON, request paths and every manifest table
with generated ids and clocks masked, every call served by Rust) and
`btcpay_provenance_routes_enrich_inside_the_savepoint`. Mutation entries
`publication-btcpay-enrichment` (`fetch_mutations.json`) and
`routing-btcpay-provenance-descriptor-route` (`routing_mutations.json`).

### Freshness jobs (S4) and sources without one (S5)

`freshness/native/jobs.rs` ports the three remaining
`_freshness_handlers` entries; the native handler now supports every job
type the planner builds, so the plan no longer refuses `freshness_source`.

- `btcpay_wallet_source` loads the profile and wallet at run time
  (`_load_freshness_profile_wallet`), reads the source's checkpoint (reduced
  by `full_scan_checkpoint` for a forced job), emits `discovery` and
  `backend_fetch`, checks the `btcpay_wallet` class for automatic triggers,
  runs `sync::btcpay::sync_wallet` with that checkpoint, emits `import`,
  stamps `last_synced_at` on the wallet row as loaded
  (`_mark_daemon_wallet_synced`) and answers
  `{"wallet", "status", **outcome}`, `force_full` first when forced.
- `btcpay_provenance` with an `account_route_id` refreshes the store's
  provenance (`sync_btcpay_commercial_provenance`, suggestions included)
  with the stored checkpoint; a wallet route runs
  `enrich_wallet_from_btcpay_provenance` (`enrichment_or_empty`, which
  answers zero totals when the routes are gone). Both emit
  `backend_fetch` and `decode_enrich`.
- Greenfield page reads emit Python's `emit_sync_progress` events
  (`backend_fetch`, then `pages_fetched` and `page_size`) through the job's
  progress, which persists and commits each one; the CLI and the chain
  publication's enrichment pass no hook, as Python suppresses them there.
- `market_rate_coverage` fills the bundled hourly Kraken seed first (an
  offline cache fill), then answers `market_rates_disabled` when the class is
  off, else checks the class (also for explicit runs, as Python does) and
  runs the latest-quote refresh and, for Coinbase Exchange and mempool, the
  transaction-need sync without idle warming. `bundled_seed.path` is the
  compiled-in seed directory with `..` resolved
  (`kassiber_rates::bundled_seed_path`); Python reports its package's
  `kassiber/data/rates/kraken/btc_hourly`, the same directory in a source
  checkout. That stored path is environment-specific in both and is masked
  in the parity tests.

S5: the plan no longer refuses books holding `coreln`, `lnd`, `nwc`,
`phoenix`, `silent-payment` or file-sourced wallets. Python plans no job for
them and reaches them only through the direct sync of a `ui.wallets.sync`
that queued nothing. The plan builds its direct-sync action with
`Sync::new_deferred`: a wallet whose route the sync service cannot take is
recorded instead of refusing. The plan refuses up front (before any effect)
only when such a wallet has a freshness job, or when the run can only sync
directly (every wallet source paused or absent); otherwise the direct sync
refuses if it ever runs.

Evidence: `tests/test_rust_freshness_sources.py` (BTCPay jobs on plaintext
and SQLCipher over three runs including a forced one, `ui.wallets.sync`
planning them, egress refusals with no traffic, the market-rate job with
the seed and a refused live lookup for two providers, books with node,
Phoenix, NWC and silent-payment wallets, and a file-sourced selection's
direct sync), each comparing daemon lines and every manifest table with
Python's daemon with generated ids and clocks masked.

### BIP329 matching (S10)

`_match_bip329_record` reads `ownership.build_owned_index(conn, profile,
load_profile_wallets(...))` with derivation, which `wallets identify`
already ports (`kassiber-wallets::identify`). The metadata service sits below
the wallets crate, so `kassiber-transactions` defines the `Ownership`
interface (address verdict with `ownership_ambiguous`, outpoint lookup, input
legs) and the binary registers the identify index once at startup
(`register_bip329_ownership`). With it, `addr` answers `exact` or
`ambiguous` from `classify_address`, `output` from `lookup_outpoint`, and
`input` from `load_local_tx_legs` (the leg's outpoint, else its script),
with Python's match sources; the index's non-fatal warnings are the
preview's. Without a registration (crate tests), the old local index and its
`not_implemented` refusal remain.

Evidence: `test_descriptor_wallets_match_through_the_derived_ownership_index`
(derived receive and change addresses, an address two wallets own, a listed
address, a foreign one, an output and inputs of a stored Esplora-shaped
graph: preview bytes, import results and four wallet-scoped exports equal
Python's). Mutation entries `routing-bip329-*` (`routing_mutations.json`).

### Silent-payment wallets (S6)

A wallet with `sp_descriptor` material takes the scanner route ahead of any
descriptor, as `resolve_wallet_sync_targets` does:

- Preparation (`fetch.rs`, `fetch/silent.rs`) ports discovery up to the
  adapter: the backend's chain instance, `build_plan` (the stored
  configuration's validation), `validate_backend_for_wallet` and
  `validate_backend_capability` (chain, network, backend kind, the
  server-assisted transport and the capability flags or scanner fields).
- The scan (`silent_payments/scan.rs`) reads the backend's
  `silent_payment_scan_file` without following a final symlink and only as
  a regular file this user owns with no group or other permissions, or, in
  server-assisted mode, POSTs Python's scan request once to the backend's
  scan path through `kassiber-net` (`HttpClient::post_json`) under the
  sync's consent. A local file makes no request, so offline mode does not
  refuse it, as in Python. `normalize_scan_payload` keeps Python's value
  model: the wallet binding, owned outputs and inputs, fee rules, BTC
  amounts, timestamps, UTXO rows, spent markers, range coverage and the
  checkpoint, in Python's key order.
- Publication (R4) inserts through `insert_wallet_records` without observer
  authority unless the scan came through the Esplora or Electrum
  compatibility route complete with every record timestamped; only an
  authoritative scan records chain observation provenance. A degraded scan
  skips the output inventory and marks the wallet `partial_success` with
  blocking reports. `sync_mode` is `silent_payment`, and the negative-balance
  repair widens a silent-payment plan from gap 0 as Python does.
- Progress follows Python's coordinator: the direct sync streams a
  wallet-labelled `discovery` event (and `backend_fetch` outside the
  compatibility route) and, as for every chain wallet, no import progress
  from the publication.
- Failures Python raises as other exceptions keep its wrapping: the
  compatibility route's `backend_sync_failed` with the redacted error text,
  or `Source refresh failed ... during backend_fetch` on a custom backend.
- `sync_wallets`' routing now matches `classify_wallet_sync`: addresses,
  descriptor material (`descriptor`, or `xpub` with `script_types`) or
  silent-payment material select the chain route, and the meaningless
  `silent_payment(s)` keys, a lone `change_descriptor` or bare `xpub` are
  skipped as Python skips them. A stray `sync_source`, `source_file` or
  `source_format` no longer refuses a chain route.

A silent-payment wallet on a Core RPC backend followed on the chain branch
([C11](#c11-silent-payments-on-core-rpc)), after
the owner ruled that an incomplete scan must not be authoritative and Python
stopped granting it.

Evidence: `tests/test_rust_wallet_sync_silent_payments.py` (custom, Esplora
and Electrum backends: a receive, the same scan again, a spend, a partial
scan; eight payload shapes including a spend whose receive the scan
omits (the negative-balance repair); eighteen scanner failures; server-assisted
complete, partial, unconfigured, Electrum, HTTP status and invalid-body
cases; offline mode; the daemon's direct sync), comparing output, requests
and every manifest table with Python, generated ids and clocks masked, and
proving Rust served each command. Mutation entries `routing-sp-*`.

### Core Lightning wallets (S7)

A `coreln` wallet with a backend takes `sync_wallets`' node route, after the
BTCPay source and before file and chain routes, inside the wallet's
savepoint as Python's coordinator runs it:

- `kassiber-chain` `lightning/cln/wallet.rs` ports
  `fetch_core_lightning_snapshot` over the existing `lightning-cli` process
  transport (a `ClnReportClient` bound to the wallet sync: book, profile,
  wallet and backend): `getinfo` must answer, every other allowed method that
  fails is recorded in `errors` with its message and read as empty, and each
  resource passes Python's `_sanitize_*` allowlist. `wallet_records` ports
  `snapshot_records`: daily forwards per out channel, paid invoices,
  completed pays, invoice-only bookkeeper income without an L1 txid, daily
  balance snapshots and channel-lifecycle txids keyed by (transaction,
  account), in Python's order and `_stable_hash` identities.
- `kassiber-wallets` `sync/node.rs` ports the persistence: the
  `lightning_node_syncs` row (`running`, then the node, counts, method errors
  and `partial` or `synced`), `_upsert_lightning_record` with the legacy
  channel-row cleanup, `_record_to_import`/`_pay_to_import` and the daily
  routing income as wallet transactions under `core-lightning` (the import
  now stamps `_kassiber_provenance` for the `lnd` and `core-lightning`
  labels, as `normalize_import_record` does), the network stamp, journal
  invalidation and Python's outcome. Bull Bitcoin export routes enrich the
  result; only a `synced` outcome stamps the wallet's `last_synced_at`.
- Progress follows `_wrap_sync_progress_for_wallet`: a wallet-labelled
  `backend_fetch` event and the import's `importing` events, streamed by a
  desktop `ui.wallets.sync` (whose direct sync is the only path to a node
  wallet). The wallet sync and the freshness run use the Lightning gate, so
  test builds may launch the registered fake `lightning-cli` only.
- Native errors keep the documented privacy difference of the report
  acquisition: no `lightning-cli` command, stderr or malformed amount value
  in messages or details.

Evidence: `tests/test_rust_wallet_sync_lightning.py` (CLI `--wallet` and
`--all` twice, `getinfo` and optional method failures, seven reference node
fixtures including refused amounts, and the daemon's direct sync with its
progress events), comparing output and every manifest table with Python on
the same bytes, generated ids and clocks masked. Mutation entries
`routing-cln-*` and `routing-node-import-provenance`.

### LND wallets (S8)

An `lnd` wallet with a backend takes the same node route as a `coreln`
wallet:

- `kassiber-chain` `lightning/lnd_wallet.rs` ports `fetch_lnd_wallet_history`
  over the existing LND REST client (an `LndReportClient` bound to the wallet
  sync): `getinfo`, every payment and invoice page and every forwarding page,
  then `lnd_import_records` (settled invoices, succeeded payments and daily
  routing income, in Python's order and identities, with the lenient
  `binascii.a2b_base64` payment-hash decoding), `authoritative_settlement_ids`
  (payments whose succeeded HTLCs carry a valid `resolve_time_ns`) and
  `channels_to_lightning_records` over the open and closed channel lists.
- `kassiber-wallets` `sync/node.rs` imports the records under `lnd` with those
  settlement ids, persists the channel lifecycle rows, invalidates journals
  and returns Python's outcome. Bull Bitcoin export routes do not enrich an
  LND wallet, as in Python.
- The import ports live settlement authority: for a listed id whose stored
  row is the same LND payment (`_same_lnd_settlement_identity`), a changed
  `confirmed_at` or `occurred_at` rewrites both dates and the fingerprint,
  clears a cached rate whose typed provenance is `fmv_provider` or
  `manual_rate_cache`, and replaces the observation payload even when it is
  committed evidence.

Evidence: `test_lnd_wallet_sync_matches_python` (CLI `--wallet` and `--all`,
then a second sync whose HTLC settled at another time; output, the node's
requests and every manifest table equal Python's, and both correct the
payment date) and `test_lnd_direct_wallet_sync_events_match_python` (the
daemon's direct sync and its progress events). Mutation entries
`routing-lnd-*` and the widened `routing-cln-node-route`.

A `coreln` or `lnd` wallet without a node backend is no node route: like
Python's `sync_wallets`, its addresses or descriptor sync as any chain wallet
does, where Rust used to refuse the kind (`wallet_route`).
`test_node_kind_without_a_backend_syncs_its_chain_material`, mutation entry
`routing-node-kind-chain-route`.

### Public graph lookup (S9)

`ui.transactions.graph` with an explicit `allowPublicLookup` and no complete
local or cached reference asks the user's backends natively
(`journals/src/transaction_graph/public.rs`, transport in
`wallets/src/freshness/graph/public.rs`):

- Backend choice ports `_graph_lookup_backends`: the wallet's own backend,
  the runtime backends (`Config::graph_runtime`, default first), the default
  setting's row, the preferred HTTP explorer and every configured row in
  Python's kind and recency order, deduplicated by transport identity. A
  shipped default (`mempool`, `fulcrum`, `liquid`, `liquid-blockstream`)
  counts only when the user chose it, marked it their own infrastructure or
  defined it in the env file or process environment.
- Each candidate runs under an explicit grant for this request within the
  ten-second budget, timeouts capped at five seconds. Electrum answers are
  decoded and verified against the requested txid (Bitcoin with its parents
  fetched in batches and cached, at most 250 uncached; Liquid with
  confidential legs and the explicit fee output); Esplora answers are
  sanitized and checked for the txid, both leg lists and complete previous
  outputs. Every failure, the offline preference and the no-egress switch
  included, becomes Python's warning on the caller's graph; the first
  complete answer is cached and wins.
- The graph cache uses Python's schema version 2 (10719a7d5) for reads and
  writes, here and in the background graph worker, which read and wrote
  version 1 before. That mismatch had kept
  `test_product_automatic_graph_lookup_matches_reference` and fifteen
  `tests/test_rust_freshness_graph.py` cases failing at the base commit; the
  Electrum repro test now expects Python's refusal of mismatched bytes.

A Core RPC backend in the list followed on the chain branch
([C12](#c12-core-rpc-graph-lookups)), together
with the background graph fill of a wallet that syncs through Core.

Evidence: `tests/test_rust_transaction_graph_public.py` (an Esplora lookup and
its cached reread without a request; six explorer failures; an Electrum lookup
with parents; a missing parent and wrong bytes; a Liquid Electrum lookup;
backend choice for unconfigured, shipped, chosen-shipped, wallet-first and
offline books), comparing responses and every table (cache rows without their
clocks) with the Python daemon. `tests/test_rust_desktop_projections_egress.py`
proves the product switch refuses before any connection. Mutation entries
`graph-*`.

### BTCPay 2.4 wallet permission (S11, #692)

BTCPay Server 2.4 moved the wallet address preview and wallet history behind
the Wallets plugin's read-only `btcpay.store.canviewwallet`. Read-only keys
lost the preview and network detection, setup still asked for the
store-modifying key, and a 2.4 read-only key's wallet-history 403 failed the
sync instead of falling back to invoice provenance. Python's fix ("Read
BTCPay 2.4 wallets with the read-only key", also on the main-based branch
`t3code/btcpay-24-canviewwallet`) is cherry-picked here as the reference,
with its store-scoped fallback check kept; the Rust port is "Port the BTCPay
2.4 wallet permission natively". Both were checked against real BTCPay 2.3.9
and 2.4.7 regtest servers (`dev/regtest/btcpay_permission_probe.py`): 2.3's
authorize page drops the unknown `canviewwallet` and grants the rest, and on
2.4 the read-only key now imports wallet history. See
[rust-shell.md](rust-shell.md#btcpay) for the evidence.

### Bark through BTCPay (S12, #691)

The Bark plugin (0.10 and later) serves a store's whole Bark wallet at the
standard wallet-history path, behind `canviewwallet`. Python: "Import Bark
wallets from BTCPay" and its follow-ups, also on the main-based branch
`t3code/btcpay-bark-wallet`; the Rust port is "Port Bark wallet imports
natively". The owner's decisions on the seven questions:

1. Migration: the next sync of a `BTC-ARK` payment ledger probes
   `/payment-methods/BTC-ARK/wallet` and, when the plugin serves it,
   switches that wallet to the Bark import. Every active ledger row must
   match exactly one Bark record of the same direction and principal; it is
   kept under its id with Bark's fee, else the sync refuses
   (`bark_migration_conflict`, `unmatched_ledger_rows`) and the book is
   unchanged. Older plugins, stores without Bark and keys that cannot view
   the wallet keep the ledger (`bark_wallet_history`).
2. Fees: receives book their gross amount and the fee as a fee-only row;
   sends book `-amount - fee` with the fee; fee-only rows (refresh, board,
   exit fees, an offboard into the wallet's own on-chain part) book one
   fee-only row. Each row's booked effect equals Bark's `amount` to the msat
   (exact, context-free decimal arithmetic). Unknown kinds, inflows on send
   kinds, fees larger than the amount and fees without an amount refuse
   (`bark_row_unsupported`).
3. Lightning overlap: a `BTC-LN` ledger row of the same store booking a
   payment hash the Bark wallet books in the same direction moves into the
   Bark wallet (id, notes, reviews kept; Bark's routing fee added); the
   ledger then holds it back. Another wallet booking it, or review evidence
   binding the row, refuses.
4. `BARK` (and every spelling Kassiber canonicalized to `BARK-CHAIN`) maps
   to `BTC-ARK`. There is no separate on-chain Bark method: the plugin
   reports the wallet's on-chain part as `onchain-*` rows of the same
   wallet history.
5. Network: no Bark field names it. Setup gives the Bark wallet the server's
   on-chain network (as ledgers get), and each sync reads the Ark addresses
   the store's Bark payments went to (`ark1` mainnet, `tark1` every test
   network): a book bound to another network refuses
   (`book_network_mismatch`) before anything is booked.
6. Verified against a real regtest Ark stack
   (`scripts/integration-harness.sh btcpay-bark`: bitcoind 31, Esplora,
   captaind 0.8.1 with Core Lightning + hold, an external node, a
   counterparty `bark`, BTCPay 2.4.7 with plugin 0.13.0, and 0.9.6 before and
   after upgrading). The recordings (`tests/fixtures/btcpay_bark/`) changed
   three things: pre-0.10 payment ids are `{invoice}#ark-movement-N-I`
   (accepted, the amount separating one movement's VTXOs), the network shows
   only in destinations, and the README lists every difference from the
   plugin's docs (the `exit` row stays Unconfirmed with amount 0, sends to the
   wallet's own address are `onchain-send` fee rows, `onchain-internal` never
   appeared). Python's `RecordedStoreTest` and the Rust parity replays book
   the recorded stores to the plugin's own confirmed balance.
7. #353 (Bark's own exports) stays separate: it would import a wallet
   Kassiber cannot reach through BTCPay. A `BTC-ARK` method cannot be mapped
   onto another tracked wallet (`existing_wallet` refuses), so the two paths
   never book one Bark wallet twice.

Found while porting and fixed in Python: marking a wallet synced wrote
`last_synced_at` into the config the sync started from, undoing a switch;
a freshness job's committing progress ended the Bark import's savepoint;
and a receive's fee row carried the payment hash where the book-open
backfill would copy it.

Evidence: `tests/test_btcpay_bark.py` (Python);
`crates/wallets/tests/vectors/python_btcpay_bark.json` (91 cases) with
`python_bark_*_vectors`, `bark_pages_match_python` and the payment-method
vectors; `tests/test_rust_wallet_sync_bark.py` compares the wallet import,
the switch (and each kept-ledger answer), every refusal, the Lightning
moves in both orders, CLI routes, inspection and setup, and the freshness
jobs on plaintext and SQLCipher books with unchanged Python (envelopes,
requests, every table). Mutation entries `W-bark-*`, `btcpay-bark-*` and
`routing-bark-*` are killed by those tests.

### Remaining refusals and parity choices

Refusals that remain on this group's paths, each before any write:

- A silent-payment wallet on a Core RPC backend (S6): decided (fail closed)
  and native on the chain branch
  ([C11](#c11-silent-payments-on-core-rpc)).
- A Core RPC backend reached in the graph's public lookup list (S9): native
  on the chain branch ([C12](#c12-core-rpc-graph-lookups)).
- Internal guards no product caller reaches: a publication or freshness run
  without the BTCPay context or the journal handler it needs
  (`btcpay_provenance`, `wallet_route`, `freshness_journal_refresh`).

Parity choices listed for review, all pinned by tests: a report-read on-chain
job reads BTCPay routes under the on-chain permission (S3); Python's
Samourai multipath Electrum failure is kept (S2); the market-rate job's
bundled seed path is environment-specific (S4); native Core Lightning errors
omit `lightning-cli` details (S7); silent-payment scanner paths of the form
`~user` resolve through the account database only, and decimal NaN or
infinity amounts surface Python's exception names (S6).

## Native chain routes, round 2

Chain-observation paths Rust now serves natively. Item ids C1–C12 reuse the
row numbers above; tests cite them by id.

Parity policy: every stored row (transactions, outputs, checkpoints,
provenance) must match Python byte for byte. Tests run Python's CLI and the
native binary on copies of one book against loopback fakes only
(`tests/rust_sync_chain.py`, plus the SOCKS5 and TLS fakes this branch adds).

### C1–C4: compatibility Electrum observer (E1)

One route serves all four: [rust-shell.md](rust-shell.md#compatibility-electrum-observer-wallet-sync-gap-e1)
describes it. Parity: `tests/test_rust_electrum_compatibility.py` (eight
cases, envelope and stored bytes) and the converted Electrum cases of
`tests/test_rust_wallet_sync_gaps.py`. Mutations: 26 new entries in
`crates/wallets/scripts/fetch_mutations.json`, all killed; four inherited
entries the route moved (`fetch-esplora-transport-native`,
`fetch-address-electrum`, `routing-bdk-egress-envelope`,
`routing-address-route`) are re-anchored and killed again.

Python finding (not fixed, requests only): `_stored_wallet_chain_history`
offers stored Electrum graphs, but the decoded graph has no `txid`, which
`stored_transaction` requires, so none is ever reused. The native route
keeps the rule, and both refetch every graph; pinned by
`test_stored_graphs_are_offered_like_python`.

### C5: Liquid compatibility observers

Python runs LWK only over plain or platform-TLS Electrum with the default
timeout and a canonical descriptor; every other Liquid wallet, a trimmed one
and an overlap LWK discovers take its compatibility observers, which decode
and unblind the transactions themselves. The native fetch now follows
`lwk_compatibility_reason` the same way: the reason selects the
compatibility route (no LWK observer is prepared), online discovery accepts
Liquid targets, and LWK's dependency-discovery overlap falls back to it.

- `Acquisition::new_cached` observes Liquid targets (the checkpoint's
  `backend.chain` is `liquid`). Electrum keeps each verified raw transaction,
  fetches every input's previous transaction (coinbase included, as Python
  looks them up), skips the stored-graph cache Python offers Bitcoin only,
  and fetches each listed output's transaction over the output connection.
  Esplora reads `tx/<txid>/hex` for every history transaction (by block time,
  then txid), their parents and each listed output.
- `fetch/liquid.rs` ports `record_components_from_liquid_tx` (one record per
  asset; an explicit fee only for a wallet that funded the transaction;
  owned outputs unblind with the script's SLIP-77 key, explicit foreign
  values stay evidence and confidential foreign ones stay script-only; the
  claim and refund witness links) and `_liquid_utxo_record_from_output`.
  `Plan::liquid_blinding_secret` matches embit's SLIP-77 derivation
  (`liquid_blinding_secrets_match_embit_slip77`).
- Block headers: Elements headers are longer than 80 bytes; the timestamp is
  read from bytes 68..72 as Python's `block_header_timestamp` does.

Differences, each deliberate: Rust verifies an Esplora `tx/<txid>/hex` reply
against its txid as Python's Electrum path does (Python decodes it
unchecked), and reuses a transaction it already fetched for an output's
record instead of fetching it again (requests only).

Private view-key descriptors (`ct(<key>, ...)`) followed in
[C10](#c10-liquid-view-keys).

Evidence: `tests/test_rust_liquid_compatibility.py` (Electrum with a custom
timeout and Esplora: fresh, restart, reorg, an incremental block, a forced
rebuild, a wrongly blinded owned output that fails the sync like Python, a
trimmed descriptor and an LWK-discovered overlap; CLI envelopes and stored
bytes) over `tests/rust_liquid_chain.py`, whose Electrum and Esplora fakes
serve the new `compat_*` phases of `crates/wallets/tests/vectors/lwk_chain.json`
(the generator now builds a chain with its foreign parents, after the LWK
chains, whose bytes are unchanged). Mutations: `routing-liquid-*` (16, all
killed); `fetch-esplora-empty-inventory` is re-anchored and killed again.

### C6: Bitcoin Core RPC wallets

`fetch/core.rs` ports `bitcoinrpc_sync_adapter` for descriptors and address
lists. Python keeps one watch-only node wallet per Kassiber wallet (the
backend's `wallet`, else `<walletprefix>-<wallet id>`): listed, loaded or
created with private keys disabled (three `createwallet` shapes, as Python
tries them). A descriptor imports each branch's range with
`importdescriptors` (`getdescriptorinfo` first, the birthday as timestamp,
ends from `highest_used` plus the gap and the stored
`bitcoinrpc_descriptor_range_ends`); an address list imports the addresses
the node does not already watch, falling back to `importmulti`. A pruned
node without a birthday refuses (`bitcoinrpc_birthday_required`), and a
failed rescan maps to `bitcoinrpc_pruned_below_birthday` or
`bitcoinrpc_rescan_failed` with Core's text redacted of the request's
credentials. The history attestation (endpoint, node wallet, birthday,
target fingerprint) forces a re-import when anything it binds changes; a
recreated node wallet ignores the stored checkpoint.

Reads follow Python: `listsinceblock` from the stored last block once the
range is stable and the graph normalization version is current (with its
fallbacks to `listtransactions` paging and the retraction set), else the full
list; `gettransaction` verbose graphs with wallet-visible previous outputs
(`-5` keeps a foreign parent an explicit unknown; any other failure leaves
the txid unavailable and drops the last block); records from the details
(a complete graph books through the shared observation builder, otherwise
the wallet delta with `implicit_wallet_delta`); curated HTLC claim/refund
hints; `listunspent` outputs with their target's derivation; the checkpoint
with `highest_used` and range ends. Core gets the fetch's own targets (its
import range, `_bitcoinrpc_descriptor_targets_for_checkpoint`), not the
offline gap horizon; Liquid on Core refuses as Python does.

`CoreClient` stays observation-only with one exception, the watch-only
wallet a sync uses: `loadwallet`, `createwallet` with
`disable_private_keys`, `importdescriptors` whose descriptors carry no
private key text (extended `?prv` or WIF), and `importmulti` requests that
are `watchonly` without `keys` or `redeemscript`. Imports use a client with
at least Python's 30-minute import timeout. Every call goes through the
fetch's consent grant.

Evidence: `tests/test_rust_core_rpc_sync.py` (descriptor rounds with a
widened range, `sinceblock` and a forced round; an address list on a named
node wallet, twice; a pruned node without a birthday; a birthday with a
trimmed descriptor; a collaborative spend with a foreign parent; an existing
unloaded node wallet; a recreated node wallet; a receive spent within one
round; a deficit whose in-savepoint rescan (C7 covers Core too) widens the
stored range, which the next round's overlap preflight then spans; HTLC
claim and refund), each
comparing envelopes, stored bytes and the multiset of Core calls against
`tests/rust_core_chain.py`, a loopback Core fake over the shared chain model.
`watch_only_wallet_administration_refuses_key_material_before_connection`
pins the client guard. Mutations: `routing-core-*` (19) and the chain
catalog's `core-*` guard entries, all killed. The tests that used
`bitcoinrpc` as the example of an unsupported backend now use `custom`
(Silent Payments scanners, group SOURCES).

### C7: the rescan after applying (R2)

After publishing a compatibility (or address-list) fetch, Python checks the
wallet's balances inside the same savepoint. A deficit there re-syncs the
wallet once more, still inside the savepoint: forced, at the widened gap
(an address list has none and keeps its targets; Python's prefetch repair
skips it for that reason), from `full_scan_checkpoint` of the checkpoint the
first pass published, with the plain wallet row (no stored graphs). That
pass replaces the outcome, which carries the first pass's
`journal_invalidated` and `observer_superseded` forward and gains
`negative_balance_rescan` (`triggered`, `resolved`, the initial and
remaining events, the original and widened gaps, in Python's key order).
The rescan pass itself never rescans. Dependency (BDK, LWK) passes never
reach it; they repair before the savepoint (R1).

The native publication now splits one `sync_wallet_from_backend` pass
(`apply_pass`) from the enrichment and synced stamp, and takes the rescan as
a callback: `Fetch::rescan` prepares inside the savepoint (the ordinary
route still refuses to prepare in a transaction), and the CLI service and
freshness jobs fetch it under the original grant, with its revocation,
cancellation and report-read permission checks. As in Python, that request
runs while the wallet's write savepoint is open; nothing else changes. A
caller without the callback (the replay fixtures) still refuses with
`negative_balance_repair`.

Evidence: `test_address_list_rescan_after_apply_matches_python` and
`test_descriptor_rescan_after_apply_matches_python` (an excluded receive the
prospective overlay counts; Esplora rescans at gap 500, BDK never does) in
`tests/test_rust_wallet_sync_gaps.py`, and
`test_rescan_starts_from_the_published_checkpoint` (a dropped receive the
first pass retracts is not retracted again) and
`test_native_freshness_rescan_after_apply_matches_python`, each comparing
envelopes and stored bytes. Mutations: `routing-rescan-*` and
`freshness-rescan-after-apply`, all killed; `fetch-transaction`,
`fetch-history-repair-inherits`, `fetch-overlap-partial-electrum` (moved
by E1) and `service-preapply-backend`, `service-apply-cancel`,
`service-apply-permission` are re-anchored and killed again.

Comparing a freshness job on Electrum showed the native compatibility
Electrum route without Python's `backend_fetch` progress events (status
round, history round); `cached_electrum` now emits both
(`test_native_freshness_electrum_address_list_matches_python`,
`freshness-progress-electrum-*`).

### C8: report-read refresh without a backend (Q7)

When the book's merged backend view is empty (every built-in deleted, none
stored, no default), Python's `_run_auto_sync_wallets` runs
`_wallets_sync_payload({"all": True}, strict=False)` instead of its
backend path. The daemon now plans that wallet sync (`wallets_sync`, no
rates) and runs it through the native handler: under the report-read
trigger it enqueues only the source classes the policy enables, runs each
job alone (no batch prefetch), never syncs directly and adds no journal
step; forced, it is the explicit `ui.wallets.sync` with its journal step.
`ok` is recomputed from the error rows, as Python does. The emptiness test
reads the merged view (`Config::names` after `merge`); the previous check
read the unmerged runtime list, which always holds the built-ins, so the
refusal guarded a state it could not detect. Books whose wallets the native
planner refuses (node or file sources) still answer `not_implemented`
(group SOURCES). Evidence: `tests/test_rust_report_read_unconfigured.py`
(report read, rate-limited cache, disabled class, forced; wire bytes with
generated ids and clocks masked, every table). The forced case keeps the
journal class off: Rust stamps journal semantics 2 where Python stamps 7, a
known, separate difference. Mutations: `routing-report-read-*` in
`crates/wallets/scripts/routing_mutations.json`, all killed.

### C9: backend kind aliases

Found while closing C6: `backends create` accepts `mempool` and
`liquid-esplora`, and Python routes both as Esplora through
`normalize_backend_kind` (with the Core and Core Lightning spellings). The
native fetch matched the raw kind, so such a wallet still went to Python, and
its compatibility cache identity normalized `mempool` only. The fetch now
routes by the normalized kind and reports it as the fetch's `kind`, while the
backend it reports (and prepares a repair, rescan or revalidation from) keeps
the kind as resolved, so output inventory rows still store `mempool`, as
Python's do. Evidence: `tests/test_rust_backend_kind_aliases.py` (a `mempool`
descriptor wallet over two fixtures, a `mempool` address list and a Liquid
`liquid-esplora` wallet; the hybrid runner must serve each natively, with
envelopes and stored bytes equal to Python's). Mutations: `routing-kind-*`
(4), all killed.

### C10: Liquid view keys

Several Liquid wallets export `ct(<view key>, ...)` instead of SLIP-77.
Python reads the view key as embit's `BlindingKey` over a descriptor `Key`:
a WIF, compressed public key or extended key with an optional origin and an
allowed derivation (fixed steps, one `<a;b>` set, one `*`; hardened steps on
private extended keys only), whose wildcard and branch count must mirror the
spending descriptor's. The blinding key of a script is the view key derived
like the spending keys (branch, then index) plus
`tagged_hash("elements/blindingkey", script_pubkey)`. That is not ELIP-150's
tweak, so Elements Miniscript's addresses differed. embit means to negate an
odd key but discards the copy `ec_privkey_negate` returns, so it never
negates; the public form agrees. Stored data follows Python, so the native
plan follows embit (`descriptors/view.rs`): its spelling (`h` steps,
lowercase fingerprints), targets and confidential addresses, `can_unblind`
(private keys only) and `liquid_blinding_secret`.

LWK rejects ranged view keys, so these wallets take the compatibility
observers (`descriptor_unsupported`) on both sides; fixed view keys LWK
accepts keep the LWK route on both, through the same descriptor text.

Fixed branches (Python round 14): `liquid_blinding_secret` used to skip
`derive(0)` for a non-ranged branch, so it tweaked the account key's script
(and an underived view key) while the target's address carried the fixed
suffix (`elwpkh(xpub/0/7)`, a view key `xprv/0/7`). Every confidential output
to such a wallet failed the compatibility sync, for SLIP-77 and
`ct(<key>, ...)` alike: missing balance. Python now derives the secret
exactly as the target (`LiquidBlindingSecretTests` in
`tests/test_wallet_descriptors.py`, on `t3code/python-reference-fixes-r7-14`,
cherry-picked here), and so does the native plan (no base-key script any
more). An existing book needs nothing: the failure rolled the whole sync
back, leaving the records, outputs and checkpoint of the last good sync
(`test_unblinding_failure_leaves_records_and_checkpoint_untouched`), so the
next ordinary sync fetches and unblinds the history; a forced rebuild is not
required (`test_fixed_branch_picks_up_confidential_outputs_like_python`: an
explicit-only first sync, then an ordinary one picks the confidential
output up). Fixed shapes LWK accepts on plain Electrum keep LWK's own
unblinding on both sides.

Also matched: uncompressed WIF and 65-byte public view keys (embit keeps
their spelling and blinds with the compressed point) and legacy `{a,b}`
branch sets, which embit reads wherever a key's derivation has one and
spells `<a;b>`, in the view key and on the spending side (Bitcoin too).

Evidence: 32 view-key plans (with uncompressed keys and legacy sets) plus three spending-side legacy-set plans in `crates/wallets/tests/vectors/python_descriptors.json`
(multipath on all three networks, single wildcard, a receive/change pair,
origins, hardened and apostrophe steps, `sh`/`tr`/`wsh` scripts, public keys,
fixed and static keys, and each embit refusal), with previews and configs,
and a new `blinding_secrets` section (per-target secrets or Python's errors)
checked by `liquid_blinding_secrets_match_python_vectors`.
`tests/test_rust_liquid_view_keys.py` syncs a `ct(<tprv>/<0;1>/*, ...)`
wallet over the new `view_*` phases of `lwk_chain.json` (confidential L-BTC,
an issued asset and explicit outputs; fresh, restart, extend, reorg, forced;
Electrum and Esplora) and refuses a public view key like Python, comparing
envelopes and stored bytes. The vector generator computes the view wallet's
addresses with its own implementation of embit's rule, and every earlier
phase keeps its bytes. The `fixed_*` phases pay receive script 7 of the
same key explicitly, then confidentially to a fixed SLIP-77 wallet and to a
fixed WIF (compressed and uncompressed) wallet, and a legacy-set view wallet
syncs the `view_*` phases. Mutations: `W-view-*`, `W-blinding-*` and
`W-legacy-sets` in `crates/wallets/scripts/mutations.json` (14) and
`routing-liquid-view-secret`, `routing-liquid-legacy-sets`, all killed.

### C11: silent payments on Core RPC

The sources port (S6) left a silent-payment wallet on a Core RPC backend
with Python: its ordinary adapter path marked every Core RPC scan an
authoritative chain observation, even an incomplete or untimestamped one,
which could retire or supersede what the book already held. The owner ruled
that a correctness bug under the fail-closed rule. Python round 14
(`t3code/python-reference-fixes-r7-14`, cherry-picked here) applies the
compatibility route's rule on every backend
(`core.sync.silent_payment_scan_authoritative`: the scanner reports the scan
complete and every record carries its confirmation time), pinned by
`test_core_rpc_scans_are_authoritative_only_when_complete` in
`tests/test_silent_payments.py`.

The native route no longer refuses Core RPC: the scan reads the backend's
scanner file or posts to its scan path as for a custom backend (progress and
failures take the ordinary `backend_fetch` form), and publishes with
authority only for a complete, timestamped scan, like Esplora and Electrum.
Evidence: `tests/test_rust_wallet_sync_silent_payments.py` (a Core receive,
the same scan again, a spend and a partial scan; an incomplete and an
untimestamped scan publish without chain observation provenance, a complete
one with it), each comparing output and every manifest table with Python.
Mutations: `routing-sp-core-*` (2), with `routing-sp-authority` and
`routing-sp-custom-not-authoritative` re-anchored; all killed.

### C12: Core RPC graph lookups

`ui.transactions.graph` with `allowPublicLookup` (sources S9) answered
`not_implemented` when a Core RPC backend was in the lookup list, and the
background graph fill after a sync through Core (`wallet_graph_references`,
whose grant names the wallet's own backend) failed the same way. Python
asks Core for `getrawtransaction <txid> true`, decodes the verbose reply
(`_bitcoinrpc_decoded_to_graph_raw`: Core's BTC values, its inline
verbosity-2 prevouts), then resolves each missing parent once through the
cache and the prevout budget: a parent the node cannot see (`-5`) is
skipped, any other failure stops asking, and an incomplete graph returns
with `bitcoin_reference_lookup_incomplete`; a failure before that warns
`bitcoin_reference_lookup_failed_bitcoinrpc`. Both native paths now do the
same at the five-second lookup timeout, under the explicit request's grant
(the public lookup) or the sync's graph grant (the background fill).

Evidence: `tests/test_rust_transaction_graph_public.py` (a Core lookup with
its parent and the cached reread, inline prevouts without a parent request,
five failures, and two parents where a `-5` lets the second through while
another error stops) and `test_native_graph_core_rpc_parity` in
`tests/test_rust_freshness_graph.py` (graphless, partial, complete and a
hidden first parent), comparing responses or job results and every table
with Python against a loopback Core fake. Mutations: `routing-graph-core-*`
(6) and `graph-core-*` in `freshness_graph_mutations.json` (3), all killed.

### Shape guards that stay refusals

Unchanged from round 1 and not chain routes: F4 (nested `config_json`,
flattened by backend resolution before fetch), F5 (`backend_selection`), F8
(`observer_branches`, `observer_identity`), F9 (`scan_horizon`), F11 and Q4
(internal guards). Each refuses a configuration the product does not produce.
