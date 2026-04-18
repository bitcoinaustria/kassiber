# ADR-01: Desktop UI Stack

**Status:** Accepted, pending build-out in Phase 1.
**Decision:** PySide6 + QML, packaging via `briefcase`.
**Date:** 2026-04-18.

## Context

Kassiber today is a Python CLI. The product owner wants a desktop UI inspired by the old Clams.tech desktop app (cream/red palette, white tiled dashboard, serif + mono typography) without compromising on the project's constraints:

- Solo maintainer + AI-assisted development (the author does not read source directly; models write, author vibes)
- "Make cybersecurity people happy" — minimal attack surface, auditable dependency graph
- No Node in the shipped runtime; no bundled Chromium
- Pre-release, no users to preserve compatibility for, willing to rewrite
- Bitcoin-only product, not a broad crypto tool
- No mobile target now or planned
- CLI stays first-class; UI and CLI are peers over a shared Python library

## Options considered

### A. PySide6 + QML (chosen)

Python-only stack. QML declarative layout, Python signals/slots for logic. Qt's Python bindings are LGPL and first-party from The Qt Company.

**Pros**
- **One language.** The UI imports `kassiber.core` directly. No IPC boundary. No subprocess JSON envelope parsing. Debugging is single-process.
- **Vibecoding fluency.** Claude writes idiomatic Qt/QML fluently — training corpus is large. Python+Qt is more predictable output than Rust+Leptos would be.
- **No webview at all.** Native widgets render via Qt's own stack. Zero JavaScript runtime surface. Good for cybersec posture.
- **No Node in any form.** Not runtime, not dev-time.
- **Mature and stable.** Qt is 30+ years old. PySide6 is LTS and actively maintained.
- **Packaging is strong on the targets that matter.** `briefcase` gives us a clean macOS path now and credible Linux/Windows options later, without introducing a second runtime stack.
- **QtCharts** gives us the Balance Over Time chart without a third-party library.
- **Accessibility and native feel** on each OS come for free (menu bar on macOS, Wayland on Linux, etc.).

**Cons**
- **UI styling is more verbose than CSS.** Qt uses a different styling model from the web. See the "pixel-perfect Clams look" discussion below.
- **Custom component look-and-feel** requires more code than equivalent Tailwind/shadcn in a web stack. Filter-pill fills, soft card shadows, and dashboard tile resize handles all take extra work.
- **Smaller component ecosystem** than the web world. Qt has a rich built-in set, but "pretty" third-party widget libraries are fewer.
- **Bundle size** ≈ 80–120 MB fully packaged. Not small, not large.

**Cost of Clams-aesthetic approximation**

Items that are one-liners in CSS and multi-line components in QML: pastel-filled filter pills, soft `box-shadow` on cards, gridstack-style drag/resize tile handles. The approximation we accept for v1:

- Same palette, same typography (bundle the serif + mono fonts as Qt resources)
- White rounded cards with a simpler single-shadow layer
- Filter pills with colored outlines and same-hue text, no pastel fill
- Fixed tile layout (no drag/resize) for MVP; drag/resize is a Phase 5+ item
- Logo and icons via embedded SVGs

Result: ~85–95% of the Clams feel at ~40% of the styling effort. A Clams user recognizes it; a designer sees Qt's bones rather than the web's.

### B. Tauri 2 + SvelteKit + Python sidecar (honest second place)

Tauri (Rust) hosts a system webview; SvelteKit frontend compiled to a static bundle; Python kassiber runs as a sidecar invoked with JSON envelope IPC.

**Pros**
- Pixel-perfect Clams aesthetic is trivial — it's the same tech Clams itself uses.
- Svelte is arguably Claude's single strongest UI target by training-data density.
- Tauri is the current best-in-class security posture for webview-based desktop apps.
- Node is build-time-only, not shipped.
- Small Rust binary; system webview adds no Chromium payload.

