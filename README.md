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

Kassiber is an open-source, local-first Bitcoin accounting CLI with a
pre-alpha desktop preview built on Tauri 2 + React + TypeScript and a Python
sidecar daemon (see [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md)
and [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md)). The CLI remains
the most complete control surface today; the desktop preview is now backed by
real daemon calls for the main review, setup, report, export, assistant, and
diagnostics workflows.

It keeps your accounting state on your machine, syncs from Bitcoin-native sources, and processes journals locally before generating reports. Built from scratch, it takes early visual cues from Clams and other tools in the space without inheriting the cloud trust model.

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md). It covers backend visibility, external requests, the V4.1 SQLCipher-based at-rest encryption (and what it does *not* protect — sidecar files, attachments, the OS-level threat model), and incomplete Tor support.
The desktop secret-management direction is tracked in
[docs/plan/10-secret-management.md](docs/plan/10-secret-management.md): the
unlocked Python daemon is the runtime trust boundary, SQLCipher remains the
at-rest perimeter for DB-resident secrets, and desktop builds can move AI
provider API keys only into native OS credential stores behind a narrow
daemon/supervisor bridge.

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

- keeps a local SQLite system of record (optionally encrypted at rest via SQLCipher 4 with a passphrase you choose; see `kassiber secrets init`)
- ships a single-file `tar | age` backup format (`kassiber backup export`) that is recoverable with stock `age` + `tar` + `sqlcipher` if Kassiber stops being maintained
- supports local sets of books, separate books for private/business tax scopes,
  wallet buckets, and wallets
- syncs Bitcoin and Liquid wallets through `esplora` / `electrum`, supports
  address-based Bitcoin refresh through `bitcoinrpc`, and pulls confirmed
  BTCPay Greenfield wallet history
- imports generic CSV/JSON, BTCPay exports, Phoenix exports, River exports, and BIP329 labels
- pulls confirmed BTCPay on-chain wallet history directly from a BTCPay server via the Greenfield API
- keeps a local BTC-USD / BTC-EUR rates cache from Coinbase Exchange,
  CoinGecko fallback, manual overrides, and optional local Kraken OHLCVT CSV
  archives
- stores notes, tags, exclusions, transfer pairs, swap review views,
  source-of-funds sources/links/cases, and attachments
- processes journals explicitly before reports are trusted
- exposes every command through a deterministic JSON envelope
- has a daemon-backed desktop preview for onboarding, connections,
  transactions, swap matching, journals, reports, source-of-funds review,
  settings, diagnostics, and the optional assistant

## Architecture

Kassiber is the local-first accounting product layer. It owns:

- watch-only source refresh and import adapters
- local storage and provenance capture
- metadata, attachments, and transfer pairing
- review and quarantine workflows
- CLI and desktop UX

