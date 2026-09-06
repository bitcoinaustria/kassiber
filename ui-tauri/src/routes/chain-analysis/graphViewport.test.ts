import { describe, expect, it } from "vitest";
import { layoutAnalysisGraph, type AnalysisEdge, type AnalysisNode } from "@/lib/chainAnalysis";
import { fitGraphCamera, focusGraphCamera, graphNodeInView, graphOverview, graphSelectionInView, graphSelectionPosition, initialGraphCamera, resizeGraphCamera } from "./graphViewport";

const node = (id: string, kind: AnalysisNode["kind"] = "transaction"): AnalysisNode => ({
  id, kind, chain: "bitcoin", network: "regtest", label: id,
  wallet_ids: [], amount_msat: null, asset: "BTC", evidence: [],
});
const edge = (source: string, target: string): AnalysisEdge => ({
  id: `${source}:${target}`, source, target, kind: "spends", evidence_level: "observed",
  amount_msat: null, label: "spend", evidence: [],
});
function longChain() {
  const nodes: AnalysisNode[] = [];
  const edges: AnalysisEdge[] = [];
  for (let i = 0; i < 13; i++) {
    nodes.push(node(`tx${i}`), node(`out${i}`, "output"), node(`change${i}`, "output"));
    edges.push(edge(`tx${i}`, `out${i}`), edge(`tx${i}`, `change${i}`));
    if (i) edges.push(edge(`out${i - 1}`, `tx${i}`));
  }
  return { nodes, edges, layout: layoutAnalysisGraph(nodes, edges) };
}

describe("readable graph viewport", () => {
  it.each([280, 600, 1000])("starts the 39-node chain with readable nodes at width %i while preserving explicit full fit", (width) => {
    const { nodes, edges, layout } = longChain();
    const viewport = { width, height: 430 };
    const initial = initialGraphCamera(layout, viewport, nodes, edges, null);
    const fitted = fitGraphCamera(layout, viewport);
    expect(initial.scale).toBe(1);
    expect(fitted.scale).toBeLessThan(0.15);
    expect(graphSelectionInView(layout.positions.get("tx0")!, initial, viewport)).toBe(true);
    expect([...layout.positions.values()].filter((point) => graphNodeInView(point, initial, viewport)).length).toBeLessThan(39);
    for (const point of layout.positions.values()) expect(graphNodeInView(point, fitted, viewport)).toBe(true);
  });

  it("anchors a distant selection and a selected edge at readable size", () => {
    const { nodes, edges, layout } = longChain();
    const viewport = { width: 480, height: 430 };
    const selection = { kind: "node" as const, id: "tx10" };
    const camera = initialGraphCamera(layout, viewport, nodes, edges, selection);
    const position = layout.positions.get(selection.id)!;
    expect(position.x * camera.scale + camera.x).toBe(viewport.width / 2);
    expect(graphSelectionInView(position, camera, viewport)).toBe(true);
    const midpoint = graphSelectionPosition(layout, edges, { kind: "edge", id: "tx10:out10" })!;
    expect(midpoint.x).toBe((position.x + layout.positions.get("out10")!.x) / 2);
    expect(focusGraphCamera(midpoint, viewport).scale).toBe(1);
    expect(graphSelectionPosition(layout, edges, { kind: "edge", id: "missing" })).toBeUndefined();
  });

  it("keeps an intentionally navigated world center and scale through a resize", () => {
    const before = { width: 1000, height: 560 };
    const next = { width: 280, height: 430 };
    const camera = { x: -4200, y: -300, scale: 1.4 };
    const resized = resizeGraphCamera(camera, before, next);
    expect(resized.scale).toBe(camera.scale);
    expect((next.width / 2 - resized.x) / resized.scale).toBe((before.width / 2 - camera.x) / camera.scale);
    expect((next.height / 2 - resized.y) / resized.scale).toBe((before.height / 2 - camera.y) / camera.scale);
  });

  it("shows a readable root in a wide fan-out and bounds the overview to actual graph extents", () => {
    const nodes = [node("tx"), ...Array.from({ length: 1000 }, (_, i) => node(`output${i}`, "output"))];
    const edges = nodes.slice(1).map((item) => edge("tx", item.id));
    const layout = layoutAnalysisGraph(nodes, edges);
    const viewport = { width: 280, height: 430 };
    const camera = initialGraphCamera(layout, viewport, nodes, edges, null);
    expect(camera.scale).toBe(1);
    expect(graphSelectionInView(layout.positions.get("tx")!, camera, viewport)).toBe(true);
    const fitted = fitGraphCamera(layout, viewport);
    for (const point of layout.positions.values()) expect(graphNodeInView(point, fitted, viewport)).toBe(true);
    const overview = graphOverview(layout, camera, viewport);
    expect(overview.scale).toBeGreaterThan(0);
    expect(overview.viewport.x).toBeGreaterThanOrEqual(0);
    expect(overview.viewport.y).toBeGreaterThanOrEqual(0);
    expect(overview.viewport.x + overview.viewport.width).toBeLessThanOrEqual(164);
    expect(overview.viewport.y + overview.viewport.height).toBeLessThanOrEqual(60);
  });

  it("does not magnify tiny graphs and tolerates empty results or a stale selection", () => {
    const nodes = [node("tx")];
    const viewport = { width: 1000, height: 560 };
    const layout = layoutAnalysisGraph(nodes, []);
    expect(initialGraphCamera(layout, viewport, nodes, [], { kind: "node", id: "old" }).scale).toBe(1);
    expect(initialGraphCamera(layoutAnalysisGraph([], []), viewport, [], [], null).scale).toBe(1);
  });
});
