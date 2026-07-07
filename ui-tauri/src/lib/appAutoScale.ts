/**
 * Automatic screen-fit scale.
 *
 * The whole UI is sized in `rem`, off a single root font-size:
 *
 *   html { font-size: calc(16px * var(--app-auto-scale) * var(--app-ui-scale)); }
 *
 * `--app-ui-scale` is the user's manual nudge (Settings → Appearance, and the
 * Cmd/Ctrl +/−/0 hotkeys); default 0.9. `--app-auto-scale` — computed here —
 * derives a base density from the actual window size so the app keeps a
 * consistent *information density* (roughly the same rows/columns of content)
 * across a 14" laptop and a 27" external monitor, rather than laying out a
 * fixed number of logical pixels that eats a big fraction of a small screen.
 *
 * The webview only ever sees logical CSS points, not physical inches (macOS
 * abstracts DPI away — `devicePixelRatio` is just the Retina backing scale), so
 * the honest signal for "how much room do we have" is the logical viewport
 * width/height. We take the *more constrained* of the two ratios against a
 * reference viewport: a window has to be large in both dimensions to earn the
 * full (1.0) factor, so a wide-but-short window densifies instead of ballooning.
 *
 * Product call ("shrink laptops only"): the factor is capped at 1.0, which maps
 * to today's feel (16 × 1.0 × 0.9 = 14.4px root) — large monitors are unchanged
 * — and only dips toward the floor on smaller laptop viewports.
 */

/**
 * Reference viewport (logical CSS px) at which the factor reaches its 1.0 cap.
 * Sized so a 16" MacBook and any external monitor sit at the cap (unchanged
 * from today), while 14"/13" laptop viewports fall below it. Width is the
 * dominant signal for typical (reasonably tall) windows; height only binds when
 * a window is deliberately short, in which case densifying is the right call.
 */
export const AUTO_SCALE_REF_WIDTH = 1680;
export const AUTO_SCALE_REF_HEIGHT = 1000;

/** Never larger than today's baseline (large screens stay put). */
export const MAX_AUTO_SCALE = 1;
/** Floor so the smallest supported window stays legible (≈11.5px root). */
export const MIN_AUTO_SCALE = 0.8;
/**
 * Quantize the factor so incidental resize pixels during a drag don't churn the
 * root font-size (each change reflows the entire page). 0.02 ≈ a 0.3px step at
 * the 16px base — imperceptible, but coarse enough to coalesce a drag.
 */
export const AUTO_SCALE_STEP = 0.02;

/**
 * Derive the automatic screen-fit factor for a viewport of `width` × `height`
 * logical pixels. Pure and deterministic so it can be unit-tested across the
 * device matrix and reused for the pre-paint set. Guards non-finite / zero
 * inputs (e.g. a hidden/zero-size window) by falling back to the 1.0 cap.
 */
export function computeAutoScale(width: number, height: number): number {
  if (
    typeof width !== "number" ||
    typeof height !== "number" ||
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return MAX_AUTO_SCALE;
  }
  const raw = Math.min(
    width / AUTO_SCALE_REF_WIDTH,
    height / AUTO_SCALE_REF_HEIGHT,
  );
  const stepped = Math.round(raw / AUTO_SCALE_STEP) * AUTO_SCALE_STEP;
  const clamped = Math.min(MAX_AUTO_SCALE, Math.max(MIN_AUTO_SCALE, stepped));
  return Number(clamped.toFixed(3));
}
