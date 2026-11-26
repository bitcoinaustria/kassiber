# 1:1 parity with the Tauri app

Started 2026-11-15, after the owner asked for the Rune build to be identical to the Tauri app in
look and functionality. Tauri is the source of truth: any visible or behavioural difference is a
bug, unless it is pure rasterisation noise.

> **Historical.** The Tauri app was removed from the tree after commit `1c8065d4b`, so these
> results are the record of the migration rather than a live comparison. The harness still runs
> against a separate checkout of that commit (`PARITY_TAURI_CHECKOUT`, see
> [the harness README](../parity/README.md)). `ui-tauri/` paths below refer to that commit.

## How it is measured

[`parity/`](../parity/README.md) drives both apps into the same state on identical copies of the demo
book. It captures each scene and diffs four things:

- **Pixels:** fuzzed ImageMagick diff, with a heatmap.
- **Text layout:** every visible text is matched between the browser DOM and Rune's element tree,
  and compared for position, ink width, height and colour.
- **Controls:** accessible names present on one side only.
- **Containers:** size and position of large painted boxes.

```bash
PARITY_TAURI_CHECKOUT=<checkout> parity/run.sh <out-dir>   # ONLY=scene,scene; SKIP_TAURI=1 reuses captures
```

## Results

Measured over 55 scenes: shell menus and popovers, Overview, chart, Transactions, the detail
panel (all tabs), the graph, and German. Run 2 is the first measurement, run 3 the merged parity
round.

| | Run 2 | Run 3 |
| --- | --- | --- |
| Median pixel difference | 3.61 % | 2.06 % |
| Median text position error | 17.3 px | 3.9 px |
| Texts present only in Tauri | 1,360 | 553 |
| Controls present only in Tauri | 733 | 123 |

Per area in run 3:

| Area | Text position error (mean) | Status |
| --- | --- | --- |
| Shell and its menus | 2–3 px | Every control within 0.1 px; colours within 1/255 |
| Overview and chart | 2–4 px | 0 missing controls (except the controls sheet) |
| Transactions | 3–5 px | |
| Detail panel | about 18 px | Header and graph-card vertical drift |
| Graph | 5–10 px | Real 3D glass |

Notes on the residuals:
- `settings` stays a placeholder: the full Settings screen is out of scope.
- The 42 texts missing in `chart-expanded` are a tooltip left over in the Tauri capture.
- Flow-chart bars were made pointer-only after run 3, so they are no longer unnamed tab stops.

## Findings

- **The shipped web app does not render in DM Sans.** `ui-tauri/index.html` has an unlayered boot
  rule, `body { font-family: -apple-system, … Arial, sans-serif }`, which beats globals.css's
  `@layer base` DM Sans rule. Every screen therefore draws in the platform UI font: Liberation Sans
  on Linux, SF on macOS, Segoe UI on Windows. Only `<kbd>` uses DM Sans.
  - The Rune build copies the shipped behaviour, with a per-platform stack in `main.rs`.
  - Text widths now match Chromium to a ratio of 1.000.
  - This is probably a web bug worth fixing upstream; if it is fixed, Rune switches back to DM Sans
    in one line.
- **Font smoothing:** Rune `Off` (CSS `-webkit-font-smoothing: antialiased`) is the closest match
  to Chromium.
- **Rune borders don't take layout space** unless `border_layout()` is set. CSS borders do. This
  made every card 2 px short.
- **`El::border_sides` takes `[top, right, bottom, left]`.** Every module had assumed a different
  order; all uses were audited.
- **Popovers must be non-modal,** as Radix makes them. Rune has no outside-press hook, so a click
  on another control activates it but doesn't also close the popover.
- **The graph is now real GPU 3D** through Rune's compositor hook (`src/txdetail/graph/gpu/`):
  - wgpu with 4× MSAA;
  - a MeshPhysicalMaterial-style shader (GGX, clearcoat, sheen, attenuation, background-sampled
    transmission);
  - an analytic room environment fitted to the Tauri capture.

  Sampled colours match within about 5/255. It falls back to the 2.5D drawing if the GPU path
  fails.
- **Table scroll p95 on an idle machine:** 5.2–5.3 ms at first, then 4.7–5.8 ms after slimming
  the rows. The floor is Rune mounting new virtual rows: there is no overscan setting, and
  `.cache()` is bypassed while the list scrolls.

## Not yet 1:1

As of the 2026-11-15 runs above. Several areas listed here have been built since (Settings,
update checks, passphrase and Touch ID unlock, the assistant); re-run the harness for current
numbers before quoting these.


**Whole areas still placeholders or simplified:**
- the Settings screen;
- the Add connection setup forms (the first step is 1:1);
- the AI assistant panel ("Ask about this" opens the side panel only);
- per-book persistence;
- the native menu bar;
- update checks;
- passphrase and Touch ID unlock;
- configured explorer URLs.

**Residual visual differences:**
- the detail-panel vertical drift;
- the merged-dot count halo (Rune has no text stroke);
- partially blurred rich text under the privacy modes (blur applies per element);
- refraction is approximated, and the environment is analytic rather than RoomEnvironment;
- the 12.6 px baseline is 1 px low (Rune's baseline snapping).
