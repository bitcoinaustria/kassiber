```text
██╗  ██╗ █████╗ ███████╗███████╗██╗██████╗ ███████╗██████╗
██║ ██╔╝██╔══██╗██╔════╝██╔════╝██║██╔══██╗██╔════╝██╔══██╗
█████╔╝ ███████║███████╗███████╗██║██████╔╝█████╗  ██████╔╝
██╔═██╗ ██╔══██║╚════██║╚════██║██║██╔══██╗██╔══╝  ██╔══██╗
██║  ██╗██║  ██║███████║███████║██║██████╔╝███████╗██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝╚═════╝ ╚══════╝╚═╝  ╚═╝
```

> **Kassiber** is a local-first, Bitcoin-native accounting suite — a desktop
> app and a CLI backed by the same local daemon. Your wallets, your books,
> your taxes — all on your machine.

> [!WARNING]
> Pre-alpha software. Expect crashes, breaking changes, and inaccurate
> accounting or tax output. Do not rely on Kassiber as the only source of
> truth for filings, audits, or financial decisions.

## Why Kassiber

Most Bitcoin accounting tools are SaaS: you upload your wallets and
descriptors, pay a yearly subscription, and trust the provider — while every
customer's identified holdings sit in one database, waiting to be breached,
subpoenaed, or sold. Kassiber runs on your machine, talks directly to the
Bitcoin sources you choose, and keeps every byte of accounting state in a
local SQLite file you control. No Kassiber account. No server. No telemetry.

Kassiber is **Bitcoin-only**: descriptors, xpubs, Electrum, Esplora,
Bitcoin Core RPC, BTCPay, Lightning, and Liquid — not "any of 600 billion
cryptocurrencies". L-BTC is in scope; altcoins are not.

