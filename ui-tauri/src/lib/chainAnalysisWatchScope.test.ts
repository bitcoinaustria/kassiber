import { describe, expect, it } from "vitest";
import { DEFAULT_ANALYSIS_QUERY } from "./chainAnalysis";
import { watchQueryScope, type WatchBinding } from "./chainAnalysisWatchScope";
const binding: WatchBinding = { state: "bound", environment: "main", domains: [{ chain: "bitcoin", network: "main" }, { chain: "liquid", network: "liquidv1" }] };
describe("watch scope from bound book", () => {
  it("keeps generic overview and paths across all bound chains", () => {
    for (const query of [DEFAULT_ANALYSIS_QUERY, { ...DEFAULT_ANALYSIS_QUERY, mode: "path" as const, subject: "a", target: "b" }]) {
      expect(watchQueryScope(query, binding)).toBe(query);
      expect(watchQueryScope(query, binding)?.chain).toBeUndefined();
    }
  });
  it("derives missing network for an explicit chain without adding controls", () => {
    expect(watchQueryScope({ ...DEFAULT_ANALYSIS_QUERY, chain: "liquid", observer: "public" }, binding)).toMatchObject({ chain: "liquid", network: "main", observer: "public" });
  });
  it("keeps a physical subject's declared domain, even when incompatible", () => {
    expect(watchQueryScope({ ...DEFAULT_ANALYSIS_QUERY, subject: `bitcoin:regtest:tx:${"a".repeat(64)}` }, binding)).toMatchObject({ chain: "bitcoin", network: "regtest" });
  });
  it("does not manufacture scope for an unbound book", () => {
    expect(watchQueryScope(DEFAULT_ANALYSIS_QUERY, { state: "unbound", domains: [] })).toBe(DEFAULT_ANALYSIS_QUERY);
  });
});
