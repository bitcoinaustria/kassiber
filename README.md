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
1,500 cryptocurrencies". L-BTC is in scope; altcoins are not. Stablecoin
support is on the table for later, depending on future resources.

Tax math runs locally through the open-source
[RP2](https://github.com/bitcoinaustria/rp2) engine. Kassiber prepares,
reviews, and explains; RP2 computes. The Kassiber-maintained RP2 fork
carries a working Austrian (§ 27b EStG) plugin with E 1kv exports — a
workflow almost nobody else covers.

## Why local-first

- **Bitcoin Native** — descriptors, xpubs, BIP329, Lightning, and Liquid as
  first-class concepts.
- **Privacy First** — no telemetry, no update check, no analytics; every
  outbound request enumerated in [SECURITY.md](SECURITY.md).
- **No remote honeypot** — there is no Kassiber server holding your
  addresses, balances, and identity. Crypto tax SaaS providers have been
  breached; the dumps become targeting lists for phishing and physical
  attacks. Kassiber's database is one file on your machine.
- **Wrench-attack resistant** — watch-only by design (no spending keys to
  coerce), and optional SQLCipher 4 at-rest encryption keyed by a
  passphrase that lives only in your head. On a stolen, customs-seized, or
  border-searched cold device, the encrypted database — descriptors,
  xpubs, transactions, stored tokens — is unreadable. Attachments and a
  couple of config files sit outside the SQLCipher boundary, so pair with
  full-disk encryption for the full picture; the caveats are in
  [SECURITY.md](SECURITY.md). The
  [jlopp/physical-bitcoin-attacks](https://github.com/jlopp/physical-bitcoin-attacks)
  catalog covers the threats this addresses.
- **Local AI Chat** — assistant defaults to local
  [Ollama](https://ollama.com/); off-device providers require explicit
  per-provider acknowledgement and per-tool consent.
- **AGPL 3.0** — auditable, forkable, no vendor lock-in.

## Highlights

- **Direct Bitcoin sync** — Esplora, Electrum, Bitcoin Core RPC, BTCPay
  Greenfield, Liquid Electrum.
- **Imports** — BTCPay CSV/JSON, Phoenix, River, Bull Bitcoin, generic
  CSV/JSON, BIP329 labels.
- **Review workflows** — notes, tags, exclusions, attachments; reviewed
  transfer/swap pairing for Lightning, Liquid peg-in/peg-out, and submarine
  swaps; reviewed source-of-funds reports with immutable saved cases and
  gated PDF export.
- **Tax & reports** — RP2 lot accounting (FIFO/LIFO/HIFO/LOFO and moving
  average); Austrian § 27b EStG with E 1kv PDF / XLSX / CSV; summary,
  balance sheet, capital gains, portfolio, balance history; local
  BTC-USD / BTC-EUR rates cache (Coinbase + CoinGecko fallback + Kraken
  OHLCVT local archive).
- **Sovereign storage** — SQLite system of record; optional SQLCipher 4
  passphrase encryption; single-file `tar | age` backups recoverable with
  stock `age` + `tar` + `sqlcipher` even if Kassiber disappears.
- **Optional Touch ID unlock** — macOS desktop builds can save the database
  passphrase in Keychain behind local user presence. This is a convenience,
  not a recovery path or a replacement for the SQLCipher passphrase.
- **Two surfaces, one daemon** — desktop GUI (Tauri 2 + React) for
  day-to-day work; CLI with deterministic JSON envelopes for scripting,
  automation, and power users; both backed by the same Python daemon.

## Install

**Desktop app** — download an unsigned prerelease binary for macOS, Linux,
or Windows from the latest `v*` release. The bundle ships a CLI sidecar,
so no separate Python install is needed. Gatekeeper / SmartScreen
first-launch handling lives in
[docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md).

**From source** (CLI use or development, Python `>=3.10`):

```bash
uv sync                       # or: python3 -m venv .venv && pip install -e .
```

## Quick start

### Desktop

Launch the app. The Welcome screen walks you through optional database
encryption, your books set and first book, tax policy, your first wallet
or BTCPay connection, and the optional AI assistant. Every other flow —
Overview, Connections, Imports, Transactions, Swap Matching, Journals,
Quarantine, Reports, Source of Funds, Books, Settings, Logs,
Assistant — is one click away in the sidebar.

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

For transfer pairing, swap matching, source-of-funds, Austrian E 1kv,
BTCPay reconciliation, and the concept model, see
[docs/quickstart.md](docs/quickstart.md). Both surfaces speak the same
Python daemon, so a daily flow can move freely between them.

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
  [Daemon](docs/reference/daemon.md) ·
  [Machine output](docs/reference/machine-output.md) ·
  [Prerelease binaries](docs/reference/prerelease-binaries.md)
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

For public bug reports, run `kassiber diagnostics collect` (or
`--diagnostics-out auto` on a failing command) — the output is safe to
paste publicly. Report security-impacting issues to the maintainer
privately, not in the public tracker.

## Contributing & license

Read [CONTRIBUTING.md](CONTRIBUTING.md); run `./scripts/quality-gate.sh`
before pushing.

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
