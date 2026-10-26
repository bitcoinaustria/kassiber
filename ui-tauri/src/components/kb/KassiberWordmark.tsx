import * as React from "react";

import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";

import {
  DECRYPT_MS,
  WORDMARK_COLUMNS,
  WORDMARK_K_COLUMNS,
  WORDMARK_PATH,
  WORDMARK_ROWS,
  decryptFrame,
} from "./kassiberWordmarkGrid";

/** Bitcoin orange, as in the app icon's wedge, for cells still being decoded. */
const NOISE_FILL = "#f7931a";

/**
 * The Kassiber wordmark in `currentColor`. `cell` is the CSS-pixel size of one
 * grid cell; keep it a whole number so every cell lands on device pixels.
 *
 * `decrypt` plays the decode reveal on mount and each time it turns true
 * again, unless the OS asks for reduced motion. `folded` narrows the mark to
 * its K, wiping the other letters off to the right.
 */
export function KassiberWordmark({
  className,
  cell = 1,
  decrypt = false,
  folded = false,
}: {
  className?: string;
  cell?: number;
  decrypt?: boolean;
  folded?: boolean;
}) {
  const reducedMotion = usePrefersReducedMotion();
  const solidRef = React.useRef<SVGPathElement>(null);
  const noiseRef = React.useRef<SVGPathElement>(null);

  // A layout effect, so the first painted frame is already scrambled rather
  // than a flash of the finished word. Frames write the path data directly;
  // nothing re-renders while it plays.
  React.useLayoutEffect(() => {
    const solid = solidRef.current;
    const noise = noiseRef.current;
    if (!decrypt || reducedMotion || !solid || !noise) return;
    if (document.visibilityState === "hidden") return;
    const draw = (frame: { solid: string; noise: string }) => {
      solid.setAttribute("d", frame.solid);
      noise.setAttribute("d", frame.noise);
    };
    const settle = () => draw({ solid: WORDMARK_PATH, noise: "" });
    const start = performance.now();
    let request = 0;
    const step = (now: number) => {
      const elapsed = now - start;
      if (elapsed >= DECRYPT_MS) {
        settle();
        return;
      }
      draw(decryptFrame(elapsed));
      request = requestAnimationFrame(step);
    };
    draw(decryptFrame(0));
    request = requestAnimationFrame(step);
    return () => {
      cancelAnimationFrame(request);
      settle();
    };
  }, [decrypt, reducedMotion]);

  return (
    <span
      className={cn(
        "inline-flex shrink-0 overflow-hidden transition-[width] duration-200 ease-linear",
        className,
      )}
      // Folded, the clip falls mid-gap after the K, so no edge of the A shows.
      // Open, a pixel of slack keeps the clip off the R however the box rounds.
      style={{
        width: folded
          ? (WORDMARK_K_COLUMNS + 0.5) * cell
          : WORDMARK_COLUMNS * cell + 1,
      }}
    >
      {/* rem-based layout can still place the mark on a fractional pixel, so
          the SVG does not clip: crispEdges snaps the cells, and an unsnapped
          viewport edge would shave the last column to a grey sliver. */}
      <svg
        aria-hidden="true"
        viewBox={`0 0 ${WORDMARK_COLUMNS} ${WORDMARK_ROWS}`}
        width={WORDMARK_COLUMNS * cell}
        height={WORDMARK_ROWS * cell}
        shapeRendering="crispEdges"
        className="shrink-0 overflow-visible fill-current"
      >
        <path ref={solidRef} d={WORDMARK_PATH} />
        <path ref={noiseRef} fill={NOISE_FILL} />
      </svg>
    </span>
  );
}
