import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, ArrowDownLeft, ArrowRight, ArrowUpRight, Loader2, RefreshCw, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useAskAssistantAbout } from "@/components/ai/useAskAssistantAbout";
import { InfoNote } from "@/components/reports/InfoNote";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useDaemonStreamMutation } from "@/daemon/client";
import { localeForLanguage } from "@/i18n/config";
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
  quarantineRowAmount,
  quarantineRowMeta,
  samePairCase,
  sheetTabForCause,
  withLoadedCauses,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import { FixWithAssistant } from "./FixWithAssistant";
import { AcceptCoarsePricesDialog } from "./QuarantineAcceptPrices";
import { QuarantineFixDialog, type QuarantineFixRequest } from "./QuarantineFix";
import { formatExactDecimal, priceEntryOperations, type PriceEntryUnit } from "./priceEntry";
import { ReviewProposalDialog, type ReviewProposalRequest } from "./ReviewProposalDialog";
import type {
  QuarantineAction,
  QuarantineBookScope,
  QuarantineGroup,
  QuarantineItem,
  QuarantinePairLeg,
  QuarantineSnapshot,
} from "./types";

/** Actions whose button already says what to do, so the card need not spell it out. */
const SELF_DESCRIBING_ACTIONS = new Set(["sync_wallet", "connect_wallet", "import_history", "set_price", "pair_transfer"]);
/** Pairs or transactions a cause shows before the rest fold away. */
const SHOWN_ROWS = 3;
/** Cause cards shown before the rest fold away. */
const SHOWN_CAUSES = 4;
/** The hold a coarse price gets while the book requires reviewing it. */
const COARSE_PRICE_REASON = "pricing_review_required";

type OpenTransaction = (
  transactionId: string,
  tab: QuarantineSheetTab,
  context?: QuarantineDetailContext | null,
  /** The cause whose card it was opened from, so "Save & next" walks that card. */
  causeKey?: string,
) => void;

/** Loading the next page of what needs the user. */
export interface AttentionMore {
  hasMore: boolean;
  loading: boolean;
  /** Set when loading more failed; "" when there is no message. */
  error: string | null;
  onLoad: () => void;
}

