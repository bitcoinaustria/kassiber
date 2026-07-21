# Daemon Reference

Kassiber's desktop shell talks to the Python core through a local JSONL
daemon. The daemon is started by the Tauri supervisor, reads one JSON object
per line from stdin, and writes one JSON envelope per line to stdout.

Start it directly for development:

```bash
python -m kassiber --data-root /tmp/kassiber-demo/data daemon
```

The daemon bootstraps the normal Kassiber runtime: it creates the state
layout if needed, opens the SQLite database, and serves status from that
local store.

The Tauri supervisor starts the same command. In development it prefers the
repo-local `.venv/bin/python`, then `.venv\Scripts\python.exe` on Windows,
then `python3`. Set `KASSIBER_PYTHON=/path/to/python` to override the Python
executable, or `KASSIBER_REPO_ROOT=/path/to/checkout` when the development
binary should run against a different checkout.

Packaged prerelease desktop builds bundle a one-file PyInstaller CLI sidecar
and prefer that bundled executable before the development Python fallback when
starting the daemon. `KASSIBER_PYTHON` remains the highest-priority override
for daemon startup and installed-app CLI forwarding during debugging.

The first line is always a lifecycle envelope. The `supported_kinds` array
below is representative; use the live `daemon.ready` payload from the running
daemon for the exact current allowlist:

```json
{
  "kind": "daemon.ready",
  "schema_version": 1,
  "data": {
    "version": "...",
    "supported_kinds": [
      "status",
      "ui.overview.snapshot",
      "ui.workspace.overview.snapshot",
      "ui.transactions.list",
      "ui.transactions.resolve",
      "ui.transactions.metadata.update",
      "ui.transactions.history",
      "ui.transactions.history.revert",
      "ui.activity.history",
      "ui.activity.stale",
      "ui.wallets.list",
      "ui.wallets.utxos",
      "ui.backends.list",
      "ui.backends.options",
      "ui.reports.capital_gains",
      "ui.reports.export_pdf",
      "ui.reports.export_summary_pdf",
      "ui.reports.export_csv",
      "ui.reports.export_xlsx",
      "ui.reports.export_capital_gains_csv",
      "ui.reports.export_austrian_e1kv_pdf",
      "ui.reports.export_austrian_e1kv_xlsx",
      "ui.reports.export_austrian_e1kv_csv",
      "ui.journals.snapshot",
      "ui.journals.events.list",
      "ui.journals.quarantine",
      "ui.journals.transfers.list",
      "ui.journals.process",
      "ui.profiles.snapshot",
      "ui.profiles.create",
      "ui.profiles.switch",
      "ui.rates.summary",
      "ui.rates.kraken_csv.import",
      "ui.rates.rebuild",
      "ui.freshness.status",
      "ui.freshness.configure",
      "ui.freshness.run",
      "ui.freshness.cancel",
      "ui.freshness.pause",
      "ui.freshness.resume",
      "ui.workspace.health",
      "ui.workspace.freshness.run",
      "ui.workspace.create",
      "ui.workspace.delete",
      "ui.profiles.reset_data",
      "ui.projects.list",
      "ui.projects.create",
      "ui.projects.select",
      "ui.secrets.init",
      "ui.secrets.change_passphrase",
      "ui.next_actions",
      "ui.wallets.create",
      "ui.wallets.import_file",
      "ui.wallets.document_import.preview",
      "ui.wallets.document_import.import",
      "ui.wallets.import_samourai",
      "ui.connections.btcpay.create",
      "ui.connections.bullbitcoin_wallet.create",
      "ui.connections.btcpay.discover",
      "ui.connections.btcpay.test",
      "ui.connections.node.snapshot",
      "ui.reports.lightning_profitability",
      "ui.metadata.bip329.preview",
      "ui.metadata.bip329.import",
      "ui.metadata.bip329.export",
      "ui.wallets.update",
      "ui.wallets.delete",
      "ui.wallets.sync",
      "ui.sync.status",
      "ui.sync.enable",
      "ui.sync.disable",
      "ui.sync.transports.list",
      "ui.sync.transports.configure",
      "ui.sync.transports.delete",
      "ui.sync.push",
      "ui.sync.pull",
      "ui.sync.join_request",
      "ui.sync.invite",
      "ui.sync.join",
      "ui.sync.members.list",
      "ui.sync.members.revoke",
      "ui.sync.devices.list",
      "ui.sync.devices.revoke",
      "ui.sync.conflicts.list",
      "ui.sync.conflicts.resolve",
      "daemon.lock",
      "daemon.unlock",
      "ai.providers.list",
      "ai.providers.get",
      "ai.providers.create",
      "ai.providers.update",
      "ai.providers.set_api_key",
      "ai.providers.delete",
      "ai.providers.set_default",
      "ai.providers.clear_default",
      "ai.providers.acknowledge",
      "ai.list_models",
      "ai.test_connection",
      "ai.chat",
      "ai.chat.cancel",
      "ai.tool_call.consent",
      "wallets.reveal_descriptor",
      "backends.reveal_token",
      "daemon.shutdown"
    ]
  }
}
```

`ui.reports.export_summary_pdf` writes a managed stakeholder summary PDF. It
accepts optional `start` / `end` RFC3339 timestamps, optional `wallets` as an
array of wallet ids or labels, and `include_snapshot` to prepend an as-of-now
cover snapshot. The export response includes `data_integrity` (now with an
`internal_transfers` count + volume), period-end `wallet_holdings` /
`holdings_totals`, current `snapshot_wallets` / `snapshot_totals` when
requested, and `balance_history` so callers can reconcile the generated PDF
against the portfolio-summary and balance-history report surfaces. The
response also surfaces `metrics` (including `unrealized_pnl`, `end_cost_basis`,
and BTC stack start/end), a `benchmark` block with BTC spot performance over
the period, plus `top_movements`, `top_disposals`, and `holding_age` so
callers can re-render the same narrative outside the PDF. The export is
portfolio-shaped and deliberately omits tax tables; Austrian tax PDFs remain
the authoritative tax handoff.

