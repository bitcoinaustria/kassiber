```text
██╗  ██╗ █████╗ ███████╗███████╗██╗██████╗ ███████╗██████╗
██║ ██╔╝██╔══██╗██╔════╝██╔════╝██║██╔══██╗██╔════╝██╔══██╗
█████╔╝ ███████║███████╗███████╗██║██████╔╝█████╗  ██████╔╝
██╔═██╗ ██╔══██║╚════██║╚════██║██║██╔══██╗██╔══╝  ██╔══██╗
██║  ██╗██║  ██║███████║███████║██║██████╔╝███████╗██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝╚═════╝ ╚══════╝╚═╝  ╚═╝
```

> **Kassiber** is local-first, Bitcoin-native accounting with a desktop GUI
> and a CLI. Your wallets, your books, your taxes — all on your machine.

> [!WARNING]
> Pre-alpha software. Expect crashes, breaking changes, and inaccurate
> accounting or tax output. Do not rely on Kassiber as the only source of
> truth for filings, audits, or financial decisions.

## What Kassiber is

Most Bitcoin accounting tools want you to upload your wallets and
descriptors to a SaaS — a yearly subscription, full trust in the provider,
and every customer's identified holdings concentrated in one database for
someone to breach, subpoena, or sell. Kassiber doesn't. It runs on your
laptop, talks directly to the Bitcoin sources you choose, and keeps every
byte of accounting state in a local SQLite file you control. No server to
hack. No subpoena target. No "we regret to inform you of an incident"
email.

Kassiber is **Bitcoin-native**: descriptors, xpubs, Esplora, Electrum,
Bitcoin Core RPC, BTCPay Greenfield, Lightning, and Liquid — not "any of
600 billion cryptocurrencies". L-BTC is in scope; altcoins are not. Stablecoin
support is on the table for later, depending on future resources.

