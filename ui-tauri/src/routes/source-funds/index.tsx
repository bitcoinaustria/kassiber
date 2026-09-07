import { useContext, useEffect, useMemo, useState } from "react";
import { useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ArrowLeft, ArrowRight, Check } from "lucide-react";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { TransactionDetailController } from "@/components/transactions/dashboard/TransactionDetailController";
import { Button } from "@/components/ui/button";
import { DaemonScopeContext, useDaemon } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { bookIdentityKey, useUiStore } from "@/store/ui";
import { formatBtc } from "./model";
import { canApproveSourceFundsPreview, isReviewedSourceFundsPreview } from "./journey";
import { sourceFundsDraftKey } from "./caseScope";
import { DiscloseStage, ExportStage, TargetStage, TraceStage } from "./stages";
import { useSourceFundsCase } from "./useSourceFundsCase";

/** Resolve canonical scope before mounting draft state; switching books destroys pending UI work. */
export function SourceFunds() {
  const { t } = useTranslation("sourceFunds");
  const identity = useUiStore((state) => state.identity);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const health = useDaemon<{ workspace: { id: string }; profile: { id: string } }>("ui.workspace.health");
  const search = useRouterState({ select: (state) => state.location.search }) as Record<string, unknown>;
  const initialTarget = typeof search.tx === "string" ? search.tx : typeof search.transaction === "string" ? search.transaction : "";
  const databaseIdentity = bookIdentityKey(identity) ?? "local";
  const workspaceId = health.data?.data?.workspace?.id;
  const profileId = health.data?.data?.profile?.id;
  const boundary = useMemo(() => workspaceId && profileId ? {
    expectedScope: { workspace_id: workspaceId, profile_id: profileId }, daemonSession,
    isCurrent: () => useUiStore.getState().daemonSession === daemonSession && (bookIdentityKey(useUiStore.getState().identity) ?? "local") === databaseIdentity,
  } : null, [workspaceId, profileId, daemonSession, databaseIdentity]);
  if (!boundary) return <div className={screenShellClassName} role="status">{health.isError ? t("case.scopeError") : t("case.loading")}</div>;
  const draftKey = sourceFundsDraftKey(databaseIdentity, boundary.expectedScope);
  return <DaemonScopeContext.Provider value={boundary}>
    <SourceFundsCase key={`${draftKey}:${daemonSession}:${initialTarget}`} draftKey={draftKey} initialTarget={initialTarget} />
  </DaemonScopeContext.Provider>;
}

