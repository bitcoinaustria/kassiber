import { useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ArrowUpRight, Eye, RefreshCw, Sparkles } from "lucide-react";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DaemonScopeContext, useDaemon } from "@/daemon/client";
import { analysisInvestigationSearch, type AnalysisTab, type AnalysisWorkspace } from "@/lib/chainAnalysisNavigation";
import { formatUiNumber } from "@/lib/localeFormat";
import { groupPrivacyFindings, type PrivacyFinding, type PrivacyInvestigation, type PrivacyMirrorPayload } from "@/lib/privacyMirror";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useChainAnalysisAssistant } from "@/hooks/useChainAnalysisAssistant";
import { bookIdentityKey, useUiStore } from "@/store/ui";

const readableCode = (code: string) => code.replace(/_/g, " ");

function ExposureFinding({ finding, onInvestigate, onAsk, asking }: {
  finding: PrivacyFinding;
  onInvestigate?: (investigation: PrivacyInvestigation, workspace?: AnalysisWorkspace, tab?: AnalysisTab) => void;
  onAsk?: (investigation?: PrivacyInvestigation) => void;
  asking?: boolean;
}) {
  const { t } = useTranslation("privacyMirror");
  const contextOnly = finding.relevance === "received_context" || finding.relevance === "nearby_context";
  return (
    <article className={cn("rounded-lg border bg-card p-4", !contextOnly && finding.severity === "warning" && "border-amber-500/35")} data-testid="privacy-finding" data-relevance={finding.relevance}>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <span>{t(`category.${finding.category}`)}</span>
        <span aria-hidden="true">·</span>
        <span>{t(`relevance.${finding.relevance}`)}</span>
        <Badge variant="outline" className="ml-auto text-[10px]">{t(`authority.${finding.authority}`, { defaultValue: readableCode(finding.authority) })}</Badge>
      </div>
      <h3 className="mt-2 text-sm font-semibold">{t(`findingTitle.${finding.code}`, { defaultValue: finding.title })}</h3>
      <p className="mt-1 max-w-4xl text-sm text-muted-foreground">{t(`findingDetail.${finding.code}`, { defaultValue: finding.detail })}</p>
      {finding.affected_output_count > 0 && <p className="mt-2 text-xs text-muted-foreground">{t("affectedOutputs", { count: finding.affected_output_count })}</p>}
      {(finding.assumptions.length > 0 || finding.limitations.length > 0) && (
        <details className="mt-3 text-xs">
          <summary className="cursor-pointer font-medium text-muted-foreground">{t("findingBasis")}</summary>
          <ul className="mt-2 list-disc space-y-1 pl-4 text-muted-foreground">
            {[...finding.assumptions, ...finding.limitations].map((statement, index) => <li key={index}>{t(`statement.${statement}`, { defaultValue: statement })}</li>)}
          </ul>
        </details>
      )}
      {(onInvestigate || onAsk) && <div className="mt-3 flex flex-wrap gap-2">
        {onInvestigate && <Button variant="outline" size="sm" onClick={() => onInvestigate(finding.investigation, "graph", finding.category === "attribution" ? "exposure" : finding.category === "pattern" ? "patterns" : "findings")}><ArrowUpRight className="size-3.5" />{t("investigate")}</Button>}
        {onAsk && <Button variant="ghost" size="sm" disabled={asking} onClick={() => onAsk(finding.investigation)}><Sparkles className="size-3.5" />{t("explain")}</Button>}
      </div>}
    </article>
  );
}

