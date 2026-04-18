# Kassiber Desktop + Austrian Tax Plan — Overview

**Status:** Draft for cross-check. Decisions captured here are what the plan is built on; disagreement is welcome.

**Scope of this doc:** Context, goals, constraints, and the roadmap stitching the other docs together. Each sibling doc is self-contained.

---

## What kassiber is today

A **local-first Bitcoin accounting CLI**, written in Python.

- Entry point: `kassiber/__main__.py` → `kassiber/app.py::main(argv)`
- Business logic and CLI dispatch are currently fused into a single ~7,000-line `app.py` (explicitly flagged for extraction in `AGENTS.md`)
- Storage: SQLite at `~/.kassiber/data/kassiber.sqlite3`, integer msat amounts (never float)
- Tax engine: [RP2](https://github.com/eprbell/rp2) (open source, Python) used through an indirection layer in `app.py`
- External I/O: Esplora / Electrum / Bitcoin Core RPC; CoinGecko for rates; Phoenix/BTCPay/BIP329 importers
- Test contract: `tests/test_cli_smoke.py` pins the machine-readable JSON envelope emitted by every CLI command — this is the reliable seam the whole plan hangs on
- Data model: workspaces → profiles → accounts → wallets → transactions, with journal_entries, journal_quarantines, transaction_pairs, tags, bip329_labels, backends, rates_cache

## What we're building

Three concurrent tracks, all grounded in a single library refactor:

1. **Phase 0 — Core extraction.** Carve a reusable `kassiber.core` library out of `app.py`. CLI becomes a thin translator. Precondition for everything else. See `02-core-extraction.md`.
2. **Desktop UI.** PySide6 + QML application that imports `kassiber.core` directly. Clams-inspired layout. See `04-desktop-ui.md`.
3. **Austrian tax engine.** A sibling engine to the existing RP2 path, because the Austrian regime (moving average per wallet from 2023, non-taxable crypto-crypto swaps, Altvermögen vs Neuvermögen classification) is structurally different from what RP2 models. See `06-austrian-tax-engine.md`.

Plus one small new feature with cross-cutting impact:

4. **Transaction attachments** — tag a receipt PDF or drive link to any transaction. Useful beyond tax (audit, bookkeeping). See `05-attachments.md`.

## Design constraints (from the project owner)

| Constraint | Implication |
|---|---|
| Pre-release, no users but self | No backwards compatibility burden; freely rename/remove commands; keep docs in sync |
| Bitcoin-only product focus | No altcoin zoo complexity; Liquid L-BTC in scope; no hardcoded Liquid federation addresses |
| Solo maintainer + AI-assisted (vibecoded) | Stack must be in Claude's fluent zone. One language beats two. Conventional patterns beat exotic ones. |
| "Make cybersecurity guys happy" | Minimize attack surface. No bundled Chromium. No Node at runtime. Audit-friendly deps. |
| No Node in the shipped product | Excludes Electron, NW.js. Node as a dev-time build tool would be tolerable but unnecessary. |
| Maintainable architecture over speed | Willing to rewrite. Willing to refactor. Willing to delete. |
| CLI stays first-class | UI and CLI are peers over the same library. CLI ships first, UI catches up. |
| No mobile for now | Removes Tauri's mobile advantage as a tiebreaker |

## Stack decision (summary — detail in 01)

**PySide6 + QML.** Python everywhere, native widgets, no webview, no Node in any form, direct `import kassiber.core` from the UI. QtCharts for the balance chart. `briefcase` for packaging.

**Honest second place:** Tauri + SvelteKit + Python sidecar. Ruled out for this project because two-language maintenance costs more than pixel-perfect Clams aesthetic is worth.

## Target platforms

- **v1:** macOS (matches the reference screenshots)
- **v1.1:** Linux (briefcase supports it cheaply once the macOS path works)
- **Later:** Windows (briefcase supports it; no user demand yet)
- **Never planned:** mobile

## Roadmap

| Phase | Scope | Rough effort |
|---|---|---|
| **0** | Library extraction: `kassiber.core`, `kassiber.cli`. Smoke tests stay green. | 3–5 days |
| **0.5** | Austrian tax engine + E 1kv report + attachments feature. CLI-only for all of it. | 7–10 days |
| **1** | PySide6 app shell, empty state matching Clams screenshot 2 | 2 days |
| **2** | Read-only dashboard: six tiles wired to `core/` | 4–6 days |
| **3** | Add Connection modal, sync action with progress, CSV import, attachment drag-drop | 4–5 days |
| **4** | Welcome wizard, Settings dialog, briefcase packaging for macOS | 3–4 days |
| **5+** | Tile drag/resize, tag management UI, dark mode, Linux/Windows builds, code-signing | TBD |

**MVP surface (end of Phase 4):** single-user desktop app plus CLI, SQLite-backed, Austrian tax reports in PDF + CSV, attachments on transactions, backup/restore.

## Document index

| Doc | Scope |
|---|---|
| [00-overview.md](./00-overview.md) | This doc. Context, constraints, roadmap. |
| [01-stack-decision.md](./01-stack-decision.md) | ADR for PySide6 + QML. Alternatives and their rejections. |
| [02-core-extraction.md](./02-core-extraction.md) | Phase 0 refactor. Module map, migration approach, success criteria. |
| [03-storage-conventions.md](./03-storage-conventions.md) | SQLite discipline: WAL, FKs, migrations, repository pattern. |
| [04-desktop-ui.md](./04-desktop-ui.md) | Phases 1–4 UI spec. Tile-by-tile. QML conventions. Threading. |
| [05-attachments.md](./05-attachments.md) | Transaction attachments: schema, storage, CLI, UI, backup. |
| [06-austrian-tax-engine.md](./06-austrian-tax-engine.md) | AT engine algorithm, data model, E 1kv report layout. |
| [07-austrian-tax-open-questions.md](./07-austrian-tax-open-questions.md) | Live backlog of genuinely unsettled AT tax questions. |

## What this plan is not

- **Not legal or tax advice.** The Austrian engine ships behind a disclaimer. Production use gated on Steuerberater review.
- **Not a commitment to dates.** Effort estimates are for sequencing, not scheduling.
- **Not final on report aesthetics.** E 1kv output will evolve with user's Steuerberater feedback.
- **Not a rewrite.** The refactor in Phase 0 is mechanical, not a re-architecture. Pure logic moves; behavior identical (smoke tests prove it).

## Known risks

| Risk | Mitigation |
|---|---|
| RP2 single-maintainer stagnation | Isolated behind `core/engines/rp2_generic.py`. Vendor in-tree if upstream dies. |
| Austrian BMF guidance evolves | Engine is plain Python under our control; versioning via migrations + regime cutoff constants in code |
| app.py extraction breaks the envelope | Smoke tests are extensive enough to catch it; extract one command at a time |
| PySide6 LGPL license surprises | LGPL is fine for a freely distributed app; no Qt commercial license needed |
| Solo vibecoding drift | Each phase has concrete success criteria (smoke green, tile renders real data, packaged `.app` launches) |

## Next action

Phase 0 kickoff. Recommended: commit this `docs/plan/` directory and the companion memory snapshot, then branch fresh for the extraction work. Details in `02-core-extraction.md`.
