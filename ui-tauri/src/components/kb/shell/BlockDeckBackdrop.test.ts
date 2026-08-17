import { describe, expect, it } from "vitest";

import {
  BLOCK_DECK,
  DECK_GEOMETRY,
} from "@/components/kb/shell/BlockDeckBackdrop";

const { COLS, ROWS, CELL, PAD, BLOCK, BLOCK_GAP, BLOCKS } = DECK_GEOMETRY;

/**
 * The packer is the only logic in the art, and its one job is exact cover: a
 * block that leaves cells uncovered has holes in it, and a block that covers one
 * twice has squares overlapping their neighbours — the failure the marketing
 * site's version hit when highlighted squares were grown in place after packing.
 */
describe("block deck packing", () => {
  it("covers every cell of every block exactly once", () => {
    const covered = Array.from({ length: BLOCKS }, () =>
      Array.from({ length: ROWS }, () => Array<number>(COLS).fill(0)),
    );

    for (const q of BLOCK_DECK) {
      const block = Math.floor(q.x / (BLOCK + BLOCK_GAP));
      const col = ((q.x - PAD) % (BLOCK + BLOCK_GAP)) / CELL;
      const row = (q.y - PAD) / CELL;
      // Whole-cell alignment: a fractional index means the square is off-grid.
      expect(Number.isInteger(col)).toBe(true);
      expect(Number.isInteger(row)).toBe(true);
      expect(q.s).toBe(q.cells * CELL - PAD * 2);
      for (let y = row; y < row + q.cells; y++)
        for (let x = col; x < col + q.cells; x++) covered[block][y][x] += 1;
    }

    const counts = new Set(covered.flat(2));
    expect(counts).toEqual(new Set([1]));
  });

  it("stays inside its blocks, so nothing bleeds into the gap", () => {
    for (const q of BLOCK_DECK) {
      const left = q.x % (BLOCK + BLOCK_GAP);
      expect(left).toBeGreaterThanOrEqual(PAD);
      expect(left + q.s).toBeLessThanOrEqual(BLOCK - PAD);
      expect(q.y + q.s).toBeLessThanOrEqual(BLOCK - PAD);
    }
  });
});
