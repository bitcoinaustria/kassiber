# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting suite with a desktop GUI and a CLI, both backed by the same Python daemon.
- The CLI entrypoint lives in [kassiber/cli/main.py](kassiber/cli/main.py). The remaining command implementation surface lives in [kassiber/cli/handlers.py](kassiber/cli/handlers.py).
- Desktop UI: Tauri 2 + React + TypeScript with a Python sidecar daemon. Stack decision lives in [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md); implementation plan lives in [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md). [docs/plan/00-overview.md](docs/plan/00-overview.md) remains the orientation map. The Vite + React 19 + TS + Tailwind v4 + TanStack + Zustand frontend lives at [ui-tauri/](ui-tauri/); Claude Design source mockups are staged under `ui-tauri/claude-design/`. The Tauri supervisor (`ui-tauri/src-tauri/`) keeps one Python daemon process and demuxes JSONL responses by `request_id`; typed UI snapshot kinds and the AI provider/chat surface flow through that daemon boundary. The desktop UI is localized (English/German, expandable) with i18next under [ui-tauri/src/i18n/](ui-tauri/src/i18n/); the UI store's `lang` is the single source of truth and the CLI/daemon stay machine-deterministic (the UI translates their stable codes). Conventions live in [docs/reference/i18n.md](docs/reference/i18n.md).
- External-document reconciliation scope and architecture are captured in [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md).
- Supporting modules (bottom-up — no back-edges into the CLI layer):
  - [kassiber/errors.py](kassiber/errors.py) — `AppError` typed exception carrying `code`, `hint`, `details`, `retryable`.
  - [kassiber/time_utils.py](kassiber/time_utils.py) — timestamp parsing + RFC3339 formatting and `UNKNOWN_OCCURRED_AT`.
  - [kassiber/msat.py](kassiber/msat.py) — `SATS_PER_BTC`, `MSAT_PER_BTC`, `dec`, `btc_to_msat`, `msat_to_btc`.
  - [kassiber/util.py](kassiber/util.py) — tiny type-coercion helpers (`str_or_none`, `parse_bool`, `parse_int`, chain/network normalizers).
  - [kassiber/log_ring.py](kassiber/log_ring.py) — RAM-only bounded log ring + stdlib `logging` handler with secret-floor redaction at insert, `current_request_id` contextvar correlation, and sanitized-traceback helpers. Feeds the `ui.logs.snapshot` daemon kind; the privacy model (two-stage redaction, what reaches AI/disk) is documented in [docs/reference/logging.md](docs/reference/logging.md). Logs must never be written to disk except via explicit user export.
  - [kassiber/envelope.py](kassiber/envelope.py) — JSON envelope contract, `emit`, table/plain/csv output writers, and the `_KIND_SUBCOMMAND_ATTRS` kind map.
  - [kassiber/db.py](kassiber/db.py) — SQLite schema, `open_db`, data-root resolution, settings helpers, and msat column migrations.
  - [kassiber/backends.py](kassiber/backends.py) — named sync backends with SQLite as the canonical store plus optional dotenv bootstrap via `config/backends.env`, along with CRUD helpers.
  - [kassiber/sync_btcpay.py](kassiber/sync_btcpay.py) — BTCPay Greenfield API fetcher used by wallet-configured BTCPay sync and `wallets sync-btcpay`; it reshapes confirmed remote wallet-transaction rows into the existing BTCPay import format so Kassiber can reuse the same notes/tags pipeline.
  - [kassiber/cli/handlers.py](kassiber/cli/handlers.py) — remaining CLI command handlers and compatibility-layer imports while deeper decomposition continues.
  - [kassiber/secrets/](kassiber/secrets/) — SQLCipher keying helpers (`sqlcipher.py`), passphrase prompt/fd plumbing (`prompt.py`), plaintext→encrypted migration (`migration.py`), passphrase rotation (`passphrase.py`), dotenv→encrypted credential lift (`credentials.py`, exposes `kassiber secrets migrate-credentials`), `kassiber secrets {init,change-passphrase,verify,status,migrate-credentials}` CLI (`cli.py`), and the `--*-stdin` / `--*-fd` credential-input helpers (`cli_input.py`).
  - [kassiber/backup/](kassiber/backup/) — `tar | age` backup format: SQLCipher-aware export (`pack.py`), age subprocess + pyrage fallback (`age_cli.py`), strict tar member validation (`safe_tar.py`), and `kassiber backup {export,import}` CLI (`cli.py`).
  - [kassiber/core/attachments.py](kassiber/core/attachments.py) — transaction attachment storage, URL-reference handling, integrity verification, and orphan-file GC for the managed attachment tree.
  - [kassiber/core/transaction_history.py](kassiber/core/transaction_history.py) — append-only transaction metadata provenance. It writes grouped edit events plus field-level before/after/diff rows for notes, tags, exclusion, review/tax status, Austrian overrides, and pricing provenance/value fields; read helpers power per-transaction history, global Activity, stale-report counts, redacted AI-safe reads, audit-package inclusion, and append-only revert.
  - [kassiber/core/engines/__init__.py](kassiber/core/engines/__init__.py) — tax-engine interface/resolver. Both the generic RP2 path and the Austrian (§ 27b EStG) path route through `GenericRP2TaxEngine`; AT profiles surface rp2's `rp2.plugin.country.at.AT` plugin directly so accounting methods and engine semantics come from rp2, while Kassiber keeps Austrian disposal bucketing / Kennzahl mapping on its side.
  - [kassiber/core/tax_events.py](kassiber/core/tax_events.py) — in-memory normalization seam between raw transaction rows and tax-engine inputs, including early quarantine classification for under-specified tax semantics.
  - [kassiber/core/sync.py](kassiber/core/sync.py) — wallet sync orchestration above backend-specific transport details.
  - [kassiber/core/sync_backends.py](kassiber/core/sync_backends.py) — descriptor target discovery plus `esplora`, `electrum`, and `bitcoinrpc` live-sync adapters.
  - [kassiber/core/output_inventory.py](kassiber/core/output_inventory.py) — durable watch-only coin/UTXO inventory model updated by chain-backed wallet sync; stores current/spent outpoints, amounts, confirmation state, receive/change metadata, and source freshness without exposing descriptors, xpubs, backend URLs/tokens, raw wallet config, or raw wallet files through UI/AI surfaces.
  - [kassiber/core/ownership.py](kassiber/core/ownership.py) — pure address/txid ownership reconciliation engine behind `wallets identify`, `ui.wallets.identify` (cache-only) and `ui.wallets.identify_onchain` (verify). Given pasted addresses/txids it matches them (by canonical scriptPubKey, with address-string fallback for Liquid confidential addresses) against the watch-only inventory, imported txids, and offline descriptor derivation (receive + change, floored at the synced index, capped by `--scan-to-index`), naming the owning wallet/branch/index and flagging externals; clearly-malformed inputs are flagged `invalid`. A smart CSV importer (`extract_candidates_from_csv`: delimiter sniffing, common `address`/`txid` header aliases, plus strict content-harvest of 64-hex/real-address cells) feeds the same pipeline from `--csv` (CLI) and a desktop "Import CSV" button that sends file content as `csv_text` — never exposed to the AI tool. txids get a per-leg payment/transfer/receipt classification — locally from `transactions.raw_json` (both esplora and Electrum decode shapes) and, for unseen txids, via the opt-in caller-injected fetcher (`fetch_transaction_legs` / `verify_session` in sync_backends; the empty-script Liquid fee output is not counted). The AI variant drops scriptPubKeys, derivation paths, address indices and branch labels.
  - [kassiber/core/reports.py](kassiber/core/reports.py) — extracted report builders, balance-history calculations, and PDF export assembly behind hookable journal/runtime dependencies. `reports tax-summary` rows include `row_type=swap_fees_year` / `swap_fees_total` summarising persisted `transaction_pairs.swap_fee_msat` and `direct_swap_payouts.swap_fee_msat`.
  - [kassiber/core/report_verify.py](kassiber/core/report_verify.py) — the self-verifying XLSX layer appended to `reports export-xlsx` (default on; `--no-verify` / daemon `{"verify": false}` to skip). Adds `Verify` (how-to + tolerance cell), `Acquisitions` / `Disposals` (raw journal ledger with only msat/fiat inputs hard-typed) and `Control` (per-asset reconciliation matrix) sheets whose derived cells are live `write_formula`s carrying Kassiber's number as the cached value, each checked OK/DIFF against it. Reconciliation is per asset at profile scope (`ending basis = Σ acquisition fiat_value − Σ disposal cost_basis`, a method-independent identity); per-disposal lot selection under FIFO/LIFO/HIFO/LOFO is not reproducible by formula and is called out instead of faked. The Austrian E 1kv and exit-tax XLSX exports do not have this layer yet.
  - [kassiber/core/transfer_matching.py](kassiber/core/transfer_matching.py) — pure swap-candidate matcher with `payment_hash` (exact), `htlc_refund` (exact; failed-swap refund linked to its lockup via `swap_refund_funding_txid`, same-wallet and window-independent, default kind `swap-refund`), and time + amount (strong) confidence bands, signed fee computation, conflict cluster ids plus match-time `conflict_size` (stamped over the full candidate set so filtered views cannot make a cluster member look solo), and pair/dismissal suppression. Defaults: 24h time window, fee tolerance `max(1%, 2500 sats)`.
  - [kassiber/core/lightning/](kassiber/core/lightning/) — read-only Lightning scaffold: typed `NodeSnapshot` / `NodeChannel` / `NodeForward` shapes, `LightningAdapter` Protocol, registry (`register_adapter` / `resolve_adapter` / `registered_kinds`), and the generic `build_profitability_report` / `profitability_csv_rows` helpers. Node adapters (LND, Core Lightning, NWC, …) live in sibling modules and register themselves with the registry; the daemon kinds `ui.connections.node.snapshot` and `ui.reports.lightning_profitability` plus the `reports lightning-profitability` / `reports export-lightning-profitability-csv` CLI commands dispatch through the registry. The desktop / CLI path returns the full payload (`snapshot_to_dict` / `LightningProfitabilityReport.to_envelope_payload`); the AI tool dispatch swaps in redacted variants (`snapshot_to_dict_for_ai` / `to_ai_envelope_payload`) that drop the Tier-3 identity graph (operator pubkey, channel funding outpoints, peer pubkeys / aliases, short channel ids on channels and forwards, per-channel covers-open-cost rows). Adapters MUST follow the discard policy in [docs/reference/lightning-opsec.md](docs/reference/lightning-opsec.md): drop preimages, payment_secrets, full encoded bolt11 strings, route hop pubkey lists, route hints from received invoices, and `failure_source_pubkey` at the adapter boundary; pass `None` for `NodeChannel.peer_pubkey` on private channels (enforced at construction by `__post_init__`). `NodeChannel.__post_init__` enforces the `None`-for-private rule on `peer_pubkey` and runs format-only checks on `short_channel_id` / `funding_outpoint` so smuggling fails at the dataclass boundary; `NodeForward.failure_reason` is a categorical `NodeForwardFailureReason` Literal so adapters cannot smuggle raw node error blobs.
  - [kassiber/core/lightning/lnd.py](kassiber/core/lightning/lnd.py) — LND REST adapter implementing the scaffold's `LightningAdapter` Protocol. Registers itself on import under `kind="lnd"`. Talks to `/v1/getinfo`, `/v1/channels`, `/v1/channels/closed`, `/v1/switch`, `/v1/payments`, `/v1/invoices`, `/v1/balance/{blockchain,channels}`, and `/v1/fees`; sanitizes preimages, encoded bolt11 strings, route hops, and `failure_source_pubkey` before any payload reaches the scaffold shapes. TLS settings (`certificate`, `insecure`) are read via `backend_value` so DB-resolved backend rows are honored.
  - [kassiber/core/htlc_parser.py](kassiber/core/htlc_parser.py) — pure parser for Boltz v1 P2WSH HTLC redeem scripts (submarine + reverse variants), claim witnesses (`extract_from_claim_witness` → `payment_hash`), and refund/timeout-branch witnesses (`extract_from_refund_witness` → `role="refund"`, no preimage; the funding-link signal for failed swaps). Boltz v2 Taproot cooperative spends fall through to heuristic by physics.
  - [kassiber/core/swap_rules.py](kassiber/core/swap_rules.py) — auto-pair rules engine with predicate matching, specificity sort, conflict-cluster skip, and a `detect_repeating_patterns` helper for "create rule from pattern" prompts.
  - [kassiber/core/saved_views.py](kassiber/core/saved_views.py) — generic saved-view CRUD (surface-discriminated). First consumer is the swap-candidate queue (`surface="swap_candidates"`).
  - [kassiber/core/samourai.py](kassiber/core/samourai.py) — local-only Samourai/Whirlpool descriptor-source importer: accepts explicit public descriptor/xpub source sets for Deposit/Badbank/Premix/Postmix/Ricochet sources, creates a redacted logical wallet group, and rejects backup files, recovery words, passphrases, private keys, or other secret-bearing material.
  - [kassiber/tax_policy.py](kassiber/tax_policy.py) — profile tax-policy layer.
  - [kassiber/wallet_descriptors.py](kassiber/wallet_descriptors.py) — descriptor normalization, chain/network validation.
