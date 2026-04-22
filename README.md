# Kassiber

Kassiber is an open-source, local-first Bitcoin accounting CLI with an early desktop shell.

It keeps your accounting state on your machine, syncs from Bitcoin-native sources, and processes journals locally before generating reports. The cloud-SaaS model is the thing Kassiber is trying to avoid.

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md). It covers backend visibility, external requests, and current caveats such as missing at-rest encryption and incomplete Tor support.

## What Kassiber does

- keeps a local SQLite system of record
- supports multiple workspaces, profiles, accounts, and wallets
- syncs from `esplora` and `electrum`, plus `bitcoinrpc` for address-based Bitcoin wallets
- imports generic CSV/JSON, BTCPay exports, Phoenix exports, and BIP329 labels
- stores notes, tags, exclusions, transfer pairs, and attachments
- processes journals explicitly before reports are trusted
- exposes every command through a deterministic JSON envelope
- ships an early PySide6/QML desktop shell over the same local data

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

## Local state

By default Kassiber stores state under `~/.kassiber/`:

- `data/kassiber.sqlite3` for SQLite data
- `config/backends.env` for backend config
- `config/settings.json` for the managed path manifest and UI state
- `exports/` for generated report files
- `attachments/` for managed attachment blobs

Use `kassiber status` to see the active paths. `--data-root` and `--env-file` let you override them.

## Installation

Requirements:

- Python `>=3.10`
- `embit>=0.8.0`
- `PySide6>=6.7,<7`
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

Before pushing code or docs changes, run:

```bash
./scripts/quality-gate.sh
```

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
python3 -m kassiber reports balance-sheet
python3 -m kassiber reports capital-gains
python3 -m kassiber ui
```

## Docs

Reference docs:

- [docs/reference/backends.md](docs/reference/backends.md)
- [docs/reference/imports.md](docs/reference/imports.md)
- [docs/reference/tax.md](docs/reference/tax.md)
- [docs/reference/machine-output.md](docs/reference/machine-output.md)
- [docs/reference/desktop.md](docs/reference/desktop.md)

Planning and architecture docs:

- [docs/plan/00-overview.md](docs/plan/00-overview.md)
- [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md)
- [docs/plan/06-austrian-tax-engine.md](docs/plan/06-austrian-tax-engine.md)
- [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md)

Contributor docs:

- [AGENTS.md](AGENTS.md)
- [CONTRIBUTING.md](CONTRIBUTING.md)
- [SECURITY.md](SECURITY.md)
- [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)

## Current gaps

Notable gaps today:

- Austrian E 1kv export is not shipped yet
- BTCPay API-backed provenance import is not implemented yet; file imports remain the current path
- descriptor/xpub live sync through `bitcoinrpc` is not implemented yet
- some Lightning node adapters are declared but do not sync yet
- `custom` wallet import mapping is not implemented yet
- reports still use stored journal pricing rather than querying the rates cache live
- no REST/server mode or multi-user auth
- desktop UI is still early

See [TODO.md](TODO.md) for the active backlog.

## Development notes

- SQLite is the system of record
- BTC-denominated values are stored as integer msat
- machine-readable envelopes are a stable contract and are pinned by `tests/test_cli_smoke.py`

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`)
