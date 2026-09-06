import { describe, expect, it } from "vitest";
import {
  DEFAULT_ANALYSIS_QUERY,
  analysisCsv,
  analysisNetworkInput,
  analysisUtcInput,
  analysisNodeSubject,
  analysisQueryKey,
  formatAnalysisAmount,
  layoutAnalysisGraph,
  panAnalysisCamera,
  zoomAnalysisCamera,
  type AnalysisEdge,
  type AnalysisNode,
  type AnalysisResult,
} from "./chainAnalysis";

const node = (id: string): AnalysisNode => ({
  id,
  kind: "transaction",
  chain: "bitcoin",
  network: "main",
  label: id,
  wallet_ids: [],
  amount_msat: null,
  asset: "BTC",
  evidence: [],
});
const edge = (
  source: string,
  target: string,
  kind: AnalysisEdge["kind"] = "spends",
): AnalysisEdge => ({
  id: `${source}:${target}`,
  source,
  target,
  kind,
  evidence_level: kind === "hypothesis" ? "heuristic" : "observed",
  amount_msat: null,
  label: kind,
  evidence: [],
});

describe("exact investigation values", () => {
  it("normalizes Liquid inventory domains and sends RFC3339 including seconds from date controls", () => {
    expect(analysisNetworkInput("liquidv1")).toBe("main");
    expect(analysisNetworkInput("elementsregtest")).toBe("regtest");
    expect(analysisUtcInput("2026-09-06T12:35")).toBe("2026-09-06T12:35:00Z");
    expect(analysisUtcInput("2026-09-06T12:35:23")).toBe(
      "2026-09-06T12:35:23Z",
    );
    expect(analysisUtcInput("")).toBeUndefined();
  });
  it("preserves every msat beyond Number.MAX_SAFE_INTEGER, sub-sat fractions and signs", () => {
    expect(formatAnalysisAmount("9007199254740993", "BTC")).toBe(
      "90071.99254740993 BTC",
    );
    expect(formatAnalysisAmount("1", "BTC")).toBe("0.00000000001 BTC");
    expect(formatAnalysisAmount("-100000000001", "LBTC")).toBe(
      "−1.00000000001 LBTC",
    );
    expect(formatAnalysisAmount(null)).toBe("—");
    expect(formatAnalysisAmount("1e9")).toBe("—");
    expect(formatAnalysisAmount("100", "unknown-asset")).toBe(
      "100 msat · unknown-asset",
    );
  });
  it("keeps canonical network identity for a trace from selection", () => {
    expect(
      analysisNodeSubject({ ...node("tx:bitcoin:regtest:abc"), txid: "abc" }),
    ).toBe("tx:bitcoin:regtest:abc");
  });
  it("detects perspective, budget and evidence-toggle edits while ignoring empty optional inputs", () => {
    const query = { ...DEFAULT_ANALYSIS_QUERY };
    expect(analysisQueryKey(query)).toBe(
      analysisQueryKey({ ...query, subject: " ", target: "unused" }),
    );
    for (const change of [
      { observer: "public" as const },
      { include_hypotheses: true },
      { depth: 20 },
      { mode: "path" as const, target: "abc" },
    ]) {
      expect(analysisQueryKey({ ...query, ...change })).not.toBe(
        analysisQueryKey(query),
      );
    }
  });
  it("exports real exact values and neutralizes formula-bearing labels", () => {
    const result = {
      snapshot_id: "snapshot",
      nodes: [
        {
          ...node("a"),
          label: '=HYPERLINK("https://example.test")',
          amount_msat: "9007199254740993",
        },
      ],
      edges: [edge("a", "b")],
    } as AnalysisResult;
    const csv = analysisCsv(result);
    expect(csv).toContain('"9007199254740993"');
    expect(csv).toContain('"\'=HYPERLINK(""https://example.test"")"');
    expect(csv).toContain('"spends"');
    expect(csv.split("\r\n")).toHaveLength(4);
  });
});

describe("bounded graph interaction", () => {
  it("keeps the graph coordinate under the pointer fixed through a zoom", () => {
    const before = { x: 30, y: -10, scale: 0.5 },
      pointer = { x: 430, y: 290 };
    const after = zoomAnalysisCamera(before, 2, pointer);
    expect((pointer.x - after.x) / after.scale).toBe(
      (pointer.x - before.x) / before.scale,
    );
    expect((pointer.y - after.y) / after.scale).toBe(
      (pointer.y - before.y) / before.scale,
    );
    expect(after.scale).toBe(1);
    expect(zoomAnalysisCamera(after, 1000, pointer).scale).toBe(4);
    expect(zoomAnalysisCamera(after, 0.000001, pointer).scale).toBe(0.025);
  });
  it("pans without changing graph scale or mutating prior state", () => {
    const before = { x: 10, y: 30, scale: 2 };
    expect(panAnalysisCamera(before, { x: -60, y: 20 })).toEqual({
      x: -50,
      y: 50,
      scale: 2,
    });
    expect(before).toEqual({ x: 10, y: 30, scale: 2 });
  });
  it("lays out physical branches while ignoring dangling and hypothetical links", () => {
    const nodes = [node("a"), node("b"), node("c"), node("d")];
    const edges = [
      edge("a", "b"),
      edge("b", "c"),
      edge("a", "missing"),
      edge("c", "a", "hypothesis"),
    ];
    const layout = layoutAnalysisGraph(nodes, edges);
    expect(layout.positions.size).toBe(4);
    expect(layout.positions.get("b")!.x).toBeGreaterThan(
      layout.positions.get("a")!.x,
    );
    expect(layout.positions.get("c")!.x).toBeGreaterThan(
      layout.positions.get("b")!.x,
    );
    expect(layout.positions.get("d")!.x).toBe(layout.positions.get("a")!.x);
    expect(layoutAnalysisGraph(nodes, edges)).toEqual(layout);
  });
  it("terminates on cycles and handles an empty graph", () => {
    expect(
      layoutAnalysisGraph(
        [node("a"), node("b")],
        [edge("a", "b", "custody"), edge("b", "a", "custody")],
      ).positions.size,
    ).toBe(2);
    expect(layoutAnalysisGraph([], []).positions.size).toBe(0);
  });
});