RP2 is the tax core. Kassiber currently installs the Kassiber-maintained fork at [bitcoinaustria/rp2](https://github.com/bitcoinaustria/rp2), which carries the Austrian country plugin, moving-average accounting support, and the disposal-classifier API Kassiber uses for Austrian reporting semantics.

Today:

- the `generic` tax policy runs through RP2
- the `at` tax policy runs through RP2's Austrian plugin plus Kassiber-side category/Kennzahl mapping
- Austrian cross-asset swaps paired with `--policy carrying-value` are reviewed and marked by Kassiber, then carried through RP2's native multi-asset Austrian hook; generic cross-asset pairs still stay on the normal SELL + BUY path

The intended split is simple: Kassiber prepares and explains; RP2 computes.

Kassiber is also the planned home for external-document reconciliation around
Bitcoin payments: BTCPay provenance, local document matching, review, and
tax-normalization decisions. Invoice issuing, VAT workflows, and the merchant
general ledger stay outside Kassiber. See
[docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md).

## Concepts

Kassiber's user model is:

```text
books file / local state
`-- book(s)
    |-- wallet bucket(s)
    `-- wallet(s)

wallets -> transactions -> journals -> reports
```

- `books file` / `local state`: the local Kassiber data root for one person,
  business, or client
- `book`: one separated accounting and tax scope inside that local state
- `wallet`: a transaction source that Kassiber syncs or imports
- `account`: a wallet/reporting bucket that wallets can belong to

In the CLI and database these are still named `workspace` and `profile`.
The desktop UI uses the friendlier names above: a workspace is a local books
set, and a profile is a book. In practice, "My Books" might contain separate
books for `private` and `business`, while a company or client should usually
live in its own Kassiber state root with one main set of BTC books, buckets such
as `events`, `memberships`, and `store`, and wallets mapped to the real
underlying wallet sources that actually hold or receive funds.

Transactions flow in from wallets, journals process those transactions into
tax and accounting state, and reports read from the processed journal state.
Cost basis is pooled per asset across all wallets in a set of books, even though
reporting can still break holdings and activity down by wallet and account.
Kassiber accounts are not a double-entry chart of accounts today: fees and
external counterparties are not posted automatically to separate account rows,
and the `account_type` / `asset` fields are descriptive bucket metadata rather
than report rollup rules.

If you use multiple BTCPay stores, only model them as multiple Kassiber wallets
when they are actually different underlying wallets. If two stores point at the
same wallet, creating both in Kassiber would duplicate holdings.

BTCPay-backed wallets persist their `backend` / `store_id` /
`payment_method_id` config on the wallet itself, so later `wallets sync`,
`wallets sync --all`, and GUI flows can reuse the same source without retyping
store details. The desktop setup can create a BTCPay instance from URL + API
key, discover stores and payment methods, create BTCPay-only wallet sources, or
map BTCPay payment methods onto existing settlement wallets for provenance
enrichment. When no explicit payment method is supplied, Kassiber stores the
default BTC on-chain payment method internally.

## AI assistance

Kassiber ships with a repo-local AI skill in [`skills/kassiber/`](skills/kassiber/)
for coding and terminal assistants. It helps an assistant use the Kassiber CLI
safely for onboarding, imports, journal processing, reports, metadata cleanup,
and troubleshooting.

The desktop app ships an in-app assistant configured in
**Settings → AI providers**. The default seed entry is local Ollama
(`http://localhost:11434/v1`), and you can also add OpenAI-compatible remote
providers or fixed Claude/Codex CLI adapters (`claude-cli://default`,
`codex-cli://default`). Remote and CLI prompts may leave the device, so the
picker tags each provider as `local`, `remote`, or `tee`, and chat requires
explicit acknowledgement before any off-device prompt is sent. The same surface
is reachable from the CLI via
`kassiber ai providers …`, `kassiber ai models`, and `kassiber ai chat`.
API keys should be entered through Settings or CLI stdin/fd
(`--api-key-stdin` / `--api-key-fd FD`); the older `--api-key <value>` argv
form remains only as a warning-on-use compatibility shim.
In desktop Settings, provider keys can stay `sqlcipher_inline` or move to the
platform store selected by policy: macOS Keychain, Windows user-scope
Credential Manager/DPAPI, or Linux Secret Service when available. Backend
tokens, descriptors, xpubs, blinding keys, and reveal payloads are not moved by
this AI-key pilot.

AI is optional. Kassiber's core accounting flow does not depend on a model, and
future AI-assisted features such as OCR, extraction, and reconciliation
suggestions should stay review-gated.

The desktop assistant uses explicit daemon tools rather than raw shell, raw
filesystem, arbitrary CLI execution, descriptors, wallet files, env files, or
generic daemon dispatch. Read-only tool calls can fetch bounded local snapshots
for answers; mutating tools require user consent. AI features can be disabled
globally in Settings, which hides the assistant route, sidebar entry, and
floating chat surface while leaving provider settings available.

If you use AI with Kassiber, treat prompts as sensitive accounting data. Local
inference is the recommended default. [Ollama](https://ollama.com/) is a good
fit for local models. Claude CLI and Codex CLI are convenient if you already
use them, but Kassiber treats them as off-device because their configured model
providers may receive prompt content. If remote inference is needed, prefer a
provider with documented encrypted inference such as
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
SQLite. `backends.env` is still accepted as a bootstrap/compatibility path
for non-secret addressing (URL, `KIND`, chain, network), but secrets — API
tokens, passwords, auth headers, basic-auth usernames — belong in the
encrypted `backends` table. New credentials should be seeded with
`--token-stdin` / `--token-fd FD` so they go straight into the DB; pre-existing
entries in `backends.env` can be lifted out with
`kassiber secrets migrate-credentials` (see [SECURITY.md](SECURITY.md) for
the full at-rest boundary).

Use `kassiber status` to see the active paths. `--data-root` and `--env-file` let you override them.

## Installation

Requirements:

- Python `>=3.10`
- `embit>=0.8.0`
- `rp2` from `bitcoinaustria/rp2` (pinned in `pyproject.toml`)
- `XlsxWriter` for workbook exports and `reportlab` for styled Austrian and
  source-of-funds PDF exports
- `sqlcipher3` and `pyrage` for encrypted databases and `.kassiber` backups
- desktop builds use Rust keyring crates (`keyring-core`,
  `apple-native-keyring-store`, `windows-native-keyring-store`, and
  `zbus-secret-service-keyring-store`) for AI provider API-key storage when
  platform policy selects a native store; SQLCipher inline remains the explicit
  fallback and the CLI path

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

Tagged `v*` pushes build unsigned prerelease CLI binaries for macOS
Apple Silicon, macOS Intel, and Linux through GitHub Actions. Manual runs of
the `prerelease-binaries` workflow also upload the same `.tar.gz` artifacts,
and can attach them to an existing tag when `publish_release` and `tag_name`
are provided. Linux CLI binaries are built on Ubuntu 22.04 to keep the glibc
floor aligned with the AppImage build. CLI archives are named
`kassiber-cli-<platform>-<arch>.tar.gz`; the executable inside is named
`kassiber`.
Pull requests do not build binaries automatically; use a manual workflow run
against the PR branch when a tester artifact is needed. The workflow run and
release tag record the source commit, and the desktop shell displays the build
commit beside the version number. Artifact filenames do not embed the release
version or commit hash; use the release tag and workflow run for source
identity. Release assets include one `SHA256SUMS.txt` checksum manifest.

The same workflow also builds unsigned desktop preview artifacts: a universal
macOS `.app` zip / `.dmg`, Linux `.AppImage`, and Windows `.msi` plus NSIS
setup `.exe`, all published as short names such as
`kassiber-macos-universal.dmg`, `kassiber-linux-x64.AppImage`, and
`kassiber-windows-x64.exe`. These previews include a bundled Kassiber CLI
sidecar that the GUI uses for daemon calls, so normal daemon calls do not
require a separate Python checkout. The installed GUI executable also forwards
`--cli ...` to the bundled CLI sidecar; for example,
`./kassiber-linux-x64.AppImage --cli status` or `Kassiber.exe --cli status`.
Set `KASSIBER_PYTHON` only when intentionally overriding the bundled sidecar
for debugging. Signing, notarization, and production installer hardening remain
in active development.

> [!WARNING]
> The macOS desktop preview is currently unsigned and not notarized. Gatekeeper
> will warn before launch. Only bypass that warning for Kassiber builds you
> downloaded from this repository and verified as trustworthy; do not disable
> Gatekeeper globally. To remove quarantine for this app bundle only after
> installing it in `/Applications`, run:
>
> ```bash
> sudo xattr -dr com.apple.quarantine /Applications/Kassiber.app
> ```

Operational guidance for branch, PR, and tag builds lives in
[docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md).

Before pushing code or docs changes, run:

```bash
./scripts/quality-gate.sh
```

## Desktop UI (pre-alpha preview)

A Tauri 2 + React 19 + TypeScript desktop frontend lives at [ui-tauri/](ui-tauri/).
It is usable as a prerelease preview for core flows, but still pre-alpha; the
CLI remains the most complete and scriptable surface.

Browser dev mode now defaults to the loopback Vite daemon bridge, so `pnpm dev`
and `pnpm dev:bridge` exercise the real Python daemon. Use `pnpm dev:browser`
when you intentionally want mock fixtures for isolated UI layout work, or
`pnpm tauri:dev` to exercise the packaged Tauri command boundary. Overview,
Books, Connections, Imports, Transactions, Swap Matching, Journals, Tax Events,
Quarantine, Reports, Source of Funds, Settings, assistant chat, exports, and
diagnostics all have daemon-backed paths; mock mode remains for disconnected
development and screen polish.

Requirements:

- Node `>=20`
- `pnpm` (https://pnpm.io)
- Rust stable for the Tauri shell (`cargo check` / `pnpm tauri:dev`)

The UI uses shadcn primitives, TanStack data helpers, and Recharts;
dependencies are locked in `ui-tauri/pnpm-lock.yaml`.

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

`pnpm tauri:dev` runs the webview with the Tauri transport, starts the Python daemon, and forwards calls through the Rust supervisor. The supervisor prefers `.venv/bin/python` when present and otherwise falls back to `python3`; set `KASSIBER_PYTHON=/path/to/python` to override it, or `KASSIBER_REPO_ROOT=/path/to/checkout` to point the dev shell at another checkout. The Tauri and bridge supervisors both allowlist daemon kinds instead of exposing generic process or CLI access.

The app boots into the Welcome onboarding flow on first load, persists identity
to localStorage, and routes through Overview / Connections / Imports /
Transactions / Swaps / Journals / Reports / Source of Funds / Tax Events /
Quarantine / Diagnostics / Books / Settings / Assistant. The shared shell hosts
global search, the hide-sensitive eye, native menu intents, the global AI
feature toggle, and Settings; display currency lives inside Settings.

Other useful commands:

```bash
pnpm typecheck   # tsc --noEmit project references
pnpm lint        # ESLint flat config
pnpm build       # production bundle into dist/
pnpm tauri       # Tauri CLI
pnpm test        # Vitest unit tests
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
# pair those legs first. The matcher surfaces unpaired candidates
# (exact via Lightning payment_hash, strong via time + amount):
python3 -m kassiber transfers suggest
# Auto-apply every solo exact match without further review:
python3 -m kassiber transfers bulk-pair --confidence exact
# Or apply saved non-conflicted auto-pair rules:
python3 -m kassiber transfers rules apply
# Or pair one specific pair by id:
python3 -m kassiber transfers pair --tx-out <out-id> --tx-in <in-id> \
  --kind submarine-swap --policy carrying-value
python3 -m kassiber journals process
python3 -m kassiber reports summary
python3 -m kassiber reports tax-summary
python3 -m kassiber reports balance-sheet
python3 -m kassiber reports capital-gains
python3 -m kassiber reports export-pdf --file report.pdf
python3 -m kassiber reports export-csv --file report.csv
python3 -m kassiber reports export-xlsx --file report.xlsx
# For Austrian/EUR books:
python3 -m kassiber --machine reports austrian-e1kv --year 2024
python3 -m kassiber --machine reports austrian-tax-summary --year 2024
python3 -m kassiber reports export-austrian-e1kv-pdf --year 2024 --file e1kv-2024.pdf
python3 -m kassiber reports export-austrian --year 2024 --file austria-2024.pdf
python3 -m kassiber reports export-austrian-e1kv-xlsx --year 2024 --file e1kv-2024.xlsx
python3 -m kassiber reports export-austrian-e1kv-csv --year 2024 --dir e1kv-2024-csv
```

Build a reviewed source-of-funds report:

```bash
# Choose a purpose. For a planned exchange sale, the target transaction is the
# current funds-history anchor, not the future exchange deposit txid.
python3 -m kassiber --machine reports source-funds \
  --purpose planned_exchange_sale \
  --target-transaction <current-funds-txid-or-id> \
  --target-amount 1.00000000 \
  --planned-destination "Exchange or broker" \
  --planned-note "Pre-disclosure before expected bank proceeds"

# Seed target-scoped suggestions from existing transfers, pairs, and
# one-to-one provider/import ids. Suggestions are review items, not proof.
# Broad account ids and time/amount heuristics require --include-broad-hints.
python3 -m kassiber source-funds suggest --target-transaction <txid-or-id>

# Deterministic same-external-id hops, reviewed transaction_pairs, and
# one-to-one per-transaction provider/import ids can be accepted in bulk for
# this target path; broad account ids and weak matches remain manual.
python3 -m kassiber source-funds links bulk-review \
  --target-transaction <target-txid-or-id>

# Add reviewed root evidence and explicit flow allocations.
python3 -m kassiber source-funds sources create \
  --type fiat_purchase \
  --label "Reviewed exchange purchase" \
  --asset BTC \
  --amount 0.10000000
python3 -m kassiber source-funds links create \
  --from-source <source-id> \
  --to-transaction <transaction-id> \
  --type manual_source \
  --allocation-amount 0.10000000

# Optional: save recipient-specific disclosure defaults for repeat exports.
python3 -m kassiber source-funds recipients create \
  --label "Relationship bank" \
  --kind bank \
  --default-reveal-mode standard

# Preview gates and disclosure, saving an immutable case before export.
python3 -m kassiber --machine reports source-funds \
  --target-transaction <target-txid-or-id> \
  --reveal-mode standard \
  --save-case
# Export only renders a saved case snapshot, never live mutable tables.
python3 -m kassiber reports export-source-funds-pdf \
  --case <case-id> \
  --file source-of-funds.pdf
```

Source-of-funds reports carry local overview metrics, deterministic narrative
text, a simplified reviewed flow path, data-source rollups, source mix,
level-by-level flow rows, transaction details, review gates, and disclosure
notes. PDFs render saved case snapshots with the ReportLab exporter and only
include reviewed evidence. The simplified flow chart follows reviewed local
source, wallet-transfer, and consolidation-style links; CoinJoin/PayJoin
traversal is deferred and shown as a privacy boundary rather than ownership
proof through unrelated participant inputs. Kassiber does not
claim chain heuristics prove ownership, does not expose descriptors/xpubs/wallet
files/seeds/backend tokens, and treats opening balances as attested
prior-history stops rather than real root sources. Export gates also reject
cycle paths, self-transfer asset mismatches, source/edge asset mismatches,
concrete sources without amounts, cumulative source over-allocation, and
reviewed paths that require more value from a transaction than it contains.

The desktop Source of Funds screen keeps the default path to target selection,
local case summary, review gates, and gated PDF export. Advanced target filters,
historical coverage diagnostics, suggested-link review, allocation editing,
evidence attachment, manual transaction links, and root-source / missing-history
editing stay available as optional panels.
For planned sales, fiat-funds evidence for the original bitcoin purchase
remains a separate source attachment.

For a basic Austrian workflow, create the profile with `--tax-country at` and
`--fiat-currency EUR`. The source-of-funds PDF then uses the
`Mittelherkunftsnachweis / Source of Funds Report` title, includes Austria/EUR
report context, and renders an evidence checklist covering fiat-purchase proof,
reviewed wallet-transfer / consolidation hops, target broker or exchange
deposit, and immutable saved-case export. Full German localization,
country-specific legal templates, and CoinJoin/PayJoin traversal remain
deferred. A fictitious AT/EUR sample report can be generated locally with:

```bash
uv run python scripts/generate-source-funds-demo-report.py \
  --output /tmp/kassiber-source-funds-demo.pdf \
  --json-output /tmp/kassiber-source-funds-demo.json
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
- [docs/reference/prerelease-binaries.md](docs/reference/prerelease-binaries.md)

Planning and architecture docs:

- [docs/plan/00-overview.md](docs/plan/00-overview.md)
- [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md) (desktop stack ADR)
- [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md) (desktop implementation plan)
- [docs/plan/06-austrian-tax-engine.md](docs/plan/06-austrian-tax-engine.md)
- [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md)
- [docs/plan/09-source-of-funds.md](docs/plan/09-source-of-funds.md)

Contributor docs:

- [AGENTS.md](AGENTS.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [SECURITY.md](SECURITY.md)
- [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## Current gaps

Notable gaps today:

- Austrian E 1kv CSV/PDF/XLSX export is review-gated and currently targets the ausländisch / self-custody Kennzahlen; the styled PDF output includes Steuerbericht-style summary/detail pages, holdings, Besonderheiten, explanations, a transaction appendix, a FinanzOnline-style Kennzahl summary, and FAQ, while the XLSX and CSV bundle use an `Übersicht`, numbered section tabs/files, and `Erläuterungen zum Steuerreport`; domestic-provider withheld KESt metadata is not modeled yet
- full BTCPay invoice/payment provenance ingest is not implemented yet; BTCPay source refresh currently covers confirmed on-chain wallet history, comments/labels, and enrichment routes for existing settlement wallets
- Coinbase Exchange is the default online BTC-USD / BTC-EUR rate source; it fetches coalesced 300-minute windows for missing transaction minutes and records checked sparse minutes so repeat syncs avoid dead zones. Kraken's local OHLCVT CSV archive is wired as an optional offline historical backfill from local CSVs, ZIPs, or extracted directories in the CLI and desktop Settings. Provider-derived cached prices can be rebuilt from Settings or `rates rebuild`; exact exchange execution prices should come from source CSV/API imports with pricing provenance
- descriptor/xpub source refresh through `bitcoinrpc` is not implemented yet
- some Lightning node adapters are declared but do not sync yet
- `custom` wallet import mapping is not implemented yet
- reports still use stored journal pricing rather than querying the rates cache live
- generic text PDF export is still Latin-1-only; Austrian E 1kv and
  source-of-funds PDFs use ReportLab renderers, but the generic PDF exporter
  still substitutes characters outside Latin-1
- no REST/server mode or multi-user auth
- the desktop preview is broad but not production-hardened yet: long-running
  worker-pool/progress plumbing, remaining Settings daemon calls, production
  signing/notarization, and OS-keychain convenience are still open

See [TODO.md](TODO.md) for the active backlog.

## Development notes

- SQLite is the system of record
- BTC-denominated values are stored as integer msat
- machine-readable envelopes are a stable contract and are pinned by `tests/test_cli_smoke.py`

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`)
