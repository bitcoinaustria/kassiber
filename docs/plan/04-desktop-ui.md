# Desktop UI Plan

**Status:** Active implementation guide.
**Current source of truth:** `kassiber/ui/`, `docs/design/`, `docs/reference/desktop.md`, and `TODO.md`.
**Stack:** PySide6 + QML. No webview. No Node runtime.
**Packaging:** `briefcase` is intended but not configured yet.

## Product Intent

The desktop app is a local, document-oriented companion to the first-class CLI.
It imports `kassiber.core` directly and reads/writes the same local SQLite
store. It must make common workflows calmer and more visible without creating a
second accounting engine.

Current UI code already has a routed QML shell, live read-only snapshot data,
and preview surfaces for welcome, overview, connection detail, transactions,
reports, profiles, and settings. Live creation/sync/import actions still happen
through the CLI until worker-backed UI actions land.

## Architecture Rules

- QML owns layout only. Business logic lives in Python view-models or core.
- View-models expose Qt properties/signals and call `kassiber.core`.
- Any sync, import, backup, restore, or journal processing work runs in a
  worker thread with its own SQLite connection.
- Do not share SQLite connections across threads.
- Add cooperative cancellation before live workers ship: app quit should stop
  workers cleanly, then close their DB connections.
- Keep CLI and UI behavior aligned through shared core functions, not
  subprocess parsing.

## Source Shape

```text
kassiber/ui/
  app.py                 # QApplication, QML engine, lifecycle
  dashboard.py           # read-only snapshot builder
  theme.py               # design tokens exposed to QML
  viewmodels/            # QObject surfaces for QML
  resources/qml/
    Main.qml
    pages/
    dialogs/
    components/
```

Future live actions may add `kassiber/ui/workers/`.

## Design Workflow

Use `docs/design/README.md` for screen work:

1. Freeze screenshots / screen spec.
2. Build a static QML pass.
3. Capture screenshot review.
4. Wire runtime behavior after layout is stable.

Do not generate QML directly from JSX or HTML exports. Treat them as visual
evidence only.

## Visual Direction

- warm neutral background, white work surfaces, restrained borders
- serif/display accent type plus mono/reporting type where already established
- compact accounting/productivity layout, not a marketing landing page
- stable dimensions for charts, tables, buttons, and tile controls
- no decorative webview-style flourishes that complicate QML

Current charting uses a lightweight QML `Canvas`. If a packaged chart module is
needed later, prefer Qt Graphs over QtCharts.

## Current Surfaces

### Welcome

Shows first-run/profile-oriented messaging. Live onboarding is still pending.
When it becomes live, it should create a default workspace and profile through
core APIs, then refresh the same snapshot/view-model path.

### Overview

Read-only dashboard surface over current profile data:

- connection count
- transaction count
- journal/quarantine readiness
- recent activity
- report readiness

The balance chart default should be a total profile line. Per-wallet lines can
be a toggle once the data and legend behavior are clear.

### Connections

Connection picker must distinguish live wallet kinds from placeholders.

Current CLI wallet kinds: `descriptor`, `xpub`, `address`, `coreln`, `lnd`,
`nwc`, `phoenix`, `river`, `custom`.

Additional service/exchange tiles such as BTCPay, Cashu, Kraken, Bitstamp, or
Coinbase may appear as disabled/coming-soon UI only until real adapters exist.

### Transactions

Read-only recent transaction table/list with detail surface. Later live work:
tag edits, notes, exclusions, attachment management, and transfer/swap pairing
review. Those must invalidate journals just like CLI mutations.

### Reports

Read-only readiness and preview surface. Reports remain trustworthy only after
`journals process` has run and quarantines are resolved or intentionally
excluded.

Generic reports are already CLI-backed. Austrian tax processing and the
review-gated E 1kv CLI/PDF/XLSX export exist in core.

### Settings

Current settings are mostly path/status and preference preview. Live settings
actions still need implementation:

- hide sensitive data
- logs export
- project backup
- project reset
- future tax-country/profile controls

Do not add hot in-place restore in the MVP. Prefer "close project, import as new
project" for the first restore/import design.

## MVP Remaining Work

- polish current routed QML screens against `docs/design/`
- implement worker-backed sync/import/journal actions with cancellation
- implement supported Add Connection flows through core APIs
- expose attachment/link management in transaction detail
- finish Settings live actions
- add `briefcase` packaging config and prove a macOS `.app`

## Packaging Notes

- `pyproject.toml` currently has no Briefcase configuration.
- VCS-pinned `rp2` may be fragile in packaged builds; prefer a published fork
  artifact before relying on Briefcase packaging.
- Keep Qt libraries dynamically linked for LGPL compliance.
- Do not promise final bundle size until measured from an actual `.app`.

Illustrative future config:

```toml
[tool.briefcase.app.kassiber]
formal_name = "Kassiber"
description = "Local-first Bitcoin accounting"
license = "AGPL-3.0-only"
icon = "kassiber/ui/resources/images/icon"

[tool.briefcase.app.kassiber.macOS]
requires = [
  "PySide6",
  "embit>=0.8.0",
  "rp2 @ git+https://github.com/bitcoinaustria/rp2.git@<pin>",
]
```

E 1kv PDF export uses the existing line-oriented PDF writer, so no `reportlab`
dependency is needed. Styled E 1kv workbook export uses XlsxWriter.

## Out Of Scope

- browser/web companion
- server or remote deployment
- multi-user auth
- mobile
- plugin system
- invoice/VAT/general-ledger workflow
