import * as React from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowRight,
  BarChart3,
  BookOpen,
  CheckCircle2,
  Gauge,
  Loader2,
  RefreshCw,
  ShieldAlert,
  TableProperties,
  WalletCards,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
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
} from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type {
  WorkspaceBookOverview,
  WorkspaceOverviewSnapshot,
  WorkspaceTx,
} from "@/mocks/workspaceOverview";
import { useUiStore } from "@/store/ui";

type BookRoute = "/overview" | "/transactions" | "/journals" | "/quarantine" | "/connections" | "/reports";

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

const BOOK_ROUTES: Array<{
  label: string;
  to: BookRoute;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
}> = [
  { label: "Overview", to: "/overview", icon: Gauge },
  { label: "Transactions", to: "/transactions", icon: TableProperties },
  { label: "Ledger", to: "/journals", icon: BookOpen },
  { label: "Quarantine", to: "/quarantine", icon: ShieldAlert },
  { label: "Wallets", to: "/connections", icon: WalletCards },
  { label: "Reports", to: "/reports", icon: BarChart3 },
];

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
    return (
      <div className={screenShellClassName}>
        <Card className="border-destructive/30 bg-destructive/10">
          <CardContent className="py-4 text-sm text-destructive">
            {error instanceof Error
              ? error.message
              : "Could not load Bird's Eye."}
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <BirdsEyeView
      snapshot={overviewEnvelope?.data ?? null}
      workspaceId={workspaceId}
      isFetching={isFetching}
    />
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
  const navigate = useNavigate();
  const switchProfile = useDaemonMutation<{ activeProfileId: string }>(
    "ui.profiles.switch",
  );
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
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
      const book = record.profile?.label ?? "book";
      const phase = formatPhase(record.phase);
      updateNotification(noticeRef.current, {
        body: `${book}: ${phase}`,
        progress: { indeterminate: true, label: phase },
      });
    },
  });

  const title = snapshot?.workspace?.label ?? "Book set";
  const fiat = snapshot?.fiat ?? null;
  const books = snapshot?.books ?? [];
  const readyBooks = snapshot?.status.readyBooks ?? books.filter((book) => book.readiness.ready).length;
  const blockedBooks = snapshot?.status.blockedBooks ?? books.length - readyBooks;

  const handleRefresh = React.useCallback(() => {
    if (!workspaceId || refreshWorkspace.isPending) return;
    setProgress([]);
    setRefreshSummary(null);
    noticeRef.current = addNotification({
      title: "Book set refresh started",
      body: "Refreshing each book in this set.",
      tone: "warning",
      dedupeKey: `workspace-refresh-${workspaceId}`,
      progress: { indeterminate: true, label: "Starting" },
    });
    refreshWorkspace.mutate(
      { workspace_id: workspaceId, rates: true, journals: true, run: true },
      {
        onSuccess: (envelope) => {
          setRefreshSummary(envelope.data?.summary ?? null);
          const summary = envelope.data?.summary;
          const needsAttention = Boolean(
            summary && (summary.errors > 0 || summary.reports_blocked > 0),
          );
          const notification = {
            title: needsAttention
              ? "Book set refresh needs attention"
              : "Book set refresh finished",
            body: summary
              ? `${summary.synced_books}/${summary.books} books refreshed; ${summary.reports_blocked} still blocked.`
              : "Refresh finished.",
            tone: needsAttention ? "warning" : "success",
            dedupeKey: `workspace-refresh-${workspaceId}`,
            progress: undefined,
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
            title: "Book set refresh failed",
            body:
              refreshError instanceof Error
                ? refreshError.message
                : "Could not refresh this book set.",
            tone: "error",
            dedupeKey: `workspace-refresh-${workspaceId}`,
            progress: undefined,
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
    updateNotification,
    workspaceId,
  ]);

  const openBookRoute = React.useCallback(
    (profileId: string, route: BookRoute) => {
      void switchProfile
        .mutateAsync({ profile_id: profileId })
        .then(() => navigate({ to: route }))
        .catch((switchError: unknown) => {
          addNotification({
            title: "Could not open book",
            body:
              switchError instanceof Error
                ? switchError.message
                : "Kassiber could not switch to that book.",
            tone: "error",
            dedupeKey: `birds-eye-open-book-${profileId}`,
          });
        });
    },
    [addNotification, navigate, switchProfile],
  );

  return (
    <div
      className={cn(screenShellClassName, "relative")}
      aria-busy={isFetching || refreshWorkspace.isPending}
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
        <div className="min-w-0 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-2xl font-semibold tracking-tight">
              Bird&apos;s Eye
            </h2>
            <Badge variant="secondary">Book set</Badge>
            {fiat?.mixed ? <Badge variant="outline">Mixed fiat</Badge> : null}
          </div>
          <p className="max-w-3xl text-sm text-muted-foreground">
            {title}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" asChild>
            <Link to="/books">Books</Link>
          </Button>
          <Button
            type="button"
            onClick={handleRefresh}
            disabled={refreshWorkspace.isPending || !workspaceId}
          >
            {refreshWorkspace.isPending ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            Refresh book set
          </Button>
        </div>
      </div>

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          label="BTC total"
          value={hideSensitive ? "Hidden" : formatBtc(fiat?.btcBalance ?? 0)}
          detail={`${books.length} books`}
        />
        <MetricCard
          label={fiat?.mixed ? "Fiat rollup" : "Fiat total"}
          value={
            hideSensitive
              ? "Hidden"
              : fiat?.mixed
                ? "Mixed"
                : formatFiatAmount(fiat?.eurBalance ?? 0, fiat?.fiatCurrency ?? "EUR")
          }
          detail={
            fiat?.mixed
              ? fiat.label ?? "Per-book fiat rows only"
              : fiat?.fiatCurrency ?? "No fiat currency"
          }
        />
        <MetricCard
          label="Readiness"
          value={`${readyBooks}/${books.length}`}
          detail={blockedBooks ? `${blockedBooks} books need attention` : "All books ready"}
        />
        <MetricCard
          label="Quarantine"
          value={String(snapshot?.status.quarantines ?? 0)}
          detail={
            snapshot?.status.needsJournals
              ? "Journals need processing"
              : "No stale journals"
          }
        />
      </div>

      {refreshWorkspace.isPending || progress.length || refreshSummary ? (
        <RefreshPanel
          progress={progress}
          summary={refreshSummary}
          isRunning={refreshWorkspace.isPending}
        />
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_360px]">
        <Card>
          <CardHeader className="border-b pb-3">
            <CardTitle className="text-base">Books</CardTitle>
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
                This book set does not have any books yet.
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b pb-3">
            <CardTitle className="text-base">Fiat Rows</CardTitle>
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
                        ? "Hidden"
                        : formatFiatAmount(row.balance, row.fiatCurrency)}
                    </p>
                  </div>
                  <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
                    <span>Basis {hideSensitive ? "Hidden" : formatFiatAmount(row.costBasis, row.fiatCurrency)}</span>
                    <span>YTD {hideSensitive ? "Hidden" : formatFiatAmount(row.realizedYTD, row.fiatCurrency)}</span>
                  </div>
                </div>
              ))
            ) : (
              <div className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
                No fiat rows yet.
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      <RecentWorkspaceActivity
        txs={snapshot?.activityTxs?.length ? snapshot.activityTxs : snapshot?.txs ?? []}
        hideSensitive={hideSensitive}
      />
    </div>
  );
}

function MetricCard({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <Card className="py-4">
      <CardContent className="space-y-1 px-4">
        <p className="text-sm text-muted-foreground">{label}</p>
        <p className="text-2xl font-semibold tracking-tight">{value}</p>
        <p className="line-clamp-2 min-h-8 text-xs text-muted-foreground">
          {detail}
        </p>
      </CardContent>
    </Card>
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
              {ready ? "Ready" : "Attention"}
            </Badge>
            <Badge variant="outline">{fiatCurrency}</Badge>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
            <span>{book.connections.length} wallets</span>
            <span>{book.status.transactionCount ?? 0} txs</span>
            <span>{book.journals.journal_entry_count} journal rows</span>
            <span>{book.journals.quarantine_count} quarantines</span>
          </div>
          {!ready ? (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {book.readiness.hints[0] ?? book.journals.reason}
            </p>
          ) : null}
        </div>
        <div className="grid gap-1 text-left text-sm sm:grid-cols-2 lg:min-w-[260px]">
          <span>{hideSensitive ? "Hidden" : formatBtc(btcBalance)}</span>
          <span>
            {hideSensitive
              ? "Hidden"
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
  const Icon = route.icon;
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      data-testid={`book-route-${profileId}-${route.label.toLowerCase()}`}
      disabled={disabled}
      onClick={onOpenRoute}
    >
      <Icon className="size-3.5" aria-hidden="true" />
      {route.label}
    </Button>
  );
}

function RecentWorkspaceActivity({
  txs,
  hideSensitive,
}: {
  txs: WorkspaceTx[];
  hideSensitive: boolean;
}) {
  return (
    <Card>
      <CardHeader className="border-b pb-3">
        <div className="flex items-center justify-between gap-3">
          <CardTitle className="text-base">Recent Activity</CardTitle>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/transactions">
              Active book
              <ArrowRight className="size-4" aria-hidden="true" />
            </Link>
          </Button>
        </div>
      </CardHeader>
      <CardContent className="pt-0">
        {txs.length ? (
          <div className="divide-y">
            {txs.slice(0, 10).map((tx) => (
              <div
                key={`${tx.profileId}-${tx.id}`}
                className="grid gap-2 py-3 sm:grid-cols-[minmax(0,1fr)_auto]"
              >
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">
                    {tx.counter || tx.type}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {tx.book.label} · {tx.date}
                  </p>
                </div>
                <p
                  className={cn(
                    "text-sm font-medium",
                    tx.amountSat >= 0
                      ? "text-emerald-700 dark:text-emerald-300"
                      : "text-red-700 dark:text-red-300",
                  )}
                >
                  {hideSensitive
                    ? "Hidden"
                    : formatBtc(tx.amountSat / 100_000_000, { sign: true })}
                </p>
              </div>
            ))}
          </div>
        ) : (
          <div className="py-6 text-sm text-muted-foreground">
            No recent activity across this book set.
          </div>
        )}
      </CardContent>
    </Card>
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
  return (
    <Card className="border-primary/20 bg-primary/5">
      <CardHeader className="border-b pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          {isRunning ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <CheckCircle2 className="size-4" aria-hidden="true" />
          )}
          Book Set Refresh
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
                  {item.profile?.label ?? "Book"}
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
              Waiting for refresh progress.
            </div>
          )}
        </div>
        <div className="rounded-md border bg-background/80 p-3 text-sm">
          {summary ? (
            <div className="space-y-1">
              <p className="font-medium">
                {summary.synced_books}/{summary.books} books refreshed
              </p>
              <p className="text-muted-foreground">
                {summary.reports_blocked} reports blocked · {summary.rate_limited} rate-limited sources
              </p>
            </div>
          ) : (
            <p className="text-muted-foreground">
              Per-book sync, rate, and journal status appears here.
            </p>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function formatPhase(phase: string | undefined) {
  if (!phase) return "Working";
  return phase
    .replace(/_/g, " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}
