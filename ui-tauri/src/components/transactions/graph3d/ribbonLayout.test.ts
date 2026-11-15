import { describe, expect, it } from "vitest";

import type { TransactionGraphNode, TransactionGraphPayload } from "../TransactionGraphModel";
import { GLASS_LINE_LIMIT, legendKeys, ribbonLayout } from "./ribbonLayout";

const leg = (
  id: string,
  sats: number | null,
  ownership = "external",
  valueState?: TransactionGraphNode["valueState"],
): TransactionGraphNode => ({
  id,
  outpoint: `${id.padEnd(64, "0")}:0`,
  valueSats: sats,
  valueBtc: sats == null ? null : sats / 1e8,
  valueState: valueState ?? (sats == null ? "missing" : "known"),
  role: "leg",
  ownership,
});

const graph = (
  inputs: TransactionGraphNode[],
  outputs: TransactionGraphNode[],
  fee: number | null = 1_000,
  chain = "bitcoin",
): TransactionGraphPayload => ({
  transaction: { id: "tx", chain },
  supportLevel: "full",
  inputs,
  outputs,
  fee:
    fee == null
      ? null
      : { id: "fee", valueSats: fee, valueBtc: fee / 1e8, role: "fee", ownership: "network_fee" },
});

type Layout = ReturnType<typeof ribbonLayout>;
const ribbon = (layout: Layout, id: string) =>
  layout.ribbons.find((entry) => entry.legId.endsWith(`:${id}`))!;
const blocks = (layout: Layout, side: "input" | "output") =>
  layout.legs.filter((entry) => entry.side === side);
// mempool spreads the strands' outer edges, the fee's included, over the full
// height on each side.
const outerSpan = (layout: Layout, side: "input" | "output") => {
  const ends = layout.ribbons
    .filter((entry) => entry.side === side)
    .map((entry) => ({ y: entry.points[0][1], half: entry.thickness / 2 }));
  return (
    Math.max(...ends.map((end) => end.y + end.half)) - Math.min(...ends.map((end) => end.y - end.half))
  );
};

const send = graph(
  [leg("in", 50_000_000, "owned")],
  [leg("pay", 10_000_000), leg("change", 39_999_000, "owned")],
);

