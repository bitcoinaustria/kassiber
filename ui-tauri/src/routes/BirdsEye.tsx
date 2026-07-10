import * as React from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  BarChart3,
  BookOpen,
  CheckCircle2,
  Gauge,
  Info,
  Loader2,
  RefreshCw,
  ShieldAlert,
  TableProperties,
  WalletCards,
} from "lucide-react";

import { MetricCard } from "@/components/kb/MetricCard";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { BtcActivityChart } from "@/components/overview-dashboard/BtcActivityChart";
import { RecentTransactionsTable } from "@/components/overview-dashboard/RecentTransactionsTable";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  useDaemon,
  useDaemonMutation,
  useDaemonStreamMutation,
} from "@/daemon/client";
import {
  formatBtc,
  formatFiatAmount,
  type Currency,
  useCurrency,
} from "@/lib/currency";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";
import type {
  WorkspaceBookOverview,
  WorkspaceOverviewSnapshot,
  WorkspaceTx,
} from "@/mocks/workspaceOverview";
import { useUiStore } from "@/store/ui";
import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  toDashboardTransaction,
  type OverviewTranslate,
  type Transaction,
} from "@/components/overview-dashboard/model";
import { syncProgressPhaseLabel } from "@/lib/syncProgress";
import { formatCount } from "@/lib/localeFormat";

type BookRoute = "/overview" | "/transactions" | "/journals" | "/quarantine" | "/connections" | "/reports" | "/exit-tax";

interface WorkspaceFreshnessProgress {
  workspace?: { id: string; label: string };
  profile?: { id: string; label: string };
  phase?: string;
  source_label?: string;
  source_type?: string;
  processed?: number;
  total?: number;
}

interface WorkspaceFreshnessRun {
  workspace: { id: string; label: string } | null;
  books: Array<{
    profile: { id: string; label: string };
    attention?: {
      blockedReports?: boolean;
      rateLimited?: boolean;
      errors?: number;
    };
    summary?: {
      blocking_reports?: number;
      rate_limited?: number;
    };
  }>;
  summary: {
    books: number;
    completed: number;
    errors: number;
    rate_limited: number;
    blocked_books: number;
    synced_books: number;
    ok: boolean;
    reports_blocked: number;
  };
}

// `id` is the stable slug used for test ids and lookups; `labelKey` indexes the
// `nav` translation namespace. Keep `id` in sync with the keys in nav.json.
const BOOK_ROUTES: Array<{
  id: string;
  labelKey:
    | "book.overview"
    | "book.transactions"
    | "book.ledger"
    | "book.quarantine"
    | "book.wallets"
    | "book.reports";
  to: BookRoute;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
}> = [
  { id: "overview", labelKey: "book.overview", to: "/overview", icon: Gauge },
  {
    id: "transactions",
    labelKey: "book.transactions",
    to: "/transactions",
    icon: TableProperties,
  },
  { id: "ledger", labelKey: "book.ledger", to: "/journals", icon: BookOpen },
  {
    id: "quarantine",
    labelKey: "book.quarantine",
    to: "/quarantine",
    icon: ShieldAlert,
  },
  {
    id: "wallets",
    labelKey: "book.wallets",
    to: "/connections",
    icon: WalletCards,
  },
  { id: "reports", labelKey: "book.reports", to: "/reports", icon: BarChart3 },
];

