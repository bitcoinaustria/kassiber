# Desktop UI Spec — Phases 1–4

**Stack:** PySide6 + QML (see `01-stack-decision.md`).
**Packaging:** `briefcase`, macOS first, Linux second.
**Launch:** `kassiber ui` subcommand.
**Reference:** Clams.tech old desktop UI (screenshots provided by project owner).

This doc specifies the desktop UI through MVP (end of Phase 4). Post-MVP items are listed at the end.

## Launch and lifecycle

- Entry point: `kassiber ui` — a CLI subcommand that imports `kassiber.ui.app` and calls `run()`.
- On start:
  1. Call the shared runtime/bootstrap layer from `core.runtime`
  2. Resolve data root / env file / settings / backend overlay through the same path as the CLI
  3. Open the DB through the canonical bootstrap path and ensure pending migrations / compatibility fixes are applied
  4. If no profile exists or `ui.first_run == True`, show Welcome wizard (Phase 4)
  5. Otherwise, show main window
- Closes cleanly on window close. Persists window geometry + tile-hide flags to `~/.kassiber/config/settings.json` under a `ui:` subkey.

## Source layout

```
kassiber/ui/
  __init__.py
  app.py                 # QApplication, window lifecycle, signal wiring
  viewmodels/
    dashboard_vm.py      # QObject; exposes Qt properties to QML
    connections_vm.py
    transactions_vm.py
    fiat_vm.py
    filters_vm.py
    balances_vm.py
    settings_vm.py
  workers/
    sync_worker.py       # QThread for long syncs
    import_worker.py
    backup_worker.py
  resources/
    qml/
      Main.qml
      tiles/
        BalanceOverTime.qml
        Connections.qml
        Filters.qml
        Fiat.qml
        Transactions.qml
        Balances.qml
        Exports.qml
      dialogs/
        AddConnection.qml
        WalletDetail.qml
        TransactionDetail.qml
        Settings.qml
        Welcome.qml
      components/
        Card.qml          # white rounded card w/ shadow
        FilterPill.qml    # outlined pill with hue
        PrimaryButton.qml # red CTA
        IconButton.qml
        EmptyState.qml
    fonts/
      *.ttf (serif display + mono body; see Typography)
    icons/
      *.svg (Phosphor/Lucide subset)
    images/
      logo.svg
  theme.py               # color tokens, font families, spacing scale
```

Rule: no business logic in QML. QML binds to view-model Qt properties. View-models call `kassiber.core`. Workers run in `QThread` for anything that might take longer than 100ms.

## Theme tokens

```python
# theme.py — single source of truth, exported into QML via qmlRegisterType
COLORS = {
    "bg":          "#F5F1E8",   # cream page background
    "card":        "#FFFFFF",
    "card_border": "#EDEAE0",
    "ink":         "#1A1916",
    "ink_muted":   "#6B6862",
    "accent":      "#8B0000",   # deep red — primary CTA
    "accent_dim":  "#C08A8A",   # disabled CTA
    "chip_border": "#C9C4B8",
    # filter pill hues: each value is an outline color, background is white
    "pill_amber":  "#D97706",
    "pill_yellow": "#CA8A04",
    "pill_teal":   "#0F766E",
    "pill_green":  "#166534",
    "pill_olive":  "#65A30D",
    "pill_indigo": "#4338CA",
    # status
    "ok":          "#166534",
    "warn":        "#B45309",
    "err":         "#991B1B",
}
FONTS = {
    "display":     "Source Serif 4",   # or similar EB-Garamond-alike
    "body":        "Courier Prime",    # or JetBrains Mono / IBM Plex Mono
}
SPACING = {"xs": 4, "sm": 8, "md": 16, "lg": 24, "xl": 40}
RADIUS  = {"sm": 4, "md": 8, "lg": 12}
SHADOW  = {"card": "0 2px 8px rgba(0,0,0,0.05)"}  # translated to Qt DropShadow
```

