import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import {
  REVEAL_MS,
  WORDMARK_COLUMNS,
  WORDMARK_GRID,
  WORDMARK_K_COLUMNS,
  WORDMARK_K_PATH,
  WORDMARK_K_ROWS,
  WORDMARK_PATH,
  WORKING_CYCLE_MS,
  decryptFrame,
  revealFrame,
  workingFrame,
  type WordmarkReveal,
} from "./kassiberWordmarkGrid";

const REVEALS: WordmarkReveal[] = ["decrypt", "rain", "assemble"];

describe("kassiber wordmark grid", () => {
  it("keeps every row the same width, drawn in cells and gaps only", () => {
    for (const row of WORDMARK_GRID) {
      expect(row).toHaveLength(WORDMARK_COLUMNS);
      expect(row).toMatch(/^[#.]+$/);
    }
  });

  it("leaves the column after the K empty, so folding to the K clips no letter", () => {
    for (const row of WORDMARK_GRID) {
      expect(row[WORDMARK_K_COLUMNS]).toBe(".");
    }
  });

  it("keeps the K within its own box", () => {
    WORDMARK_GRID.slice(WORDMARK_K_ROWS).forEach((row) => {
      expect(row.slice(0, WORDMARK_K_COLUMNS)).not.toContain("#");
    });
    expect(WORDMARK_K_PATH).not.toBe("");
  });

  it.each(REVEALS)("starts %s with no settled cells and ends on the plain wordmark", (reveal) => {
    const first = revealFrame(reveal, 0);
    expect(first.solid).toBe("");

    const last = revealFrame(reveal, REVEAL_MS[reveal]);
    expect(last).toEqual({ solid: WORDMARK_PATH, noise: "" });
  });

  it("shows decrypt's noise from the first frame", () => {
    expect(decryptFrame(0).noise).not.toBe("");
  });

  it.each(["on-light", "on-dark"])("keeps the README's %s wordmark drawn from this grid", (variant) => {
    const svg = readFileSync(
      new URL(`../../../../docs/assets/kassiber-wordmark-${variant}.svg`, import.meta.url),
      "utf8",
    );
    expect(svg).toContain(`d="${WORDMARK_PATH}"`);
  });

  it("loops the working K between scrambled and whole", () => {
    expect(workingFrame(0).solid).toBe("");
    expect(workingFrame(REVEAL_MS.decrypt)).toEqual({
      solid: WORDMARK_K_PATH,
      noise: "",
    });
    expect(workingFrame(WORKING_CYCLE_MS)).toEqual(workingFrame(0));
  });

  it("keeps the working K's noise inside its own height", () => {
    for (let elapsed = 0; elapsed < WORKING_CYCLE_MS; elapsed += 70) {
      const frame = workingFrame(elapsed);
      for (const path of [frame.solid, frame.noise]) {
        for (const match of path.matchAll(/M\d+ (\d+)h/g)) {
          expect(Number(match[1])).toBeLessThan(WORDMARK_K_ROWS);
        }
      }
    }
  });
});
