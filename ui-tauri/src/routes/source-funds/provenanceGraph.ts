import type { EvidenceGraphEdge, EvidenceGraphNode } from "@/components/evidence-graph/EvidenceGraph";
import { formatBtc, pretty, type SourceFundsPreview } from "./model";

export type ProvenanceFlow = NonNullable<SourceFundsPreview["simplified_flow"]>;

/** Presentation adapter only. Reachability, allocations and privacy stops come from the report. */
export function provenanceGraph(flow: ProvenanceFlow, roleLabel: (role: string) => string, directionLabel: (direction: string) => string = pretty, linkLabel: (type: string) => string = pretty) {
  const byId = new Map(flow.levels.flatMap(level => level.nodes.map(node => [node.id, { node, role: level.role || "flow" }] as const)));
  const nodes: EvidenceGraphNode[] = [...byId.values()].map(({ node, role }) => {
    const label = node.wallet
      ? [node.wallet, ["inbound", "outbound"].includes(node.kind || "") ? directionLabel(node.kind!) : ""].filter(Boolean).join(" · ")
      : node.label || node.id;
    const amount = formatBtc(node.amount, node.asset || "BTC");
    return {
      id: node.id,
      label,
      detail: amount,
      title: [...new Set([roleLabel(role), label, node.label, amount].filter(Boolean))].join(" · "),
      tone: node.deferred_privacy_hop ? "muted" : node.node_type === "source" ? "positive" : "default",
      preferInitialFocus: role === "target",
    };
  });
  const edges: EvidenceGraphEdge[] = [];
  for (const edge of flow.edges ?? []) {
    if (!edge.id || !edge.from || !edge.to || !byId.has(edge.from) || !byId.has(edge.to)) continue;
    const label = linkLabel(edge.link_type || "") || roleLabel("flow");
    edges.push({
      id: edge.id,
      source: edge.from,
      target: edge.to,
      label,
      title: [label, formatBtc(edge.amount, edge.asset || "BTC"), edge.deferred_privacy_hop ? roleLabel("privacy_boundary") : ""].filter(Boolean).join(" · "),
      appearance: edge.deferred_privacy_hop ? "warning" : "secondary",
      dash: edge.deferred_privacy_hop ? "dashed" : "solid",
    });
  }
  return { nodes, edges, subjectId: [...byId.values()].find(value => value.role === "target")?.node.id, byId };
}