Concrete font picks are deferred — exact faces can be negotiated during Phase 2; API and tokens are what matter.

## Threading model

| Work | Thread |
|---|---|
| Any DB read under ~50ms | Main thread, inline |
| Any DB read over ~50ms | Worker QThread with a dedicated connection |
| `core.sync.*` | Always worker QThread |
| Importers | Always worker QThread |
| Backup/restore | Always worker QThread |
| Journal computation (RP2-backed tax engine, including Austrian fork/plugins) | Always worker QThread |
| QML rendering, input, signals | Main thread only |

Workers communicate via Qt signals. Progress callbacks into `core.*` functions emit signals that view-models re-expose as observable properties (e.g., `dashboardVM.syncProgress`).

Every worker opens **its own** SQLite connection (with the standard pragmas) on the worker thread. SQLite connections are not thread-safe; sharing across threads is forbidden.

## Design iteration via Claude Design

Each phase that introduces new screens (2 onwards) should start with visual mockups in [Claude Design](https://www.anthropic.com/news/claude-design-anthropic-labs) before touching QML. Rationale: iterating on tile layout, spacing, pill hues, empty states, and typography is ~10× faster in an HTML-output prototyping tool than in Qt, and cheaper than rendering Qt widgets just to judge a color.

Workflow:

1. Open a Claude Design session per phase (Phase 2 dashboard tiles, Phase 3 Add Connection + Transaction Detail + Wallet Detail dialogs, Phase 4 Welcome wizard + Settings).
2. Feed it the Clams reference screenshots + the theme tokens from `theme.py` above.
3. Iterate on visuals — layout, copy, pill colors, empty-state framing — until acceptable.
4. Freeze the accepted screen states as screenshots at explicit desktop widths. If Claude Design exports `.jsx`, treat those files as reference evidence only and use the screenshots as the visual source of truth.
5. Create a per-screen spec under `docs/design/phase-<n>/<screen>/` using the repo workflow in `docs/design/README.md`.
6. Implement a **static** QML pass from that spec using mock data only.
7. Capture a screenshot review against the frozen references before wiring any real runtime behavior.

What this *doesn't* do: generate QML. HTML / JSX → QML is a manual translation step. Claude Design is a design-spec tool, not a codegen tool. Its output is the visual contract; QML is still written by hand (or by Claude Code consuming the screenshots + screen spec). The Clams-fidelity tradeoffs from `01-stack-decision.md` (pastel-fill pills vs outlined pills, drop-shadow depth, serif weight) are best resolved in Claude Design before committing them to QML properties.

Artifacts from each design session live under `docs/design/phase-<n>/<screen>/` alongside the frozen screenshots and screenshot-review notes — not checked into `resources/` since they're not runtime assets.

---

## Phase 1 — App shell (2 days)

**Goal:** Window opens, frame matches Clams screenshot 2, empty state shows with a functional `+ Add Connection` CTA that opens a placeholder modal.

### Included

- Main window: title bar ("Kassiber"), top bar (logo + Project pill), main content area, bottom bar (version, settings gear, social icons, Support the App heart)
- Empty state centered: info icon + "Add a connection and automatically sync your transaction data to get started." + red CTA
- Settings gear opens an empty Settings dialog (will be filled in Phase 4)
- Project pill in top-right — opens a popover listing the current profile; click to switch or manage (read-only for Phase 1, editable in Phase 3)
- Close window triggers `QApplication.quit()` cleanly

### Not included

- Dashboard tiles (Phase 2)
- Actual Add Connection flow (Phase 3 — modal is a placeholder)
- Welcome wizard (Phase 4 — Phase 1 just opens directly to main window)

### Done when

- [ ] `kassiber ui` opens the window on macOS
- [ ] Empty state is rendered via QML, bound to `connections_vm.is_empty`
- [ ] Settings gear opens dialog (empty shell)
- [ ] Window position and size are persisted across launches
- [ ] Closing the app doesn't leak SQLite WAL or shm files (they're transient, but no open handles)

