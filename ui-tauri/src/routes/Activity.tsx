import {
  Activity as ActivityIcon,
  Bot,
  CalendarClock,
  Filter,
  RefreshCw,
  RotateCcw,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { TransactionHistoryTimeline } from "@/components/transactions/TransactionEditHistoryPanel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { screenShellClassName } from "@/lib/screen-layout";
import { useDaemon, useDaemonInfinite, useDaemonMutation } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { cn } from "@/lib/utils";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
  TransactionHistoryStaleSummary,
} from "@/lib/transactionHistory";
import { useUiStore } from "@/store/ui";

type DateFilter = "all" | "7" | "30" | "365";

function startForDateFilter(filter: DateFilter) {
  if (filter === "all") return undefined;
  const days = Number(filter);
  return new Date(Date.now() - days * 86_400_000).toISOString();
}

export function Activity() {
  const { t } = useTranslation(["review", "nav", "common"]);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const [dateFilter, setDateFilter] = React.useState<DateFilter>("30");
  const [sourceFilter, setSourceFilter] = React.useState("all");
  const [familyFilter, setFamilyFilter] = React.useState("all");
  const [walletFilter, setWalletFilter] = React.useState("");
  const [transactionFilter, setTransactionFilter] = React.useState("");
  const [pricingOnly, setPricingOnly] = React.useState(false);
  const [aiOnly, setAiOnly] = React.useState(false);
  const [staleOnly, setStaleOnly] = React.useState(false);
  const addNotification = useUiStore((state) => state.addNotification);
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({
      notifyStart: true,
      notifyAlreadyRunning: true,
    });

  const args = React.useMemo(() => {
    const payload: Record<string, unknown> = { limit: 50, include_stale: false };
    const start = startForDateFilter(dateFilter);
    if (start) payload.start = start;
    if (sourceFilter !== "all") payload.source = sourceFilter;
    if (familyFilter !== "all") payload.field_family = familyFilter;
    if (walletFilter.trim()) payload.wallet = walletFilter.trim();
    if (transactionFilter.trim()) payload.transaction = transactionFilter.trim();
    if (pricingOnly) payload.pricing_only = true;
    if (aiOnly) payload.ai_only = true;
    if (staleOnly) payload.stale_only = true;
    return payload;
  }, [
    aiOnly,
    dateFilter,
    familyFilter,
    pricingOnly,
    sourceFilter,
    staleOnly,
    transactionFilter,
    walletFilter,
  ]);

  const historyQuery = useDaemonInfinite<TransactionHistoryList>(
    "ui.activity.history",
    args,
    (lastPage) => lastPage.data?.next_cursor ?? undefined,
  );
  const pages = historyQuery.data?.pages ?? [];
  const events = pages.flatMap((page) => page.data?.events ?? []);
  const staleQuery = useDaemon<TransactionHistoryStaleSummary>(
    "ui.activity.stale",
    undefined,
    { enabled: !historyQuery.isLoading },
  );
  const latestStale = staleQuery.data?.data ?? pages[0]?.data?.stale;
  const staleCount = latestStale?.edit_count ?? 0;

  const onRevert = React.useCallback(
    async (target: HistoryRevertTarget) => {
      await revertHistory.mutateAsync({
        transaction: target.event.transaction_id,
        event: target.event.id,
        ...(target.field ? { field: target.field.field } : {}),
        reason: target.field
          ? t("activity.revert.reasonField", { label: target.field.label })
          : t("activity.revert.reasonEvent"),
      });
      addNotification({
        title: t("activity.revert.title"),
        body: t("activity.revert.body"),
        tone: "success",
        dedupeKey: `activity-history-revert-${target.event.id}-${target.field?.field ?? "event"}`,
      });
    },
    [addNotification, revertHistory, t],
  );

  return (
    <div className={screenShellClassName}>
      <div className="flex w-full flex-col gap-5">
        <header className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <ActivityIcon className="size-4" aria-hidden="true" />
              {t("activity.provenance")}
            </div>
            <h1 className="mt-1 text-2xl font-semibold tracking-normal text-foreground">
              {t("nav:book.activity")}
            </h1>
          </div>
          {staleCount > 0 ? (
            <div className="flex flex-wrap items-center gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
              <CalendarClock className="size-4" aria-hidden="true" />
              <span>{t("activity.staleEdits", { count: staleCount })}</span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-7 gap-1.5 border-amber-300 bg-amber-100 text-amber-900 hover:bg-amber-200 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
                disabled={isProcessingJournals}
                onClick={runJournalProcessing}
              >
                <RefreshCw className={cn("size-3", isProcessingJournals && "animate-spin")} aria-hidden="true" />
                {t("activity.process")}
              </Button>
            </div>
          ) : null}
        </header>

        <section className="grid gap-3 rounded-md border bg-card p-3">
          <div className="flex items-center gap-2 text-sm font-medium">
            <Filter className="size-4 text-muted-foreground" aria-hidden="true" />
            {t("activity.filters")}
          </div>
          <div className="grid gap-3 md:grid-cols-4">
            <div className="grid gap-1.5">
              <Label htmlFor="activity-date">{t("activity.date")}</Label>
              <Select value={dateFilter} onValueChange={(value) => setDateFilter(value as DateFilter)}>
                <SelectTrigger id="activity-date">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="7">{t("activity.dateLast7")}</SelectItem>
                  <SelectItem value="30">{t("activity.dateLast30")}</SelectItem>
                  <SelectItem value="365">{t("activity.dateLastYear")}</SelectItem>
                  <SelectItem value="all">{t("activity.dateAll")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="activity-source">{t("activity.source")}</Label>
              <Select value={sourceFilter} onValueChange={setSourceFilter}>
                <SelectTrigger id="activity-source">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t("activity.sourceAll")}</SelectItem>
                  <SelectItem value="gui">{t("activity.sourceDesktop")}</SelectItem>
                  <SelectItem value="cli">{t("activity.sourceCli")}</SelectItem>
                  <SelectItem value="ai_tool">{t("activity.sourceAssistant")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="activity-family">{t("activity.fieldFamily")}</Label>
              <Select value={familyFilter} onValueChange={setFamilyFilter}>
                <SelectTrigger id="activity-family">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t("activity.fieldAll")}</SelectItem>
                  <SelectItem value="metadata">{t("activity.fieldMetadata")}</SelectItem>
                  <SelectItem value="pricing">{t("activity.fieldPricing")}</SelectItem>
                  <SelectItem value="tax">{t("activity.fieldTax")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="activity-wallet">{t("activity.wallet")}</Label>
              <Input
                id="activity-wallet"
                value={walletFilter}
                onChange={(event) => setWalletFilter(event.target.value)}
                placeholder={t("activity.walletPlaceholder")}
              />
            </div>
            <div className="grid gap-1.5 md:col-span-2">
              <Label htmlFor="activity-transaction">{t("activity.transaction")}</Label>
              <Input
                id="activity-transaction"
                value={transactionFilter}
                onChange={(event) => setTransactionFilter(event.target.value)}
                placeholder={t("activity.transactionPlaceholder")}
              />
            </div>
            <div className="flex flex-wrap items-center gap-4 md:col-span-2">
              <label className="flex items-center gap-2 text-sm">
                <Switch checked={pricingOnly} onCheckedChange={setPricingOnly} />
                {t("activity.pricingChanges")}
              </label>
              <label className="flex items-center gap-2 text-sm">
                <Switch checked={aiOnly} onCheckedChange={setAiOnly} />
                <Bot className="size-3.5 text-muted-foreground" aria-hidden="true" />
                {t("activity.assistantChanges")}
              </label>
              <label className="flex items-center gap-2 text-sm">
                <Switch checked={staleOnly} onCheckedChange={setStaleOnly} />
                {t("activity.staleReports")}
              </label>
              <Button
                type="button"
                variant="ghost"
                className="h-8 gap-1.5"
                onClick={() => {
                  setDateFilter("30");
                  setSourceFilter("all");
                  setFamilyFilter("all");
                  setWalletFilter("");
                  setTransactionFilter("");
                  setPricingOnly(false);
                  setAiOnly(false);
                  setStaleOnly(false);
                }}
              >
                <RotateCcw className="size-3.5" aria-hidden="true" />
                {t("common:actions.reset")}
              </Button>
            </div>
          </div>
        </section>

        <section className="grid gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Badge variant="secondary" className="rounded-md">
                {events.length}
              </Badge>
              {t("activity.loadedEdits", { count: events.length })}
            </div>
            {historyQuery.isFetching ? (
              <span className="text-xs text-muted-foreground">{t("activity.refreshing")}</span>
            ) : null}
          </div>
          <TransactionHistoryTimeline
            events={events}
            hideSensitive={hideSensitive}
            emptyLabel={t("activity.emptyLabel")}
            showTransaction
            onRevert={onRevert}
            isReverting={revertHistory.isPending}
            isLoading={historyQuery.isLoading}
          />
          {historyQuery.hasNextPage ? (
            <div className="flex justify-center">
              <Button
                type="button"
                variant="outline"
                disabled={historyQuery.isFetchingNextPage}
                onClick={() => void historyQuery.fetchNextPage()}
              >
                {historyQuery.isFetchingNextPage ? t("activity.loadingMore") : t("activity.loadMore")}
              </Button>
            </div>
          ) : null}
        </section>
      </div>
    </div>
  );
}
