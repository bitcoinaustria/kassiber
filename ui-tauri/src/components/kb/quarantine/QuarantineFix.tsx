import * as React from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

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
import { formatShortDate } from "@/lib/date";
import { formatSats } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { fixOperations } from "./explain";
import type { QuarantineItem } from "./types";

/** Pairs named in the dialog before the rest are counted. */
const LISTED = 5;

type Phase =
  | { kind: "checking" }
  | { kind: "ready"; artifact: ReviewArtifact; key: string }
  | { kind: "applying"; artifact: ReviewArtifact; key: string }
  | { kind: "stale" }
  | { kind: "failed"; message: string };

const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

/** The book changed between preview and confirmation; preview again. */
const STALE = new Set(["review_plan_stale", "custody_review_plan_stale"]);

function isStale(error: unknown) {
  return error instanceof DaemonRequestError && STALE.has(error.envelope.error?.code ?? "");
}

/** Pairs to unpair: decided by Kassiber, or one the owner picked. */
export type QuarantineFixRequest = { items: QuarantineItem[]; chosen: boolean };

/**
 * Unpairs in one step: the daemon previews the exact change on a copy of the
 * book (what stays in quarantine, whether reports are ready), then applies
 * that same preview atomically, journals included, once the owner confirms.
 * Nothing changes before the confirmation.
 */
export function QuarantineFixDialog({
  request,
  onClose,
  hideSensitive,
}: {
  request: QuarantineFixRequest | null;
  onClose: () => void;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("journals");
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", { invalidateQueries: false });
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", { invalidateQueries: false });
  const apply = useDaemonMutation("ui.review.apply");
  const [phase, setPhase] = React.useState<Phase>({ kind: "checking" });
  const run = React.useRef(0);
  const open = request !== null;
  const fixes = React.useMemo(() => request?.items ?? [], [request]);
  const chosen = request?.chosen ?? false;

  const check = React.useCallback(async () => {
    const token = ++run.current;
    setPhase({ kind: "checking" });
    try {
      const scope = (await cases.mutateAsync({ limit: 1 })).data;
      const proposal = reviewArtifact(
        (await plan.mutateAsync({
          operations: fixOperations(fixes, chosen ? "chosen" : "decided"),
          expected_input_version: scope?.input_version ?? 0,
        })).data,
      );
      if (token !== run.current) return;
      if (!proposal) throw new Error("invalid review preview");
      setPhase({ kind: "ready", artifact: proposal, key: crypto.randomUUID() });
    } catch (error) {
      if (token !== run.current) return;
      setPhase(
        isStale(error)
          ? { kind: "stale" }
          : { kind: "failed", message: error instanceof Error ? error.message : String(error) },
      );
    }
    // The mutations are stable; the request is read when the dialog opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fixes, chosen]);

  React.useEffect(() => {
    if (open) void check();
    else run.current += 1;
  }, [open, check]);

  const confirm = async () => {
    if (phase.kind !== "ready") return;
    const { artifact, key } = phase;
    setPhase({ kind: "applying", artifact, key });
    try {
      await apply.mutateAsync({
        expected_scope: { workspace_id: artifact.workspace_id, profile_id: artifact.profile_id },
        artifact,
        idempotency_key: key,
      });
      useUiStore.getState().addNotification({
        title: t("quarantine.fix.doneTitle", { count: artifact.operations.length }),
        body: t("quarantine.fix.doneBody", { remaining: artifact.after.quarantine_count }),
        tone: "success",
        dedupeKey: "quarantine-fix",
      });
      onClose();
    } catch (error) {
      setPhase(
        isStale(error)
          ? { kind: "stale" }
          : { kind: "failed", message: error instanceof Error ? error.message : String(error) },
      );
    }
  };

  const artifact = phase.kind === "ready" || phase.kind === "applying" ? phase.artifact : null;
  const single = chosen ? fixes[0]?.evidence?.pair_legs : null;
  return (
    <Dialog open={open} onOpenChange={(next) => (!next && phase.kind !== "applying" ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {single ? t("quarantine.pair.confirmTitle") : t("quarantine.fix.title", { count: fixes.length })}
          </DialogTitle>
          <DialogDescription className={single ? sensitiveClass(hideSensitive) : undefined}>
            {single
              ? t("quarantine.pair.confirmBody", { outWallet: single.out.wallet, inWallet: single.in.wallet })
              : t("quarantine.fix.body", { count: fixes.length })}
          </DialogDescription>
        </DialogHeader>
        <div className="kb-surface-inset space-y-1 p-3 text-sm" role="status" data-testid="quarantine-fix-effect">
          {artifact ? (
            <>
              <p className="font-medium tabular-nums">
                {t("quarantine.fix.effectQuarantine", {
                  before: artifact.before.quarantine_count,
                  after: artifact.after.quarantine_count,
                })}
              </p>
              <p className={artifact.after.report_ready ? "text-emerald-700 dark:text-emerald-400" : "text-muted-foreground"}>
                {artifact.after.report_ready ? t("quarantine.fix.reportsReady") : t("quarantine.fix.reportsBlocked")}
              </p>
            </>
          ) : phase.kind === "checking" ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("quarantine.fix.checking")}
            </p>
          ) : phase.kind === "stale" ? (
            <p>{t("quarantine.fix.stale")}</p>
          ) : phase.kind === "failed" ? (
            <p className="text-destructive" role="alert">{phase.message}</p>
          ) : null}
        </div>
        <ul className="max-h-64 space-y-2 overflow-y-auto text-sm">
          {fixes.slice(0, LISTED).map((item) => {
            const legs = item.evidence?.pair_legs;
            return legs ? (
              <li key={item.transaction_id} className="grid grid-cols-[minmax(0,1fr)_auto] items-baseline gap-x-4">
                {/* Wraps rather than truncates: the wallets are what the owner checks. */}
                <span className={cn("min-w-0 break-words", sensitiveClass(hideSensitive))}>
                  {formatShortDate(legs.out.occurred_at)} · {legs.out.wallet} → {legs.in.wallet}
                </span>
                <span className={cn("tabular-nums text-muted-foreground", sensitiveClass(hideSensitive))}>
                  {formatSats(Math.round(legs.out.amount_msat / 1000))}
                </span>
              </li>
            ) : null;
          })}
        </ul>
        {fixes.length > LISTED ? (
          <p className="text-xs text-muted-foreground">
            {t("quarantine.pair.andMore", { count: fixes.length - LISTED })}
          </p>
        ) : null}
        <p className="text-xs text-muted-foreground">{t("quarantine.pair.confirmUndo")}</p>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={phase.kind === "applying"} onClick={onClose}>
            {t("quarantine.pair.cancel")}
          </Button>
          {phase.kind === "stale" || phase.kind === "failed" ? (
            <Button type="button" onClick={() => void check()}>
              {t("quarantine.fix.retry")}
            </Button>
          ) : (
            <Button type="button" disabled={phase.kind !== "ready"} onClick={() => void confirm()}>
              {phase.kind === "applying" ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {phase.kind === "applying"
                ? t("quarantine.fix.applying")
                : single
                  ? t("quarantine.pair.confirm")
                  : t("quarantine.fix.apply", { count: fixes.length })}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