- Packaging is defined in [pyproject.toml](pyproject.toml).
- User-facing behavior is documented in [README.md](README.md).
- Third-party dependency and license notes are tracked in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
- In-flight and deferred work is tracked in [TODO.md](TODO.md) — it is the
  current execution backlog. Plan docs under [docs/plan/](docs/plan/) are
  orientation and product guardrails, not the task source of truth.

Phase 0 core extraction is green: the CLI/runtime surface is split out of
the old `kassiber/app.py` monolith, the smoke suite passes, and future
work should build on the extracted modules instead of re-growing a shim.

Kassiber is currently in **dev mode**: renaming commands, breaking flags, and reshaping subcommand trees is acceptable as long as docs in the tree are updated in the same change. There is no deprecation-alias layer.

## Current architecture

- Data lives in a local SQLite database (system of record).
- Default user state lives under `~/.kassiber/{data,config,exports,attachments}` unless `--data-root` / `--env-file` overrides it; the managed layout manifest lives at `~/.kassiber/config/settings.json`.
- The CLI model is:
  - backend (canonical SQLite rows in the `backends` table plus optional dotenv bootstrap)
  - workspace
  - profile (carries tax policy defaults)
  - account
  - wallet
  - transactions
  - attachments
  - metadata (notes, tags, inclusion)
  - journals (RP2 processing + quarantine)
  - reports (summary, tax-summary, balance-sheet, portfolio-summary, capital-gains, journal-entries, balance-history, austrian-e1kv, austrian-tax-summary, exit-tax, export-pdf, export-summary-pdf, export-csv, export-xlsx, export-austrian, export-austrian-e1kv-pdf, export-austrian-e1kv-xlsx, export-austrian-e1kv-csv, export-exit-tax-pdf, export-exit-tax-xlsx)
  - rates (local cache + Coinbase Exchange sync + CoinGecko fallback + Kraken CSV archive ingest + manual override)
  - diagnostics (public-safe bug-report collection)
