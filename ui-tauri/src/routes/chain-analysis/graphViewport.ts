import type {
  AnalysisEdge,
  AnalysisLayout,
  AnalysisNode,
  GraphCamera,
  GraphPosition,
} from "@/lib/chainAnalysis";

export interface GraphViewport {
  width: number;
  height: number;
}

export interface ViewportSelection {
  kind: "node" | "edge";
  id: string;
}

export function graphSelectionPosition(
  layout: AnalysisLayout,
  edges: AnalysisEdge[],
  selection: ViewportSelection | null,
): GraphPosition | undefined {
  if (!selection) return undefined;
  if (selection.kind === "node") return layout.positions.get(selection.id);
  const edge = edges.find((item) => item.id === selection.id);
  const from = edge && layout.positions.get(edge.source);
  const to = edge && layout.positions.get(edge.target);
  return from && to
    ? { x: (from.x + to.x) / 2, y: (from.y + to.y) / 2 }
    : undefined;
}

export function fitGraphCamera(
  layout: AnalysisLayout,
  viewport: GraphViewport,
): GraphCamera {
  const scale = Math.max(
    0.001,
    Math.min(
      1,
      viewport.width / layout.width * 0.9,
      viewport.height / layout.height * 0.9,
    ),
  );
  return {
    scale,
    x: (viewport.width - layout.width * scale) / 2,
    y: (viewport.height - layout.height * scale) / 2,
  };
}

export function focusGraphCamera(
  position: GraphPosition,
  viewport: GraphViewport,
): GraphCamera {
  // A readable node at native size, including on narrow graph panels. Do not
  // scale an entire long chain into the viewport just to choose a starting view.
  const scale = Math.max(
    0.025,
    Math.min(1, (viewport.width - 40) / 198, (viewport.height - 100) / 58),
  );
  return {
    scale,
    x: viewport.width / 2 - position.x * scale,
    y: viewport.height / 2 - position.y * scale,
  };
}

export function initialGraphCamera(
  layout: AnalysisLayout,
  viewport: GraphViewport,
  nodes: AnalysisNode[],
  edges: AnalysisEdge[],
  selection: ViewportSelection | null,
): GraphCamera {
  const selected = graphSelectionPosition(layout, edges, selection);
  if (selected) return focusGraphCamera(selected, viewport);
  const fitted = fitGraphCamera(layout, viewport);
  if (fitted.scale >= 0.9 || !nodes.length) return fitted;
  // Prefer the earliest visible transaction to an arbitrary hash-sorted output.
  // This is a visual anchor only; it does not assert an ownership or flow root.
  const candidates = nodes.some((node) => node.kind === "transaction")
    ? nodes.filter((node) => node.kind === "transaction")
    : nodes;
  const positions = candidates
    .map((node) => layout.positions.get(node.id))
    .filter((position): position is GraphPosition => !!position);
  positions.sort((a, b) => a.x - b.x || a.y - b.y);
  const anchor = positions[0];
  if (!anchor) return fitted;
  const camera = focusGraphCamera(anchor, viewport);
  // Leave more room in the reading direction when beginning at the left edge.
  return {
    ...camera,
    x: Math.min(
      viewport.width / 2,
      Math.max(123 * camera.scale, viewport.width * 0.3),
    ) - anchor.x * camera.scale,
  };
}

export function graphNodeInView(
  position: GraphPosition,
  camera: GraphCamera,
  viewport: GraphViewport,
): boolean {
  const x = position.x * camera.scale + camera.x;
  const y = position.y * camera.scale + camera.y;
  return x + 99 * camera.scale > 0 &&
    x - 99 * camera.scale < viewport.width &&
    y + 29 * camera.scale > 0 &&
    y - 29 * camera.scale < viewport.height;
}

export function graphSelectionInView(
  position: GraphPosition,
  camera: GraphCamera,
  viewport: GraphViewport,
): boolean {
  const x = position.x * camera.scale + camera.x;
  const y = position.y * camera.scale + camera.y;
  return camera.scale >= 0.75 &&
    x - 99 * camera.scale >= 12 &&
    x + 99 * camera.scale <= viewport.width - 12 &&
    y - 29 * camera.scale >= 60 &&
    y + 29 * camera.scale <= viewport.height - 80;
}

export function resizeGraphCamera(
  camera: GraphCamera,
  previous: GraphViewport,
  next: GraphViewport,
): GraphCamera {
  return {
    ...camera,
    x: camera.x + (next.width - previous.width) / 2,
    y: camera.y + (next.height - previous.height) / 2,
  };
}

export function graphOverview(
  layout: AnalysisLayout,
  camera: GraphCamera,
  viewport: GraphViewport,
  width = 164,
  height = 60,
) {
  const scale = Math.min((width - 12) / layout.width, (height - 12) / layout.height);
  const x = (width - layout.width * scale) / 2;
  const y = (height - layout.height * scale) / 2;
  const left = Math.max(0, Math.min(layout.width, -camera.x / camera.scale));
  const top = Math.max(0, Math.min(layout.height, -camera.y / camera.scale));
  const right = Math.max(
    0, Math.min(layout.width, (viewport.width - camera.x) / camera.scale),
  );
  const bottom = Math.max(
    0, Math.min(layout.height, (viewport.height - camera.y) / camera.scale),
  );
  return {
    scale, x, y,
    viewport: {
      x: x + left * scale,
      y: y + top * scale,
      width: Math.max(2, (right - left) * scale),
      height: Math.max(4, (bottom - top) * scale),
    },
  };
}