function SourceFundsCase({ draftKey, initialTarget }: { draftKey: string; initialTarget: string }) {
  const { t } = useTranslation("sourceFunds");
  const state = useSourceFundsCase(draftKey, initialTarget);
  const assistant = useContext(AssistantSessionContext);
  const boundary = useContext(DaemonScopeContext);
  const context = state.preview.data?.data;
  const investigate = () => {
    if (!assistant || assistant.isStreaming || !context || state.preview.isFetching || boundary?.isCurrent?.() === false) return;
    const prompt = t("case.assistantPrompt", { target: context.target.transaction_id, recipe: JSON.stringify(context.recipe) });
    const ui = useUiStore.getState();
    ui.setAssistantDockDiscovered(true); ui.setAssistantDockMinimized(false); ui.setAssistantDockExpanded(true);
    if (assistant.selection?.model) assistant.sendPrompt(prompt);
    else useAssistantDraftStore.getState().setDraft(prompt);
  };
  const [reviewedFingerprint, setReviewedFingerprint] = useState<string | null>(null);
  const fingerprint = context?.review_fingerprint;
  const current = Boolean(state.report && context && !state.preview.isFetching && !state.preview.isError && !state.resolvedTarget.isError && !context?.scope_truncated);
  const previewReady = canApproveSourceFundsPreview({ current, exportable: Boolean(state.report?.explain_gates.exportable), diagramLoading: state.diagramQuery.isFetching || !state.diagramQuery.data, diagramError: state.diagramQuery.isError, fingerprint });
  const canExport = isReviewedSourceFundsPreview(current, Boolean(state.report?.explain_gates.exportable), fingerprint, reviewedFingerprint);
  // Refetching after save disables actions without bouncing between steps. A changed
  // fingerprint, failed refresh or persisted navigation still requires a fresh review.
  const reviewStillMatches = isReviewedSourceFundsPreview(!state.preview.isError && !state.resolvedTarget.isError && !context?.scope_truncated, Boolean(state.report?.explain_gates.exportable), fingerprint, reviewedFingerprint);
  const stage = !state.selectedTarget ? "target" : state.stage === "export" && !reviewStillMatches ? "disclose" : state.stage;
  const { setShowDisclosure } = state;
  useEffect(() => { setShowDisclosure(stage === "disclose" || stage === "export"); }, [stage, setShowDisclosure]);
  const steps = ["target", "trace", "disclose", "export"] as const;
  const position = steps.indexOf(stage);
  const forward = () => {
    if (stage === "target" && state.selectedTxId) state.goToStage("trace");
    else if (stage === "trace" && current) state.goToStage("disclose");
    else if (stage === "disclose" && previewReady && fingerprint) {
      setReviewedFingerprint(fingerprint);
      state.goToStage("export");
    }
  };
  const status = !state.selectedTarget ? "selectTarget" : state.resolvedTarget.isError || (state.resolvedTarget.isSuccess && !state.selectedTxId) ? "targetUnavailable" : state.preview.isError ? "reviewUnavailable" : state.preview.isFetching || !state.report ? "loading" : state.report.explain_gates.exportable ? "exportable" : "needsEvidence";
  return <div className={`${screenShellClassName} mx-auto max-w-6xl space-y-5`}>
    <header className="flex flex-wrap items-start justify-between gap-3">
      <div><h1 className="text-xl font-semibold tracking-tight">{t("header.title")}</h1><p className="mt-1 text-sm text-muted-foreground">{t("journey.description")}</p></div>
    </header>
    <nav aria-label={t("header.title")}>
      <ol className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {steps.map((step, index) => <li key={step}><button type="button" aria-current={stage === step ? "step" : undefined}
          disabled={index > 0 && !state.selectedTxId || step === "export" && !canExport}
          onClick={() => state.goToStage(step)}
          className={`flex w-full items-center gap-2 rounded-lg border px-3 py-3 text-left text-sm disabled:opacity-40 ${stage === step ? "border-primary/40 bg-primary/5 font-semibold" : "border-transparent text-muted-foreground hover:bg-muted"}`}>
          <span className="flex size-6 shrink-0 items-center justify-center rounded-full border text-xs">{index < position ? <Check className="size-3" /> : index + 1}</span>{t(`journey.${step}`)}
        </button></li>)}
      </ol>
    </nav>
    {stage !== "target" && <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg bg-muted/30 px-4 py-3">
      <div><p className="text-sm font-medium">{state.selectedTx?.note || state.selectedTx?.description || state.selectedTx?.wallet || state.report?.target.wallet || t("case.targetSection")}</p>
        <p className="text-xs text-muted-foreground">{formatBtc(state.report?.target.required_amount ?? state.selectedTx?.amount, state.selectedTx?.asset ?? "BTC")} · {t(`case.${status}`)}</p></div>
      <Button variant="ghost" size="sm" onClick={() => state.goToStage("target")}>{t("journey.changeTarget")}</Button>
    </div>}
    {context?.scope_truncated && <p role="status" className="text-sm text-amber-700 dark:text-amber-300">{t("case.scopeTruncated")}</p>}
    {state.selectedTarget && state.preview.isError && <div role="alert" className="rounded-lg border border-destructive/30 p-4 text-sm"><p>{t("journey.targetError")}</p><Button variant="outline" size="sm" className="mt-2" onClick={() => void state.preview.refetch()}>{t("journey.retry")}</Button></div>}
    {stage === "disclose" && (state.diagramQuery.isError ? <div role="alert" className="rounded-lg border p-4 text-sm"><p>{t("journey.previewError")}</p><Button variant="outline" size="sm" onClick={() => void state.diagramQuery.refetch()}>{t("journey.retry")}</Button></div> : state.diagramQuery.isFetching && <p role="status" className="text-sm text-muted-foreground">{t("case.loading")}</p>)}
    {stage === "disclose" && state.report && !state.report.explain_gates.exportable && <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 text-sm" role="status"><p>{t("case.exportBlocked")}</p><Button variant="outline" size="sm" onClick={() => state.goToStage("trace")}>{t("journey.resolveQuestions")}</Button></div>}
    {state.exportError && <p role="alert" className="text-sm text-destructive">{t("case.exportError")}</p>}
    <section className="rounded-xl border bg-card p-4 sm:p-6" aria-label={t(`journey.${stage}`)}>
      {stage === "target" && <TargetStage state={state} />}
      {stage === "trace" && <TraceStage state={state} onInvestigate={investigate} assistantAvailable={Boolean(context && !state.preview.isFetching && assistant && !assistant.isStreaming)} />}
      {stage === "disclose" && <DiscloseStage state={state} />}
      {stage === "export" && <ExportStage state={state} />}
    </section>
    <footer className="flex flex-wrap items-center justify-between gap-3 border-t pt-4">
      <Button variant="ghost" disabled={position === 0} onClick={() => state.goToStage(steps[position - 1])}><ArrowLeft className="size-4" />{t("journey.back")}</Button>
      {stage !== "export" && <Button onClick={forward} disabled={stage === "target" ? !state.selectedTxId || state.resolvedTarget.isFetching : !current || stage === "disclose" && !previewReady}>
        {t(stage === "disclose" ? "journey.approvePreview" : stage === "trace" ? "journey.openPreview" : "journey.traceTarget")}<ArrowRight className="size-4" />
      </Button>}
    </footer>
    <TransactionDetailController transaction={state.detailTransaction} hideSensitive={state.hideSensitive} currency={state.currency} explorerSettings={state.explorerSettings} onOpenChange={(open) => { if (!open) state.setDetailTransaction(null); }} />
  </div>;
}
