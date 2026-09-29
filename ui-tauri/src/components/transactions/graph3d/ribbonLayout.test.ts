import { describe, expect, it } from "vitest";

import type { TransactionGraphNode, TransactionGraphPayload } from "../TransactionGraphModel";
import { ribbonLayout } from "./ribbonLayout";

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
): TransactionGraphPayload => ({
  transaction: { id: "tx" },
  supportLevel: "full",
  inputs,
  outputs,
  fee: fee == null ? null : { id: "fee", valueSats: fee, valueBtc: fee / 1e8, role: "fee", ownership: "network_fee" },
});

const ribbonsOf = (layout: ReturnType<typeof ribbonLayout>, id: string) =>
  layout.legs.find((entry) => entry.row.id === id)?.ribbons ?? 0;

const send = graph([leg("in", 50_000_000, "owned")], [leg("pay", 10_000_000), leg("change", 39_999_000, "owned")]);

describe("ribbon layout", () => {
  it("ends every ribbon in the collar, never running input to output", () => {
    const layout = ribbonLayout(send, false, 250);
    for (const ribbon of layout.ribbons) {
      if (ribbon.side === "input") expect(ribbon.to[0]).toBeCloseTo(0);
      else expect(ribbon.from[0]).toBeCloseTo(0);
    }
    expect(layout.ribbons.some((ribbon) => ribbon.side === "input")).toBe(true);
    expect(layout.ribbons.some((ribbon) => ribbon.side === "output")).toBe(true);
  });

  it("gives each leg ribbons in proportion to its amount, and dust at least one", () => {
    const layout = ribbonLayout(send, false, 250);
    expect(ribbonsOf(layout, "change")).toBeGreaterThan(ribbonsOf(layout, "pay") * 3);
    expect(ribbonsOf(layout, "in")).toBe(ribbonsOf(layout, "pay") + ribbonsOf(layout, "change"));
    const dusty = ribbonLayout(
      graph([leg("in", 100_000_000, "owned")], [leg("dust", 546), leg("rest", 99_998_454, "owned")]),
      false,
      250,
    );
    expect(ribbonsOf(dusty, "dust")).toBe(1);
  });

  it("marks owned blocks and frosts legs whose amount is not known", () => {
    const layout = ribbonLayout(
      graph(
        [leg("own", 600_000, "owned"), leg("foreign", null, "external", "confidential")],
        [leg("recipient", null, "external", "confidential"), leg("mine", 150_000, "owned")],
        40,
      ),
      false,
      250,
    );
    const byId = Object.fromEntries(layout.legs.map((entry) => [entry.row.id, entry]));
    expect(byId.own.owned).toBe(true);
    expect(byId.own.estimated).toBe(false);
    expect(byId.foreign.estimated).toBe(true);
    expect(byId.recipient.estimated).toBe(true);
    expect(layout.ribbons.filter((ribbon) => ribbon.legId === "output:recipient").every((ribbon) => ribbon.estimated)).toBe(true);
    expect(layout.uniform).toBe(false);
  });

  it("splits one shared band evenly on each side when nothing but the fee is known", () => {
    const confidential = (id: string) => leg(id, null, "external", "confidential");
    const even = ribbonLayout(
      graph([confidential("a"), confidential("b")], [confidential("c"), confidential("d")], 40),
      false,
      250,
    );
    expect(even.uniform).toBe(true);
    expect(new Set(even.legs.map((entry) => entry.ribbons)).size).toBe(1);

    const fanIn = ribbonLayout(
      graph([confidential("a"), confidential("b"), confidential("c")], [confidential("d")], 40),
      false,
      250,
    );
    const ribbons = (side: "input" | "output") =>
      fanIn.legs.filter((entry) => entry.side === side).map((entry) => entry.ribbons);
    expect(new Set(ribbons("input")).size).toBe(1);
    expect(ribbons("output")[0]).toBe(ribbons("input").reduce((total, count) => total + count, 0));
  });

  it("gives hidden values no say in the shape", () => {
    const other = graph([leg("in", 90_000_000, "owned")], [leg("pay", 80_000_000), leg("change", 9_999_000, "owned")]);
    const shape = (payload: TransactionGraphPayload) =>
      ribbonLayout(payload, true, 250).legs.map((entry) => entry.ribbons);
    expect(shape(send)).toEqual(shape(other));
    // A hidden fee is an unknown amount too, and is drawn frosted.
    expect(ribbonLayout(send, true, 250).fee?.estimated).toBe(true);
    expect(ribbonLayout(send, false, 250).fee?.estimated).toBe(false);
  });

  it("keeps the ribbon count bounded when one side's widths overflow the band", () => {
    // No inputs: every amountless output would otherwise claim the whole band.
    const outputs = Array.from({ length: 250 }, (_, index) => leg(`o${index}`, null));
    const layout = ribbonLayout(graph([], outputs, null), false, 250);
    expect(layout.ribbons.length).toBeLessThanOrEqual(600);
    expect(layout.legs.every((entry) => entry.ribbons >= 1)).toBe(true);
  });

  it("draws the fee as a single filament, not as ribbons", () => {
    const layout = ribbonLayout(send, false, 250);
    expect(layout.fee).not.toBeNull();
    expect(layout.legs.some((entry) => entry.row.side === "fee")).toBe(false);
    expect(ribbonLayout(graph([leg("in", 1_000, "owned")], [leg("out", 1_000)], null), false, 250).fee).toBeNull();
  });
});
