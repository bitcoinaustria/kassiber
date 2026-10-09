import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, ArrowRight, Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
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
  MAX_FIX_OPERATIONS,
  quarantineDetailContext,
  quarantineBookScope,
  quarantineGroupContext,
  samePairCase,
  sheetTabForCause,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import { FixWithAssistant } from "./FixWithAssistant";
import { QuarantineFixDialog, type QuarantineFixRequest } from "./QuarantineFix";
import type {
  QuarantineAction,
  QuarantineBookScope,
  QuarantineGroup,
  QuarantineItem,
  QuarantinePairLeg,
  QuarantineSnapshot,
} from "./types";

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
  /** Re-reads the attention page, so an unpair is checked against the book as it is now. */
  onRefresh: () => Promise<{ items: QuarantineItem[]; scope: QuarantineBookScope | null } | null>;
  /**
   * The open unpair step, when the page owns it (so the transaction sheet
   * can start one too); the panel keeps its own otherwise.
   */
  fixing?: QuarantineFixRequest | null;
  onFixingChange?: (request: QuarantineFixRequest | null) => void;
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
 * how many transactions need the user and the one step that fixes them, then
 * one card per cause with what was seen, what to do, and its transactions.
 * Pairs are unpaired only as the owner picks them, one or several at once;
 * the assistant takes the rest. Every change is previewed by the daemon and
 * confirmed once.
 */
