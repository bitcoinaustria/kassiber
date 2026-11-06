import * as React from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ReviewTaxDeltaView } from "@/components/ai/ReviewWorkflowCard";
import { reviewArtifact, type ReviewArtifact } from "@/components/ai/reviewWorkflow";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DaemonRequestError, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

import type { QuarantineBookScope } from "./types";

/** The changes to preview, in the book they were decided in. */
export interface ReviewProposalRequest {
  scope: QuarantineBookScope;
  operations: Array<Record<string, unknown>>;
}

/** What the dialog says; `null` is the proposal before the daemon answered. */
export interface ReviewProposalCopy {
  title: (artifact: ReviewArtifact | null) => string;
  body: string;
  footnote: string;
  /** What the owner decided, shown back above the effect so a misread is caught. */
  details?: React.ReactNode;
  confirm: (artifact: ReviewArtifact | null) => string;
  applying: string;
  done: (artifact: ReviewArtifact) => string;
  dedupeKey: string;
}

type Phase =
  | { kind: "checking" }
  | { kind: "ready"; artifact: ReviewArtifact; key: string }
  | { kind: "applying"; artifact: ReviewArtifact; key: string }
  | { kind: "stale" }
  | { kind: "failed"; message: string }
  /** Apply failed without saying whether it ran: retry the same proposal. */
  | { kind: "uncertain"; artifact: ReviewArtifact; key: string; message: string };

const STALE = new Set([
  "review_plan_stale",
  "custody_review_plan_stale",
  "review_case_changed",
  "stale_context",
]);

const isStale = (error: unknown) =>
  error instanceof DaemonRequestError && STALE.has(error.envelope.error?.code ?? "");

const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * Previews review changes the owner decided on a cause card, then applies
 * exactly that preview once confirmed. The daemon names what changes (held
 * rows, reports, tax per year); a book changed meanwhile asks for a fresh
 * preview, and a failed apply retries the same proposal and key.
 */
export function ReviewProposalDialog({
  request,
  copy,
  onClose,
  onApplied,
  hideSensitive,
}: {
  /** Null closes the dialog; a new request previews again. */
  request: ReviewProposalRequest | null;
  copy: ReviewProposalCopy;
  onClose: () => void;
  onApplied?: (artifact: ReviewArtifact) => void;
  hideSensitive?: boolean;
}) {
  const { t } = useTranslation("journals");
  const storedHideSensitive = useUiStore((s) => s.hideSensitive);
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", { invalidateQueries: false });
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", { invalidateQueries: false });
  const apply = useDaemonMutation("ui.review.apply");
  const [phase, setPhase] = React.useState<Phase>({ kind: "checking" });
  const run = React.useRef(0);

  const check = React.useCallback(async () => {
    const token = ++run.current;
    setPhase({ kind: "checking" });
    if (!request) return;
    try {
      const version = (await cases.mutateAsync({ limit: 1, expected_scope: request.scope })).data;
      const proposal = reviewArtifact(
        (await plan.mutateAsync({
          operations: request.operations,
          expected_input_version: version?.input_version ?? 0,
          expected_scope: request.scope,
        })).data,
      );
      if (token !== run.current) return;
      if (!proposal) throw new Error("invalid review preview");
      setPhase({ kind: "ready", artifact: proposal, key: crypto.randomUUID() });
    } catch (error) {
      if (token !== run.current) return;
      setPhase(isStale(error) ? { kind: "stale" } : { kind: "failed", message: messageOf(error) });
    }
    // The mutations are stable; each request is previewed once it opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request]);

  React.useEffect(() => {
    void check();
  }, [check]);

  const confirm = async (artifact: ReviewArtifact, key: string) => {
    if (!request) return;
    setPhase({ kind: "applying", artifact, key });
    try {
      await apply.mutateAsync({ expected_scope: request.scope, artifact, idempotency_key: key });
      useUiStore.getState().addNotification({
        title: copy.done(artifact),
        body: t("quarantine.fix.doneBody", { remaining: artifact.after.quarantine_count }),
        tone: "success",
        dedupeKey: copy.dedupeKey,
      });
      onApplied?.(artifact);
      onClose();
    } catch (error) {
      // A failure after the change was stored keeps the same proposal and
      // key: retrying applies it or returns the receipt of the run that did.
      setPhase(isStale(error) ? { kind: "stale" } : { kind: "uncertain", artifact, key, message: messageOf(error) });
    }
  };

  const artifact = phase.kind === "ready" || phase.kind === "applying" ? phase.artifact : null;
  const retry =
    phase.kind === "uncertain"
      ? () => void confirm(phase.artifact, phase.key)
      : phase.kind === "stale" || phase.kind === "failed"
        ? () => void check()
        : null;
  return (
    <Dialog open={request !== null} onOpenChange={(next) => (!next && phase.kind !== "applying" ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{copy.title(artifact)}</DialogTitle>
          <DialogDescription>{copy.body}</DialogDescription>
        </DialogHeader>
        {copy.details}
        <div className="kb-surface-inset space-y-3 p-3 text-sm" role="status" data-testid="review-proposal-effect">
          {artifact ? (
            <>
              <div className="space-y-1">
                <p className="font-medium tabular-nums">
                  {t("quarantine.fix.effectQuarantine", {
                    before: artifact.before.quarantine_count,
                    after: artifact.after.quarantine_count,
                  })}
                </p>
                <p className={artifact.after.report_ready ? "text-emerald-700 dark:text-emerald-400" : "text-muted-foreground"}>
                  {artifact.after.report_ready ? t("quarantine.fix.reportsReady") : t("quarantine.fix.reportsBlocked")}
                </p>
              </div>
              <div className={(hideSensitive ?? storedHideSensitive) ? "sensitive" : undefined}>
                <ReviewTaxDeltaView artifact={artifact} />
              </div>
            </>
          ) : phase.kind === "checking" ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("quarantine.fix.checking")}
            </p>
          ) : phase.kind === "stale" ? (
            <p>{t("quarantine.fix.stale")}</p>
          ) : phase.kind === "failed" || phase.kind === "uncertain" ? (
            <p className="text-destructive" role="alert">{phase.message}</p>
          ) : null}
        </div>
        <p className="text-xs text-muted-foreground">{copy.footnote}</p>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={phase.kind === "applying"} onClick={onClose}>
            {t("quarantine.pair.cancel")}
          </Button>
          {retry ? (
            <Button type="button" onClick={retry}>
              {t("quarantine.fix.retry")}
            </Button>
          ) : (
            <Button
              type="button"
              disabled={phase.kind !== "ready"}
              onClick={() => (phase.kind === "ready" ? void confirm(phase.artifact, phase.key) : undefined)}
            >
              {phase.kind === "applying" ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {phase.kind === "applying" ? copy.applying : copy.confirm(artifact)}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
