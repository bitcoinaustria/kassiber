import { describe, expect, it } from "vitest";

import { niceCeil, zeroBasedAxis } from "./motion";

describe("axis bounds", () => {
  it("rounds up to a clean number", () => {
    expect(niceCeil(299.17)).toBe(300);
    expect(niceCeil(1.37)).toBe(1.5);
    expect(niceCeil(0.0042)).toBeCloseTo(0.005);
    expect(niceCeil(0)).toBe(0);
  });

  it("puts a tick on zero and keeps zero inside the domain", () => {
    const losses = zeroBasedAxis([-299.17, -266.52]);
    expect(losses.ticks).toContain(0);
    expect(losses.domain[1]).toBe(0);
    expect(losses.domain[0]).toBeLessThanOrEqual(-299.17);
    const flows = zeroBasedAxis([0.105, -0.0355, 0.01]);
    expect(flows.ticks).toContain(0);
    expect(flows.ticks.every((tick) => Number.isFinite(tick))).toBe(true);
    expect(flows.domain[0]).toBeLessThanOrEqual(-0.0355);
    expect(flows.domain[1]).toBeGreaterThanOrEqual(0.105);
  });
});