export function QuarantineCausePanel({
  snapshot,
  isProcessingJournals,
  onProcessJournals,
  onOpenTransaction,
  onConnectWallet,
  onImportHistory,
  onShowWaiting,
  onRefresh,
  fixing: fixingProp,
  onFixingChange,
  hideSensitive = false,
}: QuarantineCausePanelProps) {
  const { t } = useTranslation("journals");
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [lockedGapNotice, setLockedGapNotice] = React.useState<string | null>(null);
  const [allCauses, setAllCauses] = React.useState(false);
  const [ownFixing, setOwnFixing] = React.useState<QuarantineFixRequest | null>(null);
  const fixing = fixingProp !== undefined ? fixingProp : ownFixing;
  const setFixing = onFixingChange ?? setOwnFixing;
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
        <div
          className="kb-surface flex flex-col gap-4 p-(--kb-card-padding) sm:flex-row sm:items-center sm:justify-between"
          data-testid="quarantine-summary"
        >
          <div className="min-w-0 space-y-1">
            <p className="text-base font-semibold">
              {t("quarantine.summary.needsYou", { count: attentionCount })}
            </p>
            {waitingCount ? (
              <p className="text-sm text-muted-foreground">
                {t("quarantine.summary.waiting", { count: waitingCount })}
              </p>
            ) : null}
            {summary.reports_blocked ? (
              <p className="text-sm text-red-700 dark:text-red-300">
                {t("quarantine.summary.reportsBlocked")}
              </p>
            ) : null}
          </div>
          {/* Kassiber decides no fix on its own: pairs go as the owner picks
              them on their card; the assistant works through the rest. */}
          <div className="flex shrink-0 flex-wrap gap-2">
            <FixWithAssistant attentionCount={attentionCount} primary />
          </div>
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
                onUnpair={(picked) => setFixing({ items: picked, scope: quarantineBookScope(snapshot) })}
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
      <QuarantineFixDialog
        request={fixing}
        onClose={() => setFixing(null)}
        onRefresh={onRefresh}
        hideSensitive={hideSensitive}
      />
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
}: {
  group: QuarantineGroup;
  lockedGapNotice: boolean;
  syncPending: boolean;
  hideSensitive: boolean;
  onAction: (action: QuarantineAction) => void;
  onOpenRoot: (transactionId: string) => void;
  onShowWaiting: () => void;
  rootItems: Map<string, QuarantineItem>;
  onUnpair: (items: QuarantineItem[]) => void;
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
 * Each pair behind a suspense in its own box; clicking it opens the pair.
 * Whether a pair is one movement is the owner's call, pair by pair: every
 * pair keeps its own Unpair, and several go in one step only as the owner
 * ticks them. Different txids are shown as a hint and never pick a pair.
 */
function QuarantinePairList({
  pairs,
  totalPairs,
  hideSensitive,
  onOpen,
  onUnpair,
}: {
  pairs: QuarantineItem[];
  /** Pairs in the whole cause; more than listed when the rest are on later pages. */
  totalPairs: number;
  hideSensitive: boolean;
  onOpen: (transactionId: string) => void;
  onUnpair: (items: QuarantineItem[]) => void;
}) {
  const { t } = useTranslation("journals");
  const [expanded, setExpanded] = React.useState(false);
  // A tick is the pair as it read when ticked, by pair id. It never moves
  // to whatever the row holds later: a pair revised, replaced or cleared
  // since loses its tick, and the owner is told to tick it again.
  const [picked, setPicked] = React.useState<Map<string, QuarantineItem>>(() => new Map());
  const shown = expanded ? pairs : pairs.slice(0, SHOWN_PAIRS);
  const pairIdOf = (item: QuarantineItem) => item.evidence?.pair_id ?? "";
  const isPicked = (item: QuarantineItem) => {
    const tick = picked.get(pairIdOf(item));
    return Boolean(tick && samePairCase(tick, item));
  };
  const pickedItems = [...picked.values()].filter((tick) =>
    pairs.some((item) => pairIdOf(item) === pairIdOf(tick) && samePairCase(tick, item)),
  );
  const lapsed = picked.size - pickedItems.length;
  const full = pickedItems.length >= MAX_FIX_OPERATIONS;
  const toggle = (item: QuarantineItem, on: boolean) =>
    setPicked((current) => {
      const next = new Map(current);
      if (on) next.set(pairIdOf(item), item);
      else next.delete(pairIdOf(item));
      return next;
    });
  return (
    <div className="mt-3 space-y-2 border-t pt-3" data-testid="quarantine-pairs">
      <p className="text-xs font-medium text-muted-foreground">
        {t("quarantine.pair.title", { count: pairs.length })}
      </p>
      <ul className="space-y-2">
        {shown.map((item) => {
          const pairLegs = item.evidence!.pair_legs!;
          const facts = causeFacts(item.evidence, t);
          const on = isPicked(item);
          return (
            <li key={item.transaction_id} className="kb-surface-inset flex items-stretch gap-2 overflow-hidden">
              {pairs.length > 1 ? (
                <div className="flex shrink-0 items-center pl-3">
                  <Checkbox
                    checked={on}
                    disabled={!on && full}
                    aria-label={t("quarantine.pair.pick", {
                      outWallet: pairLegs.out.wallet,
                      inWallet: pairLegs.in.wallet,
                    })}
                    onCheckedChange={(value) => toggle(item, value === true)}
                  />
                </div>
              ) : null}
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
                <Button type="button" size="sm" variant="outline" onClick={() => onUnpair([item])}>
                  {t("quarantine.pair.unpair")}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
      {lapsed ? (
        <p className="text-xs text-amber-700 dark:text-amber-300" role="status">
          {t("quarantine.pair.tickLapsed", { count: lapsed })}
        </p>
      ) : null}
      {pickedItems.length || lapsed ? (
        <div className="flex flex-wrap items-center gap-2">
          {pickedItems.length ? (
            <Button type="button" size="sm" onClick={() => onUnpair(pickedItems)}>
              {t("quarantine.pair.unpairPicked", { count: pickedItems.length })}
            </Button>
          ) : null}
          <Button type="button" size="sm" variant="ghost" onClick={() => setPicked(new Map())}>
            {t("quarantine.pair.clearPicked")}
          </Button>
          {full ? (
            <span className="text-xs text-muted-foreground">
              {t("quarantine.pair.pickLimit", { count: MAX_FIX_OPERATIONS })}
            </span>
          ) : null}
        </div>
      ) : null}
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
    </div>
  );
}
