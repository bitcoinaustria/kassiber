import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { pathNodeIds, shortAnalysisId, type AnalysisResult, type AnalysisNode } from "@/lib/chainAnalysis";
import { EvidenceDetails, Fact } from "./EvidenceDetails";
import { EntropyPanel } from "./EntropyPanel";
import { LabelsPanel } from "./LabelsPanel";
import type { GraphSelection } from "./InvestigationGraph";
export type ResultTab = "findings" | "frontier" | "clusters" | "paths" | "patterns" | "exposure" | "coverage" | "entropy" | "labels";
export function ResultPanels({result, tab, setTab, node, pickedSubject, select, setHighlighted, reportError}: {
 result: AnalysisResult; tab: ResultTab; setTab: (tab: ResultTab) => void; node?: AnalysisNode; pickedSubject: string;
 select: (value: GraphSelection) => void; setHighlighted: (ids: string[]) => void; reportError: (error: unknown) => void;
}) {
 const { t } = useTranslation("chainAnalysis");
 return (
          <section className="overflow-hidden rounded-xl border bg-card">
            <div className="flex overflow-auto border-b px-2" role="tablist">
              {(
                [
                  "findings",
                  "frontier",
                  "clusters",
                  "paths",
                  "patterns",
                  "exposure",
                  "coverage",
                  "entropy",
                  "labels",
                ] as const
              ).map((item) => (
                <button
                  key={item}
                  className="ca-tab"
                  role="tab"
                  aria-selected={tab === item}
                  onClick={() => setTab(item)}
                >
                  {item === "labels" ? t("labels.title") : t(item)}
                  {["findings", "frontier", "clusters", "paths"].includes(
                    item,
                  ) && (
                    <span className="ml-1.5 font-mono text-[10px] opacity-60">
                      {
                        (
                          result[
                            item as
                              "findings" | "frontier" | "clusters" | "paths"
                          ] || []
                        ).length
                      }
                    </span>
                  )}
                </button>
              ))}
            </div>
            <div className="p-4" role="tabpanel">
              {tab === "findings" &&
                (result.findings.length ? (
                  <div className="space-y-3">
                    {result.findings.map((finding) => (
                      <article
                        key={finding.id}
                        className="rounded-md border p-3"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <span className="font-mono text-[10px] uppercase text-muted-foreground">
                              {finding.severity} · {finding.code}
                            </span>
                            <h3 className="mt-1 text-sm font-medium">
                              {finding.title}
                            </h3>
                            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                              {finding.detail}
                            </p>
                          </div>
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={!finding.node_ids.length}
                            onClick={() => {
                              setHighlighted(finding.node_ids);
                              select({ kind: "node", id: finding.node_ids[0] });
                            }}
                          >
                            {t("inspect")}
                          </Button>
                        </div>
                        <EvidenceDetails value={finding.evidence} />
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty />
                ))}
              {tab === "frontier" &&
                (result.frontier.length ? (
                  <div className="space-y-2">
                    {result.frontier.map((item, index) => (
                      <article
                        key={`${item.node_id}:${index}`}
                        className="flex flex-wrap items-start justify-between gap-3 rounded-md border p-3"
                      >
                        <div className="min-w-0 flex-1">
                          <p className="text-xs font-medium">
                            {item.reason.replace(/_/g, " ")}
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
                            onClick={() =>
                              select({ kind: "node", id: item.node_id! })
                            }
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
              {tab === "clusters" &&
                (result.clusters.length ? (
                  <div className="grid gap-3 md:grid-cols-2">
                    {result.clusters.map((cluster) => (
                      <article
                        key={cluster.id}
                        className="rounded-md border p-3"
                      >
                        <div className="flex items-center justify-between gap-3">
                          <h3 className="text-sm font-medium">
                            {cluster.label || shortAnalysisId(cluster.id)}
                          </h3>
                          <Button
                            size="sm"
                            variant="ghost"
                            disabled={!cluster.node_ids?.length}
                            onClick={() => {
                              setHighlighted(cluster.node_ids || []);
                              if (cluster.node_ids?.[0])
                                select({
                                  kind: "node",
                                  id: cluster.node_ids[0],
                                });
                            }}
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
              {tab === "patterns" &&
                (result.patterns?.length ? (
                  <div className="space-y-3">
                    {result.patterns.map((pattern) => (
                      <article
                        key={pattern.id}
                        className="rounded-md border p-3"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <h3 className="text-sm font-medium">
                              {pattern.title}
                            </h3>
                            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                              {pattern.detail}
                            </p>
                          </div>
                          <Button
                            size="sm"
                            variant="ghost"
                            onClick={() => {
                              setHighlighted(pattern.node_ids);
                              if (pattern.node_ids[0])
                                select({
                                  kind: "node",
                                  id: pattern.node_ids[0],
                                });
                            }}
                          >
                            {t("inspect")}
                          </Button>
                        </div>
                        <EvidenceDetails value={pattern} />
                      </article>
                    ))}
                  </div>
                ) : (
                  <Empty />
                ))}
              {tab === "exposure" && (
                <>
                  <p className="mb-3 text-xs text-muted-foreground">
                    {t("exposureHelp")}
                  </p>
                  {result.exposure?.length ? (
                    <div className="space-y-3">
                      {result.exposure.map((contact) => (
                        <article
                          key={contact.id}
                          className="rounded-md border p-3"
                        >
                          <div className="flex items-start justify-between gap-3">
                            <div>
                              <h3 className="text-sm font-medium">
                                {String(
                                  contact.claim?.label ||
                                    contact.kind.replace(/_/g, " "),
                                )}
                              </h3>
                              <p className="mt-1 font-mono text-[10px] text-muted-foreground">
                                {contact.kind} · {contact.node_ids.length}{" "}
                                {t("nodes")}
                              </p>
                            </div>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => {
                                setHighlighted(contact.node_ids);
                                if (contact.node_ids[0])
                                  select({
                                    kind: "node",
                                    id: contact.node_ids[0],
                                  });
                              }}
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
                  )}
                </>
              )}
              {tab === "coverage" && (
                <div className="grid gap-4 md:grid-cols-2">
                  <div>
                    <h3 className="text-sm font-medium">{t("coverage")}</h3>
                    <dl className="my-3 grid grid-cols-2 gap-3">
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
                    <EvidenceDetails value={result.coverage} />
                    <EvidenceDetails
                      value={result.query}
                      label={t("resultQuery")}
                    />
                  </div>
                  <div>
                    <h3 className="text-sm font-medium">{t("capabilities")}</h3>
                    <EvidenceDetails value={result.capabilities} />
                    <Fact label={t("snapshot")} value={result.snapshot_id} />
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
 return <p className="py-6 text-center text-xs text-muted-foreground">{t("noItems")}</p>;
}
