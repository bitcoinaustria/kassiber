import { describe, expect, it } from "vitest";

import {
  AUTO_SCALE_STEP,
  MAX_AUTO_SCALE,
  MIN_AUTO_SCALE,
  computeAutoScale,
} from "./appAutoScale";

const DEFAULT_MANUAL_SCALE = 0.9; // DEFAULT_APP_SCALE — kept in sync by intent.
const rootPx = (auto: number) => 16 * auto * DEFAULT_MANUAL_SCALE;

describe("computeAutoScale", () => {
  it("caps large monitors at today's baseline (no growth)", () => {
    // Default window, a docked 27" (2560×1440), and an ultrawide all clamp to
    // MAX so their root font-size stays the historical 14.4px.
    for (const [w, h] of [
      [1760, 1160],
      [2560, 1440],
      [3440, 1440],
    ] as const) {
      expect(computeAutoScale(w, h)).toBe(MAX_AUTO_SCALE);
      expect(rootPx(computeAutoScale(w, h))).toBeCloseTo(14.4, 5);
    }
  });

  it("keeps a 16\" MacBook viewport at the cap", () => {
    // 1728×1117 screen minus title bar → ~1728×1060 inner: still ≥ reference.
    expect(computeAutoScale(1728, 1060)).toBe(MAX_AUTO_SCALE);
  });

  it("shrinks a 14\" MacBook viewport below the baseline", () => {
    // 1512×982 screen minus menu/title chrome → ~1512×930 inner.
    const scale = computeAutoScale(1512, 930);
    expect(scale).toBeLessThan(MAX_AUTO_SCALE);
    expect(scale).toBeGreaterThan(MIN_AUTO_SCALE);
    // Meaningfully tighter than today, but not tiny (~12–13px root).
    expect(rootPx(scale)).toBeGreaterThan(12);
    expect(rootPx(scale)).toBeLessThan(13.5);
  });

  it("shrinks a 13\" MacBook viewport further", () => {
    const fourteen = computeAutoScale(1512, 930);
    const thirteen = computeAutoScale(1470, 905);
    expect(thirteen).toBeLessThanOrEqual(fourteen);
    expect(thirteen).toBeGreaterThanOrEqual(MIN_AUTO_SCALE);
  });

  it("clamps the smallest supported window to the floor", () => {
    // tauri.conf minWidth/minHeight is 980×700.
    expect(computeAutoScale(980, 700)).toBe(MIN_AUTO_SCALE);
  });

  it("densifies a wide-but-short window (height binds)", () => {
    // A big-monitor window dragged short should shrink, not stay large.
    expect(computeAutoScale(1760, 640)).toBe(MIN_AUTO_SCALE);
  });

  it("is monotonic: smaller viewport never yields a larger scale", () => {
    const sizes: Array<[number, number]> = [
      [980, 700],
      [1280, 800],
      [1470, 905],
      [1512, 930],
      [1680, 1000],
      [1920, 1200],
      [2560, 1440],
    ];
    for (let i = 1; i < sizes.length; i += 1) {
      const smaller = computeAutoScale(sizes[i - 1][0], sizes[i - 1][1]);
      const larger = computeAutoScale(sizes[i][0], sizes[i][1]);
      expect(smaller).toBeLessThanOrEqual(larger);
    }
  });

  it("quantizes to the step so drag-resizes coalesce", () => {
    const scale = computeAutoScale(1500, 900);
    const remainder = Math.abs(scale / AUTO_SCALE_STEP - Math.round(scale / AUTO_SCALE_STEP));
    expect(remainder).toBeLessThan(1e-9);
  });

  it("falls back to the cap for degenerate viewports", () => {
    expect(computeAutoScale(0, 0)).toBe(MAX_AUTO_SCALE);
    expect(computeAutoScale(-1, 800)).toBe(MAX_AUTO_SCALE);
    expect(computeAutoScale(Number.NaN, 800)).toBe(MAX_AUTO_SCALE);
    expect(computeAutoScale(1500, Number.POSITIVE_INFINITY)).toBe(
      MAX_AUTO_SCALE,
    );
  });
});
