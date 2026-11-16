# Second pilot: Transactions

> **Historical record.** This is the evaluation that led Kassiber to adopt the Rune app as its
> desktop (November 2026). Its "spike" wording, gaps and figures describe that moment, not the
> current app; see the [crate README](../../README.md) for the app as it is.

Started 2026-11-15. The first spike ([COMPARISON.md](comparison.md)) rebuilt the shell and
Overview. This pilot rebuilds the full Transactions screen with all its functionality, and adds
the checks the first spike left out. Its job is to produce a go/no-go answer on a full migration
from Tauri + React to Rune.

## Scope

- **The Transactions screen** (`ui-tauri/src/routes/Transactions.tsx`,
  `components/transactions/**`):
  - the table workbench: filters, search, sorting, columns, selection, bulk actions and paging;
  - the detail panel with all tabs and editing, attachments, edit history, commercial context;
  - the transaction graph;
  - the explorer and evidence-reuse dialogs;
  - exports (preview and write).
- **Localization:**
  - English and Austrian German for the shell, Overview and Transactions;
  - the same bundles as the web UI (`ui-tauri/src/i18n/locales/{en,de}`), read at build time;
  - one source of truth;
  - a language switch at run time.
- **Accessibility:** every control reports a role and a name; the AccessKit tree is dumped for
  review; keyboard-only operation of the table and the detail panel.
- **Platforms:** Linux locally. macOS and Windows builds need CI runners, which needs a pushed
  branch, so they wait for the owner's go-ahead.

## Go / no-go criteria

Go, if all of these hold at the end of the pilot:

1. **Parity.** Every Transactions feature listed in the spec works against the real daemon.
   Each deliberate omission is named with a reason. Amounts, dates and statuses match the web
   byte for byte on the demo book.
2. **Performance.**
   - The table scrolls 1,000+ rows at under 4 ms per frame (p95).
   - Opening the detail panel, sorting and filtering each finish under 50 ms of UI CPU, not
     counting daemon time.
   - The window stays under 300 MB RSS.
3. **Localization.** Switching to German changes every string on the three screens, with no
   English left except data. A missing key is visible, not silent.
4. **Accessibility.** No unnamed interactive element in the dumped tree. The table, tabs, menus
   and dialogs can be operated with the keyboard alone.
5. **Framework fit.**
   - No fork of Rune is needed. Workarounds stay inside the app.
   - Each Rune gap found has a contained in-app fix, or a written plan.
6. **Effort.** The port's size and time stay in line with the first spike, at roughly 0.7–1.0
   Rust lines per React line.

No-go, if any of these happens:

- a criterion fails without a credible path;
- Rune has to be patched;
- a required screen element (the graph, the editable table) can't be built to an acceptable
  standard.

The outcome and evidence are recorded at the end of this file.

## Result (2026-11-15)

Four parallel workers built the Transactions screen, from three spec passes:
- the table workbench;
- the detail panel;
- the transaction graph;
- localization and accessibility for the existing shell, Overview and chart.

The workers were then merged into one build. Every number below was measured on that merged
build, headless and in release mode, against the regtest demo book: 975 transactions, a
78-input graph. **The machine's load average was 650–700 on 48 cores during every
measurement**, from unrelated jobs, so frame times are pessimistic.

### Criteria

**1. Parity: met, with named omissions.**
- **Workbench:** period chips and the Year menu; six summary tiles; flow chart with
  Amount/Count, External/All and legend filters.
- **Filters and table:** filter menu and chips that map to server-side `ui.transactions.list`
  args; sort cycles; column breakpoints; the "⋯" row menu (details, classify, copy id, loan
  mark/unmark, exclude with confirm); cursor paging over all 975 rows; new-row highlight.
- **Routing and export:** URL state (`tx`, `tab`, `period`, filters, sort); the export dialog
  with format, period, wallet picker, live preview and quarantine block.
- **Detail panel:**
  - docked panel with resize and nav auto-fold, falling back to a sheet below 1023 pt;
  - coin trail;
  - header with timeline and checklist;
  - all five tabs with every edit control, a 15-field draft diff, save/discard, and ⌘S, 1–5, `e`,
    `t`, Esc;
  - attachments (file via native picker, URL, rename, remove, open, reuse from another tx);
  - edit history with revert, journal events, commercial panel, split payout, acquisition review
    (`ui.review.plan` / `apply`), explorer and evidence-reuse dialogs.
- **Graph:**
  - the web's primary 3D glass view as a 2.5D projection: rotation, hover picking, sleeve, hover
    card, click to reveal the row;
  - the 2D bowtie;
  - the inputs/outputs list with trail navigation, swap strip, annotations, warnings with backend
    action, technical details, expanded dialog;
  - "Look up on-chain" consent scoped per transaction, as in the web.
