# Kassiber

Local-first Bitcoin accounting for individuals, businesses, and associations.
Kassiber imports or syncs wallet transactions, supports review of transfers and
missing evidence, and produces portfolio, tax, and source-of-funds reports.
It has a desktop app and a CLI backed by the same Python core.

**Pre-alpha:** expect breaking changes and incorrect accounting or tax output.
Do not use Kassiber as the sole basis for filings or financial decisions.

## What it covers

- Watch-only Bitcoin, Lightning, and Liquid sources, plus exchange and wallet
  file imports.
- Transaction notes, attachments, transfer review, quarantine, and audit history.
- Local tax calculations through [RP2](https://github.com/bitcoinaustria/rp2),
  including an Austrian plugin and E 1kv exports.
- An optional AI assistant using local or explicitly authorized remote providers.

An opt-in [general ledger](docs/reference/general-accounting.md) is under
implementation for organizational books. It is separate from Bitcoin tax
journals and is operated through the CLI and agent tools, with action review in
the existing Assistant. Complete organizational bookkeeping and K2 support
are not yet claimed.

## Install

Download desktop or CLI packages from
[GitHub Releases](https://github.com/bitcoinaustria/kassiber/releases).
Packaged targets are Apple Silicon macOS, Linux x86_64, and Windows x86_64.
The desktop includes the CLI runtime; a separate Python installation is not
needed. See [installation and first launch](docs/reference/prerelease-binaries.md)
and [release verification](docs/reference/release-signing.md).

With Homebrew, choose the desktop cask or the CLI formula. Both provide the
`kassiber` command, so install only one:

```sh
brew install --cask bitcoinaustria/kassiber/kassiber
# Or, for CLI only:
brew install bitcoinaustria/kassiber/kassiber-cli
```

See [Homebrew details](docs/reference/homebrew.md) for platform and tap handling.
For source setup and development, see [Contributing](CONTRIBUTING.md).

## Get started

Open the desktop app and follow setup to create a book and connect or import a
wallet. Review transfers and unresolved inputs, process journals, then inspect
reports. The [quick start](docs/quickstart.md) covers the equivalent CLI workflow.

For scripts, `kassiber --machine commands describe` exposes the command contract.
Machine mode returns structured JSON and requests interaction instead of prompting.

## Privacy

Accounting state is stored locally by default. Optional [device sync](docs/reference/device-sync.md)
replicates selected authored records. Wallet sync, release checks, and remote AI
can contact external services when authorized. Kassiber is watch-only and does
not sign or broadcast payments.

Optional SQLCipher encryption protects the project database. Attachments,
exports, and some configuration remain outside that boundary. Read the
[privacy and security reference](docs/reference/privacy-and-security.md) before
using real wallet data, and [SECURITY.md](SECURITY.md) to report a vulnerability.

## Documentation

- [Imports](docs/reference/imports.md) and [backends](docs/reference/backends.md)
- [Tax and journals](docs/reference/tax.md) and [source-of-funds review](docs/reference/source-of-funds-review.md)
- [Desktop](docs/reference/desktop.md) and [AI assistant](docs/reference/ai.md)
- [Product and architecture](docs/plan/00-overview.md) and [backlog](TODO.md)
- [Contributor guide](CONTRIBUTING.md) and [agent instructions](AGENTS.md)

Licensed under [AGPL-3.0-only](LICENSE). See
[third-party licenses](THIRD_PARTY_LICENSES.md) for dependency notices.