Tax math runs locally through the open-source
[RP2](https://github.com/bitcoinaustria/rp2) engine. Kassiber prepares,
reviews, and explains; RP2 computes. The Kassiber-maintained RP2 fork
carries a working Austrian (§ 27b EStG) plugin with E 1kv exports.

## Why local-first

- **Bitcoin Native** — descriptors, xpubs, BIP329, Lightning, and Liquid as
  first-class concepts.
- **Privacy First** — no telemetry, no update check, no analytics; every
  outbound request enumerated in [SECURITY.md](SECURITY.md).
- **No remote honeypot** — there is no Kassiber server holding your
  addresses, balances, and identity. Crypto tax SaaS providers have been
  breached; the dumps become targeting lists for phishing and physical
  attacks. Kassiber stores each project/book-set in a local project
  container on your machine.
- **Wrench-attack resistant** — watch-only by design (no spending keys to
  coerce), and optional SQLCipher 4 at-rest encryption keyed by a
  per-project passphrase that lives only in your head. On a stolen,
  customs-seized, or border-searched cold device, the encrypted project
  database — descriptors, xpubs, transactions, stored tokens — is
  unreadable. Attachments, exports, the project catalog, and a couple of
  config files sit outside the SQLCipher boundary, so pair with full-disk
  encryption for the full picture; the caveats are in
  [SECURITY.md](SECURITY.md). The
  [jlopp/physical-bitcoin-attacks](https://github.com/jlopp/physical-bitcoin-attacks)
  catalog covers the threats this addresses.
- **Local AI Chat** — assistant ships with local
  [Ollama](https://ollama.com/) and [oMLX](https://omlx.ai/) provider presets;
  the desktop Assistant and `kassiber chat` both use the same daemon tool loop.
  Off-device providers require explicit per-provider acknowledgement and
  mutating tools require consent.
- **AGPL 3.0** — auditable, forkable, no vendor lock-in.

## Highlights

- **Direct Bitcoin sync** — Esplora, Electrum, Bitcoin Core RPC
  descriptor/xpub/address refresh, BTCPay Greenfield, Liquid Electrum,
  plus watch-only UTXO inventory for chain-backed wallet sources.
- **Imports** — BTCPay CSV/JSON, Phoenix, River, Bull Bitcoin, Coinfinity,
  21bitcoin, Pocket Bitcoin, Strike, Samourai/Whirlpool public descriptor/xpub,
  generic CSV/JSON, a fill-in Excel/CSV ledger template for manual entry,
  local-AI photo/PDF OCR drafts, BIP329 labels.
- **Review workflows** — notes, tags, exclusions, attachments; append-only
  transaction edit history with Activity review and safe revert; reviewed
  transfer/swap pairing for Lightning, Liquid peg-in/peg-out, and submarine
  swaps; reviewed source-of-funds reports with immutable saved cases,
  gated PDF export, audit evidence summaries, manual evidence reuse between
  transactions, and a DB-backed audit package manifest/export for
  trusted handoff.
- **Tax & reports** — RP2 lot accounting (FIFO/LIFO/HIFO/LOFO and moving
  average); Austrian § 27b EStG with E 1kv PDF / XLSX / CSV; summary,
  balance sheet, capital gains, portfolio, balance history; self-verifying
  XLSX export with live recompute formulas so you can check every balance,
  average price, acquisition, disposal and gain in Excel/LibreOffice yourself
  (`--no-verify` for the lean workbook); local
  BTC-USD / BTC-EUR rates cache (configurable live provider, Coinbase by
  default, CoinGecko supported, plus Kraken OHLCVT local archive and
  auto-seeded bundled BTC-only offline history for hourly values, backfilled to
  2011-01-01 with daily-derived Coin Metrics + ECB rows before Kraken hourly
  coverage begins) and
  opt-in desktop background refresh for the latest BTC price.
- **Sovereign storage** — one SQLite system of record per project/book-set;
  optional SQLCipher 4 passphrase encryption; single-project `tar | age`
  backups recoverable with stock `age` + `tar` + `sqlcipher` even if
  Kassiber disappears.
- **Encrypted device and team sync** — opt-in authored-event replication over
  sealed courier files, a shared folder, WebDAV, or S3-compatible storage;
  signed owner/editor/auditor membership, blocking financial-edit conflicts,
  owner snapshots for late joiners, plus explicit SPAKE2 LAN and optional Tor
  fast paths. No Kassiber account, server, live-database copy, or inbound port
  is required for the default mailbox workflow.
- **Optional remembered unlock** — macOS desktop builds can save a desktop-only
  database-passphrase entry for Touch ID unlock. The CLI can separately opt into
  its own item in macOS Keychain, Windows Credential Manager, or Linux Secret
  Service for prompt-free one-shot commands. This is convenience, not recovery
  or a replacement for the SQLCipher passphrase.
- **Two surfaces, one daemon** — desktop GUI (Tauri 2 + React) for
  day-to-day work; CLI with deterministic JSON envelopes for scripting,
  automation, and power users; both backed by the same Python daemon.
- **Localized desktop UI** — English and German, switchable in Settings or the
  header, with the i18n layer built to expand to more languages.

## Install

**Desktop app** — download an unsigned prerelease binary for macOS, Linux,
or Windows from the latest `v*` release. The bundle ships a CLI sidecar,
so no separate Python install is needed. Settings can install a user-local
`kassiber` terminal launcher without administrator privileges. Gatekeeper /
SmartScreen first-launch handling lives in
[docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md).

**From source** (CLI use or development, Python `>=3.10`):

```bash
./scripts/bootstrap-dev-env.sh
export KASSIBER_PYTHON="$PWD/.venv/bin/python"
```

The Python install includes `keyring` so the opt-in CLI remembered-unlock flow
can use macOS Keychain, Windows Credential Manager, or an available unlocked
Linux Secret Service. Kassiber rejects configured third-party/file keyring
backends and never falls back to a plaintext credential file.

## Quick start

### Desktop

Launch the app. The Welcome screen walks you through optional database
encryption, your books set and first book, tax policy, your first wallet
or BTCPay connection, and the optional AI assistant. Every other flow —
Overview, Connections, Imports, Transactions, Swap Matching, Journals,
Quarantine, Reports, Source of Funds, Books, Settings, Logs,
Assistant — is one click away in the sidebar.

Open a wallet in Connections to refresh its source and review its read-only
UTXOs table: currently unspent transaction outputs, amounts, confirmation state,
receive/change position when known, and source freshness. The table shows every
UTXO returned by the capped wallet inventory payload, reports when the response
is truncated, can be sorted by size, chain date, confirmations, or outpoint, and
can open a matching public explorer after the same privacy warning used by
transaction details. Kassiber never constructs
transactions, signs, broadcasts, freezes coins, or selects coins.

### CLI

```bash
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber profiles create main --workspace personal \
  --fiat-currency USD --tax-country generic --gains-algorithm FIFO
python3 -m kassiber context set --workspace personal --profile main
python3 -m kassiber wallets create --label donations --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq
python3 -m kassiber wallets sync --wallet donations
python3 -m kassiber journals process
python3 -m kassiber reports summary
```

For scripts and agents, `--machine` implies `--non-interactive`: stdout is one
JSON envelope and Kassiber returns `interaction_required` instead of prompting.
Discover the live command contract and current project readiness without
scraping help text:

```bash
kassiber --machine commands describe
kassiber --machine commands describe wallets sync
kassiber --machine health
kassiber --machine next-actions
```

Paginated envelopes preserve `next_cursor` / `has_more` and also expose the
same values under `data.page`. High-impact automatic reviews support previews:
`transfers bulk-pair --dry-run`, `transfers rules apply --dry-run`, and
`transfers components bulk-resolve --dry-run`, and
`source-funds links bulk-review --dry-run`. Long 1:N/N:1/N:M wallet migrations,
including missing historical wallets, use the versioned custody-component
workflow documented in
[docs/reference/custody-components.md](docs/reference/custody-components.md).

For an encrypted project, enroll prompt-free CLI unlock once on a trusted local
user account:

```bash
kassiber secrets remember-unlock
kassiber --machine status
kassiber --machine reports summary
kassiber secrets forget-unlock
```

Enrollment verifies the passphrase, saves a CLI-only item in the native OS credential store,
and sets a non-secret `cli_remembered_unlock` marker in the managed
`config/settings.json`. Desktop and CLI use separate per-data-root credential
entries and lifecycle controls, so neither surface silently enrolls or revokes
the other. CLI reads are
not biometric-gated; see [SECURITY.md](SECURITY.md) for the platform trust model.
Headless machines and automation without an unlocked credential service should
continue using `--db-passphrase-fd`.
`kassiber secrets status` reports the stable `access_policy` code for the
platform boundary without reading a disabled CLI credential. If an OS store
cannot remove a CLI-owned legacy item, `legacy_quarantined: true` keeps that
item unavailable to both CLI unlock and desktop migration until cleanup is
retried.

Desktop Settings can remove only Touch ID, while **Forget all unlock methods**
removes the desktop entry, CLI entry, and any migration-only legacy shared item.
Changing the passphrase in the desktop refreshes both enrolled copies. A CLI
rotation refreshes the CLI copy and invalidates desktop Touch ID until the user
re-enrolls it; no surface silently keeps using a stale passphrase.

When syncing descriptor or xpub wallets through your own Bitcoin Core node,
add a Core RPC backend (`--cookiefile` or `--username` / `--password`) and
optionally set `--birthday YYYY-MM-DD` on the wallet to bound Core's
watch-only descriptor rescan. The desktop setup can detect a local Core node
from default cookie paths or `bitcoin.conf`; it reports wallet-RPC and BIP158
filter-index availability, but Kassiber's current Core sync path is still
watch-only descriptor import, not filter-first P2P sync.

To reconcile old flows, `kassiber wallets identify` (or the desktop **Reconcile**
screen) checks whether pasted addresses / transaction ids belong to any of your
wallets — receive or change — and flags the externals, classifying each
transaction as a self-transfer, outbound payment, or inbound receipt.
Wallet script migrations retain a private ownership-history archive so old
receive/change scripts still resolve; unusually deep histories can opt into a
bounded `ownership_scan_to_index` through wallet config.

Exchange and wallet imports include Bitcoin-focused CSV/API paths for River,
Bull Bitcoin, Coinfinity, 21bitcoin, Pocket Bitcoin, Strike, Ledger Live,
Kraken, Coinbase, and Binance. Exact exchange executions are stored as pricing
provenance, while wallet movement remains separate reconciliation evidence; see
[docs/reference/imports.md](docs/reference/imports.md) for commands and limits.

For transfer pairing, swap matching, source-of-funds, Austrian E 1kv,
BTCPay reconciliation, and the concept model, see
[docs/quickstart.md](docs/quickstart.md). The desktop GUI is optional:
the Assistant sidebar and `kassiber chat` speak the same Python daemon, so a
daily flow can move freely between them.

## Architecture

Kassiber is the local-first accounting layer: watch-only source refresh,
storage and provenance, metadata, attachments, transfer pairing, review
and quarantine. [RP2](https://github.com/bitcoinaustria/rp2) is the tax
core — Kassiber prepares and explains, RP2 computes. Invoicing, VAT/RKSV,
and the company general ledger stay out of scope. See
[AGENTS.md](AGENTS.md) for the module map and
[docs/plan/00-overview.md](docs/plan/00-overview.md) for the architecture
overview.

## Documentation

- **User reference** · [Quick start](docs/quickstart.md) ·
  [AI assistant](docs/reference/ai.md) ·
  [Backends](docs/reference/backends.md) ·
  [Imports](docs/reference/imports.md) ·
  [Tax & journals](docs/reference/tax.md) ·
  [Desktop](docs/reference/desktop.md) ·
  [Localization](docs/reference/i18n.md) ·
  [AT glossary](docs/reference/i18n-glossary.md) ·
  [Daemon](docs/reference/daemon.md) ·
  [Machine output](docs/reference/machine-output.md) ·
  [Device & team sync](docs/reference/device-sync.md) ·
  [Prerelease binaries](docs/reference/prerelease-binaries.md) ·
  [Homebrew Cask](docs/reference/homebrew-cask.md)
- **Architecture & plans** · [Overview](docs/plan/00-overview.md) ·
  [Desktop stack ADR](docs/plan/01-stack-decision.md) ·
  [Desktop implementation](docs/plan/04-desktop-ui.md) ·
  [Austrian tax engine](docs/plan/06-austrian-tax-engine.md) ·
  [External document reconciliation](docs/plan/08-external-document-reconciliation.md) ·
  [Source of funds](docs/plan/09-source-of-funds.md) ·
  [Secret management](docs/plan/10-secret-management.md)
- **Contributor** · [AGENTS.md](AGENTS.md) ·
  [CONTRIBUTING.md](CONTRIBUTING.md) · [SECURITY.md](SECURITY.md) ·
  [TODO.md](TODO.md) · [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## Security & privacy

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md) —
it covers built-in backend trust, the SQLCipher boundary, AI provider
tiers, and the incomplete Tor story.
The desktop Privacy & security panel also includes local privacy tells for
synced wallets and transactions. It uses already-stored transaction and UTXO
data, reports risks and unknowns as advisory context, and does not query public
explorers or mutate accounting state from privacy heuristics.

For public bug reports, run `kassiber diagnostics collect` (or
`--diagnostics-out auto` on a failing command) — the output is safe to
paste publicly. In the desktop app, enable Developer tools, open Logs, and
choose **Export → Support bundle** to create a `.support.jsonl` file with a
short issue description, redacted log events, last-failure context, and
redacted AI provenance for local troubleshooting. Support bundles default to
High-signal for trusted maintainer debugging and offer Public-safe mode for
public posting; both modes always strip wallet and credential material such as
descriptors, private keys, recovery phrases, API keys, passwords, and bearer
tokens. Report security-impacting issues to the maintainer privately, not in
the public tracker.

For the north-star local privacy view, open Privacy Mirror or run
`kassiber reports privacy-mirror`. It shows what is linkable, who can infer it,
what proves it, what is unknown, and what a future spend would worsen. The
posture-only snapshot remains available with `kassiber reports privacy-hygiene`
and Settings -> Privacy. GUI, CLI, and assistant read tools share redacted facts
with `evidence_level`, without addresses, scripts, descriptors, xpubs, backend
URLs/tokens, wallet config, raw JSON, branch/index values, or derivation paths.
See [Privacy Mirror](docs/reference/privacy-mirror.md).

## Contributing & license

Read [CONTRIBUTING.md](CONTRIBUTING.md); run `./scripts/quality-gate.sh`
before pushing.

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