- Every command accepts `--format {table,plain,json,csv}`, `--output <path>`, `--machine` (= `--format json`), `--debug`, `--diagnostics-out <path|auto>`, and `--db-passphrase-fd FD` (used to unlock a SQLCipher-encrypted database non-interactively).
- Successful responses use `{kind, schema_version, data}`. Errors use `{kind: "error", schema_version, error: {code, message, hint, details, retryable, debug}}`.
- The Tauri supervisor routes daemon responses by `request_id`, not by kind.
  Streaming requests emit intermediate records such as `ai.chat.status`,
  `ai.chat.delta`, `ai.chat.tool_call`, `ai.chat.tool_consent_required`, and
  `ai.chat.tool_result` to the `daemon://stream` Tauri event channel; the
  exact-kind terminal record (or an error) resolves only the matching request.
  `ai.chat.status` is a progress hint for loading/thinking phases, not
  chain-of-thought content.
  Mutating tools may emit `ai.chat.tool_call` twice for the same `call_id`:
  first with `needs_consent: true`, then after approval with
  `needs_consent: false` to mark that same call as running. Clients should
  upsert tool cards by `call_id` instead of rendering duplicate cards.
  `ai.chat.cancel` and `ai.tool_call.consent` take
  `args.target_request_id` so the control request keeps its own routing
  `request_id`; cancelled chats finish with `finish_reason: "cancelled"`.
  Unsolicited daemon→UI records (e.g. the background freshness worker's
  `ui.freshness.progress` / `ui.freshness.background` /
  `ui.freshness.worker`) use a dedicated event envelope class — top-level
  `event: true`, never a `request_id`, built via
  `build_event_envelope` in [kassiber/envelope.py](kassiber/envelope.py) —
  which the supervisor forwards on the `daemon://event` channel. Any other
  post-ready record without a `request_id` is a fatal supervisor protocol
  error, so new daemon-side worker threads must emit through
  `build_event_envelope`, not `build_envelope`.
  `ai.chat` accepts `persist: true|false|"auto"` (absent = false) and
  `session_id`; persisted exchanges land in `ai_chat_sessions` /
  `ai_chat_messages` inside the SQLCipher boundary, the terminal record
  carries `session_id`, and session management is exposed via
  `ui.chat.sessions.{list,get,delete,clear}` plus
  `ui.chat.history.configure` for the GUI policy control. Chat history is
  not an AI tool — the model cannot browse/search prior sessions unless the
  user explicitly resumes one as chat context — and diagnostics/audit packages
  exclude it.
