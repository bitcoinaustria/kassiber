import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { EvidenceGraph, type EvidenceGraphSelection } from "@/components/evidence-graph/EvidenceGraph";
import { Button } from "@/components/ui/button";
import { provenanceGraph, type ProvenanceFlow } from "./provenanceGraph";
import { pretty } from "./model";

/** Uses the same canvas as Chain Analysis, with the reviewed provenance report as its only input. */
export function SourceFundsFlowGraph({ flow, onOpenTransaction }: {
  flow: ProvenanceFlow;
  onOpenTransaction?: (id: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const { t: graphText } = useTranslation("chainAnalysis");
  const [selected, setSelected] = useState<EvidenceGraphSelection | null>(null);
  const graph = useMemo(() => provenanceGraph(flow, role => t(`journey.roles.${role}`, { defaultValue: pretty(role) }), direction => t(direction === "inbound" ? "flow.incoming" : "flow.outgoing"), type => t(`linkType.${type}`, { defaultValue: pretty(type) })), [flow, t]);
  const node = selected?.kind === "node" ? graph.byId.get(selected.id)?.node : undefined;
  const detail = selected?.kind === "node" ? graph.nodes.find(item => item.id === selected.id) : graph.edges.find(item => item.id === selected?.id);
  const currentSelection = detail ? selected : null;
  return <div className="space-y-3">
    <EvidenceGraph nodes={graph.nodes} edges={graph.edges} selected={currentSelection} subjectId={graph.subjectId} highlightedIds={[]} onSelect={setSelected}
      className="source-funds-graph" height={360} initialView={graph.nodes.length <= 10 ? "fit" : "readable"} labels={{
        label: t("flowPath.title"), subject: t("journey.roles.target"),
        zoomIn: graphText("graph.zoomIn"), zoomOut: graphText("graph.zoomOut"), fit: graphText("graph.fit"),
        focusSelection: graphText("graph.focusSelection"), readableView: graphText("graph.readableView"),
        overview: graphText("graph.overview"), help: graphText("graph.help"),
        inView: (visible, total) => graphText("graph.inView", { visible, total }),
      }} />
    {detail && <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-muted/20 px-3 py-2 text-sm" aria-live="polite">
      <p className="min-w-0 flex-1 break-words">{detail.title}</p>
      {node?.node_type === "transaction" && node.transaction_id && onOpenTransaction && <Button variant="outline" size="sm" onClick={() => onOpenTransaction(node.transaction_id!)}>{t("workstation.viewTransactionDetails")}</Button>}
    </div>}
  </div>;
}
