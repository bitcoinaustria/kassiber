```text
██╗  ██╗ █████╗ ███████╗███████╗██╗██████╗ ███████╗██████╗
██║ ██╔╝██╔══██╗██╔════╝██╔════╝██║██╔══██╗██╔════╝██╔══██╗
█████╔╝ ███████║███████╗███████╗██║██████╔╝█████╗  ██████╔╝
██╔═██╗ ██╔══██║╚════██║╚════██║██║██╔══██╗██╔══╝  ██╔══██╗
██║  ██╗██║  ██║███████║███████║██║██████╔╝███████╗██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝╚═════╝ ╚══════╝╚═╝  ╚═╝
```

> [!WARNING]
> Kassiber is pre-alpha software. Expect crashes, bugs, breaking changes, and
> inaccurate accounting or tax data. Do not rely on Kassiber as the only source
> of truth for filings, bookkeeping, audits, or financial decisions. Review all
> output independently before using it.

Kassiber is an open-source, local-first Bitcoin accounting CLI. A desktop shell built on Tauri 2 + React + TypeScript with a Python sidecar daemon is in active development (see [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md) and [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md)).

It keeps your accounting state on your machine, syncs from Bitcoin-native sources, and processes journals locally before generating reports. Built from scratch, it takes early visual cues from Clams and other tools in the space without inheriting the cloud trust model.

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md). It covers backend visibility, external requests, and current caveats such as missing at-rest encryption and incomplete Tor support.

Normal `backends ...` and `wallets ...` success output now follows a narrow
safe-to-record contract for secret-bearing config values: backend inspection
returns an allowlisted safe view plus `has_*` flags for credential presence,
and wallet inspection returns allowlisted safe config plus descriptor state
flags without echoing raw descriptor material or arbitrary config keys. This
is not a general privacy guarantee; addresses, paths, notes, and `--debug`
output can still be sensitive.

For public bug reports, use `kassiber diagnostics collect`. It emits a
public-safe report with versions, command shape, sanitized error context, and
state counts without raw txids, addresses, labels, notes, exact amounts, paths,
backend hostnames, or secrets. Pass `--save` to also write the report under
`exports/diagnostics/` in the active Kassiber state root. For one-off failing
commands, add `--diagnostics-out auto` before the subcommand to write the same
kind of report when an error occurs.

## What Kassiber does

- keeps a local SQLite system of record
- supports multiple workspaces, profiles, wallet buckets, and wallets
- syncs from `esplora` and `electrum`, plus `bitcoinrpc` for address-based Bitcoin wallets and confirmed BTCPay Greenfield wallet history
- imports generic CSV/JSON, BTCPay exports, Phoenix exports, and BIP329 labels
- pulls confirmed BTCPay on-chain wallet history directly from a BTCPay server via the Greenfield API
- stores notes, tags, exclusions, transfer pairs, and attachments
- processes journals explicitly before reports are trusted
- exposes every command through a deterministic JSON envelope
- has a desktop shell on Tauri 2 + React + TypeScript with a Python sidecar daemon under construction (see [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md))

## Architecture

Kassiber is the local-first accounting product layer. It owns:

- wallet sync and import adapters
- local storage and provenance capture
- metadata, attachments, and transfer pairing
- review and quarantine workflows
- CLI and desktop UX