- In-app AI read tools are explicit daemon kinds, not generic CLI or daemon
  dispatch. Current read-only AI kinds are `status`,
  `ui.overview.snapshot`, `ui.transactions.list`,
  `ui.transactions.extremes`, `ui.transactions.search`, `ui.wallets.list`,
  `ui.backends.list`, `ui.profiles.snapshot`, `ui.reports.capital_gains`,
  `ui.reports.summary`, `ui.reports.balance_sheet`,
  `ui.reports.portfolio_summary`, `ui.reports.tax_summary`,
  `ui.reports.balance_history`, `ui.reports.lightning_profitability`,
  `ui.connections.node.snapshot`, `ui.journals.snapshot`,
  `ui.journals.quarantine`, `ui.journals.transfers.list`, `ui.rates.summary`,
  `ui.rates.coverage`, `ui.report.blockers`,
  `ui.audit.changes_since_last_answer`, `ui.audit.evidence.summary`,
  `ui.transactions.history`, `ui.activity.history`, `ui.activity.stale`,
  `ui.wallets.utxos`, `ui.wallets.identify`, `ui.maintenance.settings`, `ui.workspace.health`,
  `ui.next_actions`, and virtual
  `read_skill_reference`. Lightning kinds require a registered adapter
  (`kassiber.core.lightning.register_adapter`); LND ships an adapter that
  registers itself on import, and Core Lightning / NWC adapters will
  land later. The daemon returns an `lightning_adapter_unavailable`
  error envelope when a wallet's kind has no adapter registered. `ui.backends.options` is a desktop setup helper that
  returns safe backend names and metadata without exact URLs or tokens.
  `read_skill_reference("index")` returns only the
  compact in-app skill routing document; deeper references stay allowlisted.
  Desktop onboarding and connection setup use explicit mutating daemon kinds
  `ui.onboarding.complete`, `ui.wallets.create`, `ui.connections.btcpay.create`,
  `ui.connections.bullbitcoin_wallet.create`,
  `ui.wallets.import_samourai`,
  `ui.connections.btcpay.discover`,
  `ui.wallets.identify_onchain` (the desktop Reconcile "Verify on chain"
  action: same reconciliation as the read-only `ui.wallets.identify`, but
  fetches unseen txids through an Esplora/Electrum backend — network access, so
  it is a mutating kind and is not exposed to the AI), and
  `ui.metadata.bip329.import`; transaction editor metadata saves use
  `ui.transactions.metadata.update` and append grouped transaction edit
  history rows in the same SQLite transaction when a real value changes;
  `ui.transactions.history.revert` creates a new forward edit rather than
  rewriting old history; transaction evidence reuse uses
  `ui.attachments.copy` and must duplicate managed files under a new attachment
  id rather than sharing `stored_relpath`; desktop Settings maintenance uses
  `ui.profiles.reset_data` for confirmed per-book testing resets and
  `ui.rates.kraken_csv.import` for local Kraken CSV/ZIP history backfills and
  `ui.reports.export_audit_package` for DB-backed auditor handoff packages.
  Do not model the Connections dialog as a
  command-template picker. Connection setup should select from configured
  chain/indexer backends via `ui.backends.options`; BTCPay setup can create a
  BTCPay instance inline from URL + API key, discover stores/payment methods,
  or reuse a saved BTCPay instance for repeat store-wallet mappings. For
  discovered stores, support both BTCPay-only wallet-source setup and
  existing-settlement-wallet mapping. BTCPay-only creates one Kassiber wallet
  per selected sync-supported BTCPay payment method. Existing-wallet mapping
  records BTCPay provenance routes on already configured settlement wallets,
  so descriptor/file sync remains the balance source while BTCPay comments and
  labels enrich matching transactions. When no explicit payment method is
  supplied, Kassiber stores the default BTC on-chain payment method internally
  for repeat sync.
  `ui.transactions.list` supports bounded filters for `limit`, `direction`,
  `asset`, `wallet`, `since`, `sort`, and `order`. `ui.backends.list` is
  scoped to the active profile and exposes URL presence metadata, not exact
  endpoint URLs. `ui.source_funds.*` exposes the reviewed source-of-funds
  workstation: source/link/evidence listing, suggestion seeding, explicit link
  review, report preview, and gated PDF export without adding generic CLI
  dispatch. Stale local journals may be automatically refreshed before
  AI read/report tools and direct GUI reads of journal-derived report kinds,
  with the `ui.journals.process` result included in tool result metadata for
  AI calls. Wallet/backend sync can be allowed per active profile via
  `ui.maintenance.configure`, and `ui.maintenance.run` can explicitly sync and
  refresh journals after consent; otherwise sync remains explicit because it
  contacts external services and imports new transactions. AI/UI sync metadata
  must not expose exact backend URLs, and partial sync errors must surface as
  blocking report-readiness state. Do not expose raw shell, raw filesystem,
  arbitrary CLI execution, descriptors, xpub material, secrets, env files,
  wallet config JSON, or raw wallet files through AI tools.
- Browser dev mode can exercise the real daemon over the Vite loopback bridge:
  `pnpm --dir ui-tauri run dev:bridge` serves the React app at
  `http://127.0.0.1:5173`, forwards invokes through `/__kassiber__/daemon`,
  and streams `ai.chat` as NDJSON from `/__kassiber__/daemon/stream`.
- Live sync kinds implemented: `esplora`, `electrum`, `bitcoinrpc`. BTCPay Greenfield confirmed on-chain wallet history sync is available through wallet config and `wallets sync-btcpay`.
- BIP329 records are stored in SQLite and transaction labels are bridged into Kassiber tags.
- BTCPay CSV/JSON imports become transactions, with comments mapped to notes and labels mapped to tags. Wallet-configured BTCPay sync and `wallets sync-btcpay` reuse that same normalization for confirmed Greenfield wallet history.
- Transaction attachments are stored in a managed `attachments/` state sibling; file attachments are copied locally and URL attachments remain literal strings with no fetching or indexing.
- Profile-level tax defaults are stored on `profiles` as `fiat_currency`, `tax_country`, `tax_long_term_days`, and `gains_algorithm`.

## Desktop daemon kinds (allowlist lockstep)

The Tauri shell is deny-by-default: it forwards a daemon `kind` to the Python
daemon only when that `kind` is in a hand-maintained allowlist. A new `ui.*`
kind the desktop UI invokes must be added to every layer below, or it works in
mock dev mode and then fails in the packaged app with `kind_not_allowed` (and
in `pnpm dev:bridge` with HTTP 403). `pnpm dev:browser` (`VITE_DAEMON=mock`)
never consults an allowlist, so the mock dev server does **not** prove a kind
is wired correctly — reproduce against `dev:bridge` or the packaged shell.

When you wire a new desktop-invoked `ui.*` kind, update all of:

| Layer | List | Location |
| --- | --- | --- |
| Python daemon (must handle the kind) | `SUPPORTED_KINDS` | [kassiber/daemon.py](kassiber/daemon.py) |
| Compiled Tauri shell (forwards to daemon) | `ALLOWED_DAEMON_KINDS` | [ui-tauri/src-tauri/src/lib.rs](ui-tauri/src-tauri/src/lib.rs) |
| Browser dev bridge (`pnpm dev:bridge`) | `ALLOWED_BRIDGE_KINDS` | [ui-tauri/vite.config.ts](ui-tauri/vite.config.ts) |
| If the kind streams progress records | `STREAMING_DAEMON_KINDS` + `STREAM_CAPABLE_BRIDGE_KINDS` | same two files |

The allowlist is a privilege boundary, not just routing config:

- The webview can invoke only what is listed. AI runtime kinds stay gated
  separately (`AI_RUNTIME_KINDS` in `lib.rs`); never expose raw shell,
  filesystem, descriptors, xpubs, secrets, env files, wallet config, or
  `reveal-*` kinds to the webview invoke path.
- AI-only read tools (e.g. `ui.transactions.search`, `ui.report.blockers`,
  `ui.audit.changes_since_last_answer`) and unsolicited daemon→UI event kinds
  (e.g. `ui.freshness.background`, `ui.freshness.worker`) are intentionally
  **not** in the desktop allowlist. Do not add a kind just to silence a test —
  confirm the desktop UI actually invokes it first.

Enforced in the quality gate by
[tests/test_connection_catalog_drift.py](tests/test_connection_catalog_drift.py):
`ALLOWED_DAEMON_KINDS` and `ALLOWED_BRIDGE_KINDS` must stay equal and remain a
subset of `SUPPORTED_KINDS`, and every `ui.*` kind the React app invokes through
the real transport must be present in `ALLOWED_DAEMON_KINDS`.

## Command surface

- `init`, `status`, `daemon`, `chat`, `context {show,current,set}`
- `chats {list,show,delete,clear,config}` — persisted AI chat sessions
  (stored in the SQLCipher DB; `auto` policy persists only when encrypted)
