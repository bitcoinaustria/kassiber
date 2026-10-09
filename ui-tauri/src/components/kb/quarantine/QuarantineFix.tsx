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

import { fixOperations, reconcilePicks } from "./explain";
import type { QuarantineBookScope, QuarantineItem } from "./types";

/** Pairs named in the dialog before the rest are counted. */
const LISTED = 5;

type Phase =
  | { kind: "checking" }
  | { kind: "ready"; artifact: ReviewArtifact; key: string }
  | { kind: "applying"; artifact: ReviewArtifact; key: string }
  | { kind: "stale" }
  /** Some picks no longer read as picked: the owner looks at them again. */
  | { kind: "changed"; count: number }
  /** Every pick cleared meanwhile; there is nothing left to unpair. */
  | { kind: "nothing" }
  | { kind: "failed"; message: string }
  /** Apply failed without saying whether it ran: retry the same proposal. */
  | { kind: "uncertain"; artifact: ReviewArtifact; key: string; message: string };

const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

/** The book changed between preview and confirmation; preview again. */
const STALE = new Set([
  "review_plan_stale",
  "custody_review_plan_stale",
  "review_case_changed",
  "stale_context",
]);

function isStale(error: unknown) {
  return error instanceof DaemonRequestError && STALE.has(error.envelope.error?.code ?? "");
}

/**
 * Pairs the owner picked to unpair, as they read when picked, and the book
 * they were picked in. Every request of the step is bound to that book.
 */
export type QuarantineFixRequest = { items: QuarantineItem[]; scope: QuarantineBookScope | null };

const sameScope = (a: QuarantineBookScope | null, b: QuarantineBookScope | null) =>
  Boolean(a && b && a.workspace_id === b.workspace_id && a.profile_id === b.profile_id);

/**
 * Unpairs the pairs the owner picked in one step. Each check first reads the
 * book again: a pick that cleared is left out, and a pick that changed (the
 * pair was revised or re-pointed) stops the step until the owner has looked
 * at it again. The daemon then previews the exact change on a copy of the
 * book (what stays in quarantine, whether reports are ready) and applies that
 * same preview atomically, journals included, once the owner confirms.
 * Nothing changes before the confirmation.
 */
