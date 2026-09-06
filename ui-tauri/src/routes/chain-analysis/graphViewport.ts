import type { AnalysisEdge, AnalysisLayout, AnalysisNode } from "@/lib/chainAnalysis";
import { initialGraphCamera as initialEvidenceCamera, type GraphViewport, type ViewportSelection } from "@/components/evidence-graph/viewport";
export * from "@/components/evidence-graph/viewport";

export function initialGraphCamera(layout: AnalysisLayout, viewport: GraphViewport, nodes: AnalysisNode[], edges: AnalysisEdge[], selection: ViewportSelection | null) {
  return initialEvidenceCamera(layout, viewport, nodes.map((node) => ({ id: node.id, label: "", detail: "", title: "", preferInitialFocus: node.kind === "transaction" })), edges.map((edge) => ({ ...edge, title: "", label: edge.label || edge.kind })), selection);
}
