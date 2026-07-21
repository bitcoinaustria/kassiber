/**
 * Custody Inbox — the unified decision queue.
 *
 * One ranked list over missing-wallet gap questions (``ui.custody.gaps.list``)
 * and transfer/swap pairing candidates (``ui.transfers.suggest``), one
 * decision card at a time. Master–detail: queue on the left, the selected
 * question on the right. Everything expert-grade lives in the Advanced tab.
 */

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  TriangleAlert,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useDaemon, useDaemonInfinite } from "@/daemon/client";
import { formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  collectCustodyGapPages,
  formatCustodyMsat,
  type CustodyGapSnapshot,
} from "../custodyGapsModel";
import { GapDecisionCard } from "./GapDecisionCard";
import { PairDecisionCard } from "./PairDecisionCard";
import {
  blockedYears,
  buildInboxItems,
  countInboxItems,
  filterInboxItems,
  itemBlocksReports,
  itemHasCompetingEvidence,
  itemIsLowConfidence,
  itemIsSuggested,
  walletDisplayName,
  type InboxCandidate,
  type InboxFilter,
  type InboxItem,
} from "./inboxModel";

interface SuggestEnvelope {
  candidates: InboxCandidate[];
}

function QueueRow({
  item,
  selected,
  onSelect,
}: {
  item: InboxItem;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  // Exactly one quiet marker per row (or none): the strongest signal wins.
  // Confidence and type live in the card's eyebrow, not in the queue.
  const marker = itemBlocksReports(item)
    ? { label: t("swap.inbox.blocksBadge"), tone: "alert" as const }
    : itemHasCompetingEvidence(item)
      ? { label: t("swap.inbox.competingBadge"), tone: "alert" as const }
      : item.kind === "residual"
        ? { label: t("swap.inbox.type.followUp"), tone: "muted" as const }
        : itemIsSuggested(item)
          ? { label: t("swap.inbox.suggestedBadge"), tone: "muted" as const }
          : null;
  const line =
    item.kind === "residual"
      ? t("swap.inbox.line.residual", {
          amount: formatCustodyMsat(item.gap.residual_msat, item.gap.asset),
        })
      : item.kind === "gap"
        ? t("swap.inbox.line.move", {
            out: formatCustodyMsat(item.gap.source_total_msat, item.gap.asset),
            from: item.gap.source_wallet_label,
            in: formatCustodyMsat(item.gap.return_total_msat, item.gap.asset),
            to: item.gap.destination_wallet_labels.join(", ") || "—",
          })
        : t("swap.inbox.line.move", {
            out: formatCustodyMsat(
              item.candidate.out_amount_msat,
              item.candidate.out_asset,
            ),
            from: walletDisplayName(
              item.candidate.out_wallet_label,
              item.candidate.out_wallet_kind,
            ),
            in: formatCustodyMsat(
              item.candidate.in_amount_msat,
              item.candidate.in_asset,
            ),
            to: walletDisplayName(
              item.candidate.in_wallet_label,
              item.candidate.in_wallet_kind,
            ),
          });
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-current={selected}
      className={cn(
        "relative block w-full border-b px-3 py-2.5 text-left transition-colors last:border-b-0 hover:bg-muted/50",
        selected && "bg-muted/60",
      )}
    >
      {selected ? (
        <span
          className="absolute inset-y-0 left-0 w-0.5 bg-primary"
          aria-hidden="true"
        />
      ) : null}
      <p className={cn("text-[13px] leading-snug", hideSensitive && "sensitive")}>
        {line}
      </p>
      {marker ? (
        <p
          className={cn(
            "mt-1 text-[10px] font-medium uppercase tracking-[0.14em]",
            marker.tone === "alert"
              ? "text-rose-600 dark:text-rose-400"
              : "text-muted-foreground",
          )}
        >
          {marker.label}
        </p>
      ) : null}
    </button>
  );
}