`supported_kinds` is the public UI allowlist the Tauri supervisor mirrors;
treat this list (not the docs) as the source of truth for what the supervisor
will pass through. Reveal kinds (see below) are included in the list but still
require their own passphrase round-trip before the daemon returns raw secret
material.

The `ui.sync.*` family backs Settings → Device sync and is never registered as
an AI tool. It operates only on the active encrypted profile. Mailbox push/pull
can emit unsolicited `ui.sync.progress` records through
`build_event_envelope`; these carry `event: true` and no `request_id`, while the
terminal response still matches the invoked kind. Transport responses expose
only safe endpoint-origin/presence metadata, never credentials or full remote
paths. See [`device-sync.md`](device-sync.md).

`ui.reports.privacy_hygiene` is a local-only, read-only privacy facts payload
shared by Settings -> Privacy, `kassiber reports privacy-hygiene`, and the
assistant read tool `ui_reports_privacy_hygiene`. It performs no network probe,
does not sync wallets, and does not refresh or mutate journals. Findings carry
`evidence_level` (`exact`, `derived`, or `unknown`) plus count-only evidence.
The payload is AI/export-safe by construction: it omits addresses, scripts,
descriptors, xpubs, backend URLs/tokens, wallet config JSON, raw importer JSON,
branch/index values, and derivation paths. The desktop Settings screen may
still show operator-facing endpoint rows through the existing first-party
backend-settings permission; that richer local UI view is separate from the
privacy-hygiene payload and is not what the AI tool receives.

`ui.reports.privacy_mirror` is the north-star local privacy report behind the
Privacy Mirror page, wallet/transaction detail privacy panels,
`kassiber reports privacy-mirror`, and the assistant read tool
`ui_reports_privacy_mirror`. It composes the redacted linkage graph and
privacy-hygiene facts into exposure summary, adversary cards, wallet,
transaction, UTXO, timeline, coverage, unknown, evidence-drilldown, and
worst-risk rows. It is local-only, read-only, advisory-only, and carries
`evidence_level` on every result. The payload uses the same AI/export redaction
rules as privacy hygiene: no addresses, scripts, descriptors, xpubs, backend
URLs/tokens, wallet config JSON, raw JSON, branch/index values, or derivation
paths. `ui.reports.psbt_privacy` is a desktop/CLI local preflight path for raw
unsigned PSBT text; it is not in the assistant tool catalog, and raw PSBT
contents must not be exposed to AI. See
[`privacy-mirror.md`](privacy-mirror.md) for methodology, degraded states,
redaction, and non-goals.

`ui.transactions.resolve` is the narrow local lookup used by deep links and
global search: it accepts a Kassiber transaction id or external transaction id
scoped to the active profile and returns at most one safe transaction display
row. It does not create a browser-side search index.

`ui.transactions.graph` is the read-only transaction-detail graph model. It
returns one transaction's safe metadata, public input/output references, fee
metadata when locally known, graph support level, warnings, ownership/accounting
annotations, and reviewed paired-route context. Bitcoin transactions with
stored valued vin/vout can render a proportional flow graph; records with only
safe references render a reference/amountless graph; graphless imports return a
typed empty state. Liquid confidential transactions may expose public
references while keeping confidential amounts unsized or hidden. When the user
allows a configured public backend lookup, the daemon caches only the sanitized
reference graph inside the local DB/SQLCipher boundary, keyed by schema version,
chain, network, and txid, so reopening the same transaction does not refetch the
same public tx/prevtx material. Kassiber deliberately does not persist raw
serialized transactions for this graph cache: the graph endpoint needs only the
normalized refs, prevout values/scripts, and size metadata required to rebuild a
complete current-transaction graph, not witnesses, arbitrary script payloads, or
backend response shape. The payload never returns descriptors, xpubs, backend
URLs/tokens, wallet config, raw files, raw JSON blobs, or other secret-bearing
material.

`ui.transactions.review_context` is the AI-oriented composite review packet.
It accepts one transaction id/txid plus bounded include flags and joins the
safe transaction row with the local graph, journal events, append-only edit
history, audit/evidence readiness, attachment labels, commercial provenance,
source-funds links, reviewed transfer/direct-payout state, loan marks/hints,
privacy findings, staleness, and deterministic next-action descriptors. It
never performs public lookup. Raw evidence URLs, managed paths,
document bytes, backend identities, descriptors, xpubs, and raw importer JSON
are removed before provider context. The kind is AI-only and intentionally
absent from the desktop invoke allowlist.

`ui.review.worklist` is the AI-only bounded accounting queue. It composes
report readiness, quarantine, stale edits, transfer candidates, and heuristic
loan locks by default; commercial and source-funds sections are opt-in
categories. `ui.workspace.overview.snapshot` is AI-readable only for the
chat's frozen workspace and keeps every profile/book boundary visible.

`ui.wallets.utxos` accepts `{"wallet":"<wallet id or label>"}` and returns the
active local UTXO inventory for one wallet. Rows include outpoint, txid, vout,
asset, amount, confirmation status, block/time when known, address or safe
receive/change label, branch/index when known, and first/last-seen freshness.
Wasabi-imported wallets also include privacy-accounting fields such as
`anonymity_score`, `spent_by`, `excluded_from_coinjoin`, and `key_state`; the
AI variant drops addresses, labels, branch/index values, and anonymity-history
details.
The row payload is capped and includes `summary.returned_count`,
`summary.count`, `summary.truncated`, and `summary.row_limit`; asset totals and
freshness counts are computed against the full active inventory, not just the
returned rows. The response includes backend name/kind only; it never returns
descriptors, xpubs, backend URLs/tokens, raw wallet config, wallet files, or raw
backend payloads. AI-facing UTXO rows further redact address, label, branch,
and index details. Unsupported sources return
`support.status="unsupported_source"`. Liquid wallets return
`support.status="liquid_unblind_blocked"` unless their descriptor material can
unblind and account for outputs locally.