**Cons**
- **Two languages.** Rust for the shell and commands, Python for the engine. Every feature potentially touches both.
- **IPC boundary fragility.** Streaming progress during long syncs crosses process + pipe + parse boundaries. Qt's `QThread` + signals handles this in-process.
- **Triple dep graph.** `cargo` + `npm/pnpm` + `pip/uv`. More SBOM surface, more supply-chain to audit.
- **Debugging is tri-modal** (frontend devtools + Rust tauri logs + Python stderr).
- **Mobile upside is zero** for this product (constraint: no mobile).
- **Reproducible builds** would earn Tauri points but are not a priority for this phase.

**Why rejected:** The two-language cost is a permanent tax paid in exchange for a one-time aesthetic win. For a solo vibecoder who doesn't read the code, that tax is significant — every change potentially needs coordination across languages, and Claude's quality is more consistent when staying in one ecosystem. We accept "Qt-flavored Clams" to keep the project in one language.

### C. Tauri + Leptos / Dioxus (Rust frontend) — rejected earlier

Same Rust backend as B, but a Rust-based WASM frontend (Leptos/Dioxus) instead of Svelte, eliminating Node entirely.

**Why rejected:** Still two languages (Rust + Python). Leptos/Dioxus have small training corpora compared to Svelte/React — AI output quality drops. Signal-based reactivity has corner cases Claude frequently gets wrong. All the downsides of B without Svelte's AI-fluency upside.

### D. Slint + Python bindings — rejected

Declarative `.slint` markup, official Python bindings, ~5 MB runtime. Favored by embedded and security audiences.

**Why rejected:** Younger ecosystem, smaller component catalog, less training data. The user interface surface of kassiber (dashboard, modals, tables, charts, file dialogs) would require hand-building more than in Qt. Slint is a better fit for embedded UIs than document-style accounting dashboards.

### E. Flet (Python + Flutter renderer) — rejected

Python API, Flutter renders the pixels.

**Why rejected:** Hides Flutter as a runtime dep. Hot-reload developer experience is nice but bundle size (~150 MB) and Dart runtime contradict the "minimal deps" goal. Smaller community around Flet specifically than Qt.

### F. Electron / NW.js — rejected by constraint

Bundled Chromium + Node runtime. Rejected up front.

## Decision

**PySide6 + QML.**

The driving factor is: for a solo AI-assisted project, **one language beats two every time**, as long as the single-language option can meet the UX bar. Qt can meet the UX bar for a Clams-inspired accounting UI. It cannot match every web-native aesthetic detail, but the approximation is good enough that the product owner recognizes the look-and-feel target from Phase 2 onward.

## Consequences

**Positive**
- UI and core are in one Python process, one import graph, one test runner, one debugger.
- `kassiber ui` becomes a subcommand of the existing CLI. One binary, two faces.
- No new transport to design (no REST, no gRPC, no JSON-over-stdin). UI calls `core.list_wallets(conn)` and gets a `list[Wallet]` back.
- Deployment is one `briefcase build` per platform.

**Negative**
- Custom theming takes more code than CSS would.
- Any web-centric designer contributing later will need to learn QML.
- Qt's licensing (LGPL) means dynamic linking only. Not a concern for this project; noting for the record.

**Neutral**
- QtCharts is fine for Phase 2. If we ever need interactive zoom/pan with more polish, pyqtgraph is a drop-in alternative.
- If PySide6 ever disappoints, the Phase 0 extraction means the UI layer is replaceable. The core library is untouched.

## Revisit triggers

Re-open this decision only if one of these becomes true:

1. A real mobile target appears (Tauri/Flutter/Slint regain relevance).
2. The product needs a multi-user, remote-access deployment (web frontend starts making sense).
3. QtCharts becomes a blocker for a required chart interaction.
4. A second contributor joins who works much better in a web stack.

Otherwise, stay the course.

## References

- [PySide6 documentation](https://doc.qt.io/qtforpython-6/)
- [briefcase — BeeWare packaging tool](https://briefcase.readthedocs.io/)
- [QtCharts module](https://doc.qt.io/qt-6/qtcharts-index.html)
- [Clams.tech GitHub](https://github.com/clams-tech) — reference for the target aesthetic