Tax math runs locally through the open-source
[RP2](https://github.com/bitcoinaustria/rp2) engine — Kassiber prepares,
reviews, and explains; RP2 computes. The Kassiber-maintained fork carries a
working Austrian (§ 27b EStG) plugin with E 1kv exports. Invoicing, VAT/RKSV,
and the company general ledger stay out of scope.

## Highlights

- **Watch-only by design** — Kassiber never constructs, signs, or broadcasts
  transactions. No spending keys to steal, leak, or coerce; paired with
  optional SQLCipher 4 at-rest encryption, a stolen or border-searched device
  yields an unreadable database (caveats in [SECURITY.md](SECURITY.md)).
- **Direct Bitcoin sync** — Esplora, Electrum, Bitcoin Core RPC, BTCPay
  Greenfield, and Liquid Electrum; descriptor, xpub, and address wallets with
  a read-only UTXO inventory.
- **Imports** — BTCPay, Phoenix, River, Bull Bitcoin, Coinfinity, 21bitcoin,
  Pocket Bitcoin, Strike, Ledger Live, Kraken, Coinbase, Binance,
  Samourai/Whirlpool descriptors, generic CSV/JSON, a fill-in Excel/CSV
  template, and BIP329 labels. See [docs/reference/imports.md](docs/reference/imports.md).
- **Review workflows** — notes, tags, exclusions, attachments; append-only
  edit history with safe revert; transfer and swap pairing for Lightning,
  Liquid peg-in/peg-out, and submarine swaps; reviewed source-of-funds cases
  with audit evidence packages and gated PDF export.
- **Tax & reports** — RP2 lot accounting (FIFO/LIFO/HIFO/LOFO, moving
  average); Austrian § 27b EStG with E 1kv PDF/XLSX/CSV; balance sheet,
  capital gains, portfolio, and balance history; self-verifying XLSX exports
  with live recompute formulas you can audit in Excel/LibreOffice; local
  BTC-USD/EUR rate cache with bundled offline history back to 2011.
- **Cross-device & team sync** — opt-in, end-to-end encrypted replication of
  a book between your devices and your team, with no Kassiber account,
  server, or open port. See below.
- **Sovereign storage** — one SQLite system of record per project; optional
  SQLCipher 4 passphrase encryption; `tar | age` backups recoverable with
  stock `age` + `tar` + `sqlcipher` even if Kassiber disappears. Optional
  remembered unlock via the native OS credential store (Keychain, Windows
  Credential Manager, Linux Secret Service).
- **Private by default** — no telemetry, no update checks, no analytics;
  every outbound request enumerated in [SECURITY.md](SECURITY.md); a local
  [Privacy Mirror](docs/reference/privacy-mirror.md) shows what is linkable
  on-chain, who can infer it, and what a future spend would worsen.
- **Local AI assistant** — optional, with local
  [Ollama](https://ollama.com/) and [oMLX](https://omlx.ai/) presets;
  off-device providers require explicit acknowledgement, mutating tools
  require consent. Desktop Assistant and `kassiber chat` share the same
  daemon tool loop.
- **Agent-friendly CLI** — `--machine` yields one deterministic JSON envelope
  per command, `commands describe` exposes the live contract, and high-impact
  bulk reviews support `--dry-run`. See
  [docs/reference/machine-output.md](docs/reference/machine-output.md).
- **Localized desktop UI** — English and German, switchable at runtime.
- **AGPL 3.0** — auditable, forkable, no vendor lock-in.

## Cross-device & multi-user sync

One book can converge across laptops, desktops, and people — owners, editors,
and read-only auditors — without a Kassiber account, trusted server, or
inbound port. Sync is strictly opt-in and replicates the signed authored-event
layer, never the database file: secrets, keys, backend credentials, and
derived accounting state stay local, and identifiers are pseudonymized on the
wire.

Devices exchange sealed, append-only `tar | age` bundles through storage you
control: a shared folder (Dropbox, Drive, iCloud, Nextcloud, Syncthing — the
host sees only ciphertext), WebDAV, or S3-compatible storage. A courier file
on a USB stick works offline; an explicit SPAKE2-paired LAN connection and an
optional Tor leg cover the rest. Every event is Ed25519-signed and merges
deterministically — concurrent financial edits never silently lose to
last-writer-wins, they block journals until a human resolves them.

```bash
kassiber sync enable --member-name "Alice" --device-label "Laptop"
kassiber sync transport add --kind folder --label Team --path "$HOME/Shared/Kassiber"
kassiber sync push --transport Team
```

Invitations, roles, snapshots for late joiners, conflict resolution, and the
desktop Settings → Device sync panel are covered in
[docs/reference/device-sync.md](docs/reference/device-sync.md).

## Install

**Desktop app** — download an unsigned prerelease binary for macOS, Linux, or
Windows from the latest `v*` release. The bundle ships a CLI sidecar (no
separate Python needed), and Settings can install a user-local `kassiber`
terminal launcher. First-launch Gatekeeper/SmartScreen steps:
[docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md).

**From source** (CLI use or development, Python `>=3.10`):

```bash
./scripts/bootstrap-dev-env.sh
export KASSIBER_PYTHON="$PWD/.venv/bin/python"
```

## Quick start

**Desktop** — launch the app. The Welcome screen walks you through optional
database encryption, your first book, tax policy, your first wallet or BTCPay
connection, and the optional assistant. Everything else — Transactions,
Journals, Reports, Source of Funds, Reconcile, Settings — is one click away
in the sidebar.

**CLI**:

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

For scripts and agents:

```bash
kassiber --machine commands describe
kassiber --machine health
kassiber --machine next-actions
```

`kassiber wallets identify` (or the desktop **Reconcile** screen) checks
whether pasted addresses or txids belong to your wallets and classifies each
transaction as self-transfer, outbound, or inbound. For transfer pairing,
swap matching, source-of-funds, Austrian E 1kv, your own Bitcoin Core node,
and the concept model, start with [docs/quickstart.md](docs/quickstart.md)
and [docs/reference/backends.md](docs/reference/backends.md).

## Documentation

- **User reference** · [Quick start](docs/quickstart.md) ·
  [AI assistant](docs/reference/ai.md) ·
  [Backends](docs/reference/backends.md) ·
  [Imports](docs/reference/imports.md) ·
  [Tax & journals](docs/reference/tax.md) ·
  [Desktop](docs/reference/desktop.md) ·
  [Device & team sync](docs/reference/device-sync.md) ·
  [Localization](docs/reference/i18n.md) ·
  [Daemon](docs/reference/daemon.md) ·
  [Machine output](docs/reference/machine-output.md) ·
  [Prerelease binaries](docs/reference/prerelease-binaries.md) ·
  [Homebrew Cask](docs/reference/homebrew-cask.md)
- **Architecture & plans** · [Overview](docs/plan/00-overview.md) ·
  [Desktop stack ADR](docs/plan/01-stack-decision.md) ·
  [Austrian tax engine](docs/plan/06-austrian-tax-engine.md) ·
  [Source of funds](docs/plan/09-source-of-funds.md) ·
  [Device sync](docs/plan/13-device-sync.md)
- **Contributor** · [AGENTS.md](AGENTS.md) ·
  [CONTRIBUTING.md](CONTRIBUTING.md) · [SECURITY.md](SECURITY.md) ·
  [TODO.md](TODO.md) · [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## Security & privacy

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md) —
it covers backend trust, the SQLCipher boundary, AI provider tiers, and the
incomplete Tor story.

For public bug reports, run `kassiber diagnostics collect` — the output is
safe to paste publicly. The desktop app can export a redacted support bundle
from Logs; both modes always strip descriptors, keys, recovery phrases, and
tokens. Report security-impacting issues to the maintainer privately, not in
the public tracker.

## Contributing & license

Read [CONTRIBUTING.md](CONTRIBUTING.md); run `./scripts/quality-gate.sh`
before pushing.

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`).