function workspaceChartSnapshot(
  snapshot: WorkspaceOverviewSnapshot,
): OverviewSnapshot {
  const firstMarketRate = snapshot.books.find((book) => book.marketRate)?.marketRate;
  const fiatCurrency =
    snapshot.fiat.fiatCurrency ?? firstMarketRate?.fiatCurrency ?? "EUR";
  const fallbackRate =
    firstMarketRate?.rate ??
    snapshot.portfolioSeries
      .map((point) => point.priceEur)
      .find((rate): rate is number => typeof rate === "number" && rate > 0) ??
    0;
  const portfolioSeries = snapshot.portfolioSeries.map((point) => ({
    date: point.date,
    label: point.label,
    balanceBtc: point.balanceBtc,
    valueEur: typeof point.valueEur === "number" ? point.valueEur : 0,
    costBasisEur:
      typeof point.costBasisEur === "number" ? point.costBasisEur : 0,
    priceEur: point.priceEur,
    priceTimestamp: point.priceTimestamp,
    priceSource: point.priceSource,
  }));
  return {
    priceEur: fiatCurrency === "EUR" ? fallbackRate : 0,
    priceUsd: 0,
    marketRate: snapshot.fiat.mixed
      ? undefined
      : {
          asset: "BTC",
          fiatCurrency,
          pair: firstMarketRate?.pair ?? `BTC-${fiatCurrency}`,
          rate: fallbackRate || null,
          timestamp: firstMarketRate?.timestamp ?? null,
          source: firstMarketRate?.source ?? null,
          fetchedAt: firstMarketRate?.fetchedAt ?? null,
          granularity: firstMarketRate?.granularity ?? null,
          method: firstMarketRate?.method ?? null,
        },
    connections: snapshot.connections,
    txs: snapshot.txs,
    activityTxs: snapshot.activityTxs,
    balanceSeries: snapshot.balanceSeries,
    portfolioSeries,
    fiat: {
      fiatCurrency,
      eurBalance: snapshot.fiat.eurBalance ?? 0,
      eurCostBasis: snapshot.fiat.eurCostBasis ?? 0,
      eurUnrealized: snapshot.fiat.eurUnrealized ?? 0,
      eurRealizedYTD: snapshot.fiat.eurRealizedYTD ?? 0,
    },
    status: {
      workspace: snapshot.workspace?.label ?? null,
      profile: "Book set",
      transactionCount: snapshot.status.transactionCount,
      needsJournals: snapshot.status.needsJournals,
      quarantines: snapshot.status.quarantines,
    },
  };
}

function workspaceTransactionRows(txs: WorkspaceTx[]): Transaction[] {
  return txs.map((tx, index) => ({
    ...toDashboardTransaction(tx, index),
    profileId: tx.profileId,
    scopeLabel: tx.book.label,
  }));
}

export function BirdsEye() {
  const params = useParams({ strict: false }) as { workspaceId?: string };
  const workspaceId = params.workspaceId ?? "";
  const { data, error, isLoading, isFetching } =
    useDaemon<WorkspaceOverviewSnapshot>(
      "ui.workspace.overview.snapshot",
      { workspace_id: workspaceId },
      { enabled: Boolean(workspaceId) },
    );
  const overviewEnvelope = data as
    | { data?: WorkspaceOverviewSnapshot }
    | undefined;

  if (isLoading && !overviewEnvelope?.data) {
    return <ScreenSkeleton titleWidth="w-44" metricCount={4} />;
  }
  if (error) {
    return <BirdsEyeError error={error} />;
  }

  return (
    <BirdsEyeView
      snapshot={overviewEnvelope?.data ?? null}
      workspaceId={workspaceId}
      isFetching={isFetching}
    />
  );
}

function BirdsEyeError({ error }: { error: unknown }) {
  const { t } = useTranslation("overview");
  return (
    <div className={screenShellClassName}>
      <Card className="border-destructive/30 bg-destructive/10">
        <CardContent className="py-4 text-sm text-destructive">
          {error instanceof Error ? error.message : t("birdsEye.couldNotLoad")}
        </CardContent>
      </Card>
    </div>
  );
}

