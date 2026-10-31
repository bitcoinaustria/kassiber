import * as React from "react";

import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";

const COUNT_UP_MS = 900;

function easeOutCubic(t: number) {
  return 1 - (1 - t) ** 3;
}

/**
 * Animate from zero to `target` once, on mount. Static renders and reduced
 * motion show the final value straight away, so nothing ever reads a number
 * that is still moving except the animation itself.
 */
export function useCountUp(target: number | null, delayMs = 0): number | null {
  const reduced = usePrefersReducedMotion();
  const [value, setValue] = React.useState<number | null>(() =>
    target === null || typeof window === "undefined" ? target : 0,
  );
  React.useEffect(() => {
    if (target === null || reduced || typeof window === "undefined") {
      setValue(target);
      return undefined;
    }
    let frame = 0;
    let start: number | null = null;
    const tick = (now: number) => {
      if (start === null) start = now + delayMs;
      const elapsed = Math.max(0, now - start);
      const progress = Math.min(1, elapsed / COUNT_UP_MS);
      setValue(target * easeOutCubic(progress));
      if (progress < 1) frame = window.requestAnimationFrame(tick);
    };
    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [target, reduced, delayMs]);
  return value;
}

/** Stagger for the n-th element of a list, capped so long lists finish quickly. */
export function stagger(index: number, step = 45, max = 600) {
  return Math.min(index * step, max);
}

/**
 * Axis for bars that grow from zero: round ticks on a common step that always
 * include zero. A domain that left zero out (only losses, say) would hang the
 * bars from the top of the chart, and ticks off the zero line would label the
 * baseline with a value it does not have.
 */
export function zeroBasedAxis(values: number[]): { domain: [number, number]; ticks: number[] } {
  const low = Math.min(0, ...values);
  const high = Math.max(0, ...values);
  const span = high - low;
  if (!Number.isFinite(span) || span <= 0) return { domain: [0, 1], ticks: [0, 1] };
  const step = niceCeil(span / 4);
  const first = Math.floor(low / step) * step;
  const last = Math.ceil(high / step) * step;
  const ticks: number[] = [];
  for (let tick = first; tick <= last + step / 2; tick += step) {
    ticks.push(Math.abs(tick) < step / 1e6 ? 0 : Number(tick.toPrecision(12)));
  }
  return { domain: [first, last], ticks };
}

/** The next round number above `value` (300 for 299, 1.5 for 1.37), so ticks read cleanly. */
export function niceCeil(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  const power = 10 ** Math.floor(Math.log10(value));
  const step = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10].find((candidate) => value <= candidate * power) ?? 10;
  return step * power;
}
