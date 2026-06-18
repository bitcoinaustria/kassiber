import { Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
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
  transactionId?: string;
  transactionExternalId?: string;
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
  const { t } = useTranslation(["journals", "nav", "common"]);
  const navigate = useNavigate();
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
          <h2 className="text-base font-semibold">
            {t("ledger.unavailable.title")}
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ?? t("ledger.unavailable.fallback")}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const status = snapshot.status;
  const readiness = journalReadiness(status, t);
  const displayEntryTypes = groupEntryTypesForDisplay(snapshot.entryTypes);
  const maxEntryCount = Math.max(
    ...displayEntryTypes.map((entry) => entry.count),
    1,
  );
  const filteredRecent = entryTypeFilter
    ? recentRowsForDisplayType(snapshot, entryTypeFilter)
    : snapshot.recent;
  const filteredEntryType = entryTypeFilter
    ? displayEntryTypes.find((entry) => entry.type === entryTypeFilter)
    : null;
  const filteredJournalEntryCount =
    filteredEntryType?.count ?? status.journalEntryCount;
  const journalRowsDescription = describeJournalRows(
    {
      rowCount: filteredRecent.length,
      totalCount: filteredJournalEntryCount,
      entryTypeFilter,
    },
    t,
  );
  const openTransaction = (entry: RecentJournalEntry) => {
    if (!entry.transactionId) return;
    void navigate({
      to: "/transactions",
      search: { tx: entry.transactionId },
    });
  };
  return (
    <div className={screenShellClassName}>
      <Tabs value={activeTab} onValueChange={setActiveTab} className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <TabsList className="w-full justify-start overflow-x-auto sm:w-fit">
            <TabsTrigger value="state">{t("ledger.tabs.state")}</TabsTrigger>
            <TabsTrigger value="reportable">
              {t("ledger.tabs.reportable")}
            </TabsTrigger>
          </TabsList>
          {activeTab === "state" ? (
            <div className="flex flex-wrap gap-2">
              {status.quarantines ? (
                <Button asChild variant="outline" className="h-8">
                  <Link to="/quarantine">
                    <ShieldAlert className="size-4" aria-hidden="true" />
                    {t("nav:book.quarantine")}
                  </Link>
                </Button>
              ) : null}
              <Button asChild variant="outline" className="h-8">
                <Link to="/reports">
                  <FileText className="size-4" aria-hidden="true" />
                  {t("nav:book.reports")}
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
                {t("ledger.actions.processJournals")}
              </Button>
            </div>
          ) : null}
        </div>

        <TabsContent value="state" className="mt-0 space-y-3">
          <div className="rounded-xl border bg-card">
            <div className="grid grid-cols-2 divide-x-0 divide-y divide-border sm:grid-cols-4 sm:divide-x sm:divide-y-0">
              <JournalMetric
                label={t("ledger.metric.status")}
                value={readiness.label}
                sub={readiness.detail}
                tone={readiness.tone}
              />
              <JournalMetric
                label={t("ledger.metric.transactions")}
                value={status.transactionCount.toLocaleString("en-US")}
                sub={t("ledger.metric.transactionsSub")}
                tone="neutral"
              />
              <JournalMetric
                label={t("ledger.metric.journalEntries")}
                value={status.journalEntryCount.toLocaleString("en-US")}
                sub={t("ledger.metric.journalEntriesSub")}
                tone={status.journalEntryCount ? "good" : "neutral"}
              />
              <JournalMetric
                label={t("ledger.metric.quarantine")}
                value={status.quarantines.toLocaleString("en-US")}
                sub={
                  status.quarantines
                    ? t("ledger.metric.quarantineHeld")
                    : t("ledger.metric.quarantineClear")
                }
                tone={status.quarantines ? "alert" : "good"}
              />
            </div>
          </div>

          <div className="grid min-w-0 grid-cols-1 gap-3 xl:grid-cols-[360px_minmax(0,1fr)]">
            <div className="min-w-0 rounded-xl border bg-card">
              <div className="border-b p-3 sm:px-4">
                <h2 className="flex items-center gap-2 text-base font-semibold">
                  <BookOpen className="size-4" aria-hidden="true" />
                  {t("ledger.entryMix.title")}
                </h2>
                <p className="mt-0.5 text-xs text-muted-foreground sm:text-sm">
                  {t("ledger.entryMix.description")}
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
                            {localizedEntryType(entry.type, t)}
                          </p>
                          <p className="text-xs text-muted-foreground">
                            {entry.type === "transfer"
                              ? t("ledger.entryMix.transferRows", {
                                  count: entry.count,
                                })
                              : t("ledger.entryMix.rows", {
                                  count: entry.count,
                                })}
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
                    {t("ledger.entryMix.empty")}
                  </p>
                )}
              </div>
            </div>

            <div className="min-w-0 rounded-xl border bg-card">
              <div className="flex items-start justify-between gap-3 border-b p-3 sm:px-4">
                <div className="min-w-0">
                  <h2 className="text-base font-semibold">
                    {t("ledger.entries.title")}
                  </h2>
                  <p className="mt-0.5 text-xs text-muted-foreground sm:text-sm">
                    {journalRowsDescription}
                  </p>
                </div>
                {status.journalEntryCount > snapshot.recent.length ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 shrink-0"
                    onClick={() => setActiveTab("reportable")}
                  >
                    {t("ledger.actions.viewLedgerEntries")}
                  </Button>
                ) : null}
              </div>
              <div className="overflow-x-auto">
                <Table className="min-w-[840px]">
                  <TableHeader>
                    <TableRow className="bg-muted/50 hover:bg-muted/50">
                      <TableHead className="w-[130px]">
                        {t("common:field.date")}
                      </TableHead>
                      <TableHead className="w-[150px]">
                        {t("common:field.type")}
                      </TableHead>
                      <TableHead className="min-w-[180px]">
                        {t("ledger.entries.column.wallet")}
                      </TableHead>
                      <TableHead className="w-[170px] text-right">
                        {t("ledger.entries.column.quantity")}
                      </TableHead>
                      <TableHead className="w-[130px] text-right">
                        {t("ledger.entries.column.fiat")}
                      </TableHead>
                      <TableHead className="w-[130px] text-right">
                        {t("ledger.entries.column.gainLoss")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {filteredRecent.length ? (
                      filteredRecent.map((entry, index) => {
                        const canOpenTransaction = Boolean(entry.transactionId);
                        return (
                        <TableRow
                          key={`${entry.date}-${entry.type}-${entry.transactionId ?? index}`}
                          role={canOpenTransaction ? "button" : undefined}
                          tabIndex={canOpenTransaction ? 0 : undefined}
                          aria-label={
                            canOpenTransaction
                              ? t("ledger.entries.openTransactionAria", {
                                  type: localizedEntryType(
                                    displayEntryType(entry.type),
                                    t,
                                  ),
                                })
                              : undefined
                          }
                          className={cn(
                            canOpenTransaction &&
                              "cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset",
                          )}
                          onClick={() => openTransaction(entry)}
                          onKeyDown={(event) => {
                            if (!canOpenTransaction) return;
                            if (event.key !== "Enter" && event.key !== " ") return;
                            event.preventDefault();
                            openTransaction(entry);
                          }}
                        >
                          <TableCell className="font-mono text-xs text-muted-foreground">
                            {entry.date}
                          </TableCell>
                          <TableCell>
                            <div className="grid gap-1">
                              <span>
                                {localizedEntryType(
                                  displayEntryType(entry.type),
                                  t,
                                )}
                              </span>
                              {isTransferDirection(entry.type) ? (
                                <span className="text-xs text-muted-foreground">
                                  {entry.type === "transfer_out"
                                    ? t("ledger.entries.outgoingSide")
                                    : t("ledger.entries.incomingSide")}
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
                        );
                      })
                    ) : (
                      <TableRow>
                        <TableCell
                          colSpan={6}
                          className="h-24 text-center text-sm text-muted-foreground"
                        >
                          {entryTypeFilter
                            ? t("ledger.entries.emptyFiltered", {
                                label: entryTypeDescription(entryTypeFilter, t),
                              })
                            : t("ledger.entries.empty")}
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

function journalReadiness(
  status: JournalsSnapshot["status"],
  t: TFunction<"journals">,
): {
  label: string;
  detail: string;
  tone: JournalTone;
} {
  if (status.quarantines > 0) {
    return {
      label: t("ledger.readiness.blocked"),
      detail: t("ledger.readiness.blockedDetail"),
      tone: "alert",
    };
  }
  if (status.needsJournals) {
    return {
      label: t("ledger.readiness.stale"),
      detail: status.freshnessReason ?? t("ledger.readiness.staleDetail"),
      tone: "warning",
    };
  }
  if (status.journalEntryCount > 0) {
    return {
      label: t("ledger.readiness.current"),
      detail: status.freshnessReason ?? t("ledger.readiness.currentDetail"),
      tone: "good",
    };
  }
  return {
    label: t("ledger.readiness.empty"),
    detail: t("ledger.readiness.emptyDetail"),
    tone: "neutral",
  };
}

function formatEntryType(type: string) {
  return type
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

// Daemon entry-type CODES map to translated labels; unknown codes fall back to
// a structural humanizer of the code itself (the code is the stable id).
const ENTRY_TYPE_LABEL_KEYS: Record<string, string> = {
  acquisition: "ledger.entryType.acquisition",
  disposal: "ledger.entryType.disposal",
  income: "ledger.entryType.income",
  fee: "ledger.entryType.fee",
  transfer_fee: "ledger.entryType.transferFee",
  transfer: "ledger.entryType.transfer",
  transfer_in: "ledger.entryType.transferIn",
  transfer_out: "ledger.entryType.transferOut",
  neutral_swap: "ledger.entryType.neutralSwap",
};

function localizedEntryType(type: string, t: TFunction<"journals">) {
  const key = ENTRY_TYPE_LABEL_KEYS[type];
  // dynamic key
  return key ? t(key as never) : formatEntryType(type);
}

function displayEntryType(type: string) {
  return isTransferDirection(type) ? "transfer" : type;
}

function isTransferDirection(type: string) {
  return type === "transfer_in" || type === "transfer_out";
}

function entryTypeDescription(type: string, t: TFunction<"journals">) {
  return type === "transfer"
    ? t("ledger.entries.labelTransferRows")
    : t("ledger.entries.labelTypedRows", {
        type: localizedEntryType(type, t),
      });
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

function describeJournalRows(
  {
    rowCount,
    totalCount,
    entryTypeFilter,
  }: {
    rowCount: number;
    totalCount: number;
    entryTypeFilter: string | null;
  },
  t: TFunction<"journals">,
) {
  const total = totalCount.toLocaleString("en-US");
  const rows = rowCount.toLocaleString("en-US");
  const label = entryTypeFilter
    ? entryTypeDescription(entryTypeFilter, t)
    : t("ledger.entries.labelProcessedRows");
  return t("ledger.entries.rowsDescription", { rows, total, label });
}

const toneTextStyles: Record<JournalTone, string> = {
  good: "text-emerald-600 dark:text-emerald-400",
  warning: "text-amber-600 dark:text-amber-400",
  alert: "text-red-600 dark:text-red-400",
  neutral: "text-foreground",
};
