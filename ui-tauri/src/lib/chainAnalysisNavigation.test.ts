import { describe, expect, it } from "vitest";
import { DEFAULT_ANALYSIS_QUERY } from "./chainAnalysis";
import { ANALYSIS_SECTIONS, analysisInvestigationSearch, analysisSearchQuery, analysisSection, parseAnalysisSearch, transactionAnalysisSearch, type AnalysisTab } from "./chainAnalysisNavigation";

const txid = "a".repeat(64);
describe("Chain Analysis navigation", () => {
  it("preserves the public observer, domain and bounded query across a finding handoff", () => {
    const query = { ...DEFAULT_ANALYSIS_QUERY, mode: "trace" as const, subject: txid, observer: "public" as const, chain: "liquid" as const, network: "elementsregtest", include_relations: false, depth: 3, node_limit: 120, edge_limit: 300 };
    const search = analysisInvestigationSearch(query, "graph", "patterns");
    expect(analysisSearchQuery(parseAnalysisSearch(search))).toEqual({ ...query, network: "regtest" });
    expect(search).toMatchObject({ workspace: "graph", tab: "patterns", observer: "public" });
  });
  it("selects an existing tool without accepting acquisition flags or external endpoints", () => {
    expect(parseAnalysisSearch({ workspace: "psbt", tab: "entropy", observer: "public", acquire: true, execute: true, url: "https://unapproved.invalid", backend: "remote", token: "secret" })).toEqual({ workspace: "psbt", tab: "entropy", observer: "public" });
    expect(parseAnalysisSearch({ workspace: "unknown", observer: "all", subject: "x".repeat(1025), chain: "evil", network: "https://remote.invalid", depth: -1, node_limit: 1e20 })).toEqual({});
    expect(parseAnalysisSearch({ node_limit: 24, edge_limit: 49 })).toEqual({});
    expect(parseAnalysisSearch({ node_limit: 2001, edge_limit: 6001 })).toEqual({});
    expect(parseAnalysisSearch({ subject: "script:" + "51".repeat(400), node_limit: 2000, edge_limit: 6000 })).toEqual({ subject: "script:" + "51".repeat(400), node_limit: 2000, edge_limit: 6000 });
  });
  it("links physical transaction identities in their exact known domain", () => {
    const search = transactionAnalysisSearch({ explorerId: txid, chain: "bitcoin", network: "regtest" });
    expect(search).toEqual({ subject: txid, mode: "trace", observer: "public", chain: "bitcoin", network: "regtest", include_relations: false, include_hypotheses: false });
    expect(analysisSearchQuery(search).observer).toBe("public");
  });
  it.each([["liquidv1", "main"], ["liquidtestnet", "test"], ["elementsregtest", "regtest"]])("normalizes Liquid domain %s for the query API", (network, expected) => {
    expect(transactionAnalysisSearch({ explorerId: txid, chain: "liquid", network })).toMatchObject({ subject: txid, chain: "liquid", network: expected, observer: "public", mode: "trace" });
  });
  it("groups every canonical tab into exactly one presentation section", () => {
    const tabs: AnalysisTab[] = ["findings", "frontier", "clusters", "paths", "coverage", "entropy", "labels", "patterns", "exposure"];
    const grouped = Object.values(ANALYSIS_SECTIONS).flat();
    expect([...grouped].sort()).toEqual([...tabs].sort());
    expect(analysisSection("patterns")).toBe("findings");
    expect(analysisSection("exposure")).toBe("findings");
    expect(analysisSection("frontier")).toBe("coverage");
    expect(analysisSection("entropy")).toBe("tools");
    expect(analysisSection("paths")).toBe("paths");
    // A Mirror deep link to a subview must land on that subview, not the group's first tab.
    expect(parseAnalysisSearch(analysisInvestigationSearch(DEFAULT_ANALYSIS_QUERY, "graph", "exposure")).tab).toBe("exposure");
  });
  it("uses an honest public overview when the physical identity or domain is unavailable", () => {
    for (const record of [{ txnId: "private-record" }, { txnId: txid }, { txnId: txid, chain: "bitcoin" }]) {
      const search = transactionAnalysisSearch(record);
      expect(search.mode).toBe("overview");
      expect(search.observer).toBe("public");
      expect(search.subject).toBeUndefined();
      expect(JSON.stringify(search)).not.toContain("private-record");
    }
  });
});