---

## Phase 2 — Read-only dashboard (4–6 days)

**Goal:** All six tiles render live data from `kassiber.core`. No writes from the UI yet.

### Tile 1 — Balance Over Time

- Chart type: line chart, one line per wallet aggregate (default: total across all wallets for selected filter range)
- Library: **QtCharts**
- Data source: `core.reports.balance_history(conn, *, from_date, to_date, account_id=None, tag_id=None) -> list[BalancePoint]`
- Bound to filters from Tile 3
- Empty state: info icon + "No Data — Sync your connections to import some data."
- Interactions for MVP: hover tooltip showing date + sat balance + fiat equivalent. No zoom/pan in MVP.

### Tile 2 — Connections

- List of wallets with kind icon, name, last sync time, and a tiny status dot (green = ok, amber = stale, red = error)
- Header buttons: "Sync all" (refresh icon) — triggers `core.sync.sync_all(conn, on_progress=...)` in a worker
- Footer: "+ Add" button opens Add Connection modal (Phase 3)
- Empty state: "No Connections — Add a new connection to get started."
- Clicking a wallet opens Wallet Detail dialog (Phase 3)

### Tile 3 — Filters

- Date range inputs: two date-pickers ("From", "To")
- Quick-select pills: "One week", "Month to date", "Last month", "Current quarter", "Last quarter", "Year to date" — each colored per theme `pill_*` tokens
- Account dropdown: lists all accounts from `core.accounts.list_accounts(conn)`
- Tag dropdown: lists all tags from `core.tags.list_tags(conn)`
- "Reset" button clears all filters
- Filter state is exposed as view-model properties that other tiles observe

### Tile 4 — Fiat

- Shows:
  - Current Bitcoin price (from `core.rates.get_latest_rate(conn, fiat="USD")` — or profile-configured fiat, e.g., "EUR")
  - Average cost basis
  - Total cost (FIFO)
  - Current market value
  - Total profit / loss
- All values respect the active filter range
- "Hide sensitive data" toggle in Settings redacts the numeric values as "•••" (typography preserved)

### Tile 5 — Transactions

- Paginated table: date, kind icon (in/out/transfer), amount (sats), fiat equivalent at tx time, wallet name, tags
- Click row opens Transaction Detail dialog (Phase 3)
- Pagination: default 50 rows, Show More button at bottom
- Empty state: "No Data — Sync your connections to import some data."
- Bound to filter state

### Tile 6 — Balances

- Account-type rollups: Assets, Income, Expenses, Liabilities, Equity
- Chevron expands to show sub-accounts under Income and Expenses
- Sats displayed with 8-decimal grouping and B symbol per the mockup
- Values come from `core.reports.balances(conn, *, as_of=filters.to_date, account_id=filters.account_id)`

### Tile 7 — Exports

- Static card with two links:
  - "Capital Gains Report" — opens file-save dialog, writes CSV via `core.reports.capital_gains.to_csv(...)`
  - "Journal Entries Export" — same pattern for `core.reports.journal_entries.to_csv(...)`
- Will grow an "Austrian E 1kv (PDF)" link once the Austrian export path lands on top of the already-active RP2 fork integration (see `06-austrian-tax-engine.md`)

### Layout

Fixed grid for MVP (QML `GridLayout` or manual `anchors`):

```
row 1 (top): [Balance Over Time (wide)] [Connections] [Filters] [Fiat]
row 2 (mid): [Transactions (wide)]           [Balances (wide)]
row 3 (bot): [Exports]
```

No tile drag-resize in MVP. That's a Phase 5+ item.

### Done when

- [ ] All six tiles render with live data on a DB that has at least one synced wallet and a few transactions
- [ ] Filters propagate to all affected tiles within 200ms
- [ ] Empty states render correctly when DB is fresh
- [ ] "Hide sensitive data" redacts all numeric values
- [ ] Exports produce valid CSVs matching the existing CLI output byte-for-byte