- `secrets {init,init-resume,change-passphrase,verify,status,migrate-credentials}`
- `backup {export,import}`
- `workspaces {list,create}`
- `profiles {list,create,get,set}`
- `accounts {list,create}`
- `wallets {kinds,list,create,get,update,delete,reveal-descriptor,sync,sync-btcpay,attach-btcpay,attach-bullbitcoin-wallet,derive,identify,import-json,import-csv,import-btcpay,import-phoenix,import-river,import-bull,import-coinfinity,import-21bitcoin,import-strike,import-ledger,ledger-template,import-samourai}`
- `backends {kinds,list,get,create,update,delete,reveal-token,set-default,clear-default}`
- `transactions {list,export}` (`export --export-format {csv,xlsx} --file [--wallet]` writes the styled transaction ledger — notes, tags, counterparty, linked-file/URL attachments — reusing the report's Transactions sheet; daemon kinds `ui.transactions.export_csv` / `ui.transactions.export_xlsx`)
- `attachments {add,list,remove,verify,gc}`
- `metadata records {list,get,note {set,clear},tag {add,remove},excluded {set,clear},history {list,activity,stale,revert}}`
- `metadata bip329 {import,list,export}`
- `journals {process,list,transfers {list},quarantined,events {list,get},quarantine {show,clear,resolve {price-override,exclude}}}`
- `transfers {pair,list,unpair,payouts {list,create,delete},suggest,bulk-pair,dismiss,rules {list,create,apply,delete,enable,disable}}`
- `views {list,create,delete}` — generic saved-view CRUD; ``swap_candidates`` is the first surface consumer
- `source-funds {sources {list,create,attach},links {list,create,review,attach,bulk-review},suggest,cases {list}}`
- `reports {summary,tax-summary,balance-sheet,portfolio-summary,capital-gains,journal-entries,balance-history,lightning-profitability,source-funds,austrian-e1kv,austrian-tax-summary,exit-tax,export-pdf,export-summary-pdf,export-csv,export-xlsx,export-lightning-profitability-csv,export-source-funds-pdf,export-austrian,export-austrian-e1kv-pdf,export-austrian-e1kv-xlsx,export-austrian-e1kv-csv,export-exit-tax-pdf,export-exit-tax-xlsx}`
- `rates {pairs,sync,rebuild,latest,range,set}`
- `diagnostics {collect}`
- `ai providers {list,get,create,update,delete,set-default,clear-default}`
- `ai {models}` — provider/model management; chat lives at top-level `chat`,
  which drives the daemon `ai.chat` tool loop with consent/cancel parity.

## Pagination

List endpoints with `--limit` also accept `--cursor`. The cursor is an opaque base64 urlsafe token built from `<occurred_at>|<created_at>|<id>`. Responses include `next_cursor` (or `null`) and `has_more`.

## Tax engine

- The tax engine now goes through `kassiber/core/engines.build_tax_engine(...)`; both `generic` and `at` profiles route through `kassiber/core/engines/rp2.py`, with Austrian profiles selecting `rp2.plugin.country.at.AT` through the shared seam.
- Journal processing first normalizes raw transaction rows into in-memory tax events via `kassiber/core/tax_events.py`; raw `transactions` rows remain the source of truth and no derived regime state is persisted back onto them.
- Under-specified tax semantics that used to fall through raw-row handling should quarantine at the normalization boundary instead of being guessed. That includes malformed same-asset transfers, missing required pricing, and unsupported tax directions.
- The generic RP2 engine now owns the per-profile journal orchestration behind the engine seam: transfer detection, manual-pair application, per-asset grouping, normalized event preparation, and holdings aggregation all live in `kassiber/core/engines/rp2.py`, while CLI handlers only load rows and persist the resulting journal state.
- Snapshot coverage for the current generic transfer path lives in [tests/fixtures/generic_rp2_transfer_snapshot.json](tests/fixtures/generic_rp2_transfer_snapshot.json) and is enforced by `tests/test_review_regressions.py` in addition to the CLI smoke suite.
- Policy selection and RP2 country defaults are centralized in `kassiber/tax_policy.py`.
- RP2 runs per-asset (pooled across all wallets of a profile) so `IntraTransaction` (MOVE) carries cost basis between user-owned wallets. Wallet identity is preserved by setting RP2's `exchange` to the wallet label and recovering per-wallet quantity buckets via `BalanceSet`.
- Self-transfer detection lives in `kassiber/transfers.py`. The detector pairs same-`external_id` outbound + inbound rows across two wallets of the same profile; the journal pipeline turns each pair into an `IntraTransaction` plus `transfer_out` / `transfer_in` (and, when there's a fee, `transfer_fee`) ledger entries.
- Cross-asset swap-candidate detection lives in [kassiber/core/transfer_matching.py](kassiber/core/transfer_matching.py). It surfaces unpaired candidates via `transfers suggest` / `ui.transfers.suggest`, computes the `swap_fee_msat` once at match time, clusters conflicting candidates so bulk-pair can never silently choose wrong, and respects active pairs + non-expired dismissals from the `transaction_pair_dismissals` table. See [skills/kassiber/references/swap-matching.md](skills/kassiber/references/swap-matching.md) for the full surface.
- Manual pairing via `transfers pair / list / unpair` (table `transaction_pairs`) overrides auto-detection: `apply_manual_pairs` in `kassiber/transfers.py` filters out any auto-pair that touches a manually-paired row. Pair rows persist `swap_fee_msat`, `swap_fee_kind`, `confidence_at_pair`, and `pair_source` (one of `manual` / `bulk_exact` / `bulk_selected` / `rule_auto`); `unpair` soft-deletes via `deleted_at` instead of hard-DELETE so the audit row survives and the partial unique indexes from commit-1 schema migrations immediately stop blocking re-pair of the same legs. Same-asset manual pairs currently support `--policy carrying-value` and feed the existing IntraTransaction path; same-asset `--policy taxable` is rejected and users should leave those legs unpaired to preserve normal SELL + BUY treatment. Cross-asset pairs (BTC ↔ LBTC peg-ins/peg-outs, submarine swaps) are always surfaced via `cross_asset_pairs` in the ledger state and the `journals process` envelope. For `generic` profiles they still process as normal SELL + BUY because RP2 `IntraTransaction` is same-asset only. For Austrian (`at`) profiles, reviewed cross-asset `--policy carrying-value` pairs get `at_swap_link` markers and then run through rp2's native multi-asset carry hook, while cross-asset `--policy taxable` pairs stay on the normal SELL + BUY path. Same-wallet cross-asset pairs are allowed so manual peg-ins/peg-outs can be recorded without forcing duplicate wallet records. Direct swap payouts (`transfers payouts create`) model a source-wallet outbound that a swap provider settles directly to an external recipient; the reviewed `payout_fiat_value` is the taxable source-row proceeds for ordinary direct payouts, while Austrian cross-asset `carrying-value` payouts synthesize an in-memory target-asset acquisition and immediate disposal so the swap stays neutral but the payout remains a taxable sale.
- Liquid peg-in/peg-out detection must not lean on hardcoded federation addresses (per-claim tweaked, federation keys rotate). Use the manual pair CLI or non-address heuristics (time + amount + direction inversion + same-profile constraint) instead.
- Per-wallet portfolio rows show that wallet's residual quantity at the asset's average residual basis — an allocation, not a physical-lot answer.
- Supported lot selection: `FIFO`, `LIFO`, `HIFO`, `LOFO`.
- Profiles support `generic` and `at` (Austrian, § 27b EStG) tax policies. AT profiles delegate engine defaults to `rp2.plugin.country.at.AT` (`moving_average_at`, accepted accounting methods, `open_positions`, English fallback), while Kassiber consumes rp2's `classify_disposal()` API to persist Austrian semantic buckets and current Kennzahl mappings. Typed Austrian fields on `NormalizedTaxEvent` (`at_regime`, `at_pool`, `at_swap_link`) are Kassiber's internal source of truth for the marker wire format; rp2 owns native carried-basis computation. See [docs/austrian-handoff.md](docs/austrian-handoff.md) for the full current carry-basis contract.
- Journals must be reprocessed after any transaction, metadata, or exclusion change before reports are trusted.
- Transactions without usable fiat pricing are quarantined during journal processing instead of receiving zero-basis tax treatment.

## Working rules

- Keep the project local-first.
- For user-facing desktop UI strings, add keys to the i18n resource bundles
  (English + German in lockstep) and render via `t(...)` instead of hardcoding
  literals; keep the CLI/daemon English and machine-deterministic. Migrate a
  whole surface at a time so a screen is never half-translated. German is
  Austrian German in the informal `du` register — use the canonical terms in
  [docs/reference/i18n-glossary.md](docs/reference/i18n-glossary.md) (Bitcoin
  jargon stays English; Austrian BMF tax wording), and the mechanics in
  [docs/reference/i18n.md](docs/reference/i18n.md). Verify UI string changes
  from `ui-tauri/` with `pnpm typecheck` (type-safe keys catch typos/missing
  keys) and `pnpm test --run` (the en/de key-parity guard catches a
  half-translated namespace); both run in CI under the `verify` check.
- Treat code, README, AGENTS.md, and TODO.md as current truth. Treat
  `docs/plan/` as concise guardrails; if code and plans drift, inspect code and
  update the docs in the same change.
- Frontend package management uses `pnpm` under `ui-tauri/`; use the
  `packageManager` pin in `ui-tauri/package.json`. Keep
  `ui-tauri/pnpm-lock.yaml` committed with any `ui-tauri/package.json` change,
  do not add npm/yarn lockfiles, and do not bypass the pnpm 90-day minimum
  release-age policy in `ui-tauri/.npmrc` without explicit owner approval.
- Do not run `npx ...@latest`, `pnpm dlx ...@latest`, shadcn blocks, or other
  remote scaffolders that can rewrite project files unless the user explicitly
  approves that exact run. Treat approved scaffolder runs as overwrites and log
  them in `AGENT_OVERWRITES.md`.
- Any dependency change must explain why existing local code or the standard
  library is not enough, keep the relevant lockfile in the same commit, update
  `THIRD_PARTY_LICENSES.md`, and update README/setup docs if runtime or install
  expectations change.
- Do not overwrite, regenerate, or scaffold over existing files unless the user
  explicitly grants that specific overwrite. Record every approved overwrite in
  `AGENT_OVERWRITES.md` with the date, files, approval source, reason, and
  command/tool used. If an overwrite happens accidentally, stop, document it in
  that file, and report it before continuing.
- When committing is in scope, prefer small reviewable commits for cohesive
  behavior/test/doc slices instead of one large end-of-branch dump. Commit after
  each coherent green checkpoint when practical, separate refactors from feature
  behavior, and keep each commit easy to inspect on its own.
- Favor component-based UI implementation and focused modules. Do not grow
  multi-thousand-line files when a clear split by component, hook, helper, or
  domain responsibility would make the code easier to review; avoid purely
  mechanical splits that do not improve ownership or readability. When touching
  a UI file over roughly 800 lines or a Python file over roughly 1200 lines,
  consider an extraction and mention why a split was or was not made.
- Keep generated or scaffolded code isolated from hand-written behavior changes
  in commits whenever practical. Mark generated files clearly when the generator
  supports it, and avoid editing generated output by hand unless that exception
  is documented near the change.
- Keep Kassiber as the BTC-side subledger and reconciliation layer; invoice issuance, VAT workflow, and the company general ledger stay outside Kassiber.
- For merchant and document-linked flows, keep provenance capture, commercial matching, and RP2-facing tax normalization as separate layers.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Keep `--machine` output deterministic — add a `kind` to every new envelope.
- Keep envelope error shapes consistent: use `AppError(code=..., hint=..., retryable=..., details=...)`.
- Per-asset pooling is intentional so RP2 `IntraTransaction` works across wallets; per-wallet output remains via `BalanceSet`. Do not regress to per-wallet RP2 calls without thinking through the transfer story first.
- RP2 owns tax primitives and computation; do not push invoice, ERP, or broader business-workflow concepts into RP2 unless the tax math itself truly requires them.
- Austrian tax semantics live on the rp2 side (plugin: `rp2.plugin.country.at`). Kassiber emits typed markers, feeds reviewed pairs into rp2's native carry path, and maps rp2's disposal categories onto current Austrian report buckets / Kennzahlen; it does not re-implement Alt/Neu classification, cross-asset carry, or moving-average math beyond the documented marker/quarantine contract in [docs/austrian-handoff.md](docs/austrian-handoff.md).
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.
- Prefer lightweight compatibility migrations for existing SQLite databases when adding profile fields.
- When a `TODO.md` item is completed or materially reshaped, update
  `TODO.md` in the same change and check or split the item so the backlog
  stays truthful.
- Before pushing a code or docs change, review both `git diff --cached`
  and any unstaged `git diff` separately from the implementation pass.
  When second-agent tooling is available, have that reviewer inspect the
  same diff; otherwise do a manual second-pass review yourself. Fix any
  P1/P2 correctness or consistency issues before push, and mention any
  deferred lower-severity concerns in the handoff.
- For non-trivial changes touching CLI behavior, tax logic, schema,
  reports, or multiple docs, gather repo evidence first, then restate the
  requirement, risks, and step plan before editing.
- Prefer the repo-local `skills/kassiber/` references before generic
  agent habits when working on Kassiber-specific flows.
- Before calling work push-ready, run `./scripts/quality-gate.sh`. That gate is
  Python-only; for `ui-tauri/` changes also run `pnpm typecheck && pnpm test --run && pnpm lint`
  there (the desktop UI's typecheck, en/de i18n key-parity, and lint).
- When adding a new runtime dependency, update both the README dependency story and `THIRD_PARTY_LICENSES.md`.
- Keep `THIRD_PARTY_LICENSES.md` concise: direct dependencies and notable license constraints matter more than a hand-maintained transitive dump.

## Prerelease binary workflow

- `.github/workflows/prerelease-binaries.yml` is intentionally not a normal PR
  workflow. Do not add PR-triggered binary builds unless the user explicitly
  asks for that policy change.
- `v*` tag pushes build CLI and desktop artifacts and publish them to a GitHub
  prerelease. Manual `workflow_dispatch` runs build/upload artifacts for the
  selected ref; they only publish when `publish_release=true` and `tag_name`
  names an existing tag.
- If the user asks for binaries for a PR or branch, run the workflow manually
  against that branch and leave the result as workflow artifacts. Do not create
  a release for PR/tester builds.
- CLI archives are named `kassiber-cli-<target>.tar.gz`, and the extracted
  executable is named `kassiber`. Desktop preview files are named with the
  `kassiber-desktop-<target>-...` prefix. Raw bundled sidecar files use Rust
  target triples internally and must not be published as release assets.
- Desktop preview artifacts bundle one-file `kassiber-cli-*` sidecars and
  should not require an external Python checkout for normal daemon calls. The
  GUI executable forwards `--cli ...` to the bundled CLI sidecar.
- The workflow run and release tag identify the source commit, and the desktop
  shell displays the build commit beside the version number. CLI artifact
  filenames and `.sha256` sidecars do not embed the commit hash yet. Do not
  claim full embedded build metadata until the workflow adds a `BUILD_INFO`
  file.
- Operational commands and artifact details live in
  [docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md).

## Verification

All commands below assume project dependencies are installed — either via `uv sync` (then prefix with `uv run`) or via `pip install -e .` inside an activated venv (then use `python3` directly). The examples use `uv run python` because it works without pre-activation; swap in `python3` when working inside an activated venv. For the baseline push/PR pass, use `./scripts/quality-gate.sh` as the single trusted entrypoint; the commands below are the underlying pieces. The full testing model (framework, gate, determinism seams, and the planned real-node integration harness in issue #312) is documented in [docs/reference/testing.md](docs/reference/testing.md).

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py
```

- End-to-end CLI smoke test (stdlib `unittest`, no pytest dep, ~1s):

```bash
uv run python -m unittest tests.test_cli_smoke -v
```

  This is the behavior pin. If you refactor internals the suite MUST
  still pass unchanged — it asserts envelope `kind` + `schema_version`,
  msat fields, Phoenix import counts, balance-sheet totals, and
  error-envelope shape. Prefer extending this suite to adding new test
  files.

- CLI smoke checks:

```bash
uv run python -m kassiber --help
uv run python -m kassiber --machine status
uv run python -m kassiber backends list
uv run python -m kassiber wallets kinds
uv run python -m kassiber wallets sync-btcpay --help
uv run python -m kassiber wallets identify --help
uv run python -m kassiber wallets import-river --help
uv run python -m kassiber wallets import-coinfinity --help
uv run python -m kassiber wallets import-21bitcoin --help
uv run python -m kassiber wallets import-strike --help
uv run python -m kassiber wallets import-ledger --help
uv run python -m kassiber wallets ledger-template --help
uv run python -m kassiber profiles create --help
uv run python -m kassiber metadata records --help
uv run python -m kassiber attachments list --help
uv run python -m kassiber journals events --help
uv run python -m kassiber journals transfers list --help
uv run python -m kassiber reports summary --help
uv run python -m kassiber reports tax-summary --help
uv run python -m kassiber reports austrian-e1kv --help
uv run python -m kassiber reports austrian-tax-summary --help
uv run python -m kassiber reports export-austrian --help
uv run python -m kassiber reports export-austrian-e1kv-xlsx --help
uv run python -m kassiber reports export-austrian-e1kv-csv --help
uv run python -m kassiber reports balance-history --help
uv run python -m kassiber rates --help
uv run python -m kassiber diagnostics collect --help
uv run python -m kassiber chat --help
uv run python -m kassiber chats --help
uv run python -m kassiber ai --help
uv run python -m kassiber ai providers --help
uv run python -m kassiber ai providers create --help
```

- Safe local workflow:
  - create a temp data root via `--data-root /tmp/smoke/data`
  - `init`, then create workspace/profile/wallet and seed transactions
  - verify `profiles list` shows `tax_country` and `tax_long_term_days`
  - import priced CSV, BTCPay CSV, or Phoenix CSV
  - import BIP329 JSONL
  - process journals
  - run each report, including `reports summary`, `reports tax-summary`, and `reports balance-history --interval month`
  - exercise the rates cache: `rates pairs`, `rates set BTC-USD <ts> <rate>`, `rates latest BTC-USD`, `rates range BTC-USD --start <ts>`; optionally `rates sync --pair BTC-USD --days 7` when network access is acceptable

## Known gaps

- BTC-denominated amounts are stored as INTEGER msat in SQLite. Machine envelopes expose both `amount` (BTC float) and `amount_msat` (integer), and the same for `fee` / `quantity`. Fiat columns (`fiat_value`, `fiat_rate`, etc.) are still REAL.
- Rates cache (`rates pairs/sync/latest/range/set`) stores BTC-USD / BTC-EUR samples from Coinbase Exchange by default, CoinGecko fallback, local Kraken OHLCVT CSV archive ingest (`rates sync --source kraken-csv --path <csv-zip-or-directory>`), or manual upsert. Coinbase Exchange sync stores sparse 1-minute candles from chunked 300-minute public API windows. Kraken CSV ingest is local-file only, keeps 1-minute sparse candles, and stores the close as the lookup rate plus OHLCVT metadata. `journals process` can auto-fill missing transaction prices from the cache when a matching sample exists at or before the transaction timestamp, but reports still use stored transaction and journal pricing rather than querying the cache live.
- Phoenix Lightning wallet CSV import is implemented (`wallets import-phoenix`). River Bitcoin Activity / Account Activity CSV import is implemented (`wallets import-river` and `--source-format river_csv`). Bull Bitcoin order CSV import is implemented as exchange evidence (`wallets import-bull` and `--source-format bullbitcoin_csv`), while Bull's unified wallet transaction CSV is wallet-scoped (`--source-format bullbitcoin_wallet_csv`) and can be split by `bullbitcoin_wallet_network` or mapped onto existing wallets with `wallets attach-bullbitcoin-wallet`. Coinfinity order CSV import is implemented (`wallets import-coinfinity` and `--source-format coinfinity_csv`). 21bitcoin transaction CSV import is implemented (`wallets import-21bitcoin` and `--source-format 21bitcoin_csv`). Strike CSV import is implemented (`wallets import-strike` and `--source-format strike_csv`). A generic manual ledger importer is implemented (`wallets import-ledger`, source format `generic_ledger`): a fill-in `.xlsx` (read via `openpyxl`) or CSV/TSV template whose `Type` column maps onto real `(direction, kind)` pairs, one Bitcoin leg per row, with the fiat leg becoming exact `exchange_execution` pricing. `wallets ledger-template` writes the blank template (no DB; `.xlsx` via XlsxWriter or `.csv` by extension), and `ui.transactions.ledger_template` is its desktop kind; the desktop import reuses `ui.wallets.import_file` with `source_format=generic_ledger`. A preview-before-import path is also available: `wallets import-ledger --dry-run` (no DB) and the read-only `ui.wallets.ledger_preview` daemon kind both reuse the loader + per-row normalizer to return `{rows_read, mapped, errors, problems[], preview[]}` without persisting (collecting every rejected row's problem at once, unlike the real import which stops at the first); the desktop Generic-ledger setup panel renders that preview before the import action. Non-template files are auto-detected: `infer_ledger_columns(header)` remaps an arbitrary export's columns (date; a Type/Side column, or direction/sign, or separate sent/received columns; fee; fiat currency/price/value; note; tx id) onto the ledger shape and feeds the same normalizer (taxonomy + exact pricing preserved); rows without an explicit type become `Buy`/`Sell` when a cash counterleg is present, otherwise `Deposit`/`Withdrawal` by direction. Unrecognized columns raise `ledger_unrecognized` (preview returns `confident: false` + detected columns). Template files keep the native path. An explicit `column_map` overrides the guess.
- No `custom` wallet kind CSV mapping DSL yet.
- No account adjustments yet.
- No per-profile Tor proxy configuration yet.
- No descriptor/xpub-native live sync through `bitcoinrpc` yet.
- No self-hosted Liquid `elements_rpc` backend yet.
- No BTCPay invoice/payment provenance ingest yet beyond confirmed on-chain wallet history plus comment/label carry-through from wallet-configured BTCPay sync.
- LND (`kind="lnd"`) and Core Lightning (`coreln`) are implemented as read-only node snapshot adapters behind the shared scaffold (`kassiber/core/lightning/lnd.py` and `kassiber/core/lightning/cln.py`); NWC (`nwc`) is declared but does not sync yet.
- No REST/server mode or multi-user auth yet.
- Generic cross-asset carrying-value is still unsupported: outside Austrian profiles, BTC ↔ LBTC peg-ins/peg-outs and submarine swaps remain audit-linked SELL + BUY pairs rather than a cost-basis-carry primitive.
