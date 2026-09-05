import { describe, expect, it } from "vitest";
import { custodyGapRedirectSearch, parseTransfersCustodySearch, transfersCustodyView } from "./routeSearch";

describe("Transfers and custody navigation", () => {
  it("honors a new ownership deep link over stale history and swap mode", () => {
    expect(transfersCustodyView({ tab: "components", mode: "swaps", view: "paired", focus: "tx-1", method: "ownership_graph" }, true)).toEqual({
      tab: "review", mode: "transfers", view: "review", focus: "tx-1", method: "ownership_graph",
    });
  });
  it("uses review defaults when navigating back to the bare route", () => {
    expect(transfersCustodyView({}, true)).toEqual({ tab: "review", mode: "transfers", view: "review" });
  });
  it("preserves safe gap ids on the legacy redirect and discards unrelated state", () => {
    expect(custodyGapRedirectSearch({ gap_id: "custody-gap:123", focus: "tx-1", unsafe: "/Users/private" })).toEqual({ tab: "gaps", gap: "custody-gap:123" });
  });
  it("does not select an embedded developer surface while gated", () => {
    expect(transfersCustodyView({ tab: "gaps", gap: "gap-1" }, false)).toEqual({ tab: "review", mode: "transfers", view: "review" });
    expect(transfersCustodyView({ tab: "components" }, false).tab).toBe("review");
  });
  it("accepts only canonical fields and opaque local ids", () => {
    expect(parseTransfersCustodySearch({ tab: "wat", focus: "https://bad.example", gap: "/Users/private", method: "guess", mode: "other", view: "other" })).toEqual({});
  });
});