export function BirdsEyeView({
  snapshot,
  workspaceId,
  isFetching = false,
}: {
  snapshot: WorkspaceOverviewSnapshot | null;
  workspaceId: string;
  isFetching?: boolean;
}) {
  const { t } = useTranslation(["nav", "common"]);
  const { t: to } = useTranslation("overview");
  const navigate = useNavigate();
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const currency = useCurrency();
  const [progress, setProgress] = React.useState<WorkspaceFreshnessProgress[]>([]);
  const [refreshSummary, setRefreshSummary] =
    React.useState<WorkspaceFreshnessRun["summary"] | null>(null);
  const noticeRef = React.useRef<string | null>(null);
  const refreshWorkspace = useDaemonStreamMutation<
    WorkspaceFreshnessRun,
    WorkspaceFreshnessProgress
  >("ui.workspace.freshness.run", {
    onProgress: (record) => {
      setProgress((current) => [record, ...current].slice(0, 8));
      if (!noticeRef.current) return;
      const book = record.profile?.label ?? to("birdsEye.refreshPanel.fallbackBook");
      const phase = formatPhase(record.phase);
      updateNotification(noticeRef.current, {
        body: `${book}: ${phase}`,
        progress: {
          indeterminate: !hasProgressCounter(record),
          // loose translator
          label: formatWorkspaceProgressLabel(
            record,
            to as (key: string, options?: Record<string, unknown>) => string,
          ),
          value: workspaceProgressValue(record),
        },
      });
    },
  });

  const title = snapshot?.workspace?.label ?? to("birdsEye.fallbackTitle");
  const fiat = snapshot?.fiat ?? null;
  const books = React.useMemo(() => snapshot?.books ?? [], [snapshot?.books]);
  const readyBooks = snapshot?.status.readyBooks ?? books.filter((book) => book.readiness.ready).length;
  const blockedBooks = snapshot?.status.blockedBooks ?? books.length - readyBooks;
  const chartSnapshot = React.useMemo(
    () => (snapshot ? workspaceChartSnapshot(snapshot) : null),
    [snapshot],
  );
  const workspaceTransactions = React.useMemo(
    () =>
      workspaceTransactionRows(
        snapshot?.activityTxs?.length ? snapshot.activityTxs : snapshot?.txs ?? [],
      ),
    [snapshot?.activityTxs, snapshot?.txs],
  );
  const transactionProfiles = React.useMemo(() => {
    const pairs = [
      ...(snapshot?.activityTxs ?? []),
      ...(snapshot?.txs ?? []),
    ].map((tx) => [tx.id, tx] as const);
    return new Map(pairs);
  }, [snapshot?.activityTxs, snapshot?.txs]);
  const chartCurrency: Currency = fiat?.mixed ? "btc" : currency;
  const fiatSeriesEnabled = !fiat?.mixed;

  const handleRefresh = React.useCallback(() => {
    if (!workspaceId || refreshWorkspace.isPending) return;
    setProgress([]);
    setRefreshSummary(null);
    noticeRef.current = addNotification({
      title: to("birdsEye.toast.refreshStartedTitle"),
      body: to("birdsEye.toast.refreshStartedBody"),
      tone: "warning",
      dedupeKey: `workspace-refresh-${workspaceId}`,
      progress: { indeterminate: true, label: to("birdsEye.toast.starting") },
      // Clear any target left on the deduped notification by a prior run.
      target: undefined,
    });
    refreshWorkspace.mutate(
      { workspace_id: workspaceId, journals: true, run: true },
      {
        onSuccess: (envelope) => {
          setRefreshSummary(envelope.data?.summary ?? null);
          const summary = envelope.data?.summary;
          const needsAttention = Boolean(
            summary && (summary.errors > 0 || summary.reports_blocked > 0),
          );
          const notification = {
            title: needsAttention
              ? to("birdsEye.toast.refreshNeedsAttentionTitle")
              : to("birdsEye.toast.refreshFinishedTitle"),
            body: summary
              ? to("birdsEye.toast.refreshFinishedBody", {
                  synced: summary.synced_books,
                  total: summary.books,
                  blocked: summary.reports_blocked,
                })
              : to("birdsEye.toast.refreshDoneBody"),
            tone: needsAttention ? "warning" : "success",
            dedupeKey: `workspace-refresh-${workspaceId}`,
            progress: undefined,
            // Route by an explicit, language-independent target: a warning-tone
            // "needs attention" title isn't recognized by the English keyword
            // router (and warning tone has no error fallback), so a localized
            // title would otherwise be non-clickable.
            target: needsAttention ? "/logs" : undefined,
          } as const;
          if (noticeRef.current) {
            updateNotification(noticeRef.current, notification);
            noticeRef.current = null;
          } else {
            addNotification(notification);
          }
        },
        onError: (refreshError) => {
          const notification = {
            title: to("birdsEye.toast.refreshFailedTitle"),
            body:
              refreshError instanceof Error
                ? refreshError.message
                : to("birdsEye.toast.refreshFailedBody"),
            tone: "error",
            dedupeKey: `workspace-refresh-${workspaceId}`,
            progress: undefined,
            // Failures go to the logs (settings when dev tools are off); set it
            // explicitly so a stale target from a prior run can't bypass that.
            target: "/logs",
          } as const;
          if (noticeRef.current) {
            updateNotification(noticeRef.current, notification);
            noticeRef.current = null;
          } else {
            addNotification(notification);
          }
        },
      },
    );
  }, [
    addNotification,
    refreshWorkspace,
    to,
    updateNotification,
    workspaceId,
  ]);

  const openBookRoute = React.useCallback(
    (profileId: string, route: BookRoute) => {
      const book = books.find((candidate) => candidate.profile.id === profileId);
      const routeEntry = BOOK_ROUTES.find((candidate) => candidate.to === route);
      const page = routeEntry
        ? t(routeEntry.labelKey)
        : t("openBook.fallbackPage");
      void switchProfile
        .mutateAsync({ profile_id: profileId })
        .then(() => {
          addNotification({
            title: t("openBook.title"),
            body: t("openBook.body", {
              book: book?.profile.label ?? t("openBook.fallbackBook"),
              page,
            }),
            tone: "info",
            dedupeKey: `birds-eye-active-book-${profileId}`,
          });
          return navigate({ to: route });
        })
        .catch((switchError: unknown) => {
          addNotification({
            title: to("birdsEye.toast.couldNotOpenBookTitle"),
            body:
              switchError instanceof Error
                ? switchError.message
                : to("birdsEye.toast.couldNotOpenBookBody"),
            tone: "error",
            dedupeKey: `birds-eye-open-book-${profileId}`,
          });
        });
    },
    [addNotification, books, navigate, switchProfile, t, to],
  );

  const openWorkspaceTransaction = React.useCallback(
    (transaction: Transaction) => {
      if (!transaction.profileId) return;
      void switchProfile
        .mutateAsync({ profile_id: transaction.profileId })
        .then(() => {
          addNotification({
            title: to("birdsEye.toast.activeBookChangedTitle"),
            body: to("birdsEye.toast.transactionBody", {
              book: transaction.scopeLabel ?? to("birdsEye.toast.fallbackBook"),
            }),
            tone: "info",
            dedupeKey: `birds-eye-active-transaction-${transaction.profileId}`,
          });
          return navigate({
            to: "/transactions",
            search: { tx: transaction.id },
          });
        })
        .catch((switchError: unknown) => {
          addNotification({
            title: to("birdsEye.toast.couldNotOpenTransactionTitle"),
            body:
              switchError instanceof Error
                ? switchError.message
                : to("birdsEye.toast.couldNotOpenTransactionBody"),
            tone: "error",
            dedupeKey: `birds-eye-open-transaction-${transaction.id}`,
          });
        });
    },
    [addNotification, navigate, switchProfile, to],
  );

  const openChartTransaction = React.useCallback(
    (transactionId: string) => {
      const tx = transactionProfiles.get(transactionId);
      if (!tx) return;
      openWorkspaceTransaction({
        ...toDashboardTransaction(tx, 0),
        profileId: tx.profileId,
        scopeLabel: tx.book.label,
      });
    },
    [openWorkspaceTransaction, transactionProfiles],
  );

  return (
    <div
      className={cn(screenShellClassName, "relative")}
      aria-busy={isFetching || refreshWorkspace.isPending}
    >
      <div className={pageHeaderClassName}>
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-2xl font-semibold tracking-tight">
              {to("birdsEye.title")}
            </h2>
            <Badge variant="secondary">{to("birdsEye.badge")}</Badge>
            {fiat?.mixed ? (
              <Badge variant="outline">{to("birdsEye.mixedFiatBadge")}</Badge>
            ) : null}
          </div>
          <p className="max-w-3xl text-sm text-muted-foreground">
            {title}
          </p>
        </div>
        <div className={pageHeaderActionsClassName}>
          <Button variant="outline" className={pageHeaderActionClassName} asChild>
            <Link to="/books">{to("birdsEye.booksLink")}</Link>
          </Button>
          <Button
            type="button"
            className={pageHeaderActionClassName}
            onClick={handleRefresh}
            disabled={refreshWorkspace.isPending || !workspaceId}
          >
            {refreshWorkspace.isPending ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            {to("birdsEye.refresh")}
          </Button>
        </div>
      </div>

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          label={to("birdsEye.metric.btcTotal")}
          value={hideSensitive ? t("common:state.hidden") : formatBtc(fiat?.btcBalance ?? 0)}
          detail={to("birdsEye.metric.books", { count: books.length })}
          icon={<Gauge className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={fiat?.mixed ? to("birdsEye.metric.fiatRollup") : to("birdsEye.metric.fiatTotal")}
          value={
            hideSensitive
              ? t("common:state.hidden")
              : fiat?.mixed
                ? to("birdsEye.metric.mixed")
                : formatFiatAmount(fiat?.eurBalance ?? 0, fiat?.fiatCurrency ?? "EUR")
          }
          detail={
            fiat?.mixed
              ? fiat.label ?? to("birdsEye.metric.perBookFiatOnly")
              : fiat?.fiatCurrency ?? to("birdsEye.metric.noFiatCurrency")
          }
          icon={<BarChart3 className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={to("birdsEye.metric.readiness")}
          value={`${readyBooks}/${books.length}`}
          detail={
            blockedBooks
              ? to("birdsEye.metric.booksNeedAttention", { count: blockedBooks })
              : to("birdsEye.metric.allBooksReady")
          }
          icon={<CheckCircle2 className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={to("birdsEye.metric.quarantine")}
          value={String(snapshot?.status.quarantines ?? 0)}
          detail={
            snapshot?.status.needsJournals
              ? to("birdsEye.metric.journalsNeedProcessing")
              : to("birdsEye.metric.noStaleJournals")
          }
          icon={<ShieldAlert className="size-4" aria-hidden="true" />}
        />
      </div>

      {refreshWorkspace.isPending || progress.length || refreshSummary ? (
        <RefreshPanel
          progress={progress}
          summary={refreshSummary}
          isRunning={refreshWorkspace.isPending}
        />
      ) : null}

      <div className="rounded-lg border bg-muted/25 px-3 py-2 text-xs text-muted-foreground">
        <Info className="mr-1.5 inline size-3.5 align-[-2px]" aria-hidden="true" />
        {to("birdsEye.readonlyNote")}
      </div>

      {chartSnapshot ? (
        <BtcActivityChart
          snapshot={chartSnapshot}
          hideSensitive={hideSensitive}
          currency={chartCurrency}
          fiatSeriesEnabled={fiatSeriesEnabled}
          onOpenTransactionDetail={openChartTransaction}
        />
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_360px]">
        <Card>
          <CardHeader className="border-b pb-3">
            <CardTitle className="text-base">{to("birdsEye.booksCard.title")}</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3 pt-4">
            {books.length ? (
              books.map((book) => (
                <BookRow
                  key={book.profile.id}
                  book={book}
                  hideSensitive={hideSensitive}
                  onOpenRoute={openBookRoute}
                  disabled={switchProfile.isPending}
                />
              ))
            ) : (
              <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
                {to("birdsEye.booksCard.empty")}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b pb-3">
            <CardTitle className="text-base">{to("birdsEye.fiatCard.title")}</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-2 pt-4">
            {fiat?.books.length ? (
              fiat.books.map((row) => (
                <div
                  key={row.profileId}
                  className="rounded-lg border bg-muted/20 p-3"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">
                        {row.profileLabel}
                      </p>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {row.fiatCurrency}
                      </p>
                    </div>
                    <p className="text-sm font-medium">
                      {hideSensitive
                        ? t("common:state.hidden")
                        : formatFiatAmount(row.balance, row.fiatCurrency)}
                    </p>
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <span>
                      {to("birdsEye.fiatCard.basis", {
                        value: hideSensitive
                          ? t("common:state.hidden")
                          : formatFiatAmount(row.costBasis, row.fiatCurrency),
                      })}
                    </span>
                    <span>
                      {to("birdsEye.fiatCard.ytd", {
                        value: hideSensitive
                          ? t("common:state.hidden")
                          : formatFiatAmount(row.realizedYTD, row.fiatCurrency),
                      })}
                    </span>
                  </div>
                </div>
              ))
            ) : (
              <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
                {to("birdsEye.fiatCard.empty")}
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {chartSnapshot ? (
        <RecentTransactionsTable
          title={to("birdsEye.recentActivityTitle")}
          transactions={workspaceTransactions}
          hideSensitive={hideSensitive}
          currency={chartCurrency}
          priceEur={activeMarketFiatRate(chartSnapshot)}
          fiatCurrency={activeMarketFiatCurrency(chartSnapshot)}
          showAllTo={null}
          onOpenTransaction={openWorkspaceTransaction}
        />
      ) : null}
    </div>
  );
}

export function BookRow({
  book,
  hideSensitive,
  onOpenRoute,
  disabled,
}: {
  book: WorkspaceBookOverview;
  hideSensitive: boolean;
  onOpenRoute: (profileId: string, route: BookRoute) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation(["overview", "common"]);
  const ready = book.readiness.ready;
  const fiatCurrency = book.profile.fiatCurrency || book.fiat.fiatCurrency || "EUR";
  const btcBalance = book.connections.reduce(
    (total, connection) => total + connection.balance,
    0,
  );
  return (
    <div className="rounded-lg border bg-background p-3">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-sm font-medium">{book.profile.label}</p>
            <Badge variant={ready ? "secondary" : "outline"}>
              {ready ? (
                <CheckCircle2 className="size-3" aria-hidden="true" />
              ) : (
                <AlertTriangle className="size-3" aria-hidden="true" />
              )}
              {ready ? t("birdsEye.bookRow.ready") : t("birdsEye.bookRow.attention")}
            </Badge>
            <Badge variant="outline">{fiatCurrency}</Badge>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
            <span>{t("birdsEye.bookRow.wallets", { count: book.connections.length })}</span>
            <span>{t("birdsEye.bookRow.txs", { count: book.status.transactionCount ?? 0 })}</span>
            <span>{t("birdsEye.bookRow.journalRows", { count: book.journals.journal_entry_count })}</span>
            <span>{t("birdsEye.bookRow.quarantines", { count: book.journals.quarantine_count })}</span>
          </div>
          {!ready ? (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {book.readiness.hints[0] ?? book.journals.reason}
            </p>
          ) : null}
        </div>
        <div className="grid gap-1 text-left text-sm sm:grid-cols-2 lg:min-w-[260px]">
          <span>{hideSensitive ? t("common:state.hidden") : formatBtc(btcBalance)}</span>
          <span>
            {hideSensitive
              ? t("common:state.hidden")
              : formatFiatAmount(book.fiat.eurBalance, fiatCurrency)}
          </span>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap gap-2">
        {BOOK_ROUTES.map((route) => (
          <BookRouteButton
            key={route.to}
            route={route}
            profileId={book.profile.id}
            disabled={disabled}
            onOpenRoute={() => onOpenRoute(book.profile.id, route.to)}
          />
        ))}
      </div>
    </div>
  );
}

function BookRouteButton({
  route,
  profileId,
  disabled,
  onOpenRoute,
}: {
  route: (typeof BOOK_ROUTES)[number];
  profileId: string;
  disabled: boolean;
  onOpenRoute: () => void;
}) {
  const { t } = useTranslation("nav");
  const Icon = route.icon;
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      data-testid={`book-route-${profileId}-${route.id}`}
      disabled={disabled}
      onClick={onOpenRoute}
    >
      <Icon className="size-3.5" aria-hidden="true" />
      {t(route.labelKey)}
    </Button>
  );
}

function RefreshPanel({
  progress,
  summary,
  isRunning,
}: {
  progress: WorkspaceFreshnessProgress[];
  summary: WorkspaceFreshnessRun["summary"] | null;
  isRunning: boolean;
}) {
  const { t } = useTranslation("overview");
  return (
    <Card className="border-primary/20 bg-primary/5">
      <CardHeader className="border-b pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          {isRunning ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <CheckCircle2 className="size-4" aria-hidden="true" />
          )}
          {t("birdsEye.refreshPanel.title")}
        </CardTitle>
      </CardHeader>
      <CardContent className="grid gap-3 pt-4 md:grid-cols-[minmax(0,1fr)_280px]">
        <div className="grid gap-2">
          {progress.length ? (
            progress.map((item, index) => (
              <div
                key={`${item.profile?.id ?? "profile"}-${item.phase ?? "phase"}-${index}`}
                className="rounded-md border bg-background/80 px-3 py-2 text-sm"
              >
                <span className="font-medium">
                  {item.profile?.label ?? t("birdsEye.refreshPanel.fallbackBook")}
                </span>
                <span className="text-muted-foreground">
                  {" "}
                  · {formatPhase(item.phase)}
                  {item.source_label ? ` · ${item.source_label}` : ""}
                </span>
              </div>
            ))
          ) : (
            <div className="rounded-md border bg-background/80 px-3 py-2 text-sm text-muted-foreground">
              {t("birdsEye.refreshPanel.waiting")}
            </div>
          )}
        </div>
        <div className="rounded-md border bg-background/80 p-3 text-sm">
          {summary ? (
            <div className="space-y-1">
              <p className="font-medium">
                {t("birdsEye.refreshPanel.booksRefreshed", {
                  synced: summary.synced_books,
                  total: summary.books,
                })}
              </p>
              <p className="text-muted-foreground">
                {t("birdsEye.refreshPanel.summaryDetail", {
                  blocked: summary.reports_blocked,
                  rateLimited: summary.rate_limited,
                })}
              </p>
            </div>
          ) : (
            <p className="text-muted-foreground">
              {t("birdsEye.refreshPanel.placeholder")}
            </p>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function hasProgressCounter(
  progress: WorkspaceFreshnessProgress,
): progress is WorkspaceFreshnessProgress & { processed: number; total: number } {
  return (
    typeof progress.processed === "number" &&
    typeof progress.total === "number" &&
    progress.total > 0
  );
}

function workspaceProgressValue(progress: WorkspaceFreshnessProgress) {
  if (!hasProgressCounter(progress)) return undefined;
  return Math.max(
    0,
    Math.min(100, ((progress.processed ?? 0) / (progress.total ?? 1)) * 100),
  );
}

function formatWorkspaceProgressLabel(
  progress: WorkspaceFreshnessProgress,
  to?: OverviewTranslate,
) {
  const book = progress.profile?.label;
  const source = progress.source_label;
  const phase = formatPhase(progress.phase);
  const parts = [book, source && source !== book ? source : null, phase].filter(
    (part): part is string => Boolean(part),
  );

  if (hasProgressCounter(progress)) {
    parts.push(
      `${formatCount(progress.processed)} / ${formatCount(progress.total)}`,
    );
  } else if (typeof progress.processed === "number") {
    parts.push(
      to
        ? to("birdsEye.refreshPanel.scanned", {
            value: formatCount(progress.processed),
          })
        : `${formatCount(progress.processed)} scanned`,
    );
  }

  return parts.join(" · ");
}

function formatPhase(phase: string | undefined) {
  return syncProgressPhaseLabel(phase, "In progress");
}
