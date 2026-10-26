import { describe, expect, it } from "vitest";

import {
  BLOCK_LAYERS,
  BLOCK_SIZE,
  PENDING_COUNT,
  blockCentre,
  chainBlocks,
  chainLayout,
  type ChainBox,
} from "./chainLayout";

const transactions = (boxes: ChainBox[]) =>
  boxes.filter((box) => box.kind === "tx" || box.kind === "fee");

const nextBlock = (boxes: ChainBox[]) =>
  transactions(boxes).filter(
    (box) => Math.abs(box.x) < BLOCK_SIZE / 2 && Math.abs(box.z) < BLOCK_SIZE / 2,
  );

describe("setup chain artwork", () => {
  it("draws the same artwork for the same fill", () => {
    expect(chainLayout({ mined: 3, fill: 0.6 })).toEqual(
      chainLayout({ mined: 3, fill: 0.6 }),
    );
  });

  it("keeps a block's transactions as the chain advances past it", () => {
    const before = chainBlocks({ tip: 7, mined: 3, layers: 4 });
    const after = chainBlocks({ tip: 8, mined: 3, layers: 0 });
    const found = before.find((block) => block.id === 7);
    const mined = after.find((block) => block.id === 7);
    expect(found?.offset).toBe(0);
    expect(mined?.offset).toBe(1);
    expect(mined?.boxes).toEqual(found?.boxes);
    // Every block moves back one slot; a new, empty block takes the front.
    expect(after.map((block) => [block.id, block.offset])).toEqual([
      [5, 3],
      [6, 2],
      [7, 1],
      [8, 0],
    ]);
    expect(after.at(-1)?.boxes.filter((box) => box.kind !== "shell")).toEqual([]);
  });

  it("keeps the block leaving the chain until it has faded", () => {
    const blocks = chainBlocks({ tip: 8, mined: 3, layers: 0, trailing: true });
    expect(blocks[0]).toMatchObject({ id: 4, offset: 4 });
  });

  it("draws different transactions in different blocks", () => {
    const [older, newer] = chainBlocks({ tip: 2, mined: 1, layers: 4 });
    expect(older.boxes).not.toEqual(newer.boxes);
  });

  it("gives only the next block an outline", () => {
    const shells = chainLayout({ mined: 3, fill: 0.6 }).boxes.filter(
      (box) => box.kind === "shell",
    );
    expect(shells).toEqual([
      expect.objectContaining({ x: 0, width: BLOCK_SIZE }),
    ]);
  });

  it("fills the next block layer by layer, keeping the layers below", () => {
    const empty = chainLayout({ mined: 0, fill: 0 });
    expect(empty.nextLayers).toBe(0);
    expect(transactions(empty.boxes)).toEqual([]);

    let previous: ChainBox[] = [];
    for (let layers = 1; layers <= BLOCK_LAYERS; layers += 1) {
      const layout = chainLayout({ mined: 0, fill: layers / BLOCK_LAYERS });
      expect(layout.nextLayers).toBe(layers);
      const current = nextBlock(layout.boxes);
      // A step adds transactions on top; it never reshuffles the ones below.
      expect(current.slice(0, previous.length)).toEqual(previous);
      expect(current.length).toBeGreaterThan(previous.length);
      previous = current;
    }
  });

  it("shows the mempool waiting above the next block until it is full", () => {
    const pending = (fill: number) =>
      chainLayout({ mined: 0, fill }).boxes.filter((box) => box.kind === "pending");
    expect(pending(0.5)).toHaveLength(PENDING_COUNT);
    for (const box of pending(0.5)) {
      expect(box.y - box.height / 2).toBeGreaterThan(BLOCK_SIZE / 2);
      expect(Math.abs(box.x) + box.width / 2).toBeLessThanOrEqual(BLOCK_SIZE / 2);
      expect(Math.abs(box.z) + box.depth / 2).toBeLessThanOrEqual(BLOCK_SIZE / 2);
    }
    expect(pending(1)).toEqual([]);
  });

  it("clamps the fill to whole layers of one block", () => {
    expect(chainLayout({ mined: 0, fill: 4 }).nextLayers).toBe(BLOCK_LAYERS);
    expect(chainLayout({ mined: 0, fill: -1 }).nextLayers).toBe(0);
  });

  it("keeps every transaction inside one block's outline", () => {
    const half = BLOCK_SIZE / 2;
    const centres = [0, 1, 2, 3].map(blockCentre);
    for (const box of transactions(chainLayout({ mined: 3, fill: 1 }).boxes)) {
      const inside = centres.filter(
        (centre) =>
          box.x - box.width / 2 >= centre.x - half &&
          box.x + box.width / 2 <= centre.x + half &&
          box.z - box.depth / 2 >= centre.z - half &&
          box.z + box.depth / 2 <= centre.z + half,
      );
      expect(inside).toHaveLength(1);
      expect(box.y - box.height / 2).toBeGreaterThanOrEqual(-half);
      expect(box.y + box.height / 2).toBeLessThanOrEqual(half);
    }
  });

  it("packs mined blocks solid and mixes in high-fee transactions", () => {
    const boxes = transactions(chainLayout({ mined: 3, fill: 0 }).boxes);
    expect(boxes.length).toBeGreaterThan(3 * BLOCK_LAYERS * 4);
    expect(boxes.some((box) => box.kind === "fee")).toBe(true);
    expect(boxes.some((box) => box.kind === "tx")).toBe(true);
  });
});
