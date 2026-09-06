import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { EvidenceGraph, type EvidenceGraphLabels } from "./EvidenceGraph";
import { layoutEvidenceGraph } from "./layout";
import { initialGraphCamera, graphNodeInView } from "./viewport";

const labels: EvidenceGraphLabels = {
  label: "Reviewed provenance", subject: "Selected target", zoomIn: "Zoom in", zoomOut: "Zoom out",
  fit: "Fit", focusSelection: "Focus", readableView: "Readable view", overview: "Overview",
  help: "Pan and zoom", inView: (visible, total) => `${visible}/${total}`,
};

describe("shared evidence presentation", () => {
  it("renders provenance-only nodes and exact edges without chain identities or inferred links", () => {
    const title = "Synthetic purchase <evidence> with the entire reviewed description";
    const html = renderToStaticMarkup(<EvidenceGraph
      nodes={[
        { id: "origin", label: title, detail: "0.006 BTC", title },
        { id: "target", label: "Wallet receipt", detail: "0.006 BTC", title: "Wallet receipt", shape: "capsule" },
        { id: "unconnected", label: "No known link", detail: "Unknown", title: "No known link" },
      ]}
      edges={[{ id: "reviewed", source: "origin", target: "target", label: "Reviewed allocation", title: "Explicit reviewed allocation", appearance: "secondary", dash: "long-dashed" }]}
      selected={null} highlightedIds={[]} subjectId="target" onSelect={() => {}} labels={labels} height={360}
    />);
    expect(html).toContain('height:360px');
    expect(html).toContain('aria-label="Reviewed provenance"');
    expect(html).toContain('data-subject="true"');
    expect(html.match(/data-edge=/g)).toHaveLength(1);
    expect(html).toContain('stroke-dasharray="9 3"');
    expect(html).toContain('Synthetic purchase &lt;evidence&gt; with the entire reviewed description');
    expect(html).not.toContain("<evidence>");
    expect(html).toContain('data-node="unconnected"');
  });
  it("lets callers exclude advisory edges from layout without making up connectivity", () => {
    const nodes = [{ id: "a" }, { id: "b" }, { id: "c" }];
    const base = layoutEvidenceGraph(nodes, [{ source: "a", target: "b" }]);
    const advisory = layoutEvidenceGraph(nodes, [{ source: "a", target: "b" }, { source: "b", target: "c", participatesInLayout: false }]);
    expect(advisory.positions).toEqual(base.positions);
    expect(base.positions.get("c")!.x).toBe(base.positions.get("a")!.x);
  });
});

describe("compact provenance starting view", () => {
  it("fits the whole reviewed flow even when the selected target is at the far end", () => {
    const nodes = ["source", "purchase", "withdrawal", "receipt"].map((id) => ({ id }));
    const edges = nodes.slice(1).map((node, index) => ({ id: `edge-${index}`, source: nodes[index].id, target: node.id }));
    const layout = layoutEvidenceGraph(nodes, edges);
    const viewport = { width: 900, height: 360 };
    const selection = { kind: "node" as const, id: "receipt" };
    const camera = initialGraphCamera(layout, viewport, nodes, edges, selection, "fit");
    for (const position of layout.positions.values()) expect(graphNodeInView(position, camera, viewport)).toBe(true);
    const readable = initialGraphCamera(layout, viewport, nodes, edges, selection);
    expect(readable.scale).toBeGreaterThan(camera.scale);
  });
});