`ui.privacy_hygiene.snapshot` is the read-only Phase-1 privacy-tells surface.
It accepts optional `{"wallet":"<wallet id or label>", "transaction":"<id or
txid>", "limit": 50}` arguments and returns advisory privacy tells, risk
counts, unknown counts, wallet rollups, transaction rows to review, and coverage
counts. Every finding carries `evidence_level` (`ground_truth`, `reviewed`,
`imported`, `heuristic`, or `unavailable`), remediation text, and attribution
(`user_wallet`, `counterparty`, or `local_data`). Inbound counterparty-side
tells remain visible as context but do not increase the receiving wallet's risk
weight. The engine is local-only over already stored `transactions.raw_json`
plus `wallet_utxos`: it never fetches missing prevouts, calls explorer/entity
APIs, or widens exposure through a public backend. Bitcoin rows with local
`vin`/`vout` get single-transaction heuristics such as common-input, round
amount, fee/RBF, script-type mix, change, wallet-fingerprint, OP_RETURN, and
CoinJoin/PayJoin boundary evidence. Graphless imports, current Bitcoin Core
detail rows, and confidential/incomplete shapes degrade to address-level
coverage or `not_analysable`; missing transaction refs return explicit
`not_found`. The daemon does not return raw JSON, addresses, scriptPubKeys,
descriptors, xpubs, backend URLs, tokens, wallet config, branch/index details,
or derivation paths. Privacy heuristics are advisory only and never mutate tax
lots, exclusions, balances, transfer pairs, or source-funds state. Cross-wallet
adversary reconstruction, peel chains, Boltzmann entropy, coin-selection advice
or fingerprinting, and PSBT pre-broadcast checks remain out of this Phase-1
surface.

`ui.transactions.history` and `ui.activity.history` read redacted,
append-only metadata edit events from the same local database as transactions.
They include stale-report summary metadata by default; callers that only need a
timeline page can pass `include_stale=false` and use `ui.activity.stale`
separately so the first page is not coupled to profile-wide stale aggregation.
`ui.activity.stale` summarizes edit events that happened after the last journal
processing run so report-readiness prompts can explain why reports are stale.
`ui.transactions.history.revert` applies a selected field or event snapshot as a
new forward metadata edit; it never updates or deletes prior history rows.

`ai.providers.set_api_key` accepts
`{"name":"provider","api_key":"..."}` or `{"name":"provider","api_key":null}`.
It is the desktop API-key rotate/re-enter path. The terminal envelope is
redacted and contains `has_api_key` plus `secret_ref.{store_id,state}`, never
the raw key. `ai.providers.move_api_key` accepts
`{"name":"provider","store_id":"sqlcipher_inline|macos_keychain|windows_dpapi|linux_secret_service"}`
and moves an existing provider key through the desktop-only native-store bridge.
OS-backed refs that cannot be read return `secret_ref_unavailable` with
`details.refs` and a Settings repair path.
`ai.providers.create`, `ai.providers.update`, and `ai.test_connection` reject
caller-supplied `api_key`; desktop callers must set or rotate keys through
`ai.providers.set_api_key` and then test the stored provider.

`ui.profiles.switch` accepts `{"profile_id":"..."}` and updates the active
book set / book (`context_workspace` / `context_profile` internally) after the
database is already unlocked. It does not create a per-book passphrase
boundary; SQLCipher encryption is database-level.

`ui.profiles.create` accepts `{"workspace_id":"...","label":"..."}` and creates
a new book in that book set. `workspace` and `profile` remain the daemon/API
names for book set and book. It inherits fiat currency, tax country, long-term
period, and gains algorithm from the active book in that set when available;
otherwise it uses the first book in the set, then generic EUR/FIFO defaults for
empty sets. It can also accept `source_profile_id` to copy those settings from a
specific book in the same set. Wallets, accounts/buckets, and transactions are
not copied. The new book becomes active.

`ui.workspace.create` accepts `{"label":"..."}` and creates an empty book set.
`workspace` remains the daemon/API name. The daemon makes the new book set
current and clears the active book until the user creates or switches to
books inside that set.

`ui.workspace.delete` accepts
`{"confirm":"DELETE","confirm_workspace":"..."}` for the current book set. Like
wallet deletes, encrypted databases require `args.auth_response.passphrase_secret`
and plaintext databases require `DELETE LOCAL DATA`.

`ui.profiles.reset_data` accepts
`{"confirm":"RESET","confirm_profile":"...","clear_shared_rates":false}` for
the current book. It keeps the current book set, book, bucket/account
rows, wallet connection rows, and configured backends, then clears
imported/synced transactions, journals, quarantines, swap pairs/dismissals/rules,
saved views, BIP329 labels, transaction tags, attachment metadata/files, and
source-of-funds review state so testing can redo sync, journal processing, and
swap review from the preserved wallet connections. The global local fiat-rate
cache is shared across books in the data root and is only cleared when
`clear_shared_rates` is the boolean value `true`; string values such as
`"false"` are rejected. Like other sensitive local-data changes, encrypted
databases require `args.auth_response.passphrase_secret` and plaintext databases
require `DELETE LOCAL DATA`.