RP2 is the tax core. Kassiber currently installs the Kassiber-maintained fork at [bitcoinaustria/rp2](https://github.com/bitcoinaustria/rp2), which carries the Austrian country plugin, moving-average accounting support, and the disposal-classifier API Kassiber uses for Austrian reporting semantics.

Today:

- the `generic` tax policy runs through RP2
- the `at` tax policy runs through RP2's Austrian plugin plus Kassiber-side category/Kennzahl mapping
- Austrian cross-asset swaps paired with `--policy carrying-value` now carry basis through Kassiber's two-pass handoff into RP2; generic cross-asset pairs still stay on the normal SELL + BUY path

The intended split is simple: Kassiber prepares and explains; RP2 computes.

Kassiber is also the planned home for external-document reconciliation around
Bitcoin payments: BTCPay provenance, local document matching, review, and
tax-normalization decisions. Invoice issuing, VAT workflows, and the merchant
general ledger stay outside Kassiber. See
[docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md).

## Concepts

Kassiber's model is:

```text
workspace
`-- profile
    |-- account bucket(s)
    `-- wallet(s)

wallets -> transactions -> journals -> reports
```

- `workspace`: the top-level container for an organization, person, or set of books
- `profile`: one accounting and tax scope inside a workspace
- `wallet`: a transaction source that Kassiber syncs or imports
- `account`: a wallet/reporting bucket that wallets can belong to

In practice, a workspace might be an association, with one profile for its BTC
books, buckets such as `events`, `memberships`, and `store`, and wallets
mapped to the real underlying wallet sources that actually hold or receive
funds.

Transactions flow in from wallets, journals process those transactions into
tax and accounting state, and reports read from the processed journal state.
Cost basis is pooled per asset across all wallets in a profile, even though
reporting can still break holdings and activity down by wallet and account.
Kassiber accounts are not a double-entry chart of accounts today: fees and
external counterparties are not posted automatically to separate account rows,
and the `account_type` / `asset` fields are descriptive bucket metadata rather
than report rollup rules.

If you use multiple BTCPay stores, only model them as multiple Kassiber wallets
when they are actually different underlying wallets. If two stores point at the
same wallet, creating both in Kassiber would duplicate holdings.

BTCPay-backed wallets now persist their `backend` / `store_id` /
`payment_method_id` config on the wallet itself, so later `wallets sync`,
`wallets sync --all`, and future GUI flows can reuse the same source without
retyping `--store-id`.

## AI assistance

Kassiber ships with a repo-local AI skill in [`skills/kassiber/`](skills/kassiber/)
for coding and terminal assistants. It helps an assistant use the Kassiber CLI
safely for onboarding, imports, journal processing, reports, metadata cleanup,
and troubleshooting.

AI is optional. Kassiber's core accounting flow does not depend on a model, and
future AI-assisted features such as OCR, extraction, and reconciliation
suggestions should stay review-gated.

If you use AI with Kassiber, treat prompts as sensitive accounting data. Local
inference is the recommended default. [Ollama](https://ollama.com/) is a good
fit for local models, and if remote inference is needed, prefer a provider with
documented encrypted inference such as
[Maple Proxy](https://blog.trymaple.ai/maple-proxy-documentation/).

See [docs/reference/ai.md](docs/reference/ai.md) for setup notes, example
prompts, and privacy guidance.

## Local state

By default Kassiber stores state under `~/.kassiber/`:

- `data/kassiber.sqlite3` for SQLite data
- `config/backends.env` for optional backend bootstrap overrides
- `config/settings.json` for the managed path manifest and UI state
- `exports/` for generated report files
- `exports/diagnostics/` for optional public-safe bug-report artifacts
- `attachments/` for managed attachment blobs

Backend definitions and the stored default backend now live canonically in
SQLite. `backends.env` is still accepted as a bootstrap/compatibility path,
but Kassiber only imports that bootstrap config into SQLite during explicit
bootstrap-import flows such as `kassiber init`; once imported, the DB is the
long-term source of truth.

Use `kassiber status` to see the active paths. `--data-root` and `--env-file` let you override them.

## Installation

Requirements:

- Python `>=3.10`
- `embit>=0.8.0`
- `rp2` from `bitcoinaustria/rp2` (pinned in `pyproject.toml`)

Install in a virtual environment:

```bash
python3 -m venv .venv
source .venv/bin/activate
python3 -m pip install --upgrade pip
python3 -m pip install -e .
```

Or use `uv`:

```bash
uv sync
```

### Prerelease binaries

Tagged `v*` pushes build unsigned prerelease CLI binaries for macOS and Linux
through GitHub Actions. Manual runs of the `prerelease-binaries` workflow also
upload the same `.tar.gz` artifacts, and can attach them to an existing tag
when `publish_release` and `tag_name` are provided.

The same workflow also builds unsigned desktop preview artifacts: macOS
`.app` zip / `.dmg`, Linux `.AppImage`, and Windows `.msi` plus NSIS setup
`.exe`. These previews do not yet bundle the Python sidecar; they are for
testing the shell on machines where `python3 -m kassiber daemon` already works,
or where `KASSIBER_DAEMON_PYTHON` / `KASSIBER_REPO_ROOT` are set before launch.
Fully self-contained desktop installers remain in active development.

Before pushing code or docs changes, run:

```bash
./scripts/quality-gate.sh
```

## Desktop UI (in development)

A Tauri 2 + React 19 + TypeScript desktop frontend lives at [ui-tauri/](ui-tauri/). It is under active translation per [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md); the CLI remains the primary control surface today.

The frontend currently runs against a mock daemon (`VITE_DAEMON=mock`, the default) — every screen renders against hand-rolled fixtures keyed by daemon `kind`. A minimal Tauri shell now lives in `ui-tauri/src-tauri/` and forwards whitelisted `daemon_invoke` calls to `python -m kassiber daemon`; the first real round-trip is `status`. Until typed UI snapshot kinds land, data screens still use browser mock mode for the dashboard workflow.

Requirements:

- Node `>=20`
- `pnpm` (https://pnpm.io)
- Rust stable for the Tauri shell (`cargo check` / `pnpm tauri:dev`)

Install and run the dev server:

```bash
cd ui-tauri
pnpm install
pnpm dev
# → http://localhost:5173
```

To exercise the Tauri command boundary:

```bash
cd ui-tauri
pnpm tauri:dev
```

`pnpm tauri:dev` runs the webview with the Tauri transport, starts the Python daemon, and forwards calls through the Rust supervisor. The supervisor prefers `.venv/bin/python` when present and otherwise falls back to `python3`; set `KASSIBER_DAEMON_PYTHON=/path/to/python` to override it, or `KASSIBER_REPO_ROOT=/path/to/checkout` to point the dev shell at another checkout. Screens that still require fixture data show daemon-unavailable states until typed UI snapshot kinds are wired.

The app boots into the Welcome onboarding flow on first load, persists identity
to localStorage, and routes through Overview / Connections / Transactions /
Reports / Tax Events / Quarantine / Profiles. The shared shell hosts global
search, the hide-sensitive eye, and the Settings modal; display currency lives
inside Settings.

Other useful commands:

```bash
pnpm typecheck   # tsc --noEmit project references
pnpm lint        # ESLint flat config
pnpm build       # production bundle into dist/
pnpm tauri       # Tauri CLI
pnpm test        # Vitest (no tests yet)
```

`pnpm typecheck && pnpm lint && pnpm build` is the local UI gate; pair it with `./scripts/quality-gate.sh` from the repo root before pushing changes that touch both layers.

## Quick start

Minimal setup:

```bash
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber profiles create main \
  --workspace personal \
  --fiat-currency USD \
  --tax-country generic \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
python3 -m kassiber context set --workspace personal --profile main
```

Create a simple wallet and sync it:

```bash
python3 -m kassiber wallets create \
  --label donations \
  --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq

python3 -m kassiber wallets sync --wallet donations
```

Process journals and run reports:

```bash
# If you have BTC <-> LBTC peg-ins / peg-outs or submarine swaps,
# pair those legs first with `kassiber transfers pair`.
python3 -m kassiber journals process
python3 -m kassiber reports summary
python3 -m kassiber reports tax-summary
python3 -m kassiber reports balance-sheet
python3 -m kassiber reports capital-gains
# For Austrian/EUR profiles:
python3 -m kassiber --machine reports austrian-e1kv --year 2024
python3 -m kassiber --machine reports austrian-tax-summary --year 2024
python3 -m kassiber reports export-austrian-e1kv-pdf --year 2024 --file e1kv-2024.pdf
python3 -m kassiber reports export-austrian --year 2024 --file austria-2024.pdf
python3 -m kassiber reports export-austrian-e1kv-xlsx --year 2024 --file e1kv-2024.xlsx
python3 -m kassiber reports export-austrian-e1kv-csv --year 2024 --dir e1kv-2024-csv
```

## Docs

Reference docs:

- [docs/reference/ai.md](docs/reference/ai.md)
- [docs/reference/backends.md](docs/reference/backends.md)
- [docs/reference/imports.md](docs/reference/imports.md)
- [docs/reference/tax.md](docs/reference/tax.md)
- [docs/reference/machine-output.md](docs/reference/machine-output.md)
- [docs/reference/desktop.md](docs/reference/desktop.md)
- [docs/reference/daemon.md](docs/reference/daemon.md)

Planning and architecture docs:

- [docs/plan/00-overview.md](docs/plan/00-overview.md)
- [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md) (desktop stack ADR)
- [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md) (desktop implementation plan)
- [docs/plan/06-austrian-tax-engine.md](docs/plan/06-austrian-tax-engine.md)
- [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md)

Contributor docs:

- [AGENTS.md](AGENTS.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [SECURITY.md](SECURITY.md)
- [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## Current gaps

Notable gaps today:

- Austrian E 1kv CSV/PDF/XLSX export is review-gated and currently targets the ausländisch / self-custody Kennzahlen; the PDF/JSON output includes Steuerbericht-style sections 1.1-4.5 with unsupported placeholders, while the XLSX and CSV bundle use an `Übersicht`, numbered section tabs/files, and `Erläuterungen zum Steuerreport`; domestic-provider withheld KESt metadata is not modeled yet
- full BTCPay invoice/payment provenance ingest is not implemented yet; BTCPay sync currently covers confirmed on-chain wallet history plus comments/labels
- descriptor/xpub live sync through `bitcoinrpc` is not implemented yet
- some Lightning node adapters are declared but do not sync yet
- `custom` wallet import mapping is not implemented yet
- reports still use stored journal pricing rather than querying the rates cache live
- no REST/server mode or multi-user auth
- desktop UI (Tauri 2 + React + Python sidecar) is under construction; the CLI is the primary control surface today

See [TODO.md](TODO.md) for the active backlog.

## Development notes

- SQLite is the system of record
- BTC-denominated values are stored as integer msat
- machine-readable envelopes are a stable contract and are pinned by `tests/test_cli_smoke.py`

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`)
