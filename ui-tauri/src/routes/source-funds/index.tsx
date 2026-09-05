import { useContext, useMemo } from "react";
import { useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { TransactionDetailController } from "@/components/transactions/dashboard/TransactionDetailController";
import { Button } from "@/components/ui/button";
import { DaemonScopeContext, useDaemon } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { bookIdentityKey, useUiStore } from "@/store/ui";
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
  const status = !state.selectedTarget ? "selectTarget" : state.resolvedTarget.isError || (state.resolvedTarget.isSuccess && !state.selectedTxId) ? "targetUnavailable" : state.preview.isError ? "reviewUnavailable" : state.preview.isFetching || !state.report ? "loading" : state.report.explain_gates.exportable ? "exportable" : "needsEvidence";
  return <div className={`${screenShellClassName} space-y-5`}>
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div><h1 className="text-xl font-semibold tracking-tight">{t("header.title")}</h1><p className="mt-1 max-w-2xl text-sm text-muted-foreground">{t("case.description")}</p></div>
      <Button onClick={investigate} disabled={!context || state.preview.isFetching || !assistant || assistant.isStreaming}>
        <Sparkles className="size-4" aria-hidden="true" />{t("case.investigate")}
      </Button>
    </header>
    <div className="rounded-lg border bg-muted/20 px-4 py-3" role="status">
      <p className="text-sm font-medium">{t(`case.${status}`)}</p>
      {state.selectedTarget && <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{state.selectedTarget}{state.targetAmount ? ` · ${state.targetAmount} ${state.selectedTx?.asset ?? "BTC"}` : ` · ${t("case.fullAmount")}`}</p>}
      {state.report && <p className="mt-1 text-xs text-muted-foreground">{t("case.findings", { blockers: state.blockers.length, warnings: state.warnings.length })}</p>}
    </div>
    <details className="rounded-lg border p-4" open={!state.selectedTarget || state.stage === "target"}>
      <summary className="cursor-pointer text-sm font-medium">{t("case.targetSection")}</summary><div className="pt-4"><TargetStage state={state} /></div>
    </details>
    {context?.scope_truncated && <p role="status" className="text-sm text-amber-700 dark:text-amber-300">{t("case.scopeTruncated")}</p>}
    {state.exportError && <p role="alert" className="text-sm text-destructive">{t("case.exportError")}</p>}
    {state.selectedTarget && <>
      <section className="rounded-lg border p-4"><TraceStage state={state} /></section>
      <details className="rounded-lg border p-4" onToggle={(event) => state.setShowDisclosure(event.currentTarget.open)}><summary className="cursor-pointer text-sm font-medium">{t("case.disclosureSection")}</summary><div className="pt-4"><DiscloseStage state={state} /></div></details>
      <details className="rounded-lg border p-4"><summary className="cursor-pointer text-sm font-medium">{t("case.exportSection")}</summary><div className="pt-4"><ExportStage state={state} /></div></details>
    </>}
    <TransactionDetailController transaction={state.detailTransaction} hideSensitive={state.hideSensitive} currency={state.currency} explorerSettings={state.explorerSettings} onOpenChange={(open) => { if (!open) state.setDetailTransaction(null); }} />
  </div>;
}