`ui.backends.options` returns safe backend setup choices for desktop forms. It
lists configured backend names, kinds, chain/network metadata, presence flags,
and default state, but does not expose exact endpoint URLs or tokens.
`ui.backends.create` / `ui.backends.update` can mark a backend with
`silent_payments=true` and store either `silent_payment_scan_file` for a local
scanner result or `silent_payment_scan_path` for an explicitly selected
server-assisted scanner API. The file/path values are backend config, not
normal safe output fields. Desktop forms may send replacement values, but
`ui.backends.list` / `ui.backends.options` expose only the safe capability bit
and presence-style metadata, never the saved scanner path itself.
`ui.backends.detect_core` probes common local Bitcoin Core RPC endpoints with
default cookie-file locations plus local `bitcoin.conf` RPC settings and
returns candidate URL/network/auth-source metadata without cookie contents.
When `bitcoin.conf` contains `rpcuser`/`rpcpassword`, the daemon may use those
credentials internally to prove reachability, but the response does not include
the username or password. Cookie-file candidates include a bounded
`credential_ref` for immediate local probing; renderer-supplied cookiefile
probes and desktop-created Bitcoin Core cookiefile backends are constrained to
default `~/.bitcoin/**/.cookie` paths and loopback RPC URLs. Users who want to
save basic auth still enter it explicitly. `ui.backends.bitcoinrpc.test` probes
a saved or inline Core RPC backend with `getblockchaininfo`, `getnetworkinfo`,
wallet RPC, and `getblockfilter`; it reports reachability, peer/sync state,
pruning/IBD, wallet-RPC availability, and BIP158 filter-index availability.
Both endpoints are desktop mutating kinds because they touch local RPC/cookie
state and are not AI tools.

`ui.wallets.create` is the desktop connection setup path for local/imported
wallet sources. It accepts `label`, `kind`, and the same wallet config fields
the CLI stores (`backend`, `chain`, `network`, `descriptor`,
`change_descriptor`, `source_file`, `source_format`, `store_id`,
etc.) and returns the redacted wallet row. Desktop callers can pass
`wallet_material` instead of separate descriptor fields; the daemon recognizes
common descriptor export shapes, including plaintext BSMS descriptor records,
and stores receive/change descriptors when the material contains both.
For `kind="silent-payment"`, callers pass `sp_descriptor` plus one scan
birthday field (`sp_scan_start_height` or `sp_scan_start_date`) or explicit
full-history acknowledgement (`sp_full_history` with
`sp_acknowledge_full_history_warning`). Server-assisted scan mode also requires
`sp_acknowledge_server_warning`. The stored `sp_descriptor` is secret-bearing
watch-only privacy material and never appears in normal list/get/UI/AI payloads;
the redacted wallet row exposes only safe state such as material format, scan
mode, start point, and full-history flag.

`ui.wallets.import_samourai` is the desktop Samourai/Whirlpool watch-only path.
It accepts `label`, optional `backend`, `network`, and `gap_limit`, plus exactly
one public source-set input: `source_set_file` containing explicit
descriptor/xpub sources, or inline `source_set` from the Add Connection form.
Backup files, recovery words, passphrases, and other secret-bearing material are
not accepted. The response returns a redacted logical group plus child wallet
summaries and safe warnings; it does not return descriptors, xpubs, PayNym
secrets, backend URLs, tokens, or raw file payloads.
Samourai descriptor source sets need a receive descriptor per scanned section.
When neither an explicit `change_descriptor` nor a multipath (`<0;1>`)
expression is supplied, Kassiber synthesizes the standard `/1/*` change chain
from the receive descriptor, so internal/change coverage is never missed.

`ui.wallets.preview_descriptor` is a read-only helper for the connection
setup form. It accepts `wallet_material` (or explicit `descriptor` /
`change_descriptor`), optional `chain`/`network`/`count` (1–20, default 5),
and returns the first N derived receive addresses plus the first change
address when present. The daemon does not persist anything; callers use the
preview to confirm a descriptor produces the expected wallet before
committing.

`ui.wallets.import_file` accepts `wallet`, `source_file`, and `source_format`
for wallet-scoped CSV/JSON imports. Wallet CSV import results include
`inserted_records`, `updated_records`, and `unchanged` so the desktop can show
what changed after an exchange CSV import. For
`source_format="bullbitcoin_csv"` or `source_format="coinfinity_csv"`, `wallet`
is optional. The default `mode="relevant"` treats the export as book-wide
exchange evidence and enriches only unique matching transactions in the active
profile. `mode="full"` imports all normalized provider rows into the selected
or default provider wallet as excluded evidence, then flags each row as
`matched`, `unmatched`, or `ambiguous` against this book's wallet transactions.
`source_format="binance_supplemental_csv"` follows the same exchange-evidence
contract, defaults to a `Binance` provider wallet in full mode, and returns
`binance_rows`.
`source_format="bullbitcoin_wallet_csv"` is different: it imports Bull's
unified mobile wallet transaction export as active wallet-scoped BTC/LBTC/
Lightning activity, returns `bullbitcoin_wallet_rows`, skips failed/expired and
self-direction rows, and redacts exported preimages before raw metadata is
stored. Wallet config may include `bullbitcoin_wallet_network` (`bitcoin`,
`liquid`, or `lightning`) to split one unified export into separate source
wallets; filtered results also include `bullbitcoin_wallet_rows_total` and
`bullbitcoin_wallet_network`. Coinfinity imports return `coinfinity_rows`. For
`source_format="21bitcoin_csv"`, the default `mode="full"` imports active
custodial ledger rows into the selected or default `21bitcoin` wallet; explicit
`mode="relevant"` keeps the evidence-only matching behavior for L1 withdrawal
rows. `source_format="pocketbitcoin_csv"` follows the Bull Bitcoin mode
contract, uses a default `Pocket Bitcoin` wallet in full mode, and returns
`pocketbitcoin_rows`. Because Pocket's CSV does not expose the blockchain txid,
relevant-mode matching uses the net BTC amount, direction, asset, and nearby
timestamp. `source_format="strike_csv"` imports active Strike platform ledger
rows into the selected or default `Strike` wallet, including exchange buy/sell
rows plus Lightning and on-chain wallet activity. It keeps Lightning payment
hashes when exported, skips fiat-only platform rows, and returns `strike_rows`.
`source_format="ledgerlive_csv"` imports Ledger Live BTC/LBTC `IN`/`OUT` rows
as wallet movement only, ignores informational countervalues, redacts account
xpub columns from raw metadata, and returns normal wallet import counters.
`source_format="wasabi_bundle"` imports sanitized Wasabi RPC/export bundles
into a wallet-scoped `wasabi` source, returns `wasabi_transactions`,
`wasabi_coins_observed`, `wasabi_coins_active`,
`wasabi_coins_marked_spent`, `wasabi_payments_in_coinjoin`,
`wasabi_wallet_json_present`, and `wasabi_listkeys_count`, and stores only safe
Wasabi wallet metadata plus durable UTXO anonymity evidence. Raw wallet JSON,
full key paths, public keys, xpub/extpub material, encrypted secrets, and
backend URLs remain outside daemon/UI/AI outputs.
The result also includes `matched`, `skipped_unmatched`, and
`skipped_ambiguous` in relevant mode, or `matched`, `unmatched`, `ambiguous`,
`excluded`, and `reconciliation_records` in Bull Bitcoin/Coinfinity/Pocket full
mode.

