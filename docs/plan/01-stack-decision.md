# ADR-01: Desktop UI Stack

**Status:** Accepted.
**Decision:** PySide6 + QML. Intended packaging path: `briefcase`.
**Date:** 2026-04-18.
**Current source of truth:** `pyproject.toml`, `kassiber/ui/`, and `TODO.md`.

## Decision

Use PySide6 + QML for the desktop app.

Why:

- one language/runtime for CLI, core, and UI
- direct `import kassiber.core`; no JSON IPC or sidecar process
- no bundled Chromium and no Node runtime
- good enough visual fidelity for an accounting/workbench app
- conventional Python + Qt patterns are easier for AI-assisted maintenance than
  a Rust/web/Python split

The UI stack is intentionally replaceable because the core extraction keeps the
product logic outside the UI.

## Tradeoffs

Accepted costs:

- custom QML styling is more verbose than CSS
- component ecosystem is smaller than the web ecosystem
- packaged app size must be measured from real builds; do not promise a small
  bundle
- web-centric contributors will need QML context

Packaging:

- `briefcase` supports the target platforms, but Kassiber has not proven the
  macOS `.app` path yet.
- Linux native packages should be evaluated after macOS works.
- VCS-pinned `rp2` may be fragile for packaged builds; prefer a published fork
  artifact before relying on Briefcase release packaging.

Charts:

- current simple balance chart can stay QML `Canvas`
- if a richer Qt-native chart module is needed, prefer Qt Graphs
- do not plan new work around QtCharts

Licensing:

- Kassiber is AGPL-3.0-only
- PySide6/Qt must remain dynamically linked for the LGPL path
- document any new runtime dependency in README and `THIRD_PARTY_LICENSES.md`

Pinning:

- current dependency range is `PySide6>=6.7,<7`
- choose a concrete Qt 6 minor during packaging if reproducible builds require
  it
- prefer supported/LTS Qt minors when possible

## Alternatives Rejected

- **Tauri + SvelteKit + Python sidecar:** best visual fidelity, but permanent
  Rust/web/Python coordination and IPC complexity.
- **Tauri + Leptos/Dioxus:** no Node, but worse AI-assistance predictability and
  still two languages.
- **Slint + Python:** small runtime, but too much custom dashboard/table/dialog
  work for this product.
- **Flet/Flutter:** hides a larger renderer/runtime stack.
- **Electron/NW.js:** rejected by the no-Chromium/no-Node-runtime constraint.

## Revisit Only If

- mobile becomes a real target
- remote multi-user deployment becomes a product requirement
- Qt/QML blocks a required desktop workflow
- a committed contributor joins who is materially stronger in a web stack

## References

- [PySide6 documentation](https://doc.qt.io/qtforpython-6/)
- [Briefcase documentation](https://briefcase.readthedocs.io/)
- [Qt Graphs](https://doc.qt.io/qt-6/qtgraphs-index.html)