---

## Phase 3 — Connections, sync, attachments (4–5 days)

**Goal:** Users can add connections, trigger sync with progress feedback, import CSVs, and attach receipt PDFs or drive links to any transaction.

### Add Connection modal

Matches screenshot 3.

- 7 kind tiles: Core Lightning, LND, NWC, XPub, Descriptor, Import, Cashu
- Only the kinds kassiber already supports open a real form:
  - XPub → name, xpub string, derivation path
  - Descriptor → name, descriptor string
  - Address → name, address string (single-address wallet)
  - Import → file picker for CSV/JSON
- Lightning kinds (Core Lightning, LND, NWC) and Cashu render a "Coming soon" state with a mailing-list signup link (no actual signup in MVP — could be a `mailto:` to the project owner)
- On successful add, modal closes and Connections tile refreshes

### Sync

- "Sync all" button triggers `sync_worker.run(conn, wallet_ids=None)` which calls `core.sync.sync_all(conn, on_progress=...)` in a QThread
- Progress reported as `(wallet_name, step_description, fraction)` via Qt signal
- UI shows a bottom banner during sync: "Syncing wallet X (3/12): fetching transactions..." with a progress bar
- Banner auto-dismisses 3s after completion
- Errors render a dismissible red banner with an "Open logs" shortcut

### Import

- Import kind in Add Connection modal opens file picker
- Detects format from extension/content: Phoenix CSV, BTCPay CSV/JSON, BIP329 JSONL, generic transactions JSON/CSV
- Import runs in a worker thread with progress
- Post-import: Connections tile shows the new virtual "imported" wallet; Transactions tile picks up the new rows on next filter apply

### Transaction Detail dialog

- Opened by clicking a row in Transactions tile
- Shows: full timestamp, msat amount, fiat at time, wallet, kind, labels (BIP329), tags, counterparties (if known), pairing info (if part of a self-transfer pair)
- **Attachments section** (see `05-attachments.md`):
  - Drag-drop zone: drop a PDF/image to attach as file
  - "Add URL" button to paste a drive link
  - Existing attachments render as chips with filename + note; click opens in system handler (Finder/Preview for files, browser for URLs)
  - Remove button per chip
- Save button commits tag changes; attachments persist immediately on drop/add

### Wallet Detail dialog

- Opened from Connections tile
- Shows: name, kind, xpub/descriptor/address (respecting Hide sensitive toggle), last sync time, transaction count
- Edit name inline; Save / Cancel

### Done when

- [ ] Adding an xpub wallet syncs its transactions and updates all tiles
- [ ] Importing a Phoenix CSV creates a virtual wallet with all rows
- [ ] Sync shows live progress in the bottom banner
- [ ] Dropping a PDF onto a transaction creates an attachment and the file lands in `~/.kassiber/attachments/`
- [ ] Adding a drive URL to a transaction persists and reopens on click

---

## Phase 4 — Onboarding, settings, packaging (3–4 days)

**Goal:** A first-time user experience matching screenshot 1, full Settings dialog matching screenshot 5, and a working signed macOS `.app` bundle.

### Welcome wizard

- Shown on first run (no profile in DB, or `ui.first_run` flag unset)
- Matches screenshot 1: Welcome title, logo, name input, "Let's go!" CTA
- Optional second step: fiat currency (USD / EUR / …), with `generic` as the initial default even though Austrian RP2 support is already available at the CLI/core layer
- On completion: creates default workspace + profile, stores `ui.first_run = False`

### Settings dialog

Matches screenshot 5.