interface QuarantineCausePanelProps {
  /** The "needs you" page: whole-book summary, plus the root rows themselves. */
  snapshot: QuarantineSnapshot;
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
  onOpenTransaction: OpenTransaction;
  /** Loads more of what needs the user when a cause has rows past this page. */
  attentionMore?: AttentionMore;
  onConnectWallet: () => void;
  onImportHistory: (walletId: string | null) => void;
  /** Lists the transactions that only wait on a cause. */
  onShowWaiting: () => void;
  /** Whether that list is open, so the summary does not offer it twice. */
  waitingShown?: boolean;
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
 * how many transactions need the user and how many only wait on them, then
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
  attentionMore,
  onConnectWallet,
  onImportHistory,
  onShowWaiting,
  waitingShown = false,
  onRefresh,
  fixing: fixingProp,
  onFixingChange,
  hideSensitive = false,
}: QuarantineCausePanelProps) {
  const { t } = useTranslation("journals");
  const askAssistantAbout = useAskAssistantAbout();
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [lockedGapNotice, setLockedGapNotice] = React.useState<string | null>(null);
  const [allCauses, setAllCauses] = React.useState(false);
  const [ownFixing, setOwnFixing] = React.useState<QuarantineFixRequest | null>(null);
  // The book the approximate prices are accepted in, while that step is open.
  const [accepting, setAccepting] = React.useState<QuarantineBookScope | null>(null);
  const fixing = fixingProp !== undefined ? fixingProp : ownFixing;
  const setFixing = onFixingChange ?? setOwnFixing;
  const syncWallet = useDaemonStreamMutation<
    { results?: SyncResult[]; journals?: JournalStepSummary | null },
    unknown
  >("ui.wallets.sync");
  const { summary, items } = snapshot;
  // The summary names at most 50 causes; causes of loaded rows beyond them
  // get a card too, and the rest load with the next page.
  const groups = React.useMemo(
    () => withLoadedCauses(summary.groups ?? [], items),
    [summary.groups, items],
  );
  const unlistedCauses = Math.max(0, (summary.group_count ?? groups.length) - groups.length);
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
      group.key,
    );
  };

  // "Ask about this" pins the cause's root as typed context; its identity
  // never enters the prefilled question.
  const askAboutCause = (group: QuarantineGroup) => {
    const rootId = group.root_transaction_id ?? group.root_transaction_ids[0];
    if (!askAssistantAbout || !rootId) return undefined;
    return () =>
      askAssistantAbout({
        kind: "quarantine",
        transactionId: rootId,
        date: group.root_occurred_at ?? group.earliest_occurred_at,
      });
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

  const bookScope = quarantineBookScope(snapshot);
  const shownGroups = allCauses ? groups : groups.slice(0, SHOWN_CAUSES);
  // A card with rows past this page offers the next page itself; the page
  // offers it too only when no card does, or for causes not named yet.
  const cardOffersMore = shownGroups.some((group) => causeOffersMore(group, rootItems));
  const hasFreshnessWarning = Boolean(freshness?.needs_processing || freshness?.last_error);
  if (!summary.count && !hasFreshnessWarning) return null;
  // A cleared timestamp only means "changed since"; the rows still carry the
  // time of the calculation they came from.
  const calculated = freshness?.last_processed_at || calculatedOn(items);

  return (
    <section className="space-y-3" aria-label={t("quarantine.section.title")}>
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
          className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between"
          data-testid="quarantine-summary"
        >
          <div className="min-w-0 space-y-0.5">
            <h2 className="flex flex-wrap items-baseline gap-x-2 text-sm font-semibold">
              {t("quarantine.section.title")}
              <span className="text-xs font-normal text-muted-foreground">
                {t("quarantine.summary.needsYou", { count: attentionCount })}
              </span>
            </h2>
            {waitingCount ? (
              <p className="text-xs text-muted-foreground">
                {t("quarantine.summary.waiting", { count: waitingCount })}
                {waitingShown ? null : (
                  <>
                    {" "}
                    <Button
                      type="button"
                      variant="link"
                      size="sm"
                      className="h-auto p-0 text-xs"
                      onClick={onShowWaiting}
                    >
                      {t("quarantine.summary.showWaiting")}
                    </Button>
                  </>
                )}
              </p>
            ) : null}
            {summary.reports_blocked ? (
              <p className="text-xs text-red-700 dark:text-red-300">
                {t("quarantine.summary.reportsBlocked")}
              </p>
            ) : null}
          </div>
          {/* Kassiber decides no fix on its own: pairs go as the owner picks
              them on their card; the assistant works through the rest. */}
          <div className="flex shrink-0 flex-wrap gap-2">
            <FixWithAssistant attentionCount={attentionCount} />
          </div>
        </div>
      ) : null}

      {groups.length ? (
        <div className="space-y-2">
          <ol className="space-y-2">
            {shownGroups.map((group) => (
              <QuarantineCauseCard
                key={group.key}
                group={group}
                lockedGapNotice={lockedGapNotice === group.key}
                syncPending={syncWallet.isPending}
                hideSensitive={hideSensitive}
                onAction={(action) => runAction(group, action)}
                onOpenRoot={(transactionId) => openRoot(group, transactionId)}
                rootItems={rootItems}
                more={attentionMore}
                onUnpair={(picked) => setFixing({ items: picked, scope: bookScope })}
                onAcceptPrices={
                  group.reason === COARSE_PRICE_REASON && bookScope
                    ? () => setAccepting(bookScope)
                    : undefined
                }
                priceScope={bookScope}
                currency={summary.fiat_currency ?? null}
                onAskAssistant={askAboutCause(group)}
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
        </div>
      ) : null}
      {/* Whether or not a loaded cause is incomplete, more of what needs the
          user can always be loaded from here, and a failed page retried. */}
      {summary.count &&
      (unlistedCauses ||
        (!cardOffersMore && (attentionMore?.hasMore || (attentionMore?.error ?? null) !== null))) ? (
        <div data-testid="quarantine-load-more">
          <NotLoadedYet more={attentionMore}>
            {unlistedCauses
              ? t("quarantine.causes.more", { count: unlistedCauses })
              : t("quarantine.causes.pageMore")}
          </NotLoadedYet>
        </div>
      ) : null}
      <AcceptCoarsePricesDialog hideSensitive={hideSensitive} scope={accepting} onClose={() => setAccepting(null)} />
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
  rootItems,
  more,
  onUnpair,
  onAcceptPrices,
  priceScope,
  currency,
  onAskAssistant,
}: {
  group: QuarantineGroup;
  lockedGapNotice: boolean;
  syncPending: boolean;
  hideSensitive: boolean;
  onAction: (action: QuarantineAction) => void;
  onOpenRoot: (transactionId: string) => void;
  rootItems: Map<string, QuarantineItem>;
  more?: AttentionMore;
  onUnpair: (items: QuarantineItem[]) => void;
  /** Accepts every approximate price this cause holds, in one reviewed step. */
  onAcceptPrices?: () => void;
  /** The book prices are entered in, for a cause a price answers. */
  priceScope: QuarantineBookScope | null;
  /** The book's currency, which entered prices are in. */
  currency: string | null;
  onAskAssistant?: () => void;
}) {
  const { t, i18n } = useTranslation("journals");
  const { t: tAssistant } = useTranslation("assistant");
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
  const { rootIds, rootCount, members, pairs } = causeRows(group, rootItems);
  const deprecated = (group.evidence.missing_source_wallets ?? []).filter(
    (wallet) => wallet.deprecated,
  );
  // Prices are typed on the rows themselves, for rows this cause holds on
  // its own account; rows that only wait on it clear with it.
  const priceRows = React.useMemo(
    () => (group.category === "missing_price" && priceScope ? members.filter((item) => !item.is_downstream) : []),
    [group.category, priceScope, members],
  );
  const [entering, setEntering] = React.useState(false);
  const [entries, setEntries] = React.useState<Record<string, string>>({});
  const [unit, setUnit] = React.useState<PriceEntryUnit>("value");
  const [checked, setChecked] = React.useState(false);
  const [pricing, setPricing] = React.useState<EnteredPrices | null>(null);
  const decimalSeparator = decimalSeparatorFor(i18n.language);
  const entry = priceEntryOperations(
    priceRows.map((item) => item.transaction_id),
    entries,
    unit,
    decimalSeparator,
  );
  const filled = priceRows.filter((item) => entries[item.transaction_id]?.trim()).length;
  const stopEntering = () => {
    setEntering(false);
    setEntries({});
    setChecked(false);
  };
  const previewPrices = () => {
    setChecked(true);
    if (!entry.ok || !priceScope) return;
    const labels = new Map(priceRows.map((item) => [item.transaction_id, {
      label: quarantineRowMeta(item), asset: item.asset,
    }]));
    setPricing({
      request: { scope: priceScope, operations: entry.operations },
      lines: entry.operations.map((operation) => ({
        transactionId: String(operation.transaction_id),
        label: labels.get(String(operation.transaction_id))?.label ?? "",
        asset: labels.get(String(operation.transaction_id))?.asset ?? "",
        value: String(operation.fiat_value ?? operation.fiat_rate),
      })),
      unit,
      currency,
    });
  };
  // Typing a price replaces opening the first row's pricing tab.
  const actions = (pairs.length ? group.actions.filter((action) => action.kind !== "review_pair") : group.actions)
    .filter((action) => !(priceRows.length && action.kind === "set_price"));
  const primaryTaken = Boolean(onAcceptPrices) || (priceRows.length > 0 && !entering);
  // Spelled out where no button carries it, and where a pair decision needs
  // its caution in view.
  const provideInline =
    pairs.length > 0 ||
    (!onAcceptPrices &&
      !priceRows.length &&
      !(actions.length > 0 && actions.every((action) => SELF_DESCRIBING_ACTIONS.has(action.kind))));
  return (
    <li
      className={cn(
        "kb-surface p-(--kb-card-padding)",
        group.blocks_reports && "border-red-300 dark:border-red-900/60",
      )}
      data-testid="quarantine-cause"
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
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
        {group.downstream_count ? (
          <span className="text-xs text-muted-foreground">
            · {t("quarantine.causes.waitingShort", { count: group.downstream_count })}
          </span>
        ) : null}
        {/* The buttons say what to do; the full instruction is one click away. */}
        {provideInline ? null : (
          <InfoNote
            label={t("quarantine.causes.aboutLabel", { cause: copy.title })}
            notes={[`${t("quarantine.causes.whatToDo")}: ${copy.provide}`]}
          />
        )}
      </div>
      <p
        className={cn(
          "mt-0.5 max-w-3xl text-sm text-muted-foreground",
          copy.whyQuotesAmounts && sensitiveClass(hideSensitive),
        )}
      >
        {copy.why}
      </p>
      {facts.length && !pairs.length ? (
        <ul className="mt-1 list-disc space-y-0.5 pl-5 text-sm text-muted-foreground">
          {facts.map((fact) => (
            <li key={fact}>{fact}</li>
          ))}
        </ul>
      ) : null}
      {provideInline ? (
        <p className="mt-1 max-w-3xl text-sm">
          <span className="font-medium">{t("quarantine.causes.whatToDo")}: </span>
          {copy.provide}
        </p>
      ) : null}
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
          cause={copy.title}
          pairs={pairs}
          totalPairs={rootCount}
          more={more}
          hideSensitive={hideSensitive}
          onOpen={onOpenRoot}
          onUnpair={onUnpair}
        />
      ) : members.length ? (
        <CauseTransactionList
          cause={copy.title}
          items={members}
          total={rootCount}
          more={more}
          hideSensitive={hideSensitive}
          onOpen={onOpenRoot}
          priceEntry={
            entering
              ? {
                  rows: new Set(priceRows.map((item) => item.transaction_id)),
                  entries,
                  invalid: checked && !entry.ok ? new Set(entry.invalid) : new Set(),
                  currency,
                  unit,
                  onUnitChange: setUnit,
                  onChange: (transactionId, value) =>
                    setEntries((current) => ({ ...current, [transactionId]: value })),
                }
              : null
          }
        />
      ) : null}
      {entering ? (
        <div className="mt-3 flex flex-wrap items-center gap-2" data-testid="quarantine-price-entry-actions">
          <Button type="button" size="sm" disabled={!filled} onClick={previewPrices}>
            {t("quarantine.price.preview", { count: filled })}
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={stopEntering}>
            {t("quarantine.pair.cancel")}
          </Button>
          {checked && !entry.ok && entry.invalid.length ? (
            <span className="text-xs text-destructive" role="alert">
              {t("quarantine.price.invalid", { count: entry.invalid.length })}
            </span>
          ) : checked && !entry.ok && entry.tooMany ? (
            <span className="text-xs text-destructive" role="alert">
              {t("quarantine.price.tooMany", { count: MAX_FIX_OPERATIONS })}
            </span>
          ) : null}
        </div>
      ) : actions.length || (!members.length && rootIds.length) || onAskAssistant || onAcceptPrices || priceRows.length ? (
        <div className="mt-3 flex flex-wrap gap-2">
          {onAcceptPrices ? (
            <Button type="button" size="sm" onClick={onAcceptPrices}>
              {t("quarantine.accept.action", { count: rootCount })}
            </Button>
          ) : null}
          {priceRows.length ? (
            <Button
              type="button"
              size="sm"
              variant={onAcceptPrices ? "outline" : "default"}
              onClick={() => setEntering(true)}
            >
              {onAcceptPrices ? t("quarantine.price.enterExact") : t("quarantine.price.enter")}
            </Button>
          ) : null}
          {actions.map((action, index) => (
            <Button
              key={`${action.kind}:${action.wallet_id ?? action.transaction_id ?? index}`}
              type="button"
              size="sm"
              variant={index === 0 && !primaryTaken ? "default" : "outline"}
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
          {/* Nothing of this cause loaded to list: open its first one. */}
          {!group.actions.length && !members.length && rootIds[0] ? (
            <Button type="button" size="sm" variant="outline" onClick={() => onOpenRoot(rootIds[0])}>
              {t("quarantine.cta.openTransaction")}
            </Button>
          ) : null}
          {onAskAssistant ? (
            <Button type="button" size="sm" variant="ghost" onClick={onAskAssistant}>
              <Sparkles className="size-4" aria-hidden="true" />
              {tAssistant("panel.askAbout")}
            </Button>
          ) : null}
        </div>
      ) : null}
      <EnteredPricesDialog
        prices={pricing}
        hideSensitive={hideSensitive}
        onClose={() => setPricing(null)}
        onApplied={stopEntering}
      />
    </li>
  );
}

/** In red or green by direction; neutral while values are hidden, since the colour shows through the blur. */
function directionTone(direction: string | undefined, hideSensitive: boolean) {
  if (hideSensitive) return "text-muted-foreground";
  return direction === "outbound" ? "text-red-700 dark:text-red-300" : "text-emerald-700 dark:text-emerald-300";
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

/** The cause's loaded rows that need the user; older daemons name only its roots. */
function causeMembers(
  group: QuarantineGroup,
  rootIds: string[],
  rootItems: Map<string, QuarantineItem>,
): QuarantineItem[] {
  const members = [...rootItems.values()].filter((item) => item.group_key === group.key);
  if (members.length) return members;
  return rootIds
    .map((transactionId) => rootItems.get(transactionId))
    .filter((item): item is QuarantineItem => Boolean(item && item.group_key === undefined));
}

function causePairs(group: QuarantineGroup, members: QuarantineItem[]): QuarantineItem[] {
  if (causeKeyFor(group.reason, group.evidence) !== "reviewedSuspensePair") return [];
  return members.filter((item) =>
    Boolean(!item.is_downstream && item.evidence?.pair_id && item.evidence.pair_legs),
  );
}

/**
 * What a cause card lists: its roots, how many the whole cause holds, every
 * loaded row of it (a cause can hold more than the roots the summary names),
 * and, for a suspense left by pairs, the pairs to answer one by one.
 */
function causeRows(group: QuarantineGroup, rootItems: Map<string, QuarantineItem>) {
  const rootIds = group.root_transaction_ids.length
    ? group.root_transaction_ids
    : group.root_transaction_id
      ? [group.root_transaction_id]
      : [];
  const members = causeMembers(group, rootIds, rootItems);
  return {
    rootIds,
    rootCount: Math.max(group.root_count, rootIds.length),
    members,
    pairs: causePairs(group, members),
  };
}

/** Whether a card offers more of its own rows than are loaded. */
function causeOffersMore(group: QuarantineGroup, rootItems: Map<string, QuarantineItem>) {
  const { rootCount, members, pairs } = causeRows(group, rootItems);
  const listed = pairs.length || members.length;
  return listed > 0 && rootCount > listed;
}

function decimalSeparatorFor(language: string): "." | "," {
  const parts = new Intl.NumberFormat(localeForLanguage(language)).formatToParts(1.5);
  return parts.find((part) => part.type === "decimal")?.value === "," ? "," : ".";
}

/** Prices typed on a card, as they are previewed. */
interface EnteredPrices {
  request: ReviewProposalRequest;
  /** Each row's label and the exact decimal read from what was typed. */
  lines: Array<{ transactionId: string; label: string; value: string; asset: string }>;
  unit: PriceEntryUnit;
  currency: string | null;
}

/**
 * Previews the prices the owner typed on a card, then saves exactly those.
 * Each price is shown back as read, so a misread separator is caught before
 * it is saved.
 */
function EnteredPricesDialog({
  prices,
  hideSensitive,
  onClose,
  onApplied,
}: {
  prices: EnteredPrices | null;
  hideSensitive: boolean;
  onClose: () => void;
  onApplied: () => void;
}) {
  const { t, i18n } = useTranslation("journals");
  const count = prices?.lines.length ?? 0;
  const locale = localeForLanguage(i18n.language);
  const details = prices ? (
    <ul className="max-h-48 divide-y overflow-y-auto text-sm" data-testid="entered-prices" aria-label={t("quarantine.price.listLabel")}>
      {prices.lines.map((line) => (
        <li key={line.transactionId} className="flex items-baseline justify-between gap-3 py-1.5">
          <span className={cn("min-w-0 truncate text-muted-foreground", sensitiveClass(hideSensitive))}>{line.label}</span>
          <span className={cn("shrink-0 font-medium tabular-nums", sensitiveClass(hideSensitive))}>
            {t(prices.unit === "rate" ? "quarantine.price.lineRate" : "quarantine.price.lineValue", {
              value: formatExactDecimal(line.value, locale),
              currency: prices.currency ?? "",
              asset: line.asset,
            }).trim()}
          </span>
        </li>
      ))}
    </ul>
  ) : null;
  return (
    <ReviewProposalDialog
      request={prices?.request ?? null}
      onClose={onClose}
      onApplied={onApplied}
      copy={{
        title: () => t("quarantine.price.title", { count }),
        body: t("quarantine.price.body"),
        footnote: t("quarantine.price.undo"),
        details,
        confirm: () => t("quarantine.price.confirm", { count }),
        applying: t("quarantine.price.applying"),
        done: () => t("quarantine.price.doneTitle", { count }),
        dedupeKey: "quarantine-enter-prices",
      }}
    />
  );
}

/** Typing prices on a cause's rows, while the owner does. */
interface PriceEntryProps {
  /** The rows a price can be typed for. */
  rows: Set<string>;
  entries: Record<string, string>;
  /** Rows whose input does not read as a price, once the owner asked for a preview. */
  invalid: Set<string>;
  currency: string | null;
  unit: PriceEntryUnit;
  onUnitChange: (unit: PriceEntryUnit) => void;
  onChange: (transactionId: string, value: string) => void;
}

/**
 * The transactions behind a cause, one line each; clicking one opens it.
 * The card above says what to do for all of them. While the owner enters
 * prices, each row the cause holds gets its own field.
 */
function CauseTransactionList({
  cause,
  items,
  total,
  more,
  hideSensitive,
  onOpen,
  priceEntry,
}: {
  /** The cause's title, naming its Show more apart from other cards'. */
  cause: string;
  items: QuarantineItem[];
  /** Transactions in the whole cause; more than listed when the rest are on later pages. */
  total: number;
  more?: AttentionMore;
  hideSensitive: boolean;
  onOpen: (transactionId: string) => void;
  priceEntry: PriceEntryProps | null;
}) {
  const { t } = useTranslation("journals");
  const [expanded, setExpanded] = React.useState(false);
  // A row with a typed price stays in view when the list folds: what is
  // previewed is never hidden.
  const shown = expanded
    ? items
    : items.filter(
        (item, index) => index < SHOWN_ROWS || Boolean(priceEntry?.entries[item.transaction_id]?.trim()),
      );
  const currency = priceEntry?.currency ?? "";
  const assets = new Set(items.filter((item) => priceEntry?.rows.has(item.transaction_id)).map((item) => item.asset));
  const rateUnitLabel = assets.size === 1
    ? t("quarantine.price.unitRate", { asset: [...assets][0] })
    : t("quarantine.price.unitRateGeneric");
  const priceUnitLabel = (asset: string) =>
    priceEntry?.unit === "rate"
      ? t("quarantine.price.unitRate", { asset })
      : t("quarantine.price.unitValue");
  return (
    <div className="mt-3 border-t pt-2" data-testid="quarantine-cause-transactions">
      {priceEntry ? (
        <div className="flex flex-wrap items-center justify-between gap-2 pb-1">
          <p className="text-xs text-muted-foreground">
            {currency ? t("quarantine.price.hint", { currency }) : t("quarantine.price.hintNoCurrency")}
          </p>
          <Select value={priceEntry.unit} onValueChange={(value) => priceEntry.onUnitChange(value as PriceEntryUnit)}>
            <SelectTrigger size="sm" className="h-7 text-xs" aria-label={t("quarantine.price.unitLabel")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="value">{t("quarantine.price.unitValue")}</SelectItem>
              <SelectItem value="rate">{rateUnitLabel}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      ) : null}
      <ul className="divide-y">
        {shown.map((item) => (
          <li key={item.transaction_id} className="flex items-center gap-3">
            <button
              type="button"
              className="flex min-w-0 flex-1 items-center gap-3 rounded-sm py-2 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              onClick={() => onOpen(item.transaction_id)}
            >
              {item.direction === "outbound" ? (
                <ArrowUpRight
                  className={cn("size-3.5 shrink-0", directionTone(item.direction, hideSensitive), sensitiveClass(hideSensitive))}
                  aria-hidden="true"
                />
              ) : (
                <ArrowDownLeft
                  className={cn("size-3.5 shrink-0", directionTone(item.direction, hideSensitive), sensitiveClass(hideSensitive))}
                  aria-hidden="true"
                />
              )}
              <span className={cn("min-w-0 flex-1 truncate text-muted-foreground", sensitiveClass(hideSensitive))}>
                {quarantineRowMeta(item)}
              </span>
              <span
                className={cn(
                  "shrink-0 tabular-nums",
                  directionTone(item.direction, hideSensitive),
                  sensitiveClass(hideSensitive),
                )}
              >
                {quarantineRowAmount(item)}
              </span>
              <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            </button>
            {priceEntry?.rows.has(item.transaction_id) ? (
              <div className="flex shrink-0 flex-col items-end gap-0.5 py-1">
                <Input
                  inputMode="decimal"
                  autoComplete="off"
                  className={cn("h-8 w-36 shrink-0 text-right tabular-nums", sensitiveClass(hideSensitive))}
                  placeholder={currency}
                  aria-label={
                    // Names what is typed: a total or a price per unit.
                    currency
                      ? t("quarantine.price.inputLabel", {
                          unit: priceUnitLabel(item.asset),
                          row: quarantineRowMeta(item),
                          currency,
                        })
                      : t("quarantine.price.inputLabelNoCurrency", {
                          unit: priceUnitLabel(item.asset),
                          row: quarantineRowMeta(item),
                        })
                  }
                  aria-invalid={priceEntry.invalid.has(item.transaction_id) || undefined}
                  value={priceEntry.entries[item.transaction_id] ?? ""}
                  onChange={(event) => priceEntry.onChange(item.transaction_id, event.target.value)}
                />
                {priceEntry.unit === "rate" ? (
                  <span className="text-2xs text-muted-foreground">{priceUnitLabel(item.asset)}</span>
                ) : null}
              </div>
            ) : null}
          </li>
        ))}
      </ul>
      <ListFooter
        cause={cause}
        shown={shown.length}
        loaded={items.length}
        total={total}
        expanded={expanded}
        onExpandedChange={setExpanded}
        more={more}
      />
    </div>
  );
}

/**
 * How much of a cause's list shows, and one way to see more: the rest of
 * what is loaded first, then the next page of what needs the user, without
 * resolving this one first.
 */
function ListFooter({
  cause,
  shown,
  loaded,
  total,
  expanded,
  onExpandedChange,
  more,
}: {
  cause: string;
  shown: number;
  loaded: number;
  /** Rows in the whole cause; more than loaded when the rest are on later pages. */
  total: number;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
  more?: AttentionMore;
}) {
  const { t } = useTranslation("journals");
  const hidden = loaded > shown;
  const unloaded = total > loaded;
  const canLoad = unloaded && Boolean(more?.hasMore);
  const failed = unloaded && more?.error !== null && more?.error !== undefined;
  if (!hidden && !unloaded && !(expanded && shown > SHOWN_ROWS)) return null;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 pt-1 text-xs text-muted-foreground">
      {Math.max(total, loaded) > shown ? (
        <span>{t("quarantine.causes.shownOf", { shown, total: Math.max(total, loaded) })}</span>
      ) : null}
      {hidden || canLoad ? (
        <Button
          type="button"
          variant="link"
          size="sm"
          className="h-auto p-0 text-xs"
          disabled={!hidden && more?.loading}
          aria-label={t("quarantine.causes.showMoreRowsOf", { cause })}
          onClick={() => {
            onExpandedChange(true);
            if (!hidden) more?.onLoad();
          }}
        >
          {!hidden && more?.loading ? <Loader2 className="size-3 animate-spin" aria-hidden="true" /> : null}
          {t("quarantine.causes.showMoreRows")}
        </Button>
      ) : null}
      {expanded && shown > SHOWN_ROWS ? (
        <Button
          type="button"
          variant="link"
          size="sm"
          className="h-auto p-0 text-xs"
          aria-label={t("quarantine.causes.showFewerRowsOf", { cause })}
          onClick={() => onExpandedChange(false)}
        >
          {t("quarantine.causes.showFewerTransactions")}
        </Button>
      ) : null}
      {failed ? (
        <span className="text-destructive" role="alert">
          {t("quarantine.causes.loadMoreFailed", { message: more?.error })}
        </span>
      ) : null}
    </div>
  );
}

/**
 * Each pair behind a suspense in its own box; clicking it opens the pair.
 * Whether a pair is one movement is the owner's call, pair by pair: every
 * pair keeps its own Unpair, and several go in one step only as the owner
 * ticks them. Different txids are shown as a hint and never pick a pair.
 */
function QuarantinePairList({
  cause,
  pairs,
  totalPairs,
  more,
  hideSensitive,
  onOpen,
  onUnpair,
}: {
  cause: string;
  pairs: QuarantineItem[];
  /** Pairs in the whole cause; more than listed when the rest are on later pages. */
  totalPairs: number;
  more?: AttentionMore;
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
  const shown = expanded ? pairs : pairs.slice(0, SHOWN_ROWS);
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
      <ListFooter
        cause={cause}
        shown={shown.length}
        loaded={pairs.length}
        total={totalPairs}
        expanded={expanded}
        onExpandedChange={setExpanded}
        more={more}
      />
    </div>
  );
}

/**
 * How many rows of a cause are not loaded yet, and the way to load them:
 * the next page of what needs the user, without resolving this one first.
 */
function NotLoadedYet({ more, children }: { more?: AttentionMore; children: React.ReactNode }) {
  const { t } = useTranslation("journals");
  return (
    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
      <span>{children}</span>
      {more?.hasMore ? (
        <Button
          type="button"
          variant="link"
          size="sm"
          className="h-auto p-0 text-xs"
          disabled={more.loading}
          onClick={more.onLoad}
        >
          {more.loading ? <Loader2 className="size-3 animate-spin" aria-hidden="true" /> : null}
          {t("quarantine.causes.loadMore")}
        </Button>
      ) : null}
      {more?.error !== null && more?.error !== undefined ? (
        <span className="text-destructive" role="alert">
          {t("quarantine.causes.loadMoreFailed", { message: more.error })}
        </span>
      ) : null}
    </div>
  );
}
