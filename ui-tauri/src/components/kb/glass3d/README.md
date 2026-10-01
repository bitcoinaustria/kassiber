# Shared glass scenes

Runtime imports from this directory belong only in lazily loaded scene modules.
Do not re-export them from an eagerly imported UI barrel. Types are safe to import
with `import type`. All geometry and environment lighting are generated locally.

- `materials.ts`: `glass(tone, thickness?)`, `satin(color)`, `LIGHT_TONES`,
  `DARK_TONES`, and `GlassTone`. These preserve the transaction graph artwork.
- `stage.ts`: `createGlassStage(canvas, look, populate, framing?)`. The populate
  callback receives a content group and `own(material)` for registering materials,
  including unused materials that must be released on failure.
- Every view draws through one shared `WebGLRenderer`. It owns the room
  environment, one scene and camera the views borrow per draw (so three's
  transmission buffer exists once), and never-drawn keeper materials that keep
  the glass, satin and glow shaders compiled between scenes. Each view copies
  the drawing onto its own `canvas`, which is a plain 2D canvas. The renderer is
  released 30 s after its last view. `warmGlassRenderer()` prepares it ahead of
  a view, e.g. while the data a view will draw is loading.
- `webgl.ts`: `webglAvailable()`, probed once per page.
- The returned `GlassScene` exposes `resize(width, height)`,
  `setView(yaw, pitch)`, `prepare()`, `render()`, `pick(x, y)`,
  `highlight(part)`, `onContextLost(listener)` and `dispose()`. Await prepare
  (it compiles shaders without blocking the page), call resize and setView,
  then render. Drawing is explicit; no animation loop runs.
- A scene with parts returns `{ pick, highlight }` from populate. `pick`
  receives a `GlassPointer` (a camera ray, the pointer in CSS pixels,
  `toScreen` and `pixelsPerUnit`) and names the part there or returns null.
  `Glass3DView` picks at most once a frame, reports hover and clicks, and
  lights whatever part its caller passes as `highlightedPart`.
- The stage centres the content and uses an orthographic frame that only grows
  when rotated. Defaults retain the graph's minimum half-frame of 4.4 × 2.5.
  Disposal releases content mesh geometry and owned/attached materials and
  gives the view's hold on the shared renderer back. Dispose is idempotent.

The wallet prototype lives in `../wallets/utxo3d/`. Its layout accepts the existing
inventory row projection (outpoint, asset, amount_msat, confirmation_status), with
null also accepted for unknown amounts. Only safe, nonnegative integer msat are
scaled. Invalid or unknown amounts get a neutral height and frosted material.
Mixed-asset inventories must be separated before layout.

Each coin has a fixed 0.8 × 0.6 footprint. Height is 2.8 times the ratio to the
largest known value, with a 0.12 dust floor. Heights are relative within this
inventory, not comparable between wallets. Coins are sorted by descending value
then outpoint; unknown amounts follow known amounts. At most 200 coins are drawn,
plus a fixed-height ellipsis block carrying the omitted count. That count covers
only the supplied list, not any upstream pagination.

Hidden mode uses height 1.2 for every block and outpoint ordering. It never reads
amounts, and frosting then reflects confirmation state alone. The output carries
no raw amounts. The grid uses separate rows in the xy plane to keep small coins
visible. The prototype has no product-screen integration, labels, or interaction.
