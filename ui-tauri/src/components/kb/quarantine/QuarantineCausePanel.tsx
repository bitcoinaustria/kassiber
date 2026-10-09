import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, ArrowRight, Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDaemonStreamMutation } from "@/daemon/client";
import { formatShortDate } from "@/lib/date";
import { formatSats } from "@/lib/localeFormat";
import {
  describeWalletSyncResult,
  journalStepNeedsAttention,
  type JournalStepSummary,
  type SyncResult,
} from "@/lib/syncResults";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import {
  actionLabel,
  causeCopy,
  causeFacts,
  causeKeyFor,
  quarantineDetailContext,
  quarantineGroupContext,
  sheetTabForCause,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import type {
  QuarantineAction,
  QuarantineAssumption,
  QuarantineGroup,
  QuarantineItem,
  QuarantinePairLeg,
  QuarantineSnapshot,
} from "./types";

/**
 * What came of an Unpair: done, or the pair no longer reads as it did when
 * the owner confirmed it (cleared, revised or re-pointed), so nothing changed.
 */
export type UnpairOutcome = "unpaired" | "changed";

/** Pairs shown before the rest fold away. */
const SHOWN_PAIRS = 3;
/** Cause cards shown before the rest fold away. */
const SHOWN_CAUSES = 4;

type OpenTransaction = (
  transactionId: string,
  tab: QuarantineSheetTab,
  context?: QuarantineDetailContext | null,
) => void;

interface QuarantineCausePanelProps {
  /** The "needs you" page: whole-book summary, plus the root rows themselves. */
  snapshot: QuarantineSnapshot;
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
  onOpenTransaction: OpenTransaction;
  onConnectWallet: () => void;
  onImportHistory: (walletId: string | null) => void;
  /** Lists the transactions that only wait on a cause. */
  onShowWaiting: () => void;
  /**
   * Removes the one pair review the owner confirmed, after checking that a
   * fresh read still shows it as confirmed; the panel recalculates journals
   * after it.
   */
  onUnpair: (item: QuarantineItem) => Promise<UnpairOutcome>;
  hideSensitive?: boolean;
}

function formatMsat(value: number) {
  return formatSats(Math.round(Math.abs(value) / 1000));
}

// Same masking class as the rest of the app: amounts follow "hide sensitive".
const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

function dateOnly(value: string | null | undefined) {
  return value ? value.slice(0, 10) : "";
}

/** When the listed rows were calculated: the newest row's projection time. */
function calculatedOn(items: QuarantineItem[]) {
  return items.reduce<string>((latest, item) => {
    const created = item.created_at ?? "";
    return created > latest ? created : latest;
  }, "");
}

/**
 * What needs the user, in the order to fix it: whether the list is current,
 * how many transactions need the user and how many only wait on them, then
 * one card per cause with what was seen, what to do, and its transactions.
 * Every action opens an existing, separately confirmed flow.
 */
export function QuarantineCausePanel({
  snapshot,
  isProcessingJournals,
  onProcessJournals,
  onOpenTransaction,
  onConnectWallet,
  onImportHistory,
  onShowWaiting,
  onUnpair,
  hideSensitive = false,
}: QuarantineCausePanelProps) {
  const { t } = useTranslation("journals");
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [lockedGapNotice, setLockedGapNotice] = React.useState<string | null>(null);
  const [allCauses, setAllCauses] = React.useState(false);
  const syncWallet = useDaemonStreamMutation<
    { results?: SyncResult[]; journals?: JournalStepSummary | null },
    unknown
  >("ui.wallets.sync");
  const { summary, items } = snapshot;
  const groups = summary.groups ?? [];
  const freshness = summary.freshness ?? null;
  const attentionCount = summary.attention_count ?? summary.count;
  const waitingCount =
    summary.waiting_count ??
    (summary.by_category ?? []).find((entry) => entry.category === "downstream")?.count ??
    0;
  const rootItems = React.useMemo(
    () => new Map(items.map((item) => [item.transaction_id, item])),
    [items],
  );

  const runSync = (action: QuarantineAction) => {
    if (!action.wallet_id || syncWallet.isPending) return;
    const wallet = action.wallet_label || action.wallet_id;
    addNotification({
      title: t("quarantine.panel.syncStartedTitle", { wallet }),
      body: t("quarantine.panel.syncStartedBody"),
      tone: "warning",
      dedupeKey: `quarantine-sync-${action.wallet_id}`,
    });
    syncWallet.mutate(
      { wallet: action.wallet_id },
      {
        onSuccess: (envelope) => {
          const result = envelope.data?.results?.[0];
          const journals = envelope.data?.journals;
          const failed = result?.status === "error";
          addNotification({
            title: failed
              ? t("quarantine.panel.syncFailedTitle", { wallet })
              : t("quarantine.panel.syncFinishedTitle", { wallet }),
            body: describeWalletSyncResult(result, wallet, journals),
            tone: failed ? "error" : journalStepNeedsAttention(journals) ? "warning" : "success",
            dedupeKey: `quarantine-sync-${action.wallet_id}`,
          });
        },
        onError: (error) => {
          addNotification({
            title: t("quarantine.panel.syncFailedTitle", { wallet }),
            body: error instanceof Error ? error.message : "",
            tone: "error",
            dedupeKey: `quarantine-sync-${action.wallet_id}`,
          });
        },
      },
    );
  };

  const openRoot = (group: QuarantineGroup, transactionId: string, tab?: QuarantineSheetTab) => {
    const item = rootItems.get(transactionId);
    onOpenTransaction(
      transactionId,
      tab ?? sheetTabForCause(group.reason, group.category, item?.evidence ?? group.evidence),
      quarantineDetailContext(item) ?? quarantineGroupContext(group),
    );
  };

  const runAction = (group: QuarantineGroup, action: QuarantineAction) => {
    const rootId = group.root_transaction_id;
    switch (action.kind) {
      case "sync_wallet":
        runSync(action);
        return;
      case "connect_wallet":
        onConnectWallet();
        return;
      case "import_history":
        onImportHistory(action.wallet_id ?? null);
        return;
      case "review_custody_gap":
        // Decided at click time; the /swaps route guard enforces the same gate.
        if (useUiStore.getState().developerToolsEnabled && action.gap_id) {
          void navigate({ to: "/swaps", search: { tab: "gaps", gap: action.gap_id } });
        } else {
          setLockedGapNotice(group.key);
        }
        return;
      case "process_journals":
        onProcessJournals();
        return;
      case "wait_for_confirmation":
        return;
      case "resolve_root":
        if (action.transaction_id) onOpenTransaction(action.transaction_id, "details");
        return;
      case "review_pair":
        // The pair, and its Unpair button, sit on the sheet's Linked tab.
        if (action.transaction_id) openRoot(group, action.transaction_id, "linked");
        return;
      default: {
        const target = action.transaction_id ?? rootId;
        if (target) openRoot(group, target);
      }
    }
  };

  const hasFreshnessWarning = Boolean(freshness?.needs_processing || freshness?.last_error);
  if (!summary.count && !hasFreshnessWarning) return null;
  // A cleared timestamp only means "changed since"; the rows still carry the
  // time of the calculation they came from.
  const calculated = freshness?.last_processed_at || calculatedOn(items);

  return (
    <section className="space-y-4" aria-label={t("quarantine.causes.title")}>
      {hasFreshnessWarning ? (
        <div
          role="status"
          className="flex flex-col gap-3 rounded-lg border border-amber-300 bg-amber-50 p-4 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100 sm:flex-row sm:items-center sm:justify-between"
        >
          <div className="flex min-w-0 gap-3">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div className="min-w-0 space-y-1 text-sm">
              <p className="font-medium">
                {freshness?.last_error
                  ? t("quarantine.freshness.failedTitle")
                  : t("quarantine.freshness.staleTitle")}
              </p>
              <p>
                {freshness?.last_error
                  ? t("quarantine.freshness.failedBody", {
                      message: freshness.last_error.message,
                    })
                  : calculated
                    ? t("quarantine.freshness.staleBody", { when: dateOnly(calculated) })
                    : t("quarantine.freshness.neverBody")}
              </p>
            </div>
          </div>
          <Button
            type="button"
            size="sm"
            onClick={onProcessJournals}
            disabled={isProcessingJournals}
            className="shrink-0"
          >
            {isProcessingJournals ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            {t("quarantine.cta.processJournals")}
          </Button>
        </div>
      ) : null}

      {summary.count ? (
        <div className="kb-surface p-(--kb-card-padding)" data-testid="quarantine-summary">
          <p className="text-base font-semibold">
            {t("quarantine.summary.needsYou", { count: attentionCount })}
          </p>
          {waitingCount ? (
            <p className="mt-1 text-sm text-muted-foreground">
              {t("quarantine.summary.waiting", { count: waitingCount })}
            </p>
          ) : null}
          {summary.reports_blocked ? (
            <p className="mt-1 text-sm text-red-700 dark:text-red-300">
              {t("quarantine.summary.reportsBlocked")}
            </p>
          ) : null}
        </div>
      ) : null}

      {groups.length ? (
        <div className="space-y-2">
          <h2 className="text-sm font-semibold">{t("quarantine.causes.title")}</h2>
          <ol className="space-y-2">
            {(allCauses ? groups : groups.slice(0, SHOWN_CAUSES)).map((group) => (
              <QuarantineCauseCard
                key={group.key}
                group={group}
                lockedGapNotice={lockedGapNotice === group.key}
                syncPending={syncWallet.isPending}
                hideSensitive={hideSensitive}
                onAction={(action) => runAction(group, action)}
                onOpenRoot={(transactionId) => openRoot(group, transactionId)}
                onShowWaiting={onShowWaiting}
                rootItems={rootItems}
                onUnpair={async (item) => {
                  const outcome = await onUnpair(item);
                  // Unpairing changes the book; the held rows only clear on
                  // the next calculation, which the confirmation promised.
                  if (outcome === "unpaired") onProcessJournals();
                  return outcome;
                }}
                isProcessingJournals={isProcessingJournals}
              />
            ))}
          </ol>
          {groups.length > SHOWN_CAUSES ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setAllCauses((value) => !value)}
            >
              {allCauses
                ? t("quarantine.causes.showFewer")
                : t("quarantine.causes.showMore", { count: groups.length - SHOWN_CAUSES })}
            </Button>
          ) : null}
          {(summary.group_count ?? groups.length) > groups.length ? (
            <p className="text-xs text-muted-foreground">
              {t("quarantine.causes.more", {
                count: (summary.group_count ?? groups.length) - groups.length,
              })}
            </p>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

function QuarantineCauseCard({
  group,
  lockedGapNotice,
  syncPending,
  hideSensitive,
  onAction,
  onOpenRoot,
  onShowWaiting,
  rootItems,
  onUnpair,
  isProcessingJournals,
}: {
  group: QuarantineGroup;
  lockedGapNotice: boolean;
  syncPending: boolean;
  hideSensitive: boolean;
  onAction: (action: QuarantineAction) => void;
  onOpenRoot: (transactionId: string) => void;
  onShowWaiting: () => void;
  rootItems: Map<string, QuarantineItem>;
  onUnpair: (item: QuarantineItem) => Promise<UnpairOutcome>;
  isProcessingJournals: boolean;
}) {
  const { t } = useTranslation("journals");
  const copy = causeCopy(
    {
      reason: group.reason,
      category: group.category,
      evidence: group.evidence,
      wallet: group.root_wallet,
      asset: group.root_asset,
    },
    t,
  );
  const facts = causeFacts(group.evidence, t);
  const rootIds = group.root_transaction_ids.length
    ? group.root_transaction_ids
    : group.root_transaction_id
      ? [group.root_transaction_id]
      : [];
  const rootCount = Math.max(group.root_count, rootIds.length);
  const deprecated = (group.evidence.missing_source_wallets ?? []).filter(
    (wallet) => wallet.deprecated,
  );
  // A suspense left by pairs is answered pair by pair: each one shown side by
  // side, with its own way out.
  // Every loaded row of this cause, not just the roots the summary names:
  // a cause can hold more pairs than that list carries.
  const pairs = causePairs(group, rootIds, rootItems);
  const actions = pairs.length
    ? group.actions.filter((action) => action.kind !== "review_pair")
    : group.actions;
  return (
    <li
      className={cn(
        "kb-surface p-(--kb-card-padding)",
        group.blocks_reports && "border-red-300 dark:border-red-900/60",
      )}
      data-testid="quarantine-cause"
    >
      <div className="flex flex-wrap items-center gap-2">
        <p className="text-sm font-semibold">{copy.title}</p>
        {group.blocks_reports ? (
          <span className="rounded-full bg-red-100 px-2 py-0.5 text-xs font-medium text-red-800 dark:bg-red-950/50 dark:text-red-200">
            {t("quarantine.panel.blocksReports")}
          </span>
        ) : null}
        {rootCount ? (
          <span className="text-xs text-muted-foreground">
            {t("quarantine.causes.count", { count: rootCount })}
          </span>
        ) : null}
      </div>
      <p
        className={cn(
          "mt-1 max-w-3xl text-sm text-muted-foreground",
          copy.whyQuotesAmounts && sensitiveClass(hideSensitive),
        )}
      >
        {copy.why}
      </p>
      {facts.length && !pairs.length ? (
        <ul className="mt-2 list-disc space-y-0.5 pl-5 text-sm text-muted-foreground">
          {facts.map((fact) => (
            <li key={fact}>{fact}</li>
          ))}
        </ul>
      ) : null}
      <p className="mt-2 max-w-3xl text-sm">
        <span className="font-medium">{t("quarantine.causes.whatToDo")}: </span>
        {copy.provide}
      </p>
      {deprecated.map((wallet) => (
        <p key={wallet.id} className="mt-1 text-xs text-amber-700 dark:text-amber-300">
          {t("quarantine.panel.deprecatedWallet", { wallet: wallet.label })}
        </p>
      ))}
      {lockedGapNotice ? (
        <p className="mt-2 text-xs text-muted-foreground">{t("quarantine.panel.gapReviewLocked")}</p>
      ) : null}
      {pairs.length ? (
        <QuarantinePairList
          pairs={pairs}
          totalPairs={rootCount}
          hideSensitive={hideSensitive}
          isProcessingJournals={isProcessingJournals}
          onOpen={onOpenRoot}
          onUnpair={onUnpair}
        />
      ) : null}
      {actions.length || (!pairs.length && rootIds.length) ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {actions.map((action, index) => (
            <Button
              key={`${action.kind}:${action.wallet_id ?? action.transaction_id ?? index}`}
              type="button"
              size="sm"
              variant={index === 0 ? "default" : "outline"}
              disabled={
                action.kind === "wait_for_confirmation" ||
                (action.kind === "sync_wallet" && syncPending)
              }
              onClick={() => onAction(action)}
            >
              {action.kind === "sync_wallet" && syncPending ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : null}
              {actionLabel(action, t)}
            </Button>
          ))}
          {!group.actions.length && !pairs.length && rootIds[0] ? (
            <Button type="button" size="sm" variant="outline" onClick={() => onOpenRoot(rootIds[0])}>
              {t("quarantine.cta.openTransaction")}
            </Button>
          ) : null}
        </div>
      ) : null}

      {group.downstream_count ? (
        <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          <span>{t("quarantine.causes.waiting", { count: group.downstream_count })}</span>
          <Button
            type="button"
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs"
            onClick={onShowWaiting}
          >
            {t("quarantine.causes.showWaiting")}
          </Button>
        </div>
      ) : null}
    </li>
  );
}

function PairLegLine({
  label,
  leg,
  hideSensitive,
}: {
  label: string;
  leg: QuarantinePairLeg;
  hideSensitive: boolean;
}) {
  const bitcoin = ["BTC", "LBTC"].includes(leg.asset.toUpperCase());
  return (
    <div className="grid grid-cols-[5.5rem_minmax(0,1fr)_auto] items-baseline gap-x-3 text-sm">
      <span className="text-xs text-muted-foreground">{label}</span>
      {/* Wraps rather than truncates: the wallet is what the owner compares. */}
      <span className={cn("min-w-0 break-words", sensitiveClass(hideSensitive))}>
        {formatShortDate(leg.occurred_at)} · {leg.wallet}
      </span>
      <span className={cn("tabular-nums", sensitiveClass(hideSensitive))}>
        {bitcoin ? formatMsat(leg.amount_msat) : leg.asset}
      </span>
    </div>
  );
}

function causePairs(
  group: QuarantineGroup,
  rootIds: string[],
  rootItems: Map<string, QuarantineItem>,
): QuarantineItem[] {
  if (causeKeyFor(group.reason, group.evidence) !== "reviewedSuspensePair") return [];
  const members = [...rootItems.values()].filter((item) => item.group_key === group.key);
  const candidates = members.length
    ? members
    : rootIds.map((transactionId) => rootItems.get(transactionId));
  return candidates.filter(
    (item): item is QuarantineItem =>
      Boolean(item && !item.is_downstream && item.evidence?.pair_id && item.evidence.pair_legs),
  );
}

/**
 * Each pair behind a suspense in its own box, with its own Unpair. Whether a
 * pair is one movement is the owner's call, pair by pair: different txids are
 * shown as a hint, never used to pick pairs. Long lists fold.
 */
function QuarantinePairList({
  pairs,
  totalPairs,
  hideSensitive,
  isProcessingJournals,
  onOpen,
  onUnpair,
}: {
  pairs: QuarantineItem[];
  /** Pairs in the whole cause; more than listed when the rest are on later pages. */
  totalPairs: number;
  hideSensitive: boolean;
  isProcessingJournals: boolean;
  onOpen: (transactionId: string) => void;
  onUnpair: (item: QuarantineItem) => Promise<UnpairOutcome>;
}) {
  const { t } = useTranslation("journals");
  const [expanded, setExpanded] = React.useState(false);
  const [confirming, setConfirming] = React.useState<QuarantineItem | null>(null);
  const [pending, setPending] = React.useState(false);
  const [notice, setNotice] = React.useState<{ changed: boolean; message: string } | null>(null);
  const shown = expanded ? pairs : pairs.slice(0, SHOWN_PAIRS);
  const ask = (item: QuarantineItem) => {
    setNotice(null);
    setConfirming(item);
  };
  const confirm = async () => {
    if (!confirming) return;
    setPending(true);
    setNotice(null);
    try {
      if ((await onUnpair(confirming)) === "changed") {
        setNotice({ changed: true, message: t("quarantine.pair.changed") });
      } else {
        setConfirming(null);
      }
    } catch (error) {
      setNotice({
        changed: false,
        message: t("quarantine.pair.failed", {
          message: error instanceof Error ? error.message : String(error),
        }),
      });
    } finally {
      setPending(false);
    }
  };
  const legs = confirming?.evidence?.pair_legs ?? null;
  return (
    <div className="mt-3 space-y-2 border-t pt-3" data-testid="quarantine-pairs">
      <p className="text-xs font-medium text-muted-foreground">
        {t("quarantine.pair.title", { count: pairs.length })}
      </p>
      <ul className="space-y-2">
        {shown.map((item) => {
          const pairLegs = item.evidence!.pair_legs!;
          const facts = causeFacts(item.evidence, t);
          return (
            <li key={item.transaction_id} className="kb-surface-inset flex items-stretch gap-2 overflow-hidden">
              {/* The pair itself opens it; no separate Open button. */}
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-3 p-3 text-left hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                onClick={() => onOpen(item.transaction_id)}
              >
                <span className="min-w-0 flex-1 space-y-1">
                  <PairLegLine label={t("quarantine.pair.sent")} leg={pairLegs.out} hideSensitive={hideSensitive} />
                  <PairLegLine label={t("quarantine.pair.received")} leg={pairLegs.in} hideSensitive={hideSensitive} />
                  {facts.length ? (
                    <span className="block text-xs text-muted-foreground">{facts.join(" ")}</span>
                  ) : null}
                </span>
                <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
              </button>
              <div className="flex shrink-0 items-center pr-3">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={isProcessingJournals}
                  onClick={() => ask(item)}
                >
                  {t("quarantine.pair.unpair")}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
      {pairs.length > SHOWN_PAIRS ? (
        <Button type="button" variant="ghost" size="sm" onClick={() => setExpanded((value) => !value)}>
          {expanded
            ? t("quarantine.pair.showFewer")
            : t("quarantine.pair.showAll", { count: pairs.length })}
        </Button>
      ) : null}
      {totalPairs > pairs.length ? (
        <p className="text-xs text-muted-foreground">
          {t("quarantine.pair.moreLater", { count: totalPairs - pairs.length })}
        </p>
      ) : null}
      <Dialog
        open={confirming !== null}
        onOpenChange={(open) => {
          if (!open && !pending) setConfirming(null);
        }}
      >
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("quarantine.pair.confirmTitle", { count: 1 })}</DialogTitle>
            <DialogDescription className={sensitiveClass(hideSensitive)}>
              {legs
                ? t("quarantine.pair.confirmBody", {
                    outWallet: legs.out.wallet,
                    inWallet: legs.in.wallet,
                  })
                : null}
            </DialogDescription>
          </DialogHeader>
          {legs ? (
            <div className="kb-surface-inset space-y-1 p-3">
              <PairLegLine label={t("quarantine.pair.sent")} leg={legs.out} hideSensitive={hideSensitive} />
              <PairLegLine label={t("quarantine.pair.received")} leg={legs.in} hideSensitive={hideSensitive} />
            </div>
          ) : null}
          <p className="text-xs text-muted-foreground">{t("quarantine.pair.confirmUndo")}</p>
          {notice ? (
            <p className={cn("text-sm", notice.changed ? "" : "text-destructive")} role="alert">
              {notice.message}
            </p>
          ) : null}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={pending} onClick={() => setConfirming(null)}>
              {notice?.changed ? t("quarantine.pair.close") : t("quarantine.pair.cancel")}
            </Button>
            {/* A pair that changed since it was listed is looked at again first. */}
            {notice?.changed ? null : (
              <Button type="button" disabled={pending} onClick={() => void confirm()}>
                {pending ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
                {pending ? t("quarantine.pair.unpairing") : t("quarantine.pair.confirm", { count: 1 })}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function hasAssumptions(assumptions: QuarantineSnapshot["summary"]["assumptions"]) {
  return Boolean(
    assumptions &&
      (assumptions.presumed_external_outbound.count ||
        assumptions.unclassified_inbound.count),
  );
}

/**
 * Bookings that are not held but rest on an assumption Kassiber made. They are
 * not quarantine, so they sit after it, folded.
 */
export function QuarantineAssumptions({
  assumptions,
  onOpenTransaction,
  onConnectWallet,
  hideSensitive,
}: {
  assumptions: QuarantineSnapshot["summary"]["assumptions"];
  onOpenTransaction: (transactionId: string, tab: QuarantineSheetTab) => void;
  onConnectWallet: () => void;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("journals");
  if (!assumptions || !hasAssumptions(assumptions)) return null;
  const blocks = [
    {
      key: "outbound",
      data: assumptions.presumed_external_outbound,
      title: t("quarantine.assumptions.outboundTitle", {
        count: assumptions.presumed_external_outbound.count,
      }),
      why: t("quarantine.assumptions.outboundWhy"),
      fix: t("quarantine.assumptions.outboundFix"),
    },
    {
      key: "inbound",
      data: assumptions.unclassified_inbound,
      title: t("quarantine.assumptions.inboundTitle", {
        count: assumptions.unclassified_inbound.count,
      }),
      why: t("quarantine.assumptions.inboundWhy"),
      fix: t("quarantine.assumptions.inboundFix"),
    },
  ].filter((block) => block.data.count > 0);
  return (
    <details className="kb-surface p-(--kb-card-padding)" data-testid="quarantine-assumptions">
      <summary className="cursor-pointer text-sm font-semibold">
        {t("quarantine.assumptions.title")}
      </summary>
      <p className="mt-1 text-xs text-muted-foreground">{t("quarantine.assumptions.intro")}</p>
      <div className="mt-3 grid gap-3 lg:grid-cols-2">
        {blocks.map((block) => (
          <AssumptionBlock
            key={block.key}
            title={block.title}
            why={block.why}
            fix={block.fix}
            data={block.data}
            hideSensitive={hideSensitive}
            onOpenTransaction={onOpenTransaction}
            onConnectWallet={onConnectWallet}
          />
        ))}
      </div>
    </details>
  );
}

function AssumptionBlock({
  title,
  why,
  fix,
  data,
  hideSensitive,
  onOpenTransaction,
  onConnectWallet,
}: {
  title: string;
  why: string;
  fix: string;
  data: QuarantineAssumption;
  hideSensitive: boolean;
  onOpenTransaction: (transactionId: string, tab: QuarantineSheetTab) => void;
  onConnectWallet: () => void;
}) {
  const { t } = useTranslation("journals");
  return (
    <details className="rounded-md border p-3">
      <summary className="cursor-pointer text-sm font-medium">
        {title}
        <span
          className={cn(
            "ml-2 text-xs font-normal text-muted-foreground tabular-nums",
            sensitiveClass(hideSensitive),
          )}
        >
          {t("quarantine.assumptions.total", { amount: formatMsat(data.amount_msat) })}
        </span>
      </summary>
      <p className="mt-2 text-xs text-muted-foreground">{why}</p>
      <p className="mt-1 text-xs">{fix}</p>
      <ul className="mt-2 divide-y text-xs">
        {data.items.map((item) => (
          <li key={item.transaction_id}>
            <button
              type="button"
              className="flex w-full items-center justify-between gap-3 py-1.5 text-left hover:underline"
              onClick={() => onOpenTransaction(item.transaction_id, "tax")}
            >
              <span className={cn("truncate", sensitiveClass(hideSensitive))}>
                {dateOnly(item.occurred_at)} · {item.wallet}
              </span>
              <span className={cn("shrink-0 tabular-nums", sensitiveClass(hideSensitive))}>
                {formatMsat(item.amount_msat)}
              </span>
            </button>
          </li>
        ))}
      </ul>
      {data.count > data.items.length ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {t("quarantine.assumptions.showMore", { count: data.count - data.items.length })}
        </p>
      ) : null}
      <Button type="button" size="sm" variant="outline" className="mt-2" onClick={onConnectWallet}>
        {t("quarantine.cta.connectWallet")}
      </Button>
    </details>
  );
}
