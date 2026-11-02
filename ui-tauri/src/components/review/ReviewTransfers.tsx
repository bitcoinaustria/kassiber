import * as React from "react";
import { Link } from "@tanstack/react-router";
import { ArrowRight, ArrowUpRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ReviewTaxDeltaView } from "@/components/ai/ReviewWorkflowCard";
import { reviewArtifact, type ReviewArtifact } from "@/components/ai/reviewWorkflow";
import type { ProfilesSnapshot } from "@/mocks/profiles";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { formatShortDate } from "@/lib/date";
import { formatSats } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import {
  bulkPairable,
  candidateKey,
  isSameAsset,
  type TransferCandidate,
  type TransferSuggestions,
} from "./transferModel";

/** Candidates listed before the rest fold away. */
const SHOWN = 6;
/** How long a decision can be undone from here. */
const UNDO_MS = 20_000;

/** A pair or dismissal made here, and the candidates it took off the list. */
type Decision =
  | { kind: "pair"; pairId: string; keys: string[] }
  | { kind: "dismiss"; dismissalId: string; keys: string[] };

type Undo = { decisions: Decision[]; label: string; deadline: number };

const sats = (msat: number) => formatSats(Math.round(Math.abs(msat) / 1000));

/**
 * Moves the matcher thinks went between the owner's own wallets. Each is
 * paired or set aside by the owner, one at a time or, for exact evidence
 * with no competing match, all at once after a confirmation. Every decision
 * can be undone for a short while, and the journals recalculate after.
 *
 * Until they have, a decided candidate is only hidden here: the matcher
 * reads stale journals more widely, so the list is read again once the
 * recalculation is done rather than right after each decision.
 */
