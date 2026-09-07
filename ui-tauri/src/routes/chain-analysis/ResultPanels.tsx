import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  pathNodeIds,
  shortAnalysisId,
  type AnalysisFinding,
  type AnalysisNode,
  type AnalysisResult,
} from "@/lib/chainAnalysis";
import {
  ANALYSIS_SECTIONS,
  type AnalysisTab,
} from "@/lib/chainAnalysisNavigation";
import { EvidenceDetails, Fact } from "./EvidenceDetails";
import { EntropyPanel } from "./EntropyPanel";
import { LabelsPanel } from "./LabelsPanel";
import type { GraphSelection } from "./InvestigationGraph";

const RESULT_VIEWS = Object.values(ANALYSIS_SECTIONS).flat();

function tabCount(result: AnalysisResult, tab: AnalysisTab): number | undefined {
  switch (tab) {
    case "findings":
    case "frontier":
    case "clusters":
    case "paths":
      return result[tab].length;
    case "patterns":
    case "exposure":
      return result[tab]?.length ?? 0;
    default:
      return undefined;
  }
}

export function ResultPanels({
  result,
  tab,
  setTab,
  node,
  pickedSubject,
  select,
  setHighlighted,
  reportError,
  onAcquire,
}: {
  result: AnalysisResult;
  tab: AnalysisTab;
  setTab: (tab: AnalysisTab) => void;
  node?: AnalysisNode;
  pickedSubject: string;
  select: (value: GraphSelection) => void;
  setHighlighted: (ids: string[]) => void;
  reportError: (error: unknown) => void;
  onAcquire?: () => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const { t: tm } = useTranslation("privacyMirror");
  const baseId = useId();
  const label = (item: AnalysisTab) =>
    item === "labels" ? t("labels.title") : t(item);
  const inspect = (ids: string[]) => {
    setHighlighted(ids);
    if (ids[0]) select({ kind: "node", id: ids[0] });
  };
  const findingCard = (finding: AnalysisFinding) => (
    <article key={finding.id} className="rounded-md border p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-medium">
            {tm(`findingTitle.${finding.code}`, { defaultValue: finding.title })}
          </h3>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {finding.detail}
          </p>
        </div>
        <Button
          size="sm"
          variant="ghost"
          disabled={!finding.node_ids.length}
          onClick={() => inspect(finding.node_ids)}
        >
          {t("inspect")}
        </Button>
      </div>
      <EvidenceDetails value={finding} />
    </article>
  );
  return (
    <section className="overflow-hidden rounded-xl border bg-card">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-3">
        <label className="text-sm font-medium" htmlFor={baseId}>{t("resultView")}</label>
        <select id={baseId} className="ca-select max-w-full" value={tab} onChange={(event) => setTab(event.target.value as AnalysisTab)}>
          {RESULT_VIEWS.map((item) => {
            const count = tabCount(result, item);
            return <option key={item} value={item}>{label(item)}{count === undefined ? "" : ` (${count})`}</option>;
          })}
        </select>
      </div>
      <div className="p-4" role="region" aria-label={label(tab)}>
        {tab === "findings" &&
          (result.findings.length ? (
            <div className="space-y-3">{result.findings.map(findingCard)}</div>
          ) : (
            <Empty />
          ))}
        {tab === "patterns" &&
          (result.patterns?.length ? (
            <div className="space-y-3">{result.patterns.map(findingCard)}</div>
          ) : (
            <Empty />
          ))}
        {tab === "clusters" &&
          (result.clusters.length ? (
            <div className="grid gap-3 md:grid-cols-2">
              {result.clusters.map((cluster) => (
                <article key={cluster.id} className="rounded-md border p-3">
                  <div className="flex items-center justify-between gap-3">
                    <h3 className="text-sm font-medium">
                      {cluster.label || shortAnalysisId(cluster.id)}
                    </h3>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={!cluster.node_ids?.length}
                      onClick={() => inspect(cluster.node_ids || [])}
                    >
                      {t("inspect")}
                    </Button>
                  </div>
                  <EvidenceDetails value={cluster} />
                </article>
              ))}
            </div>
          ) : (
            <Empty />
          ))}
        {tab === "exposure" &&
          (result.exposure?.length ? (
            <div className="space-y-3">
              {result.exposure.map((contact) => (
                <article key={contact.id} className="rounded-md border p-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <h3 className="text-sm font-medium">
                        {String(
                          contact.claim?.label ||
                            tm(`findingTitle.${contact.kind}`, {
                              defaultValue: contact.kind.replace(/_/g, " "),
                            }),
                        )}
                      </h3>
                      <p className="mt-1 font-mono text-[10px] text-muted-foreground">
                        {contact.node_ids.length} {t("nodes")}
                      </p>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => inspect(contact.node_ids)}
                    >
                      {t("inspect")}
                    </Button>
                  </div>
                  <EvidenceDetails value={contact} />
                </article>
              ))}
            </div>
          ) : (
            <Empty />
          ))}
        {tab === "paths" &&
          (result.paths.length ? (
            <div className="space-y-2">
              {result.paths.map((path, index) => (
                <article key={index} className="rounded-md border p-3">
                  <div className="flex items-center justify-between gap-3">
                    <p className="font-mono text-xs">
                      {index + 1} ·{" "}
                      {Array.isArray(path) ? "" : String(path.kind || "")}{" "}
                      · {pathNodeIds(path).length} {t("nodes")}
                    </p>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => setHighlighted(pathNodeIds(path))}
                    >
                      {t("inspect")}
                    </Button>
                  </div>
                  <EvidenceDetails value={path} />
                </article>
              ))}
            </div>
          ) : (
            <Empty />
          ))}
        {tab === "frontier" &&
          (result.frontier.length ? (
            <div className="space-y-2">
              {onAcquire && (
                <div className="flex justify-end">
                  <Button size="sm" variant="outline" onClick={onAcquire}>
                    {t("acquire.title")}
                  </Button>
                </div>
              )}
              {result.frontier.map((item, index) => (
                <article
                  key={`${item.node_id}:${index}`}
                  className="flex flex-wrap items-start justify-between gap-3 rounded-md border p-3"
                >
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-medium">
                      {tm(`reason.${item.reason}`, {
                        defaultValue: item.reason.replace(/_/g, " "),
                      })}
                    </p>
                    <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                      {item.node_id}
                    </p>
                    <EvidenceDetails value={item} />
                  </div>
                  {item.node_id && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => select({ kind: "node", id: item.node_id! })}
                    >
                      {t("inspect")}
                    </Button>
                  )}
                </article>
              ))}
            </div>
          ) : (
            <Empty />
          ))}
        {tab === "coverage" && (
          <div className="grid gap-4 md:grid-cols-2">
            <div>
              <dl className="mb-3 grid grid-cols-2 gap-3">
                <Fact
                  label={t("coverageMissing")}
                  value={result.coverage.missing_node_count}
                />
                <Fact
                  label={t("coverageCompleteTx")}
                  value={result.coverage.complete_transaction_count}
                />
                <Fact
                  label={t("coverageConflict")}
                  value={result.coverage.conflicting_node_count}
                />
                <Fact
                  label={t("coverageFresh")}
                  value={result.coverage.custody_fresh}
                />
              </dl>
              <p className="text-xs leading-relaxed text-muted-foreground">
                {String(result.coverage.unobserved_successors || "")}
              </p>
              <EvidenceDetails
                value={result.coverage.source_rows}
                label={t("coverageSources")}
              />
              <EvidenceDetails
                value={result.coverage.pruning}
                label={t("coveragePruning")}
              />
              <EvidenceDetails value={result.coverage} label={t("coverage")} />
            </div>
            <div>
              <dl className="grid grid-cols-2 gap-3">
                <Fact label={t("nodes")} value={result.nodes.length} />
                <Fact label={t("edges")} value={result.edges.length} />
                <div className="col-span-2">
                  <Fact label={t("snapshot")} value={result.snapshot_id} />
                </div>
              </dl>
              <EvidenceDetails value={result.query} label={t("resultQuery")} />
              <EvidenceDetails
                value={result.capabilities}
                label={t("capabilities")}
              />
            </div>
          </div>
        )}
        <div hidden={tab !== "entropy"}>
          <EntropyPanel
            subject={pickedSubject}
            query={result.query}
            observedNodes={result.nodes}
            onError={reportError}
          />
        </div>
        {tab === "labels" && (
          <LabelsPanel
            key={pickedSubject}
            initialSubject={node?.outpoint || node?.txid || pickedSubject}
            initialChain={node?.chain || result.query.chain}
            initialNetwork={node?.network || result.query.network}
            onError={reportError}
          />
        )}
      </div>
    </section>
  );
}

function Empty() {
  const { t } = useTranslation("chainAnalysis");
  return (
    <p className="py-6 text-center text-xs text-muted-foreground">
      {t("noItems")}
    </p>
  );
}
