import { describe, expect, it } from "vitest";

import {
  BLOCK_DEPTH, BLOCK_WIDTH, MAX_BLOCK_HEIGHT, MAX_UTXO_BLOCKS,
  MIN_BLOCK_HEIGHT, UNIFORM_BLOCK_HEIGHT, utxoBlocksLayout, type UtxoBlockInput,
} from "./utxoBlocksLayout";

const coin = (outpoint: string, amount_msat: number | null, confirmation_status = "confirmed"): UtxoBlockInput =>
  ({ outpoint, amount_msat, confirmation_status, asset: "BTC" });

describe("UTXO block layout", () => {
  it("scales only height linearly and keeps a visible dust floor", () => {
    const { blocks } = utxoBlocksLayout([coin("a:0", 1_000_000), coin("b:0", 500_000), coin("dust:0", 1)]);
    expect(blocks.map((block) => block.height)).toEqual([MAX_BLOCK_HEIGHT, MAX_BLOCK_HEIGHT / 2, MIN_BLOCK_HEIGHT]);
    for (const block of blocks) {
      expect(block.width).toBe(BLOCK_WIDTH);
      expect(block.depth).toBe(BLOCK_DEPTH);
    }
  });

  it("frosts unconfirmed and unknown coins, failing closed on invalid amounts", () => {
    const { blocks } = utxoBlocksLayout([
      coin("known:0", 1000), coin("pending:0", 1000, "mempool"),
      ...[null, NaN, Infinity, -1, 1.5, Number.MAX_SAFE_INTEGER + 1].map((value, index) => coin(`unknown:${index}`, value)),
    ]);
    expect(blocks.find((block) => block.outpoint === "known:0")?.frosted).toBe(false);
    expect(blocks.filter((block) => block.outpoint !== "known:0").every((block) => block.frosted)).toBe(true);
    expect(blocks.filter((block) => block.outpoint?.startsWith("unknown")).every((block) =>
      block.height === UNIFORM_BLOCK_HEIGHT)).toBe(true);
  });

  it("hides every amount signal, including sort order, cap selection and unknown status", () => {
    const rows = Array.from({ length: 205 }, (_, i) => coin(`tx:${i}`, i * 1000, i % 2 ? "confirmed" : "mempool"));
    const first = utxoBlocksLayout(rows, true);
    const changed = utxoBlocksLayout(rows.map((row, i) => ({ ...row, amount_msat: i % 3 ? (205 - i) * 1000 : null })), true);
    expect(first).toEqual(changed);
    expect(new Set(first.blocks.map((block) => block.height))).toEqual(new Set([UNIFORM_BLOCK_HEIGHT]));
  });

  it("never reads an amount in hidden mode", () => {
    const row = coin("a:0", 1);
    Object.defineProperty(row, "amount_msat", { get() { throw new Error("private"); } });
    expect(() => utxoBlocksLayout([row], true)).not.toThrow();
  });

  it("caps inventories at 200 coins plus one explicitly counted more block", () => {
    const rows = Array.from({ length: 260 }, (_, i) => coin(`tx:${i}`, i));
    const layout = utxoBlocksLayout(rows);
    expect(layout.blocks).toHaveLength(MAX_UTXO_BLOCKS + 1);
    expect(layout.omittedCount).toBe(60);
    expect(layout.blocks.at(-1)).toMatchObject({ kind: "more", omittedCount: 60, outpoint: null });
    expect(utxoBlocksLayout(rows.slice(0, 200)).blocks.every((block) => block.kind === "utxo")).toBe(true);
  });

  it("orders by descending value then outpoint, independent of input order, without mutation", () => {
    const rows = [coin("b:0", 100), coin("c:0", 200), coin("a:0", 100), coin("unknown:0", null)];
    const original = structuredClone(rows);
    const layout = utxoBlocksLayout(rows);
    expect(layout.blocks.map((block) => block.outpoint)).toEqual(["c:0", "a:0", "b:0", "unknown:0"]);
    expect(utxoBlocksLayout([...rows].reverse())).toEqual(layout);
    expect(rows).toEqual(original);
  });

  it("handles empty and all-zero inventories with finite geometry", () => {
    expect(utxoBlocksLayout([])).toEqual({ blocks: [], hidden: false, omittedCount: 0 });
    const { blocks } = utxoBlocksLayout([coin("a:0", 0), coin("b:0", 0)]);
    expect(blocks.every((block) => block.height === MIN_BLOCK_HEIGHT && Number.isFinite(block.x))).toBe(true);
  });

  it("does not compare quantities in different assets", () => {
    expect(() => utxoBlocksLayout([coin("a:0", 1), { ...coin("b:0", 1), asset: "OTHER" }])).toThrow("single asset");
  });
});