export function ReviewTransfers({
  onDecided,
  refreshing,
}: {
  onDecided: () => void;
  /** The journals are recalculating, or about to. */
  refreshing: boolean;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const suggest = useDaemon<TransferSuggestions>("ui.transfers.suggest", {});
  const quiet = { invalidateQueries: false };
  const pair = useDaemonMutation<{ id?: string }>("ui.transfers.pair", quiet);
  const dismiss = useDaemonMutation<{ id?: string }>("ui.transfers.dismiss", quiet);
  const profiles = useDaemon<ProfilesSnapshot>("ui.profiles.snapshot");
  const cases = useDaemonMutation<{ input_version: number }>("ui.review.cases", quiet);
  const plan = useDaemonMutation<ReviewArtifact>("ui.review.plan", quiet);
  const bulkPair = useDaemonMutation<{ operations: Array<{ result: { pair_id: string; transaction_ids: string[] } }> }>("ui.review.apply", quiet);
  const unpair = useDaemonMutation("ui.transfers.unpair", quiet);
  const undismiss = useDaemonMutation("ui.transfers.dismissals.delete", quiet);
  const [hidden, setHidden] = React.useState<ReadonlySet<string>>(() => new Set());
  // Decided candidates by key, so an undo can list one again at once.
  const decided = React.useRef(new Map<string, TransferCandidate>());
  const [restored, setRestored] = React.useState<TransferCandidate[]>([]);
  const [expanded, setExpanded] = React.useState(false);
  // Decisions sent and not answered yet.
  const [saving, setSaving] = React.useState(0);
  const [confirmBulk, setConfirmBulk] = React.useState(false);
  const [bulkPreview, setBulkPreview] = React.useState<{ artifact: ReviewArtifact; key: string } | null>(null);
  const bulkBusy = cases.isPending || plan.isPending || bulkPair.isPending;
  const [undo, setUndo] = React.useState<Undo | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!undo) return;
    const timer = window.setTimeout(() => setUndo(null), Math.max(0, undo.deadline - Date.now()));
    return () => window.clearTimeout(timer);
  }, [undo]);

  // While the journals recalculate, the list stays as it was read before the
  // decision, less what was decided: a read of stale journals lists more
  // candidates than are open. It is read again once the run is done.
  const live = suggest.data?.data?.candidates;
  const [frozen, setFrozen] = React.useState<TransferCandidate[] | null>(null);
  const { refetch } = suggest;
  const wasRefreshing = React.useRef(refreshing);
  // Counts runs, so a read that finishes after the next decision is not
  // taken for the list after it.
  const run = React.useRef(0);
  React.useEffect(() => {
    if (refreshing && !wasRefreshing.current) {
      run.current += 1;
      setFrozen((current) => current ?? live ?? []);
    }
    if (!refreshing && wasRefreshing.current) {
      const finished = run.current;
      void refetch().then(() => {
        if (run.current !== finished) return;
        setFrozen(null);
        setHidden(new Set());
        setRestored([]);
      });
    }
    wasRefreshing.current = refreshing;
  }, [live, refetch, refreshing]);

  const hide = (keys: string[], on: boolean) => {
    if (on) {
      for (const candidate of frozen ?? live ?? []) {
        if (keys.includes(candidateKey(candidate))) decided.current.set(candidateKey(candidate), candidate);
      }
    } else {
      const back = keys
        .map((key) => decided.current.get(key))
        .filter((candidate): candidate is TransferCandidate => Boolean(candidate));
      setRestored((current) => [...current, ...back]);
    }
    setHidden((current) => {
      const next = new Set(current);
      for (const key of keys) {
        if (on) next.add(key);
        else next.delete(key);
      }
      return next;
    });
  };

  // Each candidate once: an undone one first, then the list as read.
  const byKey = new Map<string, TransferCandidate>();
  for (const candidate of [...restored, ...(frozen ?? live ?? [])]) {
    const key = candidateKey(candidate);
    if (!hidden.has(key) && !byKey.has(key)) byKey.set(key, candidate);
  }
  const candidates = [...byKey.values()];
  const exact = bulkPairable(candidates);
  const shown = expanded ? candidates : candidates.slice(0, SHOWN);

  const fail = (cause: unknown) =>
    setError(cause instanceof Error ? cause.message : t("inbox.transfers.failed"));

  // Where keyboard focus goes once a decided row leaves the list: the row
  // now in its place, or the section heading.
  const listRef = React.useRef<HTMLUListElement>(null);
  const headingRef = React.useRef<HTMLHeadingElement>(null);
  const [focusRow, setFocusRow] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (focusRow === null) return;
    const rows = listRef.current?.querySelectorAll<HTMLElement>("li") ?? [];
    const row = rows[Math.min(focusRow, rows.length - 1)];
    (row?.querySelector<HTMLElement>("button") ?? headingRef.current)?.focus();
    setFocusRow(null);
  }, [focusRow]);

  /** The candidate and the ones that share a leg with it. */
  const withSiblings = (candidate: TransferCandidate) =>
    candidates
      .filter(
        (other) =>
          other.out_id === candidate.out_id ||
          other.in_id === candidate.in_id ||
          other.out_id === candidate.in_id ||
          other.in_id === candidate.out_id,
      )
      .map(candidateKey);

  const route = (candidate: TransferCandidate) =>
    t("inbox.transfers.route", {
      from: candidate.out_wallet_label,
      to: candidate.in_wallet_label,
    });

  // A decision leaves the list at once and comes back if it fails: the daemon
  // may hold it behind a running recalculation for a few seconds.
  const decide = async (candidate: TransferCandidate, choice: "pair" | "dismiss") => {
    setError(null);
    const index = candidates.indexOf(candidate);
    // A paired leg is taken: the matches that competed for it go too.
    const keys = choice === "pair" ? withSiblings(candidate) : [candidateKey(candidate)];
    hide(keys, true);
    setFocusRow(index);
    setSaving((count) => count + 1);
    try {
      if (choice === "pair") {
        const envelope = await pair.mutateAsync({
          tx_out: candidate.out_id,
          tx_in: candidate.in_id,
          kind: candidate.default_kind,
          policy: candidate.default_policy,
          pair_source: "manual",
          confidence_at_pair: candidate.confidence,
        });
        const id = envelope.data?.id;
        setUndo(
          id
            ? { decisions: [{ kind: "pair", pairId: id, keys }], label: t("inbox.transfers.paired", { route: route(candidate) }), deadline: Date.now() + UNDO_MS }
            : null,
        );
      } else {
        const envelope = await dismiss.mutateAsync({
          tx_out: candidate.out_id,
          tx_in: candidate.in_id,
          reason: t("swap.dismissReason"),
        });
        const id = envelope.data?.id;
        setUndo(
          id
            ? { decisions: [{ kind: "dismiss", dismissalId: id, keys }], label: t("inbox.transfers.dismissed", { route: route(candidate) }), deadline: Date.now() + UNDO_MS }
            : null,
        );
      }
      onDecided();
    } catch (cause) {
      hide(keys, false);
      fail(cause);
      void suggest.refetch();
    } finally {
      setSaving((count) => count - 1);
    }
  };

  const prepareBulk = async () => {
    setError(null);
    setBulkPreview(null);
    setConfirmBulk(true);
    // Capture the selected transactions before any asynchronous read.
    const operations = exact.map((candidate) => ({
      type: "pair", out_transaction_id: candidate.out_id, in_transaction_id: candidate.in_id,
      pair_kind: candidate.default_kind, policy: candidate.default_policy,
      reason: t("inbox.transfers.bulk.reason"),
    }));
    const snapshot = profiles.data?.data;
    const expected_scope = { workspace_id: snapshot?.activeWorkspaceId, profile_id: snapshot?.activeProfileId };
    const session = useUiStore.getState().daemonSession;
    try {
      if (!expected_scope.workspace_id || !expected_scope.profile_id) throw new Error(t("inbox.transfers.bulk.none"));
      const version = (await cases.mutateAsync({ limit: 1, expected_scope })).data?.input_version;
      if (useUiStore.getState().daemonSession !== session) throw new Error(t("inbox.transfers.bulk.none"));
      const artifact = reviewArtifact((await plan.mutateAsync({ operations, expected_input_version: version, expected_scope })).data);
      if (!artifact || useUiStore.getState().daemonSession !== session) throw new Error(t("inbox.transfers.bulk.none"));
      setBulkPreview({ artifact, key: crypto.randomUUID() });
    } catch (cause) {
      setConfirmBulk(false);
      fail(cause);
    }
  };

  const pairAllExact = async () => {
    if (!bulkPreview) return;
    setError(null);
    try {
      const { artifact, key } = bulkPreview;
      const envelope = await bulkPair.mutateAsync({ artifact, idempotency_key: key,
        expected_scope: { workspace_id: artifact.workspace_id, profile_id: artifact.profile_id } });
      const applied = envelope.data?.operations ?? [];
      setConfirmBulk(false);
      const decisions: Decision[] = applied.map(({ result }) => ({
        kind: "pair", pairId: result.pair_id,
        keys: [`${result.transaction_ids[0]}->${result.transaction_ids[1]}`],
      }));
      hide(decisions.flatMap((decision) => decision.keys), true);
      setUndo({ decisions, label: t("inbox.transfers.pairedMany", { count: applied.length }), deadline: Date.now() + UNDO_MS });
      setFocusRow(0);
      onDecided();
    } catch (cause) {
      // Keep the same artifact and idempotency key if the response was lost.
      fail(cause);
    }
  };

  // Undoes each decision on its own: a failure keeps the rest undoable and
  // still lists and recalculates what was undone before it.
  const runUndo = async () => {
    if (!undo) return;
    const current = undo;
    setUndo(null);
    setError(null);
    const undone: Decision[] = [];
    try {
      for (const decision of current.decisions) {
        if (decision.kind === "pair") {
          await unpair.mutateAsync({ pair_id: decision.pairId });
        } else {
          await undismiss.mutateAsync({ dismissal_id: decision.dismissalId });
        }
        undone.push(decision);
      }
    } catch (cause) {
      fail(cause);
      const left = current.decisions.slice(undone.length);
      setUndo({ ...current, decisions: left, deadline: Date.now() + UNDO_MS });
    } finally {
      if (undone.length) {
        hide(undone.flatMap((decision) => decision.keys), false);
        onDecided();
      }
    }
  };

  if (suggest.isLoading) return null;
  if (suggest.isError || suggest.data?.error) {
    return (
      <section className="kb-surface p-(--kb-card-padding)" role="alert">
        <h2 className="text-sm font-semibold">{t("inbox.transfers.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("inbox.transfers.unavailable")}</p>
      </section>
    );
  }
  if (!candidates.length && !undo && !error) return null;

  return (
    <section className="space-y-3" aria-labelledby="review-transfers-title">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-0.5">
          <h2
            id="review-transfers-title"
            ref={headingRef}
            tabIndex={-1}
            className="flex flex-wrap items-baseline gap-x-2 text-sm font-semibold focus-visible:outline-none"
          >
            {t("inbox.transfers.title")}
            {candidates.length ? (
              <span className="text-xs font-normal text-muted-foreground">
                {t("inbox.transfers.count", { count: candidates.length })}
              </span>
            ) : null}
          </h2>
          <p className="text-xs text-muted-foreground">{t("inbox.transfers.why")}</p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {/* While the journals recalculate the daemon could pair more than
              the list shows, so the bulk step waits for the fresh list. */}
          {exact.length ? (
            <Button type="button" size="sm" disabled={refreshing || saving > 0 || bulkBusy} onClick={() => void prepareBulk()}>
              {t("inbox.transfers.pairExact", { count: exact.length })}
            </Button>
          ) : null}
          <Button asChild type="button" size="sm" variant="ghost">
            <Link to="/swaps">
              {t("inbox.transfers.tools")}
              <ArrowUpRight className="size-3.5" aria-hidden="true" />
            </Link>
          </Button>
        </div>
      </div>

      {/* Both live regions stay mounted, so a screen reader announces what
          appears in them. */}
      <div role="status" className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground empty:hidden">
        {saving ? (
          <span className="flex items-center gap-1.5">
            <Loader2 className="size-3 animate-spin" aria-hidden="true" />
            {t("inbox.transfers.saving")}
          </span>
        ) : undo ? (
          <>
            <span>{undo.label}</span>
            <Button type="button" variant="link" size="sm" className="h-auto p-0 text-xs" onClick={() => void runUndo()}>
              {t("inbox.transfers.undo")}
            </Button>
          </>
        ) : null}
      </div>
      <p role="alert" className="text-xs text-destructive empty:hidden">
        {error ?? null}
      </p>

      {candidates.length ? (
        <ul ref={listRef} id="review-transfers-list" className="kb-surface divide-y overflow-hidden">
          {shown.map((candidate) => {
            const key = candidateKey(candidate);
            const sameAsset = isSameAsset(candidate);
            return (
              <li key={key} className="flex flex-col gap-2 p-3 sm:flex-row sm:items-center sm:gap-4">
                <div className="min-w-0 flex-1 space-y-0.5">
                  <p className={cn("flex min-w-0 items-center gap-1.5 text-sm", hideSensitive && "sensitive")}>
                    <span className="truncate">{candidate.out_wallet_label}</span>
                    <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("inbox.transfers.to")} />
                    <span className="truncate">{candidate.in_wallet_label}</span>
                  </p>
                  <p className="text-xs text-muted-foreground">
                    <span className={cn(hideSensitive && "sensitive")}>
                      {formatShortDate(candidate.out_occurred_at)} ·{" "}
                      {sameAsset
                        ? sats(candidate.out_amount_msat)
                        : `${sats(candidate.out_amount_msat)} ${candidate.out_asset} → ${sats(candidate.in_amount_msat)} ${candidate.in_asset}`}
                      {candidate.swap_fee_msat
                        ? ` · ${t("inbox.transfers.fee", { amount: sats(candidate.swap_fee_msat) })}`
                        : ""}
                    </span>
                    {" · "}
                    {candidate.method === "ownership_graph"
                      ? t("inbox.transfers.evidence.ownership")
                      : candidate.confidence === "exact"
                        ? t("inbox.transfers.evidence.exact")
                        : t("inbox.transfers.evidence.likely")}
                    {candidate.conflict_size > 1
                      ? ` · ${t("inbox.transfers.competing", { count: candidate.conflict_size - 1 })}`
                      : ""}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => void decide(candidate, "pair")}
                    aria-label={t("inbox.transfers.pairLabel", { route: route(candidate) })}
                  >
                    {t("inbox.transfers.pair")}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => void decide(candidate, "dismiss")}
                    aria-label={t("inbox.transfers.dismissLabel", { route: route(candidate) })}
                  >
                    {t("inbox.transfers.dismiss")}
                  </Button>
                  <Button asChild type="button" size="sm" variant="ghost">
                    <Link
                      to="/swaps"
                      search={{
                        focus: candidate.out_id,
                        mode: candidate.candidate_type === "swap" ? "swaps" : "transfers",
                      }}
                      aria-label={t("inbox.transfers.detailsLabel", { route: route(candidate) })}>
                      {t("inbox.transfers.details")}
                    </Link>
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      ) : null}
      {candidates.length > SHOWN ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          aria-expanded={expanded}
          aria-controls="review-transfers-list"
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded
            ? t("inbox.transfers.showFewer")
            : t("inbox.transfers.showAll", { count: candidates.length })}
        </Button>
      ) : null}

      <Dialog open={confirmBulk} onOpenChange={(open) => (bulkBusy ? null : setConfirmBulk(open))}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("inbox.transfers.bulk.title", { count: bulkPreview?.artifact.operations.length ?? exact.length })}</DialogTitle>
            <DialogDescription>{t("inbox.transfers.bulk.body")}</DialogDescription>
          </DialogHeader>
          {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
          {bulkPreview ? <div className={hideSensitive ? "sensitive" : undefined}>
            <ReviewTaxDeltaView artifact={bulkPreview.artifact} />
          </div> : <p role="status">{t("inbox.transfers.saving")}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={bulkBusy} onClick={() => setConfirmBulk(false)}>
              {t("inbox.transfers.bulk.cancel")}
            </Button>
            <Button type="button" disabled={bulkBusy || !bulkPreview} onClick={() => void pairAllExact()}>
              {bulkPair.isPending ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              {t("inbox.transfers.bulk.confirm", { count: bulkPreview?.artifact.operations.length ?? exact.length })}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
