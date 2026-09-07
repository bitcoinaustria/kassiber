import type { EvidenceGraphNode, EvidenceGraphEdge, EvidenceGraphLayout, GraphPosition, GraphCamera } from "./types";

export function zoomGraphCamera(
  previous: GraphCamera,
  factor: number,
  point: GraphPosition,
): GraphCamera {
  const scale = Math.max(0.001, Math.min(4, previous.scale * factor));
  const ratio = scale / previous.scale;
  return {
    scale,
    x: point.x - (point.x - previous.x) * ratio,
    y: point.y - (point.y - previous.y) * ratio,
  };
}

export function panGraphCamera(
  previous: GraphCamera,
  delta: GraphPosition,
): GraphCamera {
  return { ...previous, x: previous.x + delta.x, y: previous.y + delta.y };
}
/** Stable layered layout of a bounded result; callers select which edges participate. */
export function layoutEvidenceGraph(
  nodes: Pick<EvidenceGraphNode, "id">[],
  edges: Pick<EvidenceGraphEdge, "source" | "target" | "participatesInLayout">[],
): EvidenceGraphLayout {
  const ids = new Set(nodes.map((node) => node.id));
  const adjacency = new Map<string, string[]>();
  const indegree = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of edges) {
    if (
      edge.participatesInLayout === false ||
      !ids.has(edge.source) ||
      !ids.has(edge.target) ||
      edge.source === edge.target
    )
      continue;
    adjacency.set(edge.source, [
      ...(adjacency.get(edge.source) || []),
      edge.target,
    ]);
    indegree.set(edge.target, (indegree.get(edge.target) || 0) + 1);
  }
  const roots = nodes
    .filter((node) => indegree.get(node.id) === 0)
    .map((node) => node.id);
  const levels = new Map<string, number>();
  const queue = [...roots];
  roots.forEach((id) => levels.set(id, 0));
  // Breadth-first levels avoid unbounded loops in reviewed/cyclic relations.
  for (let i = 0; i < queue.length; i++) {
    const id = queue[i];
    for (const next of adjacency.get(id) || []) {
      if (!levels.has(next)) {
        levels.set(next, Math.min(50, (levels.get(id) || 0) + 1));
        queue.push(next);
      }
    }
  }
  for (const node of nodes) if (!levels.has(node.id)) levels.set(node.id, 0);
  const columns = new Map<number, Pick<EvidenceGraphNode, "id">[]>();
  for (const node of nodes) {
    const level = levels.get(node.id) || 0;
    columns.set(level, [...(columns.get(level) || []), node]);
  }
  const positions = new Map<string, GraphPosition>();
  const maxRows = Math.max(
    1,
    ...[...columns.values()].map((items) => items.length),
  );
  const height = Math.max(380, maxRows * 90 + 100);
  for (const [level, entries] of columns) {
    entries.forEach((node, index) =>
      positions.set(node.id, {
        x: 145 + level * 265,
        y: (height - entries.length * 90) / 2 + 45 + index * 90,
      }),
    );
  }
  return {
    positions,
    width: Math.max(700, (Math.max(0, ...columns.keys()) + 1) * 265 + 40),
    height,
  };
}
