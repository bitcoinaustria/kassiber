# Tauri vs Rune parity harness

This harness measures how far the Rune app is from the React/Tauri desktop. It drives both apps
into the same state on identical copies of the regtest demo book, captures each scene, and
diffs them.

## Run

```bash
parity/run.sh [out-dir]                           # every scene; ONLY=a,b limits it
SKIP_TAURI=1 parity/run.sh <out-dir>              # reuse Tauri captures (re-capture Rune only)
RUNE_ARGS="--font-smoothing off" parity/run.sh    # extra Rune flags for every scene
```

The run writes:
- `report.md`: a summary table, then per scene the worst text deltas, texts and controls found on
  one side only, and container deltas;
- `report.json`: the same data, for scripts;
- `img/<scene>.side.jpg`: Tauri | Rune | heatmap side by side.

## Setup (once)

`$PARITY_HOME` defaults to `~/rune-parity`. It holds the following.

| Item | Role |
| --- | --- |
| `book-tauri/`, `book-rune/` | Two copies of the same pristine demo book, with `config/settings.json` paths rewritten. Each app writes only to its own copy. |
| `env-tauri.sh`, `env-rune.sh` | `KASSIBER_BIN`, `KASSIBER_DEMO_DATA_ROOT` and isolated `KASSIBER_*_FILE` preference paths. The owner's real books are never read. |
| `dist/` | The production web bundle. Build it with `VITE_DAEMON=bridge pnpm exec vite build --outDir $PARITY_HOME/dist` from `ui-tauri/`. |

## Pieces

- **`scenes.json`:** one entry per scene. The `rune` side gives a scene name and args. The
  `tauri` side gives a route, the `kb.ui` state overrides, steps (click or hover by aria-label,
  label prefix, text or CSS; keys with modifiers; waits) and an optional `ready` condition. The
  Rune scenes `route` (`--route`) and `open-tx` (`--tx`, `--tab`) exist for this harness.
  A `scrollTop: <css>` step scrolls the matched element's scrollers back to the top (clicks
  scroll their target into view; Rune's scenes don't).
- **`fixtures.json`:** the demo-book rows scenes use, by name, and named book variants. Write
  `{fx:name}` (the id) or `{fx:name.field}` in Rune args, Tauri paths and steps; both capture
  scripts expand them, and Rust scenes call `crate::fixtures::id("name")`. A scene with
  `"book": "<variant>"` runs on a temporary copy of the book with the variant's local CLI
  steps applied (the Rune scene calls `fixtures::use_book` itself; the Tauri capture restarts
  its preview server on the copy). After replacing the demo book, run
  `python3 -I tools/demo-fixtures.py resolve <copy of kassiber.sqlite3>`: each fixture's
  `select` finds its row by stable properties; `check` also flags stale ids left anywhere in
  scenes, scripts or Rust scene tables.
- **`capture-tauri.mjs`:** builds the screen state and dumps it.
  - Starts `vite preview` with the daemon bridge, then headless Chromium over CDP (stdlib
    WebSocket).
  - Seeds `kb.ui` from a static same-origin file, because the SPA at `/` otherwise persists its
    own defaults over the seed.
  - Waits until no bridge request has been in flight for 700 ms, then runs the steps.
  - Writes a PNG, plus a DOM dump of texts (font, colour, opacity), controls (accessible name,
    state, hidden from assistive technology) and painted containers.
- **`capture-rune.sh`:** runs `--snap` and `--tree` per scene at `--at 0.5`, so
  `--font-smoothing` is in effect.
- **`diff.py`:** compares the two captures.
  - Pixels: ImageMagick fuzzed AE (8 %) and RMSE.
  - Texts: matched by string, using nearest position plus colour for duplicates. Offsets come
    from glyph ink boxes measured in both screenshots: Rune's tree reports layout boxes, which
    can be stretched to their container. Each match reports dx, dy, width ratio, ink height and
    ΔE.
  - Controls: compared by accessible name, with a fuzzy second pass for prefixes and word
    overlap. Controls behind a Rune modal count as present but inert.
  - Containers: matched by overlap.
- **`font_smoothing.py`:** compares glyph coverage and RMSE for each Rune `--font-smoothing`
  mode against Chromium.

## Limitations

- **Pixel percentages understate differences on dark UIs.** A wrong graph rendering is about
  4.5 % of its region. Read the side-by-side images; the numbers rank scenes, they don't grade
  them.
- **Ink measurement can be contaminated.** Text on tinted pills, chart labels over dots, and
  text close to its background colour can produce wild width ratios (the ×4 rows). Filter on
  `basis` and on plausible ratios.
- **Rune's tree has no font size or weight.** Typography is inferred from ink width and height.
- **Recipes reach the web's natural state.** Where Rune opens a state the web cannot reach on
  this book (the explorer dialog, a seeded notification list), the scene is omitted or noted.
- **Headless Chromium is not WebKitGTK,** the engine the shipped Linux app uses. Text
  rasterisation and WebGL output differ slightly from it.