- **Write paths exercised against a demo copy:** metadata update, attachment
  add/rename/remove/copy, history revert, loan mark/unmark, review plan/apply.
- **Named omissions:**
  - "Ask about this": no assistant in the spike.
  - Session-only persistence: no store.
  - Configured explorer URLs: no settings screen.
  - Native Save As after an export.
  - Right-arrow into filter submenus (Enter works).
  - Graph: true glass refraction, clearcoat and per-pixel lighting. The 2.5D view reads as frosted
    acrylic.
  - Graphless buy/sell receipt flow.
- **Not fully verified:** amounts and dates were compared on the captured screens, not by an
  automated byte-for-byte diff against the web.

**2. Performance: met, except table scrolling (inconclusive under this load).**

| Measure | Result | Target |
| --- | --- | --- |
| Sort and filter clicks (worst frame within 250 ms of each input) | 6–21 ms | < 50 ms |
| Detail tab switches | ≤ 7 ms | < 50 ms |
| Opening the panel (first frame with vs without it) | about +20 ms | < 50 ms |
| Graph rotate drag, 78 inputs | p50 2.5–2.9 ms, p95 4.1–5.4 ms | well under 8 ms |
| Table scroll over 975 rows | p50 2.4 ms, p95 5.3–6.8 ms (three runs) | p95 < 4 ms (not met) |
| Peak RSS | 226–246 MB | < 300 MB |
| Stripped binary | 39 MB | — |

The scroll cost is mounting and shaping newly visible rows; the view itself re-runs only 3 times
during the scroll. It needs a re-measurement on an idle machine, plus row memoisation or cached
text if it still misses.

**3. Localization: met.**
- Full evaluation over 74 scenes (every scene except the one that writes), in English and German, after the last fixes:
  - 0 missing keys;
  - 0 unnamed interactive nodes;
  - 0 duplicate keys (rune-kit's path-scoped part keys excepted).
- The app reads the web's own `en` and `de` bundles at build time. A missing key renders as the
  key, and `--i18n-report` lists it.
- English that remains in German is either data or text the web also hard-codes ("Unpriced",
  sync phase labels, the connection catalog).
- One gap is in the German bundle itself: `overview:holdings.lastSynced`.
- German text expansion caused one truncation in the gear menu (fixed). One remains: a status
  chip in a recent row.

**4. Accessibility: met in the tree. Not tested with a real screen reader.**
- 0 unnamed interactive nodes across every scene in both languages.
- Keyboard-only scripts work: palette, menus, book switcher, chart controls, table grid, row
  menu, filters, detail tabs and selects.
- Two Rune-specific issues, both fixed in the app:
  - Enter only presses default buttons unless `cx.web_focus()` is set, because Rune follows
    AppKit.
  - Menus focus their surface unless an item has autofocus.
- Still to do: a pass with VoiceOver, Narrator and Orca on real platforms.

**5. Framework fit: met, no Rune patch or fork.** Gaps were handled in the app:
- **Shape fills** are ear-clipped every frame (O(n²)), so the graph halves polygon vertices and
  the chart splits areas into quads. Plan: an app-side tessellation cache, or ask upstream.
- **Shapes** have no gradients, dashes, arcs, clips or patterns; these are hand-built.
- **No CSS grid, sheet or page dialog:** hand-built.
- **Virtual-row builders** get no `Cx` and no overlay layer, so row menus are anchored popovers.
- **View code is `&App`,** so caches use `thread_local`.
- **Headless scripts** fail silently on a missing key, or on a tap outside the viewport.
- **Request identity:** `Reply` carries no request id or args, so identity rides on request tags.

**6. Effort: met.**
- Rust: 12,050 lines. Of these, 4,165 are the table, 4,705 the detail panel and 3,182 the graph.
- React and three.js replaced: about 20,400 lines (19,339 in `components/transactions`, 890 in
  `kb/glass3d`, 178 for the route). The ratio is 0.59.
- Time: three spec agents plus four build agents, about 45–50 min each, then integration.
- The whole spike is now 22.9k lines of Rust.

**Platforms: open.** macOS and Windows builds need CI runners, so they wait for the owner's
go-ahead to push a branch.

### Outcome

The pilot found no blocker. Every criterion is met, except three that remain open:

- table scroll p95, measured under a 690 load;
- macOS/Windows builds;
- a real screen-reader pass.

The decision is therefore a **conditional go**, pending those three checks. The governance risk
from COMPARISON.md is unchanged by this pilot:
- Rune is pre-release;
- it has a single vendor;
- it accepts no outside pull requests.

That remains the owner's call.