- "Hide sensitive data" toggle
- "Download logs" button — zips `~/.kassiber/logs/*.jsonl` from last 14 days via `backup_worker` and opens a file-save dialog
- "Backup Data" button — creates `.kassiber.tar` archive (DB via `sqlite3 .backup` + attachments dir) via `backup_worker`
- "Restore from backup" button — confirmation dialog, then unpack archive and replace live DB (after stopping any worker)
- "Reset app" button (red, destructive) — triple-confirm, then wipes `~/.kassiber/data/` and returns to Welcome wizard
- Future section (not MVP): "Tax country" dropdown — switching it re-runs journal computation once the UI exposes the already-landed Austrian RP2 plugin / fork integration

### Packaging

- Use `briefcase` configured in `pyproject.toml`
- macOS: produce a `.app` bundle; support code-signing if Apple Developer credentials are supplied via env vars (CI-friendly)
- Notarization is optional for MVP — unsigned app is fine for the project owner's own use
- Linux: evaluate native packages / Flatpak after the macOS path is stable; do not treat AppImage as the default packaging target
- Windows: Phase 5+

### Done when

- [ ] Fresh install (empty `~/.kassiber`) shows Welcome, completes to main window
- [ ] Settings dialog matches screenshot 5 layout and all buttons work
- [ ] `briefcase build macOS` produces a `.app` that launches
- [ ] Backup + Restore round-trips cleanly (DB identical, attachments present)
- [ ] Reset app returns the user to Welcome
- [ ] Launching `.app` from Finder works on a fresh machine without Python preinstalled

---

## Phase 5+ backlog (post-MVP)

Ordered roughly by value. Not committed.

1. Tile drag/resize — matches Clams gridstack feel; persist layout per profile
2. Tag management UI — create, rename, delete, bulk-apply tags
3. Dark mode — theme.py has dual palettes; QML reads the active one
4. Chart interactivity — zoom, pan, crosshair in Balance Over Time
5. Multi-workspace switcher — UI for kassiber's existing workspace concept
6. Lightning node wiring (Core Lightning, LND, NWC) — the add-connection modal's "coming soon" tiles
7. Cashu wallet integration
8. Linux `.deb` / `.rpm` packaging
9. Windows `.msi` packaging
10. Auto-update — `briefcase` has no built-in mechanism; consider `pyupdater` or manual "new version available" banner
11. Localization — German UI strings; leaves the tax-technical terms (E 1kv, Altvermögen) untranslated in reports
12. Keyboard shortcuts for common actions
13. Export presets (quarterly, YTD, custom saved filters)

---

## Accessibility

Not in MVP scope but targeted for Phase 5+:

- Keyboard navigation through tiles and dialogs
- Screen-reader labels on all icon-only buttons
- Minimum font size of 14px for body, 12px for metadata
- Focus rings visible on all interactive elements
- `Qt::WA_MacShowFocusRect` respected

## Testing

- **Unit tests** for view-models (no QML): Python-side tests with Qt fixtures once the UI lands; mock `core.*` calls; verify signals are emitted as expected
- **Smoke tests** for QML: render each tile with a fixture view-model and snapshot the tree once a Qt test harness is added
- **Manual golden path** at end of each phase: fresh install → welcome → add xpub wallet → sync → filter last month → export capital gains CSV. Documented in `tests/ui_smoke.md` as a human-runnable script.
- No full-E2E automation in MVP (Qt GUI automation is expensive; we catch regressions in view-model unit tests)

## Packaging metadata

In `pyproject.toml`:

```toml
[tool.briefcase.app.kassiber]
formal_name = "Kassiber"
description = "Local-first Bitcoin accounting"
license = "AGPL-3.0-only"  # match the current project metadata unless intentionally changed
icon = "kassiber/ui/resources/images/icon"

[tool.briefcase.app.kassiber.macOS]
requires = ["PySide6", "embit>=0.8.0", "rp2>=1.7.2", "reportlab"]  # full runtime deps
```

Full list finalized when briefcase runs; this is illustrative.

## Out of scope

- Server / remote deployment
- Multi-user accounts within one installation
- Mobile
- Browser-based companion
- Plugin system