The desktop first uses the privileged native picker to invoke
`internal.document_import.stage`. That kind is supported by the Python daemon
but intentionally absent from the Tauri and Vite renderer allowlists. It keeps
the canonical source path in a bounded, expiring, process-local daemon session
and returns only an opaque `document_token` plus safe filename/type metadata.

`ui.wallets.document_import.preview` accepts that `document_token`, optional
`provider`, optional `model`, `confidence_threshold`, `max_pages`, and an
optional contiguous `pages` range such as `2-6`. Raw path, draft, and row fields
are rejected. PDFs within the page budget are rendered completely; longer PDFs
require an explicit range, and the result reports total/rendered pages plus
whether the selection is complete. It is read-only, but it calls a local AI
model, so the Tauri shell gates it through `AI_RUNTIME_KINDS`. The daemon stores
the normalized rows, source hash, selected page set, and active profile currency
in a new immutable preview session and returns its fresh token; the renderer
response omits the source path. A later preview therefore cannot change an
already-reviewed snapshot. Non-loopback or non-local providers are rejected,
and missing local vision/OCR models return
`document_import_model_missing` with Ollama model recommendations.

`ui.wallets.document_import.import` accepts `wallet`, `document_token`, and a
required `selected_row_ids` list. It rejects raw paths, renderer drafts/rows,
quarantined-row overrides, and attachment-policy overrides. Selected ids must
belong to ready rows in the daemon-owned preview. The daemon verifies the
stored source hash against the current file before writing, copies the source
into managed attachments for every inserted or enriched transaction, and
consumes the session only after a successful import. Both preview and import
use the long-running supervisor budget because local OCR and multi-row evidence
copying can exceed 15 seconds. A daemon restart, project switch, or expired
session requires selecting and previewing the document again. The public success
result omits both the original source path and managed attachment relative paths.
OCR requests bypass ambient HTTP proxies and reject off-origin redirects. PDF
rendering/page inspection has a hard timeout plus per-page geometry and byte
budgets before any raster is base64 encoded; model ids match exact installed
tags; preview and import cap row counts; and import hashes/copies a stable
source snapshot. Row ids are derived from source bytes and normalized economic
facts, so model response reordering cannot duplicate a retry. Every populated
accounting field requires cell confidence.
Unsupported assets, non-positive amounts, invalid/negative fees, invalid or
non-positive fiat facts, missing/mismatched fiat currencies, and source pages
outside the rendered set stay quarantined,
while projected managed-evidence copies have a bounded storage budget.

`ui.connections.sources` returns the daemon's authoritative catalog of
supported wallet kinds (with summary/config-fields metadata) and the
recognized import `source_formats`. The desktop catalog stays the source of
truth for icons, copy, and ordering, but uses this list to verify it isn't
advertising a "ready" connection backed by a wallet kind or import format
the daemon does not implement.

`ui.connections.btcpay.create` configures a BTCPay store in one of two modes.
The default `wallet_sources` mode creates wallets configured for confirmed
Greenfield wallet-history sync from a BTCPay instance, so a BTCPay-only setup is
enough when BTCPay is the source of the wallet history. The `existing_wallets`
mode maps selected BTCPay payment methods onto already configured settlement
wallets and stores BTCPay provenance routes there; those wallets keep their
normal descriptor/file sync source while BTCPay comments and labels enrich
matching transactions. Both modes accept either a saved `backend` or inline
instance credentials (`backend_label`, `server_url`, `api_key`) plus `label`,
`store_id`, and either optional `payment_method_id` (default `BTC-CHAIN`) or
`payment_method_ids` for bulk setup. In `wallet_sources`, bulk setup creates one
Kassiber wallet per selected payment method and suffixes labels with the
payment method id. In `existing_wallets`, callers pass `routes` containing
`wallet` and `payment_method_id`. Inline credentials create a local `btcpay`
backend row first, then store only the redacted backend reference on the
wallet. Use one Kassiber wallet per real underlying BTCPay-backed wallet
balance; stores that share the same BTCPay wallet should not be duplicated as
separate Kassiber wallets.

`ui.connections.bullbitcoin_wallet.create` configures Bull's unified mobile
wallet CSV in one of two modes. The default `wallet_sources` mode accepts
`label`, `source_file`, and optional `networks` (`bitcoin`, `liquid`,
`lightning`) and creates one `bullbitcoin` wallet per selected network, each
sharing the same CSV path with a different `bullbitcoin_wallet_network` filter.
The `existing_wallets` mode accepts `label`, `source_file`, and `routes`
containing `wallet` plus `network`; it stores Bull export routes on those
wallets so their normal descriptor/file source remains authoritative while Bull
metadata enriches matching rows during `ui.wallets.sync`.