describe("ribbon layout", () => {
  it("draws one ribbon and one coin per input and output, as mempool draws one strand", () => {
    const inputs = Array.from({ length: 48 }, (_, index) => leg(`in${index}`, 100_000 + index, "owned"));
    const layout = ribbonLayout(graph(inputs, [leg("a", 3_000_000), leg("b", 4_000_000)], 500), false);
    expect(blocks(layout, "input")).toHaveLength(48);
    expect(blocks(layout, "output")).toHaveLength(2);
    // The fee is a ribbon of its own, with no coin at its end.
    expect(layout.ribbons).toHaveLength(48 + 2 + 1);
    expect(layout.ribbons.filter((entry) => entry.fee)).toHaveLength(1);
  });

  it("spreads both sides over the same height, however many legs each has", () => {
    const inputs = Array.from({ length: 24 }, (_, index) => leg(`in${index}`, 1_000_000, "owned"));
    const layout = ribbonLayout(graph(inputs, [leg("a", 12_000_000), leg("b", 11_999_000)], 1_000), false);
    expect(outerSpan(layout, "output")).toBeGreaterThan(0);
    expect(Math.abs(outerSpan(layout, "input") - outerSpan(layout, "output"))).toBeLessThan(
      outerSpan(layout, "input") * 0.02,
    );
  });

  it("makes each ribbon as thick as its amount and meets them all in the band", () => {
    const layout = ribbonLayout(send, false);
    expect(ribbon(layout, "change").thickness).toBeGreaterThan(ribbon(layout, "pay").thickness * 3);
    for (const entry of layout.ribbons) {
      const end = entry.points[entry.points.length - 1];
      expect(end[0]).toBeCloseTo(0);
      expect(Math.abs(end[1])).toBeLessThanOrEqual(layout.center.halfHeight + 1e-9);
    }
  });

  it("frosts legs without a known amount and keeps the book's own coins blue", () => {
    const layout = ribbonLayout(
      graph(
        [leg("own", 600_000, "owned"), leg("foreign", null, "external", "confidential")],
        [leg("recipient", null, "external", "confidential"), leg("mychange", 150_000, "owned")],
        40,
        "liquid",
      ),
      false,
    );
    const byId = Object.fromEntries(layout.legs.map((entry) => [entry.row.id, entry]));
    expect(byId.own.owned).toBe(true);
    expect(byId.own.estimated).toBe(false);
    expect(byId.foreign.estimated).toBe(true);
    expect(ribbon(layout, "recipient").estimated).toBe(true);
  });

  it("keeps the glass limit's legs and folds the rest into one more-leg", () => {
    const inputs = Array.from({ length: 300 }, (_, index) => leg(`in${index}`, 10_000, "owned"));
    const layout = ribbonLayout(graph(inputs, [leg("out", 2_999_000)], 1_000), false);
    expect(blocks(layout, "input")).toHaveLength(GLASS_LINE_LIMIT + 1);
    expect(blocks(layout, "input").at(-1)?.row.overflowCount).toBe(300 - GLASS_LINE_LIMIT);
    // The fold is not the server's overflow row, which shares the row id.
    expect(blocks(layout, "input").at(-1)?.id).toBe("input:glass-fold");
    // Even then both sides fill the same height.
    expect(Math.abs(outerSpan(layout, "input") - outerSpan(layout, "output"))).toBeLessThan(
      outerSpan(layout, "input") * 0.02,
    );
  });

  it("counts the fee towards the destination side's legs", () => {
    const outputs = Array.from({ length: 300 }, (_, index) => leg(`out${index}`, 10_000));
    const layout = ribbonLayout(graph([leg("in", 3_001_000, "owned")], outputs, 1_000), false);
    const destination = layout.ribbons.filter((entry) => entry.side === "output");
    // The fee and 59 outputs stay individual, then one more-leg for the other 241.
    expect(destination).toHaveLength(GLASS_LINE_LIMIT + 1);
    expect(blocks(layout, "output").at(-1)?.row.overflowCount).toBe(300 - GLASS_LINE_LIMIT + 1);
  });

  it("fits zero-value stubs into the shared height", () => {
    const outputs = Array.from({ length: 260 }, (_, index) => leg(`z${index}`, index % 2 ? 0 : 5_000));
    const layout = ribbonLayout(graph([leg("in", 1_000_000, "owned"), leg("in2", 400_000, "owned")], outputs, 1_000), false);
    expect(Math.abs(outerSpan(layout, "input") - outerSpan(layout, "output"))).toBeLessThan(
      outerSpan(layout, "output") * 0.02,
    );
  });

  it("sums a Liquid total over every leg, not the folded ones", () => {
    const inputs = [
      ...Array.from({ length: 250 }, (_, index) => leg(`in${index}`, 10_000, "owned")),
      ...Array.from({ length: 10 }, (_, index) => leg(`conf${index}`, null, "external", "confidential")),
      leg("late", 5_000_000, "owned"),
    ];
    const outputs = [leg("a", null, "external", "confidential"), leg("b", null, "external", "confidential")];
    const folded = ribbonLayout(graph(inputs, outputs, 40, "liquid"), false);
    const unfolded = ribbonLayout(graph(inputs, outputs, 40, "liquid"), false, 400);
    // The two outputs share the same estimated total either way.
    expect(ribbon(folded, "a").thickness).toBeCloseTo(ribbon(unfolded, "a").thickness);
  });

  it("gives hidden values no say in the shape", () => {
    const other = graph([leg("in", 90_000_000, "owned")], [leg("pay", 80_000_000), leg("change", 9_999_000, "owned")]);
    const shape = (payload: TransactionGraphPayload) =>
      ribbonLayout(payload, true).ribbons.map((entry) => [entry.thickness, entry.points]);
    expect(shape(send)).toEqual(shape(other));
    expect(ribbonLayout(send, true).ribbons.every((entry) => entry.estimated)).toBe(true);
  });

  it("draws a zero-value output as a stub that never reaches the band", () => {
    const layout = ribbonLayout(graph([leg("in", 1_000_000, "owned")], [leg("opreturn", 0), leg("out", 999_000)]), false);
    const stub = ribbon(layout, "opreturn");
    expect(Math.abs(stub.points[stub.points.length - 1][0])).toBeGreaterThan(1);
  });

  it("names in its legend only what the drawing shows", () => {
    const known = ribbonLayout(graph([leg("in", 10_000, "owned")], [leg("out", 9_000)]), false);
    expect([...legendKeys(known)].sort()).toEqual(["fee", "known", "other", "own"]);
    const unknownOnly = ribbonLayout(graph([leg("in", null, "owned")], [leg("out", null, "owned")], null), false);
    expect([...legendKeys(unknownOnly)].sort()).toEqual(["own", "unknown"]);
  });

  it("keeps a big transaction as tall as a small one, not a column of hairlines", () => {
    const many = (prefix: string, count: number) =>
      Array.from({ length: count }, (_, index) => leg(`${prefix}${index}`, 10_000 + index));
    const small = ribbonLayout(graph(many("i", 3), many("o", 3)), false);
    const big = ribbonLayout(graph(many("i", 300), many("o", 300)), false);
    expect(outerSpan(big, "input")).toBeCloseTo(outerSpan(small, "input"), 1);
  });
});
