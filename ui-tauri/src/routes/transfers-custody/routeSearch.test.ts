import { describe, expect, it } from "vitest";
import { custodyGapRedirectSearch, parseTransfersCustodySearch, transfersCustodyView } from "./routeSearch";

describe("Transfers navigation", () => {
  it("opens the page on the bare route", () => {
    expect(transfersCustodyView({}, true)).toEqual({ section: "overview", tab: "gaps" });
  });
  it("opens Link manually on a named transaction, from old focus links too", () => {
    expect(parseTransfersCustodySearch({ link: "tx-1" })).toEqual({ link: "tx-1" });
    expect(parseTransfersCustodySearch({ link: "new" })).toEqual({ link: "new" });
    expect(parseTransfersCustodySearch({ section: "tools", focus: "tx-1", mode: "swaps", method: "ownership_graph" })).toEqual({ link: "tx-1" });
    expect(transfersCustodyView({ tab: "components", focus: "tx-1" }, true)).toEqual({ section: "overview", tab: "gaps", link: "tx-1" });
  });
  it("lands the old matching tools on the page, with a saved view as a selection", () => {
    expect(parseTransfersCustodySearch({ section: "tools", view: "paired" })).toEqual({});
    expect(parseTransfersCustodySearch({ tab: "review", mode: "swaps" })).toEqual({});
    expect(parseTransfersCustodySearch({ section: "tools", saved_view: "view-1" })).toEqual({ savedView: "view-1" });
  });
  it("maps the pre-redesign tabs onto the advanced tools", () => {
    expect(parseTransfersCustodySearch({ tab: "components" })).toEqual({ section: "advanced", tab: "components" });
    expect(parseTransfersCustodySearch({ tab: "gaps", gap: "gap-1" })).toEqual({ section: "advanced", tab: "gaps", gap: "gap-1" });
    expect(parseTransfersCustodySearch({ section: "advanced", tab: "coverage" })).toEqual({ section: "advanced", tab: "coverage" });
  });
  it("preserves safe gap ids on the legacy redirect and discards unrelated state", () => {
    expect(custodyGapRedirectSearch({ gap_id: "custody-gap:123", focus: "tx-1", unsafe: "/Users/private" })).toEqual({ section: "advanced", tab: "gaps", gap: "custody-gap:123" });
  });
  it("falls back to the overview for the advanced tools while gated", () => {
    expect(transfersCustodyView({ tab: "gaps", gap: "gap-1" }, false).section).toBe("overview");
    expect(transfersCustodyView({ section: "advanced", tab: "components" }, false).section).toBe("overview");
    expect(transfersCustodyView({ section: "advanced", tab: "components" }, true)).toMatchObject({ section: "advanced", tab: "components" });
  });
  it("accepts only canonical fields and opaque local ids", () => {
    expect(parseTransfersCustodySearch({ section: "wat", tab: "wat", link: "https://bad.example", focus: "/Users/private", gap: "/Users/private" })).toEqual({});
  });
});