`ui.connections.btcpay.discover` accepts the same saved-backend or inline
instance credential shape as `create`, performs read-only Greenfield discovery,
and returns safe store ids/names plus enabled payment method ids. It does not
persist anything and does not request payment-method config bodies, because
those may contain wallet material. Desktop setup should default to selecting all
sync-supported payment methods for the chosen store and leave unsupported
methods for future source-specific adapters.

`ui.connections.btcpay.test` makes a single Greenfield request against
the saved-backend or inline instance credentials plus `store_id` (and optional
`payment_method_id`, defaulting to `BTC-CHAIN`) to confirm the credentials and
store reference resolve. It returns
`{backend, store_id, payment_method_id, ok: true}` on success, and otherwise
propagates the same structured error codes (`auth_error`, `not_found`,
`network_error`) the sync path uses. Nothing is persisted.

`ui.transactions.commercial_context` reads the reviewed or suggested
commercial provenance for one transaction. It joins wallet transactions to
BTCPay payments, their parent invoices, payment-request ids when present,
normalized origin hints such as POS/app/external-order, and linked external
documents. The payload is a redacted UI read model; it does not expose raw
BTCPay invoice JSON, rejected matches, payment hashes, destination addresses,
full origin URLs, payment-method configuration, descriptors, xpubs, or API
tokens.

`ui.metadata.bip329.preview` accepts `file`, reads a local BIP329 JSONL label
export, and returns exact/ambiguous/unmatched/preserved counts, duplicate and
conflict counts, and the transaction-tag effects that would be applied. It does
not contact a backend or mutate the database.

`ui.metadata.bip329.import` accepts `file` and optional `apply_ambiguous`, then
imports BIP329 JSONL labels into the active profile. Labels are deduplicated by
record type and reference. Every valid row is stored/preserved; by default only
exact transaction matches are bridged into Kassiber tags through the audited
metadata history path. Ambiguous transaction labels are preserved but skipped
unless the caller explicitly applies them after review.

`ui.metadata.bip329.export` accepts optional `mode` (`stored`, `synthesized`,
or `all`) and optional `wallet`, writes a BIP329 JSONL file under the managed
exports directory, and returns `file` / `filename` for the desktop save-as flow.
Wallet-scoped export includes only records Kassiber can tie deterministically to
that wallet.

`ui.transactions.metadata.update` accepts
`{"transaction":"...","note":"...","tags":["Reviewed"],"excluded":false}` for
the active books/profile. The daemon persists the note, replaces the
transaction tag set (creating missing tag rows), updates the exclusion flag, and
invalidates processed journals so reports are rebuilt before use.

`ui.wallets.update` accepts `{"wallet":"..."}` for the active books/profile and
edits at least one of `label`, the same wallet config fields the create
endpoint takes (`backend`, `chain`, `network`, `descriptor`,
`change_descriptor`, `wallet_material`, `source_file`, `source_format`,
`store_id`, `payment_method_id`, `gap_limit`, `addresses`, `policy_asset`),
or `clear` — a list of config field names to remove. `wallet_material`
overwrites the receive/change descriptors when present so users can paste a
fresh export to fix a typo. `ui.wallets.delete` accepts
`{"wallet":"...","confirm":"DELETE","confirm_wallet":"...","cascade":true|false}`.
Both kinds are sensitive local-state changes: encrypted databases require
`args.auth_response.passphrase_secret`, verified with the same throwaway
SQLCipher round-trip used by reveal requests; plaintext databases require an
explicit acknowledgement (`CHANGE LOCAL DATA` for updates, `DELETE LOCAL DATA`
for deletes).

Requests carry a caller-chosen `request_id`, a `kind`, and optional `args`:

```json
{"request_id":"status-1","kind":"status"}
```

Responses use the normal machine envelope plus the same `request_id`.
`schema_version` follows the CLI machine-output contract; bump it only when
consumers must change how they parse daemon envelopes.

```json
{"kind":"status","schema_version":1,"data":{"database_encrypted":true},"request_id":"status-1"}
```

`status.data.database_encrypted` reflects the live database file, so desktop
views should prefer it over persisted UI identity when deciding whether a
local-auth action needs a passphrase or a plaintext acknowledgement.

Errors use the standard error envelope shape and also echo `request_id` when
the request supplied one. Malformed JSON and non-object requests cannot carry
a caller request id, so they return `request_id: null`. `daemon.shutdown`
asks the daemon to write a final shutdown envelope and exit cleanly.

Unsolicited daemon→UI events (records with no originating request, e.g. the
background freshness worker's `ui.freshness.progress`,
`ui.freshness.background`, and `ui.freshness.worker` records) use a dedicated
event envelope class: a top-level `event: true` marker and never a
`request_id`. They are built with `build_event_envelope` in
[`kassiber/envelope.py`](../../kassiber/envelope.py).

```json
{"kind":"ui.freshness.background","schema_version":1,"data":{},"event":true}
```

The Tauri supervisor forwards event records to the `daemon://event` channel
(separate from per-request `daemon://stream` records) instead of routing them
by `request_id`. Apart from the startup `daemon.ready` handshake, any other
post-ready record without a `request_id` — including an event record that
wrongly carries one — is a fatal supervisor protocol error that marks the
daemon broken. The Vite dev bridge has no push channel to the browser, so it
logs event records in the dev-server terminal instead.

`status`, the `ui.*` snapshots, report export kinds, `ui.wallets.sync`,
`ui.freshness.*`, and `ui.journals.process` are backed by real data today.
Report export kinds write files under the active project's managed
`exports/reports/` directory and return the written path plus metadata. UI
kinds not yet wired return `daemon_unavailable` instead.

