import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { EvidenceGraph, type EvidenceGraphNode, type EvidenceGraphEdge, type EvidenceGraphSelection } from "@/components/evidence-graph/EvidenceGraph";
import { analysisNodeCaption, formatAnalysisAmount, type AnalysisNode, type AnalysisEdge } from "@/lib/chainAnalysis";

export type GraphSelection = EvidenceGraphSelection;

export function InvestigationGraph({ nodes, edges, selected, highlightedIds, subjectId, onSelect }: {
  nodes: AnalysisNode[]; edges: AnalysisEdge[]; selected: GraphSelection | null;
  highlightedIds: string[]; subjectId?: string; onSelect: (selection: GraphSelection) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const graphNodes = useMemo<EvidenceGraphNode[]>(() => nodes.map((node) => ({
    id: node.id, label: analysisNodeCaption(node),
    detail: node.kind === "output" ? formatAnalysisAmount(node.amount_msat, node.asset) : `${node.chain} · ${node.network}`,
    title: `${node.label}\n${node.outpoint || node.txid || node.id}\n${formatAnalysisAmount(node.amount_msat, node.asset)}`,
    shape: node.kind === "output" ? "capsule" : "card", tone: node.wallet_ids?.length ? "positive" : "muted",
    status: node.status, preferInitialFocus: node.kind === "transaction",
  })), [nodes]);
  const graphEdges = useMemo<EvidenceGraphEdge[]>(() => edges.map((edge) => {
    const impaired = edge.status === "stale" || edge.status === "conflicting";
    const status = edge.status ? t(`observationStatus.${edge.status}`, { defaultValue: edge.status }) : "";
    const label = [edge.label || edge.kind, status].filter(Boolean).join(" · ");
    return {
      id: edge.id, source: edge.source, target: edge.target, label, title: `${label} · ${edge.evidence_level}`,
      status: edge.status,
      appearance: edge.status === "conflicting" ? "danger" : edge.status === "stale" ? "muted" : edge.kind === "hypothesis" ? "warning" : edge.kind === "custody" ? "secondary" : "default",
      dash: impaired ? "dotted" : edge.kind === "custody" ? "long-dashed" : edge.kind === "hypothesis" ? "dashed" : "solid",
      participatesInLayout: edge.kind !== "hypothesis",
    };
  }), [edges, t]);
  return <EvidenceGraph nodes={graphNodes} edges={graphEdges} selected={selected} highlightedIds={highlightedIds}
    subjectId={subjectId} onSelect={onSelect} labels={{
      label: t("graph.label"), subject: t("graph.subject"), zoomIn: t("graph.zoomIn"), zoomOut: t("graph.zoomOut"),
      fit: t("graph.fit"), focusSelection: t("graph.focusSelection"), readableView: t("graph.readableView"),
      overview: t("graph.overview"), help: t("graph.help"), inView: (visible, total) => t("graph.inView", { visible, total }),
    }} />;
}
