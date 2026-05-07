import { Link } from "@tanstack/react-router";
import {
  BookOpen,
  CheckCircle2,
  FileText,
  Loader2,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenPanelClassName, screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

interface JournalEntryType {
  type: string;
  count: number;
  gainLossEur: number;
}

interface RecentJournalEntry {
  date: string;
  type: string;
  wallet: string;
  asset: string;
  quantity: number;
  fiatValueEur: number;
  gainLossEur: number;
}

interface JournalsSnapshot {
  status: {
    workspace: string | null;
    profile: string | null;
    transactionCount: number;
    journalEntryCount: number;
    needsJournals: boolean;
    quarantines: number;
    lastProcessedAt: string | null;
  };
  entryTypes: JournalEntryType[];
  recent: RecentJournalEntry[];
}

interface JournalProcessResult {
  profile?: string;
  entries_created?: number;
  quarantined?: number;
  transfers_detected?: number;
  cross_asset_pairs?: number;
  auto_priced?: number;
  processed_transactions?: number;
  processed_at?: string;
}

type JournalTone = "good" | "warning" | "alert" | "neutral";

const eur = new Intl.NumberFormat("de-AT", {
  style: "currency",
  currency: "EUR",
});

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function Journals() {
  const { data, isLoading, isError, error } = useDaemon<JournalsSnapshot>(
    "ui.journals.snapshot",
  );
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const dataMode = useUiStore((s) => s.dataMode);
  const addNotification = useUiStore((s) => s.addNotification);
  const processJournals =
    useDaemonMutation<JournalProcessResult>("ui.journals.process");

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading journals...
      </div>
    );
  }

  if (isError || data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">Journals unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ??
                "The daemon did not return journal data."}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const status = snapshot.status;
  const readiness = journalReadiness(status);
  const maxEntryCount = Math.max(
    ...snapshot.entryTypes.map((entry) => entry.count),
    1,
  );

  const runJournalProcessing = () => {
    if (processJournals.isPending) return;
    processJournals.mutate(undefined, {
      onSuccess: (envelope) => {
        const payload = envelope.data;
        const parts = [
          payload?.processed_transactions !== undefined
            ? `${payload.processed_transactions} transactions`
            : null,
          payload?.entries_created !== undefined
            ? `${payload.entries_created} entries`
            : null,
          payload?.quarantined ? `${payload.quarantined} quarantined` : null,
        ].filter(Boolean);
        addNotification({
          title: "Journals processed",
          body: parts.join(", ") || "Journal state refreshed.",
          tone: payload?.quarantined ? "warning" : "success",
        });
      },
      onError: (mutationError) => {
        addNotification({
          title: "Journal processing failed",
          body:
            mutationError instanceof Error
              ? mutationError.message
              : "Could not process journals.",
          tone: "error",
        });
      },
    });
  };

  return (
    <div className={screenShellClassName}>
      <div className="rounded-xl border bg-card p-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex min-w-0 items-start gap-3">
            <span
              className={cn(
                "flex size-9 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
                toneStyles[readiness.tone],
              )}
              aria-hidden="true"
            >
              {readiness.tone === "good" ? (
                <CheckCircle2 className="size-4" />
              ) : (
                <RefreshCw className="size-4" />
              )}
            </span>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
                  Review · processing state
                </p>
                {dataMode === "mock" ? (
                  <Badge variant="outline" className="rounded-md">
                    Preview data
                  </Badge>
                ) : null}
              </div>
              <h1 className="mt-1 text-lg font-semibold sm:text-xl">
                Journal state
              </h1>
              <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
                Processed accounting rows that reports read from.
              </p>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            {status.quarantines ? (
              <Button asChild variant="outline" className="h-9">
                <Link to="/quarantine">
                  <ShieldAlert className="size-4" aria-hidden="true" />
                  Quarantine
                </Link>
              </Button>
            ) : null}
            <Button asChild variant="outline" className="h-9">
              <Link to="/reports">
                <FileText className="size-4" aria-hidden="true" />
                Reports
              </Link>
            </Button>
            <Button
              type="button"
              className="h-9"
              onClick={runJournalProcessing}
              disabled={processJournals.isPending}
            >
              {processJournals.isPending ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <RefreshCw className="size-4" aria-hidden="true" />
              )}
              Process journals
            </Button>
          </div>
        </div>
      </div>

      <div className="rounded-xl border bg-card">
        <div className="grid grid-cols-2 divide-x-0 divide-y divide-border sm:grid-cols-4 sm:divide-x sm:divide-y-0">
          <JournalMetric
            label="Status"
            value={readiness.label}
            sub={readiness.detail}
            tone={readiness.tone}
          />
          <JournalMetric
            label="Transactions"
            value={status.transactionCount.toLocaleString("en-US")}
            sub="input rows"
            tone="neutral"
          />
          <JournalMetric
            label="Journal entries"
            value={status.journalEntryCount.toLocaleString("en-US")}
            sub="processed rows"
            tone={status.journalEntryCount ? "good" : "neutral"}
          />
          <JournalMetric
            label="Quarantine"
            value={status.quarantines.toLocaleString("en-US")}
            sub={status.quarantines ? "held from reports" : "clear"}
            tone={status.quarantines ? "alert" : "good"}
          />
        </div>
      </div>

      <div className="grid min-w-0 grid-cols-1 gap-3 xl:grid-cols-[360px_minmax(0,1fr)]">
        <div className="min-w-0 rounded-xl border bg-card">
          <div className="border-b p-4">
            <h2 className="flex items-center gap-2 text-base font-semibold">
              <BookOpen className="size-4" aria-hidden="true" />
              Entry mix
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              Current processed journal composition.
            </p>
          </div>
          <div className="space-y-3 p-4">
            {snapshot.entryTypes.length ? (
              snapshot.entryTypes.map((entry) => (
                <div key={entry.type} className="space-y-2">
                  <div className="flex items-baseline justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">
                        {formatEntryType(entry.type)}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        {entry.count.toLocaleString("en-US")} rows
                      </p>
                    </div>
                    <p
                      className={cn(
                        "text-sm tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {eur.format(entry.gainLossEur)}
                    </p>
                  </div>
                  <div className="h-1.5 rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-primary"
                      style={{
                        width: `${Math.max(
                          (entry.count / maxEntryCount) * 100,
                          4,
                        )}%`,
                      }}
                    />
                  </div>
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">
                No processed journal rows yet.
              </p>
            )}
          </div>
        </div>

        <div className="min-w-0 rounded-xl border bg-card">
          <div className="flex flex-col gap-2 border-b p-4 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h2 className="text-base font-semibold">Recent journal rows</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                Latest rows produced by journal processing.
              </p>
            </div>
            <Badge
              variant="outline"
              className={cn("w-fit rounded-md", toneBadgeStyles[readiness.tone])}
            >
              {status.lastProcessedAt
                ? `Processed ${formatShortDate(status.lastProcessedAt)}`
                : "Never processed"}
            </Badge>
          </div>
          <div className="overflow-x-auto">
            <Table className="min-w-[840px]">
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead className="w-[130px]">Date</TableHead>
                  <TableHead className="w-[150px]">Type</TableHead>
                  <TableHead className="min-w-[180px]">Wallet</TableHead>
                  <TableHead className="w-[170px] text-right">
                    Quantity
                  </TableHead>
                  <TableHead className="w-[130px] text-right">Fiat</TableHead>
                  <TableHead className="w-[130px] text-right">
                    Gain/Loss
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {snapshot.recent.length ? (
                  snapshot.recent.map((entry, index) => (
                    <TableRow key={`${entry.date}-${entry.type}-${index}`}>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {entry.date}
                      </TableCell>
                      <TableCell>{formatEntryType(entry.type)}</TableCell>
                      <TableCell className={blurClass(hideSensitive)}>
                        {entry.wallet}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {entry.quantity.toFixed(8)} {entry.asset}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {eur.format(entry.fiatValueEur)}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {eur.format(entry.gainLossEur)}
                      </TableCell>
                    </TableRow>
                  ))
                ) : (
                  <TableRow>
                    <TableCell
                      colSpan={6}
                      className="h-24 text-center text-sm text-muted-foreground"
                    >
                      No recent rows exposed by the current journal snapshot.
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </div>
        </div>
      </div>
    </div>
  );
}

function JournalMetric({
  label,
  value,
  sub,
  tone,
}: {
  label: string;
  value: string;
  sub: string;
  tone: JournalTone;
}) {
  return (
    <div className="space-y-2 p-3 sm:p-4">
      <p className="text-xs font-medium text-muted-foreground sm:text-sm">
        {label}
      </p>
      <p
        className={cn(
          "text-xl font-semibold tabular-nums sm:text-2xl",
          toneTextStyles[tone],
        )}
      >
        {value}
      </p>
      <p className="text-xs text-muted-foreground">{sub}</p>
    </div>
  );
}

function journalReadiness(status: JournalsSnapshot["status"]): {
  label: string;
  detail: string;
  tone: JournalTone;
} {
  if (status.quarantines > 0) {
    return {
      label: "Blocked",
      detail: "review quarantine",
      tone: "alert",
    };
  }
  if (status.needsJournals) {
    return {
      label: "Stale",
      detail: "process before reports",
      tone: "warning",
    };
  }
  if (status.journalEntryCount > 0) {
    return {
      label: "Current",
      detail: "ready for reports",
      tone: "good",
    };
  }
  return {
    label: "Empty",
    detail: "no processed rows",
    tone: "neutral",
  };
}

function formatEntryType(type: string) {
  return type
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function formatShortDate(value: string) {
  return value.slice(0, 16).replace("T", " ");
}

const toneStyles: Record<JournalTone, string> = {
  good:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

const toneBadgeStyles: Record<JournalTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};

const toneTextStyles: Record<JournalTone, string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  alert: "text-red-600 dark:text-red-400",
  neutral: "text-foreground",
};