`ui.overview.snapshot` remains scoped to the active book/profile.
`ui.workspace.health` is also active-context health despite the historical
workspace name; use `ui.workspace.overview.snapshot` for whole book-set reads.
`ui.workspace.overview.snapshot` is the book-set overview read model. It
requires `args.workspace_id`, does not switch the active book, and returns an
operational rollup across all profiles in that workspace: connection tiles,
recent transactions/activity, BTC balance series, portfolio series, fiat rows,
journal freshness, quarantines, and report readiness. Every aggregate also
keeps `profileId`/book labels or a `books[]` boundary so journal and report
warnings point back to the exact book. The payload must never merge tax lots,
transfer semantics, or journal state across books; cross-book values are
treasury/readiness summaries only. If all books share one fiat currency, fiat
totals use the same latest-rate semantics as each book overview. Mixed fiat
sets return BTC-native totals plus per-book fiat rows and mark the fiat rollup
as `mode="mixed"` / `partial=true` instead of converting between currencies.
Desktop drilldowns from the book-set overview are book-scoped routes; they
must make the active-book switch visible before navigating.

## Freshness jobs

Kassiber's daemon owns source freshness. The desktop configures, observes,
retries, pauses, resumes, and cancels jobs, but it never performs network sync
itself. Persistent state lives in SQLite under `freshness_jobs` and
`freshness_source_states`.

Job types are separate so partial success stays usable:

- `onchain_wallet_history` for descriptor/address wallet history through
  Esplora, Electrum, or Bitcoin Core.
- `btcpay_wallet_source` for BTCPay confirmed wallet-history imports.
- `btcpay_provenance` for BTCPay comment/label enrichment on existing wallets.
- `market_rate_coverage` for incremental missing-minute rate coverage.
- `journal_refresh` for follow-up local journal processing.

Market-rate jobs first seed the bundled Kraken BTC hourly BTC-EUR/BTC-USD
archive into `rates_cache` when missing, then fetch a small latest quote from
the configured live market-rate provider for current BTC price display.
Coinbase Exchange is the default provider when none is configured; CoinGecko is
also supported for live latest-price refresh. When the configured provider is
Coinbase Exchange, the job also performs the existing live incremental
minute-coverage pass for exact transaction timestamps. Live provider refresh is
gated on the `market_rates` source class: the foreground and background enqueue
paths skip the market-rate source when it is disabled, and the job handler
itself also refuses any live provider call (returning `live_refresh: false`,
`skipped_reason: market_rates_disabled`) so a profile with market-rate refresh
off never reaches Coinbase Exchange, CoinGecko, or mempool — only the offline
bundled seed runs. Background jobs skip the manual 30-day warm-cache fallback
when no transaction minute is missing, so hourly price refresh stays
provider-light. The bundled offline Kraken hourly seed includes a daily-derived
pre-Kraken backfill from the existing Coin Metrics + ECB-derived daily bundle,
so cached BTC-EUR/BTC-USD coverage starts at `2011-01-01` without adding
another live provider. Kraken CSV remains an offline archive/import path
because it needs a local file or bundled archive.

Source states are `fresh`, `queued`, `syncing`, `paused`, `rate_limited`,
`partially_stale`, `failed`, and `blocking_reports`. Report reads are blocked
only when source staleness, missing rates, or journal readiness makes the
report unsafe. Rate limits are normal state: a 429/`Retry-After` cools down the
affected source/provider only, while other queued jobs can continue. The
`blocking_reports` flag, not the cooldown label alone, decides whether reports
must wait.

`ui.freshness.status` returns the active profile policy, source states, active
jobs, and summary counts. `ui.freshness.configure` writes the general freshness
policy (`background_enabled`, `report_read_sync`, and per-source-class opt-ins).
The legacy `auto_sync_before_report_reads` argument remains accepted and maps
onto `report_read_sync` plus wallet-source opt-ins. `ui.freshness.run` enqueues
and optionally drains due jobs. `ui.wallets.sync` now delegates to that same
daemon-owned queue with `rates=false` and `journals=false`; when a wallet is
supplied it is source-scoped to that wallet, while a book/global refresh can
enqueue the remaining wallet, rate, and journal jobs without duplicating the
already queued source. `ui.freshness.cancel`, `ui.freshness.pause`, and
`ui.freshness.resume` mutate the job/source state.

`ui.workspace.freshness.run` is the explicit book-set refresh path. It requires
`args.workspace_id`, loops through every profile in that workspace, recovers
interrupted jobs, enqueues wallet/rate/journal freshness work for each book,
and drains each book's due jobs without changing active context. Streaming
records use `ui.workspace.freshness.run.progress` and include the workspace and
profile/book currently being processed. The terminal payload groups per-book
results, rate-limit/backoff state, blocking-source counts, and a summary of
which books refreshed and which remain blocked.

When `background_enabled` is true, the daemon starts an opt-in freshness worker
while the app is running. The worker opens its own SQLite connection, enqueues
only policy-enabled sources that are missing, stale, failed, or past the refresh
interval, and drains one due job per pass so manual requests can still observe
and cancel jobs through the same tables. Because background passes have no
originating request, the worker reports through unsolicited event envelopes
(`event: true`, no `request_id`): `ui.freshness.progress` per job phase,
`ui.freshness.background` after a pass that enqueued or completed work, and
`ui.freshness.worker` for worker lifecycle errors. Wallet and journal sources use the
general 15-minute background interval; market-rate sources use an hourly
interval by default. Kassiber opens local databases in WAL mode with an explicit
busy timeout so the daemon foreground connection and the freshness worker can
safely serialize writes instead of failing immediately on ordinary lock
contention.