export function PrivacyMirrorPayloadView({ payload, onRefresh, refreshing = false, refreshError, onInvestigate, onAsk, asking = false, actionError }: {
  payload: PrivacyMirrorPayload;
  onRefresh?: () => void;
  refreshing?: boolean;
  refreshError?: string | null;
  onInvestigate?: (investigation: PrivacyInvestigation, workspace?: AnalysisWorkspace, tab?: AnalysisTab) => void;
  onAsk?: (investigation?: PrivacyInvestigation) => void;
  asking?: boolean;
  actionError?: string | null;
}) {
  const { t } = useTranslation("privacyMirror");
  const summary = payload.summary;
  const coverage = payload.coverage;
  const reasonText = (code: string) => t(`reason.${code}`, { defaultValue: t(`reason.${code.replace(/^entropy_/, "")}`, { defaultValue: readableCode(code) }) });
  const attention = payload.findings.filter(finding => finding.relevance === "own_spend" || finding.relevance === "owned_output");
  const context = payload.findings.filter(finding => finding.relevance === "received_context" || finding.relevance === "nearby_context");
  const groups = [{ id: "attention", findings: attention }, { id: "context", findings: context }] as const;
  const investigate = (workspace: AnalysisWorkspace = "graph", tab: AnalysisTab = "findings") => onInvestigate?.(payload.investigation, workspace, tab);
  return (
    <div className={screenShellClassName} data-testid="privacy-mirror-page">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="mb-1 flex items-center gap-1.5 text-xs text-muted-foreground"><Eye className="size-3.5" />{t("observer")}</div>
          <h1 className="text-2xl font-semibold tracking-tight">{t("title")}</h1>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">{t("subtitle")}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          {onAsk && <Button variant="outline" size="sm" disabled={asking} onClick={() => onAsk()}><Sparkles className="size-3.5" />{t("ask")}</Button>}
          {onInvestigate && <Button variant="outline" size="sm" onClick={() => investigate()}><ArrowUpRight className="size-3.5" />{t("workbench")}</Button>}
          {onRefresh && <Button variant="ghost" size="icon-sm" aria-label={t("refresh")} disabled={refreshing} onClick={onRefresh}><RefreshCw className={cn("size-4", refreshing && "animate-spin")} /></Button>}
        </div>
      </header>
      {refreshError && <div role="alert" className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-3 text-sm"><p className="font-medium">{t("refreshFailed")}</p><p className="mt-1 text-muted-foreground">{refreshError}</p></div>}
      {actionError && <p role="alert" className="text-sm text-destructive">{actionError}</p>}
      <section className="rounded-xl border bg-card p-5" data-testid="privacy-mirror-summary">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <h2 className="text-lg font-semibold">{t(`summary.${summary.status}`, { count: summary.attention_count })}</h2>
          <Badge variant="outline">{t(`coverage.status.${coverage.status}`)}</Badge>
        </div>
        <p className="mt-2 max-w-3xl text-sm text-muted-foreground">{t(`summaryBody.${summary.status}`)}</p>
        <dl className="mt-5 grid gap-4 border-t pt-4 sm:grid-cols-3">
          <div><dt className="text-xs text-muted-foreground">{t("population.outputs")}</dt><dd className="mt-1 font-mono text-lg tabular-nums">{formatUiNumber(summary.owned_output_count)}</dd></div>
          <div><dt className="text-xs text-muted-foreground">{t("population.transactions")}</dt><dd className="mt-1 font-mono text-lg tabular-nums">{formatUiNumber(summary.analyzed_transaction_count)} <span className="text-sm text-muted-foreground">/ {formatUiNumber(summary.local_transaction_count)}</span></dd></div>
          <div><dt className="text-xs text-muted-foreground">{t("population.domains")}</dt><dd className="mt-1 font-mono text-lg tabular-nums">{formatUiNumber(summary.domain_count)}</dd></div>
        </dl>
      </section>
      {groups.map(group => group.findings.length > 0 && <section key={group.id} className="space-y-2" data-testid={`privacy-mirror-${group.id}`}>
        <div><h2 className="text-sm font-semibold">{t(`groups.${group.id}`)} <span className="font-mono text-muted-foreground">{group.findings.length}</span></h2><p className="mt-1 text-xs text-muted-foreground">{t(`groups.${group.id}Body`)}</p></div>
        {groupPrivacyFindings(group.findings).map(findings => findings.length === 1 ? (
          <ExposureFinding key={findings[0].id} finding={findings[0]} onInvestigate={onInvestigate} onAsk={onAsk} asking={asking} />
        ) : (
          <details key={findings[0].id} className="rounded-lg border bg-card">
            <summary className="cursor-pointer px-4 py-3 text-sm font-medium">
              {t(`findingTitle.${findings[0].code}`, { defaultValue: findings[0].title })}
              <span className="ml-2 text-xs font-normal text-muted-foreground">{t("repeatedFindings", { count: findings.length })}</span>
            </summary>
            <div className="space-y-2 border-t p-3">
              {findings.map(finding => <ExposureFinding key={finding.id} finding={finding} onInvestigate={onInvestigate} onAsk={onAsk} asking={asking} />)}
            </div>
          </details>
        ))}
      </section>)}
      <section className="rounded-lg border bg-card p-4" data-testid="privacy-mirror-coverage">
        <h2 className="text-sm font-semibold">{t("coverage.title")}</h2>
        <p className="mt-1 text-xs text-muted-foreground">{t("coverage.body")}</p>
        <div className="mt-3 divide-y">
          {coverage.checks.map(check => <div key={check.code} className="flex flex-wrap items-start justify-between gap-2 py-2.5 text-sm">
            <div className="min-w-0"><p>{t(`checks.${check.code}`, { defaultValue: readableCode(check.code) })}</p>{check.reason && <p className="mt-1 text-xs text-muted-foreground">{t(`reason.${check.reason}`, { defaultValue: readableCode(check.reason) })}</p>}</div>
            <div className="flex shrink-0 items-center gap-3 text-xs"><span className="font-mono text-muted-foreground">{formatUiNumber(check.evaluated)} / {formatUiNumber(check.eligible)}</span><Badge variant="outline">{t(`checkStatus.${check.status}`)}</Badge></div>
          </div>)}
        </div>
        {(coverage.missing_nodes > 0 || coverage.stale_nodes > 0 || coverage.conflicting_nodes > 0 || coverage.truncated) && <p className="mt-3 text-xs text-muted-foreground">{t("coverage.gaps", { missing: coverage.missing_nodes, stale: coverage.stale_nodes, conflicting: coverage.conflicting_nodes })}{coverage.truncated ? ` ${t("coverage.truncated")}` : ""}</p>}
        {coverage.stopped_reasons.length > 0 && <ul className="mt-2 list-disc space-y-1 pl-4 text-xs text-muted-foreground">{coverage.stopped_reasons.map(reason => <li key={reason}>{reasonText(reason)}</li>)}</ul>}
        {payload.entropy.results.length > 0 && <details className="mt-3 border-t pt-3">
          <summary className="cursor-pointer text-xs font-medium">{t("entropy.title", { count: payload.entropy.evaluated })}</summary>
          <p className="mt-2 text-xs text-muted-foreground">{t("entropy.body")}</p>
          <div className="mt-2 space-y-2">{payload.entropy.results.map((result, index) => <div key={`${result.subject}:${index}`} className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-muted/30 p-2 text-xs">
            <span>{result.status === "exact" && result.interpretation_count != null ? t("entropy.interpretations", { countText: result.interpretation_count }) : ["budget_exhausted", "timeout", "model_bounded"].includes(result.status) && result.interpretation_count == null && result.interpretation_count_lower_bound != null ? t("entropy.lowerBound", { countText: result.interpretation_count_lower_bound }) : t(`reason.${result.reason || result.status}`, { defaultValue: readableCode(result.reason || result.status) })}</span>
            {onInvestigate && <Button size="sm" variant="ghost" onClick={() => onInvestigate(result.investigation, "graph", "entropy")}>{t("entropy.inspect")}<ArrowUpRight className="size-3" /></Button>}
          </div>)}</div>
          {payload.entropy.omitted > 0 && <p className="mt-2 text-xs text-muted-foreground">{t("entropy.omitted", { count: payload.entropy.omitted })}</p>}
        </details>}
        <details className="mt-3 border-t pt-3">
          <summary className="cursor-pointer text-xs font-medium">{t("assumptions")}</summary>
          <ul className="mt-2 list-disc space-y-1 pl-4 text-xs text-muted-foreground">{payload.assumptions.map((statement, index) => <li key={index}>{t(`statement.${statement}`, { defaultValue: statement })}</li>)}</ul>
        </details>
      </section>
      {onInvestigate && <footer className="flex flex-wrap items-center gap-x-4 gap-y-2 pb-2 text-xs text-muted-foreground">
        <span>{t("toolsElsewhere")}</span>
        <Button variant="link" size="sm" className="h-auto p-0" onClick={() => investigate("psbt")}>{t("checkSpend")}<ArrowUpRight className="size-3" /></Button>
        <Button variant="link" size="sm" className="h-auto p-0" onClick={() => investigate("datasets")}>{t("datasets")}<ArrowUpRight className="size-3" /></Button>
      </footer>}
    </div>
  );
}

function ScopedPrivacyMirror() {
  const { t } = useTranslation("privacyMirror");
  const navigate = useNavigate();
  const [actionError, setActionError] = useState<string | null>(null);
  const query = useDaemon<PrivacyMirrorPayload>("ui.reports.privacy_mirror", undefined, { refetchOnMount: "always" });
  const assistant = useChainAnalysisAssistant(error => setActionError(error instanceof Error ? error.message : String(error)), t("assistantPrompt"));
  const payload = query.data?.data;
  if (query.isLoading && !payload) return <ScreenSkeleton titleWidth="w-48" />;
  if (!payload || payload.payload_schema_version !== 2) return <ScreenNotice title={t("unavailable.title")} body={query.error instanceof Error ? query.error.message : t("unavailable.body")} />;
  return <PrivacyMirrorPayloadView payload={payload} onRefresh={() => void query.refetch()} refreshing={query.isFetching} refreshError={query.isError ? query.error instanceof Error ? query.error.message : t("unavailable.body") : null} onInvestigate={(investigation, workspace, tab) => void navigate({ to: "/chain-analysis", search: analysisInvestigationSearch(investigation.query, workspace, tab) })} onAsk={assistant.available ? investigation => { setActionError(null); void assistant.ask(investigation); } : undefined} asking={assistant.busy} actionError={actionError} />;
}

export function PrivacyMirror() {
  const { t } = useTranslation("privacyMirror");
  const identity = useUiStore(state => state.identity);
  const daemonSession = useUiStore(state => state.daemonSession);
  const health = useDaemon<{ workspace: { id: string }; profile: { id: string } }>("ui.workspace.health");
  const databaseIdentity = bookIdentityKey(identity) ?? "local";
  const workspaceId = health.data?.data?.workspace?.id, profileId = health.data?.data?.profile?.id;
  const boundary = useMemo(() => workspaceId && profileId ? {
    expectedScope: { workspace_id: workspaceId, profile_id: profileId },
    daemonSession,
    isCurrent: () => useUiStore.getState().daemonSession === daemonSession && (bookIdentityKey(useUiStore.getState().identity) ?? "local") === databaseIdentity,
  } : null, [workspaceId, profileId, daemonSession, databaseIdentity]);
  if (!boundary) return health.isError ? <ScreenNotice title={t("unavailable.title")} body={t("unavailable.body")} /> : <ScreenSkeleton titleWidth="w-48" />;
  return <DaemonScopeContext.Provider value={boundary}><ScopedPrivacyMirror key={`${databaseIdentity}:${workspaceId}:${profileId}:${daemonSession}`} /></DaemonScopeContext.Provider>;
}