export function CustodyInbox({
  focusTransactionId,
}: {
  focusTransactionId?: string;
}) {
  const { t } = useTranslation("review");
  const { t: tGaps } = useTranslation("custodyGaps");
  const [filter, setFilter] = useState<InboxFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hintsOpen, setHintsOpen] = useState(false);

  const gapsQuery = useDaemonInfinite<CustodyGapSnapshot>(
    "ui.custody.gaps.list",
    { limit: 100 },
    (lastPage) => lastPage.data?.next_cursor ?? undefined,
  );
  const transferQuery = useDaemon<SuggestEnvelope>("ui.transfers.suggest", {
    candidate_type: "transfer",
  });
  const swapQuery = useDaemon<SuggestEnvelope>("ui.transfers.suggest", {
    candidate_type: "swap",
  });
  const gapPages = useMemo(
    () =>
      (gapsQuery.data?.pages ?? [])
        .map((page) => page.data)
        .filter((page): page is CustodyGapSnapshot => Boolean(page)),
    [gapsQuery.data],
  );
  const snapshot = gapPages[0];
  const gaps = useMemo(() => collectCustodyGapPages(gapPages), [gapPages]);
  const candidates = useMemo(
    () => [
      ...(transferQuery.data?.data?.candidates ?? []),
      ...(swapQuery.data?.data?.candidates ?? []),
    ],
    [transferQuery.data, swapQuery.data],
  );

  const items = useMemo(
    () => buildInboxItems(gaps, candidates),
    [gaps, candidates],
  );
  const counts = useMemo(() => countInboxItems(items), [items]);
  const filtered = useMemo(
    () => filterInboxItems(items, filter),
    [items, filter],
  );
  const mainItems = filtered.filter((item) => !itemIsLowConfidence(item));
  const hintItems = filtered.filter(itemIsLowConfidence);

  // Deep links (?focus=<txid>) select the matching candidate question.
  useEffect(() => {
    if (!focusTransactionId) return;
    const focused = items.find(
      (item) =>
        item.kind === "candidate" &&
        (item.candidate.out_id === focusTransactionId ||
          item.candidate.in_id === focusTransactionId),
    );
    if (focused) setSelectedId(focused.id);
  }, [focusTransactionId, items]);

  // Keep a valid selection: when the selected question settles (vanishes from
  // the recomputed queue), advance to the first open one.
  const visibleItems = hintsOpen ? [...mainItems, ...hintItems] : mainItems;
  const selected =
    visibleItems.find((item) => item.id === selectedId) ??
    visibleItems[0] ??
    null;
  useEffect(() => {
    if (selected && selected.id !== selectedId) setSelectedId(selected.id);
  }, [selected, selectedId]);

  const refetchAll = () => {
    void transferQuery.refetch();
    void swapQuery.refetch();
    void gapsQuery.refetch();
  };

  if (gapsQuery.isLoading && transferQuery.isLoading) {
    return <ScreenSkeleton titleWidth="w-48" />;
  }

  const years = blockedYears(items);
  const derivedStateCurrent = snapshot?.summary.derived_state_current === true;
  const searchComplete = snapshot?.summary.search_complete !== false;
  const canonicalIssueCount = snapshot?.summary.canonical_issue_count ?? 0;

  const filterChips: Array<{ value: InboxFilter; label: string; count: number }> = [
    { value: "all", label: t("swap.inbox.filter.all"), count: counts.open },
    {
      value: "blocking",
      label: t("swap.inbox.filter.blocking"),
      count: counts.blocking,
    },
    {
      value: "suggested",
      label: t("swap.inbox.filter.suggested"),
      count: counts.suggested,
    },
  ];

  // At most one quiet status line — the strongest condition wins; never a
  // stack of banner cards above the work.
  const statusLine =
    snapshot && !derivedStateCurrent
      ? tGaps("processing.body")
      : !searchComplete
        ? tGaps("searchIncomplete.body")
        : null;

  return (
    <div className="space-y-4">
      {counts.open > 0 ? (
        <div>
          <h1 className="text-xl font-semibold">
            {t("swap.inbox.goal", { count: counts.open })}
          </h1>
          {counts.blocking > 0 ? (
            <p className="mt-0.5 text-sm text-muted-foreground">
              {t("swap.inbox.blockingNote", {
                count: counts.blocking,
                years: years.join(", "),
              })}
            </p>
          ) : null}
        </div>
      ) : null}

      {statusLine ? (
        <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <TriangleAlert
            className="mt-0.5 size-3.5 shrink-0 text-amber-600"
            aria-hidden="true"
          />
          {statusLine}
        </p>
      ) : null}

      {counts.open > 0 ? (
        <>
          <div className="flex flex-wrap gap-1.5">
            {filterChips.map((chip) => (
              <Button
                key={chip.value}
                type="button"
                size="sm"
                variant={filter === chip.value ? "default" : "outline"}
                className="h-7 gap-1.5 px-2.5 text-xs"
                onClick={() => setFilter(chip.value)}
              >
                {chip.label}
                <span className="font-mono tabular-nums">
                  {formatCount(chip.count)}
                </span>
              </Button>
            ))}
          </div>

          <div className="grid items-start gap-3 lg:grid-cols-[minmax(260px,340px)_1fr]">
            <div className="overflow-hidden rounded-md border bg-card">
              {mainItems.map((item) => (
                <QueueRow
                  key={item.id}
                  item={item}
                  selected={selected?.id === item.id}
                  onSelect={() => setSelectedId(item.id)}
                />
              ))}
              {mainItems.length === 0 ? (
                <p className="px-3 py-4 text-sm text-muted-foreground">
                  {t("swap.inbox.filterEmpty")}
                </p>
              ) : null}
              {hintItems.length > 0 ? (
                <>
                  <button
                    type="button"
                    className="flex w-full items-center gap-1 border-t px-3 py-2 text-left text-xs text-muted-foreground hover:bg-muted/50"
                    onClick={() => setHintsOpen((open) => !open)}
                    aria-expanded={hintsOpen}
                  >
                    {hintsOpen ? (
                      <ChevronDown className="size-3.5" aria-hidden="true" />
                    ) : (
                      <ChevronRight className="size-3.5" aria-hidden="true" />
                    )}
                    {t("swap.inbox.hintsGroup", { count: hintItems.length })}
                  </button>
                  {hintsOpen
                    ? hintItems.map((item) => (
                        <QueueRow
                          key={item.id}
                          item={item}
                          selected={selected?.id === item.id}
                          onSelect={() => setSelectedId(item.id)}
                        />
                      ))
                    : null}
                </>
              ) : null}
              {gapsQuery.hasNextPage ? (
                <div className="border-t p-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="w-full"
                    disabled={gapsQuery.isFetchingNextPage}
                    onClick={() => void gapsQuery.fetchNextPage()}
                  >
                    {gapsQuery.isFetchingNextPage
                      ? tGaps("pagination.loading")
                      : tGaps("pagination.loadMore")}
                  </Button>
                </div>
              ) : null}
            </div>

            <div>
              {selected ? (
                selected.kind === "candidate" ? (
                  <PairDecisionCard
                    key={selected.id}
                    candidate={selected.candidate}
                    onSettled={refetchAll}
                  />
                ) : (
                  <GapDecisionCard
                    key={selected.id}
                    gap={selected.gap}
                    onSettled={refetchAll}
                  />
                )
              ) : (
                <Card className="gap-2 py-8">
                  <CardContent className="px-5 text-center text-sm text-muted-foreground">
                    {t("swap.inbox.filterEmpty")}
                  </CardContent>
                </Card>
              )}
            </div>
          </div>
        </>
      ) : canonicalIssueCount > 0 ? (
        <Card className="gap-3 border-amber-500/30 bg-amber-500/5 py-6">
          <CardHeader className="px-5">
            <CardTitle className="flex items-center gap-2">
              <TriangleAlert
                className="size-5 text-amber-600"
                aria-hidden="true"
              />
              {tGaps("blocking.title")}
            </CardTitle>
            <CardDescription>
              {tGaps("blocking.body", { count: canonicalIssueCount })}
            </CardDescription>
          </CardHeader>
        </Card>
      ) : (
        <Card className="items-center gap-3 py-10 text-center">
          <CheckCircle2
            className="size-8 text-emerald-600"
            aria-hidden="true"
          />
          <CardHeader className="max-w-xl px-5">
            <CardTitle>{t("swap.inbox.done.title")}</CardTitle>
            <CardDescription>{t("swap.inbox.done.body")}</CardDescription>
          </CardHeader>
        </Card>
      )}
    </div>
  );
}
