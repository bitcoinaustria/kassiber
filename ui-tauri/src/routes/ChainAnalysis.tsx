import { useCallback, useContext, useMemo, useRef, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Download, Eye, Network, Sparkles, X } from "lucide-react";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { Button } from "@/components/ui/button";
import {
  DaemonScopeContext,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import {
  DEFAULT_ANALYSIS_QUERY,
  analysisNodeSubject,
  analysisQueryKey,
  formatAnalysisAmount,
  pathNodeIds,
  shortAnalysisId,
  type AnalysisCase,
  type AnalysisQuery,
  type AnalysisResult,
} from "@/lib/chainAnalysis";
import { exportChainAnalysis } from "@/lib/chainAnalysisExport";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { bookIdentityKey, useUiStore } from "@/store/ui";
import {
  InvestigationGraph,
  type GraphSelection,
} from "./chain-analysis/InvestigationGraph";
import { EvidenceDetails, Fact } from "./chain-analysis/EvidenceDetails";
import {
  AcquisitionPanel,
  EntropyPanel,
  SavedInvestigations,
} from "./chain-analysis/InvestigationPanels";
import { QueryControls } from "./chain-analysis/QueryControls";
import { SelectionInspector } from "./chain-analysis/SelectionInspector";
import { LabelsPanel } from "./chain-analysis/LabelsPanel";
import "./chain-analysis/workbench.css";

export function ChainAnalysis() {
  const { t } = useTranslation("chainAnalysis");
  const identity = useUiStore((state) => state.identity);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const health = useDaemon<{
    workspace: { id: string };
    profile: { id: string };
  }>("ui.workspace.health");
  const search = useRouterState({
    select: (state) => state.location.search,
  }) as Record<string, unknown>;
  const initialSubject =
    typeof search.subject === "string" ? search.subject : "";
  const databaseIdentity = bookIdentityKey(identity) ?? "local";
  const workspaceId = health.data?.data?.workspace?.id,
    profileId = health.data?.data?.profile?.id;
  const boundary = useMemo(
    () =>
      workspaceId && profileId
        ? {
            expectedScope: { workspace_id: workspaceId, profile_id: profileId },
            daemonSession,
            isCurrent: () =>
              useUiStore.getState().daemonSession === daemonSession &&
              (bookIdentityKey(useUiStore.getState().identity) ?? "local") ===
                databaseIdentity,
          }
        : null,
    [workspaceId, profileId, daemonSession, databaseIdentity],
  );
  if (!boundary)
    return (
      <div className={screenShellClassName} role="status">
        {health.isError ? t("scopeError") : t("scopeLoading")}
      </div>
    );
  return (
    <DaemonScopeContext.Provider value={boundary}>
      <ChainAnalysisWorkbench
        key={`${databaseIdentity}:${workspaceId}:${profileId}:${daemonSession}:${initialSubject}`}
        initialSubject={initialSubject}
      />
    </DaemonScopeContext.Provider>
  );
}

export function ChainAnalysisWorkbench({
  initialSubject = "",
}: {
  initialSubject?: string;
}) {
  const { t } = useTranslation("chainAnalysis");
  const [query, setQuery] = useState<AnalysisQuery>({
    ...DEFAULT_ANALYSIS_QUERY,
    ...(initialSubject ? { mode: "trace", subject: initialSubject } : {}),
  });
  const [result, setResult] = useState<AnalysisResult | null>(null);
  const [historical, setHistorical] = useState(false);
  const [selection, setSelection] = useState<GraphSelection | null>(null);
  const [highlighted, setHighlighted] = useState<string[]>([]);
  const [view, setView] = useState<"graph" | "table">("graph");
  const [tab, setTab] = useState<
    | "findings"
    | "frontier"
    | "clusters"
    | "paths"
    | "coverage"
    | "entropy"
    | "labels"
    | "patterns"
    | "exposure"
  >("findings");
  const [showAcquire, setShowAcquire] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestVersion = useRef(0);
  const boundary = useContext(DaemonScopeContext);
  const assistant = useContext(AssistantSessionContext);
  const run = useDaemonMutation<AnalysisResult>("ui.chain_analysis.query", {
    invalidateQueries: false,
  });
  const assistantContext = useDaemonMutation<{
    query: Record<string, unknown>;
    subject?: string;
    snapshot_id: string;
  }>("ui.chain_analysis.ai_context", { invalidateQueries: false });
  const reportError = useCallback(
    (value: unknown) =>
      setError(value instanceof Error ? value.message : String(value)),
    [],
  );
  const execute = async (next: AnalysisQuery = query) => {
    const version = ++requestVersion.current;
    setError(null);
    setNotice(null);
    setQuery(next);
    const args = {
      ...next,
      subject: next.subject?.trim() || undefined,
      target: next.mode === "path" ? next.target?.trim() : undefined,
    };
    try {
      const response = await run.mutateAsync(args);
      if (
        version !== requestVersion.current ||
        boundary?.isCurrent?.() === false
      )
        return;
      if (response.data) {
        setResult(response.data);
        setHistorical(false);
        setSelection(null);
        setHighlighted([]);
      }
    } catch (value) {
      if (
        version === requestVersion.current &&
        boundary?.isCurrent?.() !== false
      )
        reportError(value);
    }
  };
  const select = useCallback((next: GraphSelection) => {
    setSelection(next);
  }, []);
  const node =
    selection?.kind === "node"
      ? result?.nodes.find((item) => item.id === selection.id)
      : undefined;
  const edge =
    selection?.kind === "edge"
      ? result?.edges.find((item) => item.id === selection.id)
      : undefined;
  const pickedSubject = node
    ? analysisNodeSubject(node)
    : result?.query.subject || query.subject || "";
  const trace = (direction: AnalysisQuery["direction"]) => {
    if (node)
      void execute({
        ...query,
        mode: "trace",
        subject: analysisNodeSubject(node),
        target: undefined,
        direction,
      });
  };
  const load = (saved: AnalysisCase) => {
    if (!saved.result || boundary?.isCurrent?.() === false) return;
    ++requestVersion.current;
    setQuery(saved.query);
    setResult(saved.result);
    setHistorical(true);
    setSelection(null);
    setHighlighted([]);
    setError(null);
  };
  const ask = async () => {
    if (
      !result ||
      !assistant ||
      assistant.isStreaming ||
      assistantContext.isPending ||
      boundary?.isCurrent?.() === false
    )
      return;
    try {
      // Chain identities must cross the daemon's provider projection, including UI-authored prompts.
      const response = await assistantContext.mutateAsync({
        query: result.query,
        ...(pickedSubject ? { subject: pickedSubject } : {}),
        expected_snapshot_id: result.snapshot_id,
      });
      if (!response.data || boundary?.isCurrent?.() === false) return;
      const context = response.data;
      const prompt = t("assistantPrompt", {
        subject: context.subject || t("overview"),
        query: JSON.stringify(context.query),
        snapshot: context.snapshot_id,
      });
      const ui = useUiStore.getState();
      ui.setAssistantDockDiscovered(true);
      ui.setAssistantDockMinimized(false);
      ui.setAssistantDockExpanded(true);
      if (assistant.selection?.model) assistant.sendPrompt(prompt);
      else useAssistantDraftStore.getState().setDraft(prompt);
    } catch (value) {
      reportError(value);
    }
  };
  const exportResult = async (format: "json" | "csv") => {
    if (!result) return;
    try {
      const outcome = await exportChainAnalysis(
        result,
        format,
        t("exportTitle"),
      );
      if (outcome !== "cancelled")
        setNotice(outcome === "saved" ? t("exportSaved") : t("exportDownload"));
    } catch (value) {
      reportError(value);
    }
  };
  return (
    <div
      className={`${screenShellClassName} ca-workbench`}
      data-testid="chain-analysis-page"
    >
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="mb-1 flex items-center gap-2 text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
            <Network className="size-3.5" />
            {t("local")}
          </div>
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("title")}
          </h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" asChild>
            <Link to="/privacy-mirror">
              <Eye className="size-4" />
              {t("privacy")}
            </Link>
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void ask()}
            disabled={
              !result ||
              !assistant ||
              assistant.isStreaming ||
              assistantContext.isPending
            }
          >
            <Sparkles className="size-4" />
            {t("ask")}
          </Button>
        </div>
      </header>
      <QueryControls
        query={query}
        setQuery={setQuery}
        busy={run.isPending}
        onRun={() => void execute()}
      />
      {error && (
        <div
          role="alert"
          className="flex items-start justify-between gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
        >
          <span className="break-words">{error}</span>
          <button aria-label={t("dismiss")} onClick={() => setError(null)}>
            <X className="size-4" />
          </button>
        </div>
      )}
      {notice && (
        <p className="rounded-lg border bg-muted/20 p-3 text-xs" role="status">
          {notice}
        </p>
      )}
      {result ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap gap-4">
              <span className="ca-metric">
                <strong>{result.nodes.length}</strong>
                {t("nodes")}
              </span>
              <span className="ca-metric">
                <strong>{result.edges.length}</strong>
                {t("edges")}
              </span>
              <span className="ca-metric">
                <strong>{result.paths.length}</strong>
                {t("paths")}
              </span>
              <button className="ca-metric" onClick={() => setTab("frontier")}>
                <strong>{result.frontier.length}</strong>
                {t("frontier")}
              </button>
              <span
                className={`self-center rounded-md px-2 py-1 text-[11px] ${result.coverage.complete ? "bg-muted text-muted-foreground" : "bg-amber-500/10 text-amber-700 dark:text-amber-300"}`}
              >
                {result.coverage.budget_exhausted
                  ? t("budgetReached")
                  : result.coverage.complete
                    ? t("complete")
                    : t("incomplete")}
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-1">
              <Button
                size="sm"
                variant={view === "graph" ? "secondary" : "ghost"}
                onClick={() => setView("graph")}
              >
                {t("graphView")}
              </Button>
              <Button
                size="sm"
                variant={view === "table" ? "secondary" : "ghost"}
                onClick={() => setView("table")}
              >
                {t("table")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void exportResult("json")}
              >
                <Download className="size-3.5" />
                JSON
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void exportResult("csv")}
              >
                CSV
              </Button>
            </div>
          </div>
          {historical && (
            <p
              className="text-xs text-amber-700 dark:text-amber-300"
              role="status"
            >
              {t("case.historical")}
            </p>
          )}
          {analysisQueryKey(query) !== analysisQueryKey(result.query) && (
            <p
              className="text-xs text-amber-700 dark:text-amber-300"
              role="status"
            >
              {t("queryChanged")}
            </p>
          )}
          <div className="ca-frame">
            <div className="min-w-0">
              {view === "graph" ? (
                result.nodes.length ? (
                  <InvestigationGraph
                    nodes={result.nodes}
                    edges={result.edges}
                    selected={selection}
                    highlightedIds={highlighted}
                    onSelect={select}
                  />
                ) : (
                  <p className="grid min-h-80 place-items-center p-8 text-sm text-muted-foreground">
                    {t("noNodes")}
                  </p>
                )
              ) : (
                <div className="max-h-[560px] overflow-auto">
                  <table className="w-full text-left text-xs">
                    <thead className="sticky top-0 bg-card">
                      <tr>
                        {(["kind", "id", "amount", "network"] as const).map(
                          (key) => (
                            <th className="p-3 font-medium" key={key}>
                              {t(key)}
                            </th>
                          ),
                        )}
                      </tr>
                    </thead>
                    <tbody>
                      {result.nodes.map((item) => (
                        <tr
                          key={item.id}
                          className={
                            selection?.id === item.id
                              ? "bg-primary/10"
                              : "border-t hover:bg-muted/30"
                          }
                        >
                          <td className="p-3">{item.kind}</td>
                          <td className="p-3">
                            <button
                              className="select-text break-all text-left font-mono underline decoration-muted-foreground/40 underline-offset-4"
                              onClick={() =>
                                select({ kind: "node", id: item.id })
                              }
                            >
                              {item.outpoint || item.txid || item.id}
                            </button>
                          </td>
                          <td className="whitespace-nowrap p-3 font-mono">
                            {formatAnalysisAmount(item.amount_msat, item.asset)}
                          </td>
                          <td className="p-3">
                            {item.chain} / {item.network}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
              <div className="flex flex-wrap gap-4 border-t px-3 py-2 text-[10px] text-muted-foreground">
                {(["physical", "custody", "hypothesis"] as const).map(
                  (item) => (
                    <span className="flex items-center gap-2" key={item}>
                      <span
                        style={{
                          borderColor: `var(--ca-${item})`,
                          borderTopStyle:
                            item === "physical" ? "solid" : "dashed",
                        }}
                        className="w-6 border-t-2"
                      />
                      {t(item)}
                    </span>
                  ),
                )}
              </div>
            </div>
            <SelectionInspector
              key={selection?.id || "empty"}
              node={node}
              edge={edge}
              busy={run.isPending}
              onTrace={trace}
              onTarget={(target) =>
                setQuery((previous) => ({ ...previous, mode: "path", target }))
              }
              onSelect={select}
              onError={reportError}
            />
          </div>
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
              {tab === "entropy" && (
                <EntropyPanel
                  key={pickedSubject}
                  subject={pickedSubject}
                  query={result.query}
                  onError={reportError}
                />
              )}
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
          <p className="select-text break-all font-mono text-[10px] text-muted-foreground">
            {t("snapshot")}: {result.snapshot_id}
          </p>
        </>
      ) : (
        <div className="rounded-xl border border-dashed px-6 py-14 text-center">
          <Network className="mx-auto mb-3 size-8 text-muted-foreground/50" />
          <h2 className="text-base font-medium">{t("empty")}</h2>
          <p className="mx-auto mt-2 max-w-xl text-sm text-muted-foreground">
            {t("emptyDetail")}
          </p>
        </div>
      )}
      <SavedInvestigations
        result={result}
        onLoad={load}
        onError={reportError}
        onNotice={setNotice}
      />
      <details
        className="rounded-lg border"
        onToggle={(event) => setShowAcquire(event.currentTarget.open)}
      >
        <summary className="cursor-pointer px-4 py-3 text-sm font-medium">
          {t("acquire.title")}
        </summary>
        {showAcquire && (
          <AcquisitionPanel
            query={query}
            onError={reportError}
            onAcquired={() => setHistorical(true)}
          />
        )}
      </details>
    </div>
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
