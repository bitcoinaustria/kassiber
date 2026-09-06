import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Download, Eye, Network, Sparkles, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  DaemonScopeContext,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import {
  analysisEffectiveLayers,
  analysisNodeSubject,
  analysisQueryKey,
  analysisSubjectNodeId,
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

/** Boundary facts of the executed query, never of the edited draft controls. */
function ExecutedQueryStrip({
  result,
  setTab,
}: {
  result: AnalysisResult;
  setTab: (tab: AnalysisTab) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const query = result.query;
  const layers = analysisEffectiveLayers(query);
  const domain = [query.chain, query.network].filter(Boolean).join(" / ");
  return (
    <div className="ca-strip" aria-label={t("resultQuery")}>
      <span title={t("observer")} className="font-medium text-foreground">
        {t(query.observer)}
      </span>
      <span>
        {t(layers.relations ? "layerOn" : "layerOff", {
          layer: t("relations"),
        })}
      </span>
      <span>
        {t(layers.hypotheses ? "layerOn" : "layerOff", {
          layer: t("hypotheses"),
        })}
      </span>
      {domain && <span className="font-mono">{domain}</span>}
      <button
        type="button"
        className={
          result.coverage.complete
            ? ""
            : "bg-amber-500/10 text-amber-700 dark:text-amber-300"
        }
        onClick={() => setTab("coverage")}
      >
        {result.coverage.budget_exhausted
          ? t("budgetReached")
          : result.coverage.complete
            ? t("complete")
            : t("incomplete")}
      </button>
      <button type="button" onClick={() => setTab("frontier")}>
        <span className="font-mono">{result.frontier.length}</span>{" "}
        {t("frontier")}
      </button>
    </div>
  );
}

export function ChainAnalysisWorkbench({
  initialSearch = {},
}: {
  initialSearch?: AnalysisSearch;
}) {
  const { t } = useTranslation("chainAnalysis");
  const [entryQuery] = useState(() => analysisSearchQuery(initialSearch));
  const [entryWorkspace] = useState(initialSearch.workspace ?? "graph");
  const [query, setQuery] = useState<AnalysisQuery>(entryQuery);
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
  const mounted = useRef(false);
  const [refreshFailed, setRefreshFailed] = useState(false);
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
  const { mutateAsync } = run;
  const execute = useCallback(async (next: AnalysisQuery) => {
    if (!mounted.current || boundary?.isCurrent?.() === false) return;
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
      const response = await mutateAsync(args);
      if (
        !mounted.current || version !== requestVersion.current ||
        boundary?.isCurrent?.() === false
      )
        return;
      if (response.data) {
        setResult(response.data);
        setHistorical(false);
        setRefreshFailed(false);
        setSelection(null);
        setHighlighted([]);
      }
    } catch (value) {
      if (
        mounted.current && version === requestVersion.current &&
        boundary?.isCurrent?.() !== false
      ) {
        setRefreshFailed(true);
        reportError(value);
      }
    }
  }, [boundary, mutateAsync, reportError]);
  useEffect(() => {
    mounted.current = true;
    const requests = requestVersion;
    let cancelled = false;
    // Defer one microtask so StrictMode cleanup cancels its provisional mount.
    // This is only the bounded, cache-only graph read; edits remain explicit.
    if (entryWorkspace === "graph" && (entryQuery.mode === "overview" ||
      (entryQuery.subject?.trim() && (entryQuery.mode !== "path" || entryQuery.target?.trim())))) {
      void Promise.resolve().then(() => {
        if (!cancelled) void execute(entryQuery);
      });
    }
    return () => {
      cancelled = true;
      mounted.current = false;
      ++requests.current;
    };
  }, [entryQuery, entryWorkspace, execute]);
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
  const subjectNodeId = result ? analysisSubjectNodeId(result) : undefined;
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
    setRefreshFailed(false);
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
  const inspecting = Boolean(node || edge);
  return (
    <div
      className={`${screenShellClassName} ca-workbench`}
      data-testid="chain-analysis-page"
    >
      <header className="flex min-w-0 flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 max-w-full flex-wrap items-center gap-3">
          <h1 className="text-2xl font-semibold tracking-tight">
            {t("title")}
          </h1>
          <nav className="flex max-w-full gap-1 overflow-x-auto rounded-lg border bg-card p-1" aria-label={t("title")}>
            {(["graph", "psbt", "datasets"] as const).map(item => <Button key={item} size="sm" variant={workspace === item ? "secondary" : "ghost"} aria-pressed={workspace === item} onClick={() => setWorkspace(item)}>{t(`workbench.${item}`)}</Button>)}
          </nav>
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
      <div hidden={workspace !== "psbt"}><PsbtPanel initialNetwork={initialSearch.network} onError={reportError} /></div>
      <div hidden={workspace !== "datasets"}><DatasetsPanel onError={reportError} /></div>
      <div hidden={workspace !== "graph"} className="space-y-3">
      <QueryControls
        query={query}
        setQuery={setQuery}
        busy={run.isPending}
        onRun={() => void execute(query)}
      />
      {run.isPending && <p role="status" className="text-sm text-muted-foreground">{t("running")}</p>}
      {result && refreshFailed && <p role="status" className="text-sm text-amber-700 dark:text-amber-300">{t("refreshFailed")}</p>}
      {result ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <ExecutedQueryStrip result={result} setTab={setTab} />
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
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button size="sm" variant="ghost">
                    <Download className="size-3.5" />
                    {t("export")}
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onSelect={() => void exportResult("json")}>
                    {t("exportJson")}
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => void exportResult("csv")}>
                    {t("exportCsv")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
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
          <div className={`ca-frame${inspecting ? " has-inspector" : ""}`}>
            <div className="min-w-0">
              {view === "graph" ? (
                result.nodes.length ? (
                  <InvestigationGraph
                    nodes={result.nodes}
                    edges={result.edges}
                    selected={selection}
                    highlightedIds={highlighted}
                    subjectId={subjectNodeId}
                    onSelect={select}
                  />
                ) : (
                  <p className="grid min-h-80 place-items-center p-8 text-sm text-muted-foreground">
                    {t("noNodes")}
                  </p>
                )
              ) : (
                <div className="max-h-[520px] overflow-auto">
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
              <div className="flex flex-wrap items-center gap-4 border-t px-3 py-2 text-[10px] text-muted-foreground">
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
                {!inspecting && result.nodes.length > 0 && (
                  <span className="ml-auto">{t("select")}</span>
                )}
              </div>
            </div>
            {inspecting && (
              <SelectionInspector
                key={selection?.id}
                node={node}
                edge={edge}
                features={result.transaction_features?.find(item => item.subject === node?.id)?.features}
                busy={run.isPending}
                onTrace={trace}
                onTarget={(target) =>
                  setQuery((previous) => ({ ...previous, mode: "path", target }))
                }
                onSelect={select}
                onClose={() => setSelection(null)}
                onError={reportError}
              />
            )}
          </div>
          <ResultPanels
            result={result}
            tab={tab}
            setTab={setTab}
            node={node}
            pickedSubject={pickedSubject}
            select={select}
            setHighlighted={setHighlighted}
            reportError={reportError}
            onAcquire={() => setShowAcquire(true)}
          />
        </>
      ) : (
        <div className="flex items-center justify-center gap-3 rounded-xl border border-dashed px-6 py-10 text-sm text-muted-foreground">
          <Network className="size-5 text-muted-foreground/60" />
          {t("empty")}
        </div>
      )}
      <details className="rounded-lg border">
        <summary className="cursor-pointer px-4 py-3 text-sm font-medium">
          {t("case.title")}
        </summary>
        <SavedInvestigations
          result={result}
          onLoad={load}
          onError={reportError}
          onNotice={setNotice}
        />
      </details>
      <details
        className="rounded-lg border"
        open={showAcquire}
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
