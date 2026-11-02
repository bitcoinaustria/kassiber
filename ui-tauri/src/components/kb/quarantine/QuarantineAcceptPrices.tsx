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

/** Recorded with each acceptance in the transaction's history. */
export const ACCEPT_PRICES_REASON =
  "The owner accepted the approximate price on the Review screen.";

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

/** How many transactions the proposal accepts. */
function acceptedCount(artifact: ReviewArtifact) {
  const ids = artifact.operations[0]?.transaction_ids;
  return Array.isArray(ids) ? ids.length : 0;
}

/**
 * Accepts the approximate price of every transaction held for a coarse-price
 * review, in one reviewed step. The daemon names each transaction in the
 * preview, shows what changes (held rows, reports, tax per year) and applies
 * exactly that preview once the owner confirms. Each acceptance holds only
 * for the price as it is; a price that changes is held again.
 */
export function AcceptCoarsePricesDialog({
  scope,
  onClose,
  hideSensitive,
}: {
  /** The book the cause was read in; null closes the dialog. */
  scope: QuarantineBookScope | null;
  onClose: () => void;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("journals");
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", { invalidateQueries: false });
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", { invalidateQueries: false });
  const apply = useDaemonMutation("ui.review.apply");
  const [phase, setPhase] = React.useState<Phase>({ kind: "checking" });
  const run = React.useRef(0);
  const open = scope !== null;

  const check = React.useCallback(async () => {
    const token = ++run.current;
    setPhase({ kind: "checking" });
    try {
      const version = (await cases.mutateAsync({ limit: 1, expected_scope: scope })).data;
      const proposal = reviewArtifact(
        (await plan.mutateAsync({
          operations: [{ type: "accept_coarse_prices", reason: ACCEPT_PRICES_REASON }],
          expected_input_version: version?.input_version ?? 0,
          expected_scope: scope,
        })).data,
      );
      if (token !== run.current) return;
      if (!proposal) throw new Error("invalid review preview");
      setPhase({ kind: "ready", artifact: proposal, key: crypto.randomUUID() });
    } catch (error) {
      if (token !== run.current) return;
      setPhase(isStale(error) ? { kind: "stale" } : { kind: "failed", message: messageOf(error) });
    }
    // The mutations are stable; the scope is read when the dialog opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope]);

  React.useEffect(() => {
    if (open) void check();
    else run.current += 1;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const confirm = async (artifact: ReviewArtifact, key: string) => {
    setPhase({ kind: "applying", artifact, key });
    try {
      await apply.mutateAsync({ expected_scope: scope, artifact, idempotency_key: key });
      useUiStore.getState().addNotification({
        title: t("quarantine.accept.doneTitle", { count: acceptedCount(artifact) }),
        body: t("quarantine.fix.doneBody", { remaining: artifact.after.quarantine_count }),
        tone: "success",
        dedupeKey: "quarantine-accept-prices",
      });
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
    <Dialog open={open} onOpenChange={(next) => (!next && phase.kind !== "applying" ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {artifact
              ? t("quarantine.accept.title", { count: acceptedCount(artifact) })
              : t("quarantine.accept.titlePending")}
          </DialogTitle>
          <DialogDescription>{t("quarantine.accept.body")}</DialogDescription>
        </DialogHeader>
        <div className="kb-surface-inset space-y-3 p-3 text-sm" role="status" data-testid="quarantine-accept-effect">
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
              <div className={hideSensitive ? "sensitive" : undefined}>
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
        <p className="text-xs text-muted-foreground">{t("quarantine.accept.undo")}</p>
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
              {phase.kind === "applying"
                ? t("quarantine.fix.applying")
                : artifact
                  ? t("quarantine.accept.confirm", { count: acceptedCount(artifact) })
                  : t("quarantine.accept.confirmPending")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