export function QuarantineFixDialog({
  request,
  onClose,
  onRefresh,
  hideSensitive,
}: {
  request: QuarantineFixRequest | null;
  onClose: () => void;
  /** Re-reads the attention page; null when it could not be read. */
  onRefresh: () => Promise<{ items: QuarantineItem[]; scope: QuarantineBookScope | null } | null>;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("journals");
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", { invalidateQueries: false });
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", { invalidateQueries: false });
  const apply = useDaemonMutation("ui.review.apply");
  const [phase, setPhase] = React.useState<Phase>({ kind: "checking" });
  // What the preview covers: the picks that are still held as picked.
  const [targets, setTargets] = React.useState<QuarantineItem[]>([]);
  const [left, setLeft] = React.useState(0);
  const run = React.useRef(0);
  const open = request !== null;
  const picks = React.useMemo(() => request?.items ?? [], [request]);
  const scope = request?.scope ?? null;

  const check = React.useCallback(async () => {
    const token = ++run.current;
    setPhase({ kind: "checking" });
    try {
      // Without the book the picks came from, nothing can be bound to it.
      if (!scope) throw new Error(t("quarantine.fix.noScope"));
      const fresh = await onRefresh();
      if (token !== run.current) return;
      if (!fresh) throw new Error(t("quarantine.fix.recheckFailed"));
      if (!sameScope(fresh.scope, scope)) throw new Error(t("quarantine.fix.bookChanged"));
      const { current, cleared, changed } = reconcilePicks(picks, fresh.items);
      setTargets(current);
      setLeft(cleared.length);
      if (changed.length) {
        setPhase({ kind: "changed", count: changed.length });
        return;
      }
      if (!current.length) {
        setPhase({ kind: "nothing" });
        return;
      }
      // The picks as picked, not as re-read: the plan carries their
      // fingerprints, and the core refuses any pair that no longer has one.
      const version = (await cases.mutateAsync({ limit: 1, expected_scope: scope })).data;
      const proposal = reviewArtifact(
        (await plan.mutateAsync({
          operations: fixOperations(current),
          expected_input_version: version?.input_version ?? 0,
          expected_scope: scope,
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
  }, [picks, scope, onRefresh]);

  React.useEffect(() => {
    if (open) void check();
    else run.current += 1;
    // Re-check only when a new request opens, not when a refresh re-renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, picks]);

  const confirm = async (artifact: ReviewArtifact, key: string) => {
    if (!sameScope({ workspace_id: artifact.workspace_id, profile_id: artifact.profile_id }, scope)) {
      setPhase({ kind: "failed", message: t("quarantine.fix.bookChanged") });
      return;
    }
    setPhase({ kind: "applying", artifact, key });
    try {
      await apply.mutateAsync({ expected_scope: scope, artifact, idempotency_key: key });
      useUiStore.getState().addNotification({
        title: t("quarantine.fix.doneTitle", { count: artifact.operations.length }),
        body: t("quarantine.fix.doneBody", { remaining: artifact.after.quarantine_count }),
        tone: "success",
        dedupeKey: "quarantine-fix",
      });
      onClose();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      // A failure that may have come after the change was stored keeps the
      // same proposal and key: retrying either applies it or returns the
      // receipt of the run that already did, never a second, new change.
      setPhase(isStale(error) ? { kind: "stale" } : { kind: "uncertain", artifact, key, message });
    }
  };

  const artifact = phase.kind === "ready" || phase.kind === "applying" ? phase.artifact : null;
  const single = picks.length === 1 ? picks[0]?.evidence?.pair_legs : null;
  const shown = targets.length ? targets : picks;
  const retry =
    phase.kind === "uncertain"
      ? () => void confirm(phase.artifact, phase.key)
      : phase.kind === "stale" || phase.kind === "failed"
        ? () => void check()
        : null;
  return (
    <Dialog open={open} onOpenChange={(next) => (!next && phase.kind !== "applying" ? onClose() : undefined)}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {single ? t("quarantine.pair.confirmTitle") : t("quarantine.fix.title", { count: picks.length })}
          </DialogTitle>
          <DialogDescription className={single ? sensitiveClass(hideSensitive) : undefined}>
            {single
              ? t("quarantine.pair.confirmBody", { outWallet: single.out.wallet, inWallet: single.in.wallet })
              : t("quarantine.fix.body", { count: picks.length })}
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
              {left ? <p className="text-muted-foreground">{t("quarantine.fix.leftOut", { count: left })}</p> : null}
            </>
          ) : phase.kind === "checking" ? (
            <p className="flex items-center gap-2 text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("quarantine.fix.checking")}
            </p>
          ) : phase.kind === "stale" ? (
            <p>{t("quarantine.fix.stale")}</p>
          ) : phase.kind === "changed" ? (
            <p role="alert">{t("quarantine.fix.changed", { count: phase.count })}</p>
          ) : phase.kind === "nothing" ? (
            <p>{t("quarantine.fix.nothing", { count: picks.length })}</p>
          ) : phase.kind === "failed" || phase.kind === "uncertain" ? (
            <p className="text-destructive" role="alert">{phase.message}</p>
          ) : null}
        </div>
        <ul className="max-h-64 space-y-2 overflow-y-auto text-sm">
          {shown.slice(0, LISTED).map((item) => {
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
        {shown.length > LISTED ? (
          <p className="text-xs text-muted-foreground">
            {t("quarantine.pair.andMore", { count: shown.length - LISTED })}
          </p>
        ) : null}
        <p className="text-xs text-muted-foreground">{t("quarantine.pair.confirmUndo")}</p>
        <DialogFooter>
          <Button type="button" variant="outline" disabled={phase.kind === "applying"} onClick={onClose}>
            {phase.kind === "changed" || phase.kind === "nothing"
              ? t("quarantine.fix.close")
              : t("quarantine.pair.cancel")}
          </Button>
          {retry ? (
            <Button type="button" onClick={retry}>
              {t("quarantine.fix.retry")}
            </Button>
          ) : phase.kind === "changed" || phase.kind === "nothing" ? null : (
            <Button
              type="button"
              disabled={phase.kind !== "ready"}
              onClick={() => (phase.kind === "ready" ? void confirm(phase.artifact, phase.key) : undefined)}
            >
              {phase.kind === "applying" ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {phase.kind === "applying"
                ? t("quarantine.fix.applying")
                : single
                  ? t("quarantine.pair.confirm")
                  : t("quarantine.fix.apply", { count: targets.length || picks.length })}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
