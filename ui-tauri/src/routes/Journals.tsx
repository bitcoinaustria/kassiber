import { Link } from "@tanstack/react-router";
import { useState } from "react";
import {
  BookOpen,
  FileText,
  Loader2,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { JournalReportableEntries } from "@/components/kb/JournalReportableEntries";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
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
    freshnessStatus?: string;
    freshnessReason?: string;
  };
  entryTypes: JournalEntryType[];
  recent: RecentJournalEntry[];
  recentByType?: Record<string, RecentJournalEntry[]>;
}

interface DisplayJournalEntryType {
  type: string;
  count: number;
  gainLossEur: number;
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
  const [entryTypeFilter, setEntryTypeFilter] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState("state");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();

  if (isLoading) {
    return <ScreenSkeleton titleWidth="w-32" />;
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
  const displayEntryTypes = groupEntryTypesForDisplay(snapshot.entryTypes);
  const maxEntryCount = Math.max(
    ...displayEntryTypes.map((entry) => entry.count),
    1,
  );
  const filteredRecent = entryTypeFilter
    ? recentRowsForDisplayType(snapshot, entryTypeFilter)
    : snapshot.recent;
  return (
    <div className={screenShellClassName}>
      <Tabs value={activeTab} onValueChange={setActiveTab} className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <TabsList className="w-full justify-start overflow-x-auto sm:w-fit">
            <TabsTrigger value="state">State</TabsTrigger>
            <TabsTrigger value="reportable">Reportable entries</TabsTrigger>
          </TabsList>
          {activeTab === "state" ? (
            <div className="flex flex-wrap gap-2">
              {status.quarantines ? (
                <Button asChild variant="outline" className="h-8">
                  <Link to="/quarantine">
                    <ShieldAlert className="size-4" aria-hidden="true" />
                    Quarantine
                  </Link>
                </Button>
              ) : null}
              <Button asChild variant="outline" className="h-8">
                <Link to="/reports">
                  <FileText className="size-4" aria-hidden="true" />
                  Reports
                </Link>
              </Button>
              <Button
                type="button"
                className="h-8"
                onClick={runJournalProcessing}
                disabled={isProcessingJournals}
              >
                {isProcessingJournals ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                ) : (
                  <RefreshCw className="size-4" aria-hidden="true" />
                )}
                Process journals
              </Button>
            </div>
          ) : null}
        </div>

        <TabsContent value="state" className="mt-0 space-y-3">
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
              <div className="border-b p-3 sm:px-4">
                <h2 className="flex items-center gap-2 text-base font-semibold">
                  <BookOpen className="size-4" aria-hidden="true" />
                  Entry mix
                </h2>
                <p className="mt-0.5 text-xs text-muted-foreground sm:text-sm">
                  Current processed journal composition.
                </p>
              </div>
              <div className="space-y-2.5 p-3 sm:p-4">
                {displayEntryTypes.length ? (
                  displayEntryTypes.map((entry) => (
                    <button
                      key={entry.type}
                      type="button"
                      aria-pressed={entryTypeFilter === entry.type}
                      className={cn(
                        "w-full rounded-lg border p-2.5 text-left transition-colors",
                        entryTypeFilter === entry.type
                          ? "border-primary/45 bg-primary/5"
                          : "border-transparent hover:border-border hover:bg-muted/35",
                      )}
                      onClick={() =>
                        setEntryTypeFilter((current) =>
                          current === entry.type ? null : entry.type,
                        )
                      }
                    >
                      <div className="flex items-baseline justify-between gap-3">
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium">
                            {formatEntryType(entry.type)}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {entry.type === "transfer"
                              ? `${entry.count.toLocaleString("en-US")} in/out rows`
                              : `${entry.count.toLocaleString("en-US")} rows`}
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
                    </button>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">
                    No processed journal rows yet.
                  </p>
                )}
              </div>
            </div>

            <div className="min-w-0 rounded-xl border bg-card">
              <div className="border-b p-3 sm:px-4">
                <div>
                  <h2 className="text-base font-semibold">Recent journal rows</h2>
                  <p className="mt-0.5 text-xs text-muted-foreground sm:text-sm">
                    {entryTypeFilter
                      ? `Latest ${entryTypeDescription(entryTypeFilter)} produced by journal processing.`
                      : "Latest rows produced by journal processing."}
                  </p>
                </div>
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
                      <TableHead className="w-[130px] text-right">
                        Fiat
                      </TableHead>
                      <TableHead className="w-[130px] text-right">
                        Gain/Loss
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {filteredRecent.length ? (
                      filteredRecent.map((entry, index) => (
                        <TableRow key={`${entry.date}-${entry.type}-${index}`}>
                          <TableCell className="font-mono text-xs text-muted-foreground">
                            {entry.date}
                          </TableCell>
                          <TableCell>
                            <div className="grid gap-1">
                              <span>{formatEntryType(displayEntryType(entry.type))}</span>
                              {isTransferDirection(entry.type) ? (
                                <span className="text-xs text-muted-foreground">
                                  {entry.type === "transfer_out"
                                    ? "Outgoing side"
                                    : "Incoming side"}
                                </span>
                              ) : null}
                            </div>
                          </TableCell>
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
                          {entryTypeFilter
                            ? `No recent ${entryTypeDescription(entryTypeFilter)} exposed by the current journal snapshot.`
                            : "No recent rows exposed by the current journal snapshot."}
                        </TableCell>
                      </TableRow>
                    )}
                  </TableBody>
                </Table>
              </div>
            </div>
          </div>
        </TabsContent>

        <TabsContent value="reportable" className="mt-0">
          <JournalReportableEntries />
        </TabsContent>
      </Tabs>
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
      <p className="text-xs font-medium text-muted-foreground">
        {label}
      </p>
      <p
        className={cn(
          "text-xl font-semibold tabular-nums",
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
      detail: status.freshnessReason ?? "process before reports",
      tone: "warning",
    };
  }
  if (status.journalEntryCount > 0) {
    return {
      label: "Current",
      detail: status.freshnessReason ?? "ready for reports",
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

function displayEntryType(type: string) {
  return isTransferDirection(type) ? "transfer" : type;
}

function isTransferDirection(type: string) {
  return type === "transfer_in" || type === "transfer_out";
}

function entryTypeDescription(type: string) {
  return type === "transfer"
    ? "transfer in/out rows"
    : `${formatEntryType(type).toLowerCase()} rows`;
}

function groupEntryTypesForDisplay(
  entryTypes: JournalEntryType[],
): DisplayJournalEntryType[] {
  const grouped = new Map<string, DisplayJournalEntryType>();
  for (const entry of entryTypes) {
    const displayType = displayEntryType(entry.type);
    const existing = grouped.get(displayType);
    if (existing) {
      existing.count += entry.count;
      existing.gainLossEur += entry.gainLossEur;
    } else {
      grouped.set(displayType, {
        type: displayType,
        count: entry.count,
        gainLossEur: entry.gainLossEur,
      });
    }
  }
  return Array.from(grouped.values());
}

function recentRowsForDisplayType(
  snapshot: JournalsSnapshot,
  displayType: string,
) {
  if (displayType !== "transfer") {
    const rows =
      snapshot.recentByType?.[displayType] ??
      snapshot.recent.filter((entry) => entry.type === displayType);
    return sortRecentJournalRows(rows);
  }
  const byType = snapshot.recentByType ?? {};
  const transferRows = [
    ...(byType.transfer_out ?? []),
    ...(byType.transfer_in ?? []),
  ];
  if (transferRows.length) {
    return sortRecentJournalRows(transferRows);
  }
  return sortRecentJournalRows(
    snapshot.recent.filter((entry) => isTransferDirection(entry.type)),
  );
}

function sortRecentJournalRows(rows: RecentJournalEntry[]) {
  return [...rows].sort((left, right) => right.date.localeCompare(left.date));
}

const toneTextStyles: Record<JournalTone, string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  alert: "text-red-600 dark:text-red-400",
  neutral: "text-foreground",
};