For SQLCipher databases, the daemon keeps the verified database passphrase only
as unlocked-session state so the background worker can open its own connection.
That reference is not persisted or logged, is cleared on lock, passphrase
rotation, and shutdown paths, and the worker drops its one-shot handoff after
opening the connection. This is a practical local-daemon boundary, not Python
memory zeroization.

First sync progress phases are `discovery`, `backend_fetch`, `decode_enrich`,
`import`, `rate_coverage`, `journal_refresh`, `done`, and `error`. The streaming
`ui.wallets.sync.progress` and `ui.freshness.run.progress` records include the
phase plus source identifiers.

Checkpoints are persisted per source. Electrum stores script-hash statuses,
known txids, dirty mempool scripts, header timestamps, and highest used branch
indexes; repeated syncs batch `blockchain.scripthash.subscribe` and skip
unchanged history/tx/header calls. Esplora stores script stats fingerprints and
known txids; unchanged scripts skip paged history. BTCPay stores page
fingerprints, stable ids, stop reasons, and pagination cursors. Repeat BTCPay
syncs treat a page as unchanged only when both stable ids and the metadata
fingerprint match, stop the head scan after a bounded unchanged-page window, and
advance a deep-audit cursor over older pages so older comment, label, invoice,
or payment metadata edits can still be discovered without walking the whole
history every background pass. Rate jobs reuse the existing
`rates_checked_minutes` cache and do not run destructive rebuilds in background
freshness work.

## Lightning node kinds

`ui.connections.node.snapshot` and `ui.reports.lightning_profitability`
route through the shared
[`kassiber.core.lightning`](../../kassiber/core/lightning/) scaffold. Each
request takes `args.connection` (a wallet id or label that resolves to a
Lightning-kind wallet) and optional `args.window_days` (default 30, max 365).
The daemon resolves the wallet, looks up the registered
[`LightningAdapter`](../../kassiber/core/lightning/adapter.py) for the wallet
kind (`lnd`, `coreln`, `nwc`), and dispatches the read. LND and Core Lightning
ship adapters ([`lnd.py`](../../kassiber/core/lightning/lnd.py) and
[`cln.py`](../../kassiber/core/lightning/cln.py)) that the daemon imports at
startup; NWC remains declared but unavailable. Without a registered adapter the daemon
returns an `lightning_adapter_unavailable` error envelope so the desktop can
surface the unavailable-adapter state without inventing data.

Lightning adapters also declare explicit safe capability metadata through
`LightningCapabilities` (`node_snapshot`, `routing_profitability`,
`channel_balances`, `channel_lifecycle`, `forward_events`,
`invoice_activity`, `payment_activity`, `onchain_balance`). The daemon checks
`node_snapshot` before `ui.connections.node.snapshot` and checks both
`node_snapshot` and `routing_profitability` before
`ui.reports.lightning_profitability`. A registered adapter that lacks the
requested feature returns a deterministic `lightning_capability_unsupported`
error with safe details (`kind`, requested `capability`, and supported
capability names). Capability blocks exposed to the desktop use camelCase
booleans such as `nodeSnapshot` and `routingProfitability`; they never include
peer pubkeys, channel funding outpoints, short channel ids, descriptors,
backend URLs/tokens, raw wallet config, or other identity graph data.

## Encrypted database

When the active project's `kassiber.sqlite3` is SQLCipher-encrypted, the daemon
still bootstraps through the normal runtime path: it accepts
`--db-passphrase-fd <FD>` for that selected project and falls back to an
interactive prompt only if a controlling TTY is attached. The Tauri supervisor
uses the existing `auth_required` challenge and a secret-store bridge whose
passphrase reply is consumed in daemon memory; terminal broker workers use
inherited anonymous fds and never put the passphrase in daemon JSON.

## Reveal kinds (local-auth round-trip)

`wallets.reveal_descriptor` and `backends.reveal_token` return raw secret
material — descriptor bodies, blinding keys, BTCPay/RPC tokens. For an
encrypted database, even when the daemon already has the database open with
the user's passphrase, the first reveal request returns:

```json
{"kind":"auth_required","schema_version":1,"data":{"scope":"reveal_token","label":"Re-enter database passphrase to reveal backend 'btcpay'"},"request_id":"reveal-1"}
```

The client then resends the same request with `args.auth_response =
{"passphrase_secret": "..."}`. The daemon verifies by opening a throwaway
SQLCipher connection against the on-disk file; a wrong passphrase returns the
structured `local_auth_denied` error envelope. This is a UX gate, not
cryptographic separation — once the daemon is running with an unlocked DB it
can read every credential. The auth round-trip exists so a compromised UI
process cannot silently siphon secrets without surfacing a re-prompt.
Plaintext databases have no database passphrase to re-check, so reveal clients
must send `args.auth_response = {"plaintext_reveal_ack": "COPY LOCAL SECRET"}`
after the user types that exact acknowledgement.
The desktop wallet detail screen allowlists only `wallets.reveal_descriptor`
from this owner-reveal surface. It uses the passphrase round-trip for encrypted
books or the plaintext acknowledgement for unencrypted books, then copies the
stored descriptor material as one clipboard payload. When the wallet stores
separate receive and change descriptors, both raw descriptor strings are
included on separate lines; when the wallet stores a combined multipath
descriptor, that single string is copied unchanged. The webview still cannot
invoke arbitrary backend-token or raw-config reveals.

The supervisor and any client must redact `passphrase_secret`, `token`,
`descriptor`, `change_descriptor`, `blinding_key`, `auth_header`, `password`,
and `api_key` fields from any persisted log line.
The daemon also redacts secret-shaped strings and sensitive detail keys at the
error-envelope boundary before responses cross into Tauri, the Vite bridge, or
UI state. Provider-controlled AI error bodies are treated as hostile and are
size-limited plus redacted before they become `error.details.body`.
