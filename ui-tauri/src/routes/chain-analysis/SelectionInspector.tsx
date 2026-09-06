import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArrowDownLeft,
  ArrowUpRight,
  Check,
  ChevronRight,
  Copy,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  analysisNodeSubject,
  formatAnalysisAmount,
  type AnalysisNode,
  type AnalysisEdge,
  type AnalysisQuery,
} from "@/lib/chainAnalysis";
import { copyTextWithPolicy } from "@/lib/clipboard";
import type { GraphSelection } from "./InvestigationGraph";
import { EvidenceDetails, Fact } from "./EvidenceDetails";
import { FeatureDetails } from "./FeatureDetails";
import type { FeatureSnapshot } from "@/lib/chainAnalysisWorkbench";

export function SelectionInspector({
  node,
  edge,
  busy,
  onTrace,
  onTarget,
  onSelect,
  onClose,
  onError,
  features,
}: {
  node?: AnalysisNode;
  edge?: AnalysisEdge;
  busy: boolean;
  onTrace: (direction: AnalysisQuery["direction"]) => void;
  onTarget: (subject: string) => void;
  onSelect: (value: GraphSelection) => void;
  onClose?: () => void;
  onError: (error: unknown) => void;
  features?: FeatureSnapshot;
}) {
  const { t } = useTranslation("chainAnalysis");
  const [copied, setCopied] = useState(false);
  if (!node && !edge) return null;
  const selectedId = node?.outpoint || node?.txid || node?.id || edge?.id;
  return (
    <aside className="ca-inspector p-4" aria-label={t("inspect")}>
      <div className="flex items-start justify-between gap-2">
        <h2 className="break-words text-sm font-semibold">
          {node?.label || edge?.label || node?.kind || edge?.kind}
        </h2>
        <div className="flex shrink-0 items-center">
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={copied ? t("copied") : t("copy")}
            disabled={!selectedId}
            onClick={async () => {
              try {
                if (!navigator.clipboard) throw new Error(t("error"));
                await copyTextWithPolicy(selectedId || "");
                setCopied(true);
              } catch (value) {
                onError(value);
              }
            }}
          >
            {copied ? (
              <Check className="size-3.5" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </Button>
          {onClose && (
            <Button
              size="icon-sm"
              variant="ghost"
              aria-label={t("workbench.clearSelection")}
              onClick={onClose}
            >
              <X className="size-3.5" />
            </Button>
          )}
        </div>
      </div>
      {node && (
        <>
          <p className="mt-1 font-mono text-lg tracking-tight">
            {formatAnalysisAmount(node.amount_msat, node.asset)}
          </p>
          <div className="my-4 grid grid-cols-2 gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => onTrace("backward")}
            >
              <ArrowDownLeft className="size-3.5" />
              {t("backward")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => onTrace("forward")}
            >
              <ArrowUpRight className="size-3.5" />
              {t("forward")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="col-span-2"
              onClick={() => onTarget(analysisNodeSubject(node))}
            >
              {t("setTarget")}
              <ChevronRight className="size-3.5" />
            </Button>
          </div>
          <dl className="space-y-3">
            <Fact label={t("id")} value={node.id} />
            <Fact label={t("kind")} value={node.kind} />
            <Fact label={t("txid")} value={node.txid} />
            <Fact label={t("outpoint")} value={node.outpoint} />
            <Fact label={t("address")} value={node.address} />
            <Fact
              label={t("network")}
              value={`${node.chain} / ${node.network}`}
            />
            <Fact label={t("wallets")} value={node.wallet_ids} />
            <Fact
              label={t("status")}
              value={
                node.status
                  ? t(`observationStatus.${node.status}`, {
                      defaultValue: node.status,
                    })
                  : undefined
              }
            />
          </dl>
          <EvidenceDetails value={node.evidence} />
          {features && (
            <div className="mt-5">
              <FeatureDetails snapshot={features} />
            </div>
          )}
        </>
      )}
      {edge && (
        <>
          <dl className="mt-4 space-y-3">
            <Fact label={t("kind")} value={edge.kind} />
            <Fact label={t("showEvidence")} value={edge.evidence_level} />
            <Fact
              label={t("status")}
              value={t(`observationStatus.${edge.status || "unknown"}`, {
                defaultValue: edge.status || "unknown",
              })}
            />
            <Fact
              label={t("amount")}
              value={formatAnalysisAmount(edge.amount_msat, edge.asset)}
            />
            {(["source", "target"] as const).map((key) => (
              <div key={key}>
                <dt className="ca-detail-key">
                  {t(key === "source" ? "from" : "to")}
                </dt>
                <dd>
                  <button
                    className="mt-1 break-all text-left font-mono text-xs underline"
                    onClick={() => onSelect({ kind: "node", id: edge[key] })}
                  >
                    {edge[key]}
                  </button>
                </dd>
              </div>
            ))}
          </dl>
          <EvidenceDetails value={edge.evidence} />
        </>
      )}
    </aside>
  );
}
