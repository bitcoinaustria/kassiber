/** Presentation only: callers retain all evidence, ownership and accounting authority. */
export interface EvidenceGraphNode {
  id: string;
  label: string;
  detail: string;
  title: string;
  shape?: "card" | "capsule";
  tone?: "default" | "positive" | "muted";
  status?: string;
  preferInitialFocus?: boolean;
}
export interface EvidenceGraphEdge {
  id: string;
  source: string;
  target: string;
  label: string;
  title: string;
  appearance?: "default" | "secondary" | "warning" | "muted" | "danger";
  dash?: "solid" | "dashed" | "long-dashed" | "dotted";
  status?: string;
  participatesInLayout?: boolean;
}
export interface EvidenceGraphSelection { kind: "node" | "edge"; id: string }
export interface EvidenceGraphLabels {
  label: string;
  subject: string;
  zoomIn: string;
  zoomOut: string;
  fit: string;
  focusSelection: string;
  readableView: string;
  overview: string;
  help: string;
  inView: (visible: number, total: number) => string;
}
export interface GraphPosition { x: number; y: number }
export interface GraphCamera extends GraphPosition { scale: number }
export interface EvidenceGraphLayout { positions: Map<string, GraphPosition>; width: number; height: number }
