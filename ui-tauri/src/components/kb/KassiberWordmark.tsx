import * as React from "react";

import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";

import {
  REVEAL_MS,
  WORDMARK_COLUMNS,
  WORDMARK_K_COLUMNS,
  WORDMARK_K_PATH,
  WORDMARK_K_ROWS,
  WORDMARK_PATH,
  WORDMARK_ROWS,
  revealFrame,
  workingFrame,
  type WordmarkFrame,
  type WordmarkReveal,
} from "./kassiberWordmarkGrid";

/** Bitcoin orange, as in the app icon's wedge, for cells still in motion. */
const NOISE_FILL = "#f7931a";

const WORDMARK_REST: WordmarkFrame = { solid: WORDMARK_PATH, noise: "" };
const K_REST: WordmarkFrame = { solid: WORDMARK_K_PATH, noise: "" };

/**
 * Plays `frameAt` into a pair of paths while `active`, writing path data
 * directly so nothing re-renders. `frameAt` returns null once it is done.
 * A layout effect, so the first painted frame is already the animation's
 * rather than a flash of the resting mark. Reduced motion keeps the rest
 * frame; a one-shot that would start in a hidden window is skipped.
 */
function useAnimatedPaths({
  active,
  frameAt,
  rest,
  oneShot,
  onEnd,
}: {
  active: boolean;
  frameAt: (elapsed: number) => WordmarkFrame | null;
  rest: WordmarkFrame;
  oneShot: boolean;
  onEnd?: () => void;
}) {
  const reducedMotion = usePrefersReducedMotion();
  const solidRef = React.useRef<SVGPathElement>(null);
  const noiseRef = React.useRef<SVGPathElement>(null);
  const finishedRef = React.useRef(false);
  // Read through refs so a caller's new callbacks never restart a play.
  const frameAtRef = React.useRef(frameAt);
  const onEndRef = React.useRef(onEnd);
  React.useLayoutEffect(() => {
    frameAtRef.current = frameAt;
    onEndRef.current = onEnd;
  });

  React.useLayoutEffect(() => {
    const solid = solidRef.current;
    const noise = noiseRef.current;
    if (!active) {
      finishedRef.current = false;
      return;
    }
    if (!solid || !noise || (oneShot && finishedRef.current)) return;
    const finish = () => {
      finishedRef.current = true;
      onEndRef.current?.();
    };
    if (reducedMotion || (oneShot && document.visibilityState === "hidden")) {
      if (oneShot) finish();
      return;
    }
    const draw = (frame: WordmarkFrame) => {
      solid.setAttribute("d", frame.solid);
      noise.setAttribute("d", frame.noise);
    };
    // Fixed for the whole play, so a caller switching reveals mid-play only
    // changes the next one.
    const frameAt = frameAtRef.current;
    let start = performance.now();
    let elapsed = 0;
    let done = false;
    let request = 0;
    const step = (now: number) => {
      elapsed = now - start;
      const frame = frameAt(elapsed);
      if (!frame) {
        done = true;
        draw(rest);
        finish();
        return;
      }
      draw(frame);
      request = requestAnimationFrame(step);
    };
    const visibilityChanged = () => {
      if (done) return;
      cancelAnimationFrame(request);
      if (document.visibilityState === "hidden") {
        if (oneShot) {
          done = true;
          draw(rest);
          finish();
        }
      } else {
        start = performance.now() - elapsed;
        step(performance.now());
      }
    };
    document.addEventListener("visibilitychange", visibilityChanged);
    if (document.visibilityState !== "hidden") step(start);
    return () => {
      cancelAnimationFrame(request);
      document.removeEventListener("visibilitychange", visibilityChanged);
      draw(rest);
    };
  }, [active, reducedMotion, rest, oneShot]);

  return { solidRef, noiseRef };
}

/**
 * The Kassiber wordmark in `currentColor`. `cell` is the CSS-pixel size of one
 * grid cell; keep it a whole number so every cell lands on device pixels.
 *
 * With `animate`, the word arrives by `reveal` on mount and each time
 * `animate` turns true again; `onRevealEnd` fires once it has. A `reveal`
 * that changes mid-play takes effect from the next play. `folded` narrows the
 * mark to its K, wiping the other letters off to the right.
 */
export function KassiberWordmark({
  className,
  cell = 1,
  animate = false,
  reveal = "decrypt",
  onRevealEnd,
  folded = false,
}: {
  className?: string;
  cell?: number;
  animate?: boolean;
  reveal?: WordmarkReveal;
  onRevealEnd?: () => void;
  folded?: boolean;
}) {
  const { solidRef, noiseRef } = useAnimatedPaths({
    active: animate,
    frameAt: (elapsed) =>
      elapsed < REVEAL_MS[reveal] ? revealFrame(reveal, elapsed) : null,
    rest: WORDMARK_REST,
    oneShot: true,
    onEnd: onRevealEnd,
  });

  return (
    <span
      role="img"
      aria-label="Kassiber"
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
          viewport edge would shave the last column to a grey sliver. Rain
          enters from above the box, so the span's clip is what trims it. */}
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

/**
 * The wordmark's K as a busy indicator: it decrypts in, holds, scrambles back
 * out and repeats for as long as it is mounted. It says only that work is
 * under way; pair it with whatever reports real progress. Reduced motion
 * shows the K at rest.
 */
export function KassiberWorkingMark({
  className,
  cell = 2,
}: {
  className?: string;
  cell?: number;
}) {
  const { solidRef, noiseRef } = useAnimatedPaths({
    active: true,
    frameAt: workingFrame,
    rest: K_REST,
    oneShot: false,
  });

  return (
    <svg
      aria-hidden="true"
      viewBox={`0 0 ${WORDMARK_K_COLUMNS} ${WORDMARK_K_ROWS}`}
      width={WORDMARK_K_COLUMNS * cell}
      height={WORDMARK_K_ROWS * cell}
      shapeRendering="crispEdges"
      className={cn("shrink-0 overflow-visible fill-current", className)}
    >
      <path ref={solidRef} d={WORDMARK_K_PATH} />
      <path ref={noiseRef} fill={NOISE_FILL} />
    </svg>
  );
}
