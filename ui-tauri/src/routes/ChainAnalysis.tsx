import { useCallback, useContext, useMemo, useRef, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Download, Eye, Network, Sparkles, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DaemonScopeContext,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import {
  analysisNodeSubject,
  analysisQueryKey,
  formatAnalysisAmount,
  type AnalysisCase,
  type AnalysisQuery,
  type AnalysisResult,
} from "@/lib/chainAnalysis";
import { analysisSearchQuery, parseAnalysisSearch, type AnalysisSearch, type AnalysisTab, type AnalysisWorkspace } from "@/lib/chainAnalysisNavigation";
import { exportChainAnalysis } from "@/lib/chainAnalysisExport";
import { useChainAnalysisAssistant } from "@/hooks/useChainAnalysisAssistant";
import { bookIdentityKey, useUiStore } from "@/store/ui";
import {
  InvestigationGraph,
  type GraphSelection,
} from "./chain-analysis/InvestigationGraph";
import {
  AcquisitionPanel,
  SavedInvestigations,
} from "./chain-analysis/InvestigationPanels";
import { QueryControls } from "./chain-analysis/QueryControls";
import { SelectionInspector } from "./chain-analysis/SelectionInspector";
import { ResultPanels } from "./chain-analysis/ResultPanels";
import { PsbtPanel } from "./chain-analysis/PsbtPanel";
import { DatasetsPanel } from "./chain-analysis/DatasetsPanel";
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
  const initialSearch = parseAnalysisSearch(search);
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
        key={`${databaseIdentity}:${workspaceId}:${profileId}:${daemonSession}:${JSON.stringify(initialSearch)}`}
        initialSearch={initialSearch}
      />
    </DaemonScopeContext.Provider>
  );
}

export function ChainAnalysisWorkbench({
  initialSearch = {},
}: {
  initialSearch?: AnalysisSearch;
}) {
  const { t } = useTranslation("chainAnalysis");
  const [query, setQuery] = useState<AnalysisQuery>(() => analysisSearchQuery(initialSearch));
  const [result, setResult] = useState<AnalysisResult | null>(null);
  const [historical, setHistorical] = useState(false);
  const [selection, setSelection] = useState<GraphSelection | null>(null);
  const [highlighted, setHighlighted] = useState<string[]>([]);
  const [view, setView] = useState<"graph" | "table">("graph");
  const [tab, setTab] = useState<AnalysisTab>(initialSearch.tab ?? "findings");
  const [showAcquire, setShowAcquire] = useState(false);
  const [workspace, setWorkspace] = useState<AnalysisWorkspace>(initialSearch.workspace ?? "graph");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestVersion = useRef(0);
  const boundary = useContext(DaemonScopeContext);
  const run = useDaemonMutation<AnalysisResult>("ui.chain_analysis.query", {
    invalidateQueries: false,
  });
  const reportError = useCallback(
    (value: unknown) =>
      setError(value instanceof Error ? value.message : String(value)),
    [],
  );
  const assistant = useChainAnalysisAssistant(reportError);
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
  const ask = () => result ? assistant.ask({ query: result.query, snapshot_id: result.snapshot_id }, pickedSubject || undefined) : undefined;
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
              workspace !== "graph" || !result ||
              !assistant.available || assistant.busy
            }
          >
            <Sparkles className="size-4" />
            {t("ask")}
          </Button>
        </div>
      </header>
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
      <nav className="flex gap-1 overflow-auto rounded-lg border bg-card p-1" aria-label={t("title")}>
        {(["graph", "psbt", "datasets"] as const).map(item => <Button key={item} size="sm" variant={workspace === item ? "secondary" : "ghost"} aria-pressed={workspace === item} onClick={() => setWorkspace(item)}>{t(`workbench.${item}`)}</Button>)}
      </nav>
      <div hidden={workspace !== "psbt"}><PsbtPanel onError={reportError} /></div>
      <div hidden={workspace !== "datasets"}><DatasetsPanel onError={reportError} /></div>
      <div hidden={workspace !== "graph"} className="space-y-4">
      <QueryControls
        query={query}
        setQuery={setQuery}
        busy={run.isPending}
        onRun={() => void execute()}
      />
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
              features={result.transaction_features?.find(item => item.subject === node?.id)?.features}
              busy={run.isPending}
              onTrace={trace}
              onTarget={(target) =>
                setQuery((previous) => ({ ...previous, mode: "path", target }))
              }
              onSelect={select}
              onError={reportError}
            />
          </div>
          <ResultPanels result={result} tab={tab} setTab={setTab} node={node} pickedSubject={pickedSubject} select={select} setHighlighted={setHighlighted} reportError={reportError} />
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
    </div>
  );
}
