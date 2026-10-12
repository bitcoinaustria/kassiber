import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowUpRight,
  CalendarCheck,
  CircleCheck,
  Download,
  FileSpreadsheet,
  ListChecks,
  ShieldAlert,
  Tags,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { keepPreviousData } from "@tanstack/react-query";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { exportBasename, saveDaemonExport } from "@/lib/exportFile";
import { currentUiLocale, formatCount } from "@/lib/localeFormat";
import { pageHeaderActionClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { CrossCheckPanel } from "./CrossCheckPanel";
import { ExportFormatMark, ExportFormatPicker } from "./ExportFormatPicker";
import { ExportWalletPicker, type ExportWallet } from "./ExportWalletPicker";
import { exportFormat, isToolFormat, type ExportFormatId } from "./exportFormats";
import { defaultExportYear } from "./exportYears";
import { YearPicker } from "./YearPicker";

interface ExportResult {
  file?: string;
  rows?: number | Record<string, number>;
  reconciled?: boolean;
  warnings?: string[];
}

interface PreviewSheet {
  name: string;
  columns: string[];
  rows: string[][];
  total: number;
}

interface ExportPreview {
  file_name: string;
  row_count: number;
  sheets: PreviewSheet[];
  warnings: string[];
  reconciled: boolean | null;
}

type ExportKind = "transactions" | "yearHandoff" | "labels" | "compare";

interface QuarantineBlock {
  count: number;
  transactions: {
    transaction_id?: string;
    transaction_ref?: string;
    wallet?: string;
    occurred_at?: string;
    reason?: string;
  }[];
}

const PREVIEW_DEBOUNCE_MS = 300;
const QUARANTINE_CODES = new Set([
  "year_handoff_quarantined",
  "transaction_export_quarantined",
]);
const QUARANTINE_PREVIEW = 5;
const PREVIEW_EXPORT: Record<Exclude<ExportKind, "compare">, string> = {
  transactions: "transactions",
  yearHandoff: "year_handoff",
  labels: "labels",
};

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function quarantineBlock(error: unknown): QuarantineBlock | null {
  if (!(error instanceof DaemonRequestError)) return null;
  const daemonError = error.envelope.error;
  if (!daemonError || !QUARANTINE_CODES.has(daemonError.code ?? "")) return null;
  const details = (daemonError.details ?? {}) as Partial<QuarantineBlock>;
  return {
    count: Number(details.count ?? 0),
    transactions: Array.isArray(details.transactions) ? details.transactions : [],
  };
}

function shortDate(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), { dateStyle: "medium" }).format(parsed);
}

function shortTxid(value: string | undefined): string {
  if (!value) return "";
  return value.length > 20 ? `${value.slice(0, 10)}…${value.slice(-6)}` : value;
}

const NUMERIC_CELL = /^-?[\d.,\s']+$/;

function PreviewTable({ sheet, emptyLabel }: { sheet: PreviewSheet; emptyLabel: string }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto rounded-lg border bg-background/70">
      <table className="w-max min-w-full border-separate border-spacing-0 text-xs">
        <thead>
          <tr>
            <th className="sticky top-0 left-0 z-20 w-9 border-r border-b bg-muted px-2 py-2" />
            {sheet.columns.map((column) => (
              <th
                key={column}
                scope="col"
                className="sticky top-0 z-10 border-b bg-muted px-3 py-2 text-left font-medium whitespace-nowrap text-muted-foreground"
              >
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sheet.rows.length ? (
            sheet.rows.map((row, index) => (
              <tr key={index} className="even:bg-muted/30">
                <td className="sticky left-0 border-r bg-muted/60 px-2 py-1.5 text-right tabular-nums text-muted-foreground">
                  {index + 1}
                </td>
                {row.map((cell, cellIndex) => (
                  <td
                    key={cellIndex}
                    title={cell.length > 32 ? cell : undefined}
                    className={cn(
                      "max-w-64 truncate px-3 py-1.5 whitespace-nowrap",
                      cell && NUMERIC_CELL.test(cell) ? "text-right tabular-nums" : undefined,
                    )}
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))
          ) : (
            <tr>
              <td
                colSpan={sheet.columns.length + 1}
                className="px-3 py-10 text-center text-muted-foreground"
              >
                {emptyLabel}
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function PreviewSkeleton() {
  return (
    <div className="grid flex-1 content-start gap-2 rounded-lg border bg-background/70 p-3">
      <Skeleton className="h-6 w-full" />
      {Array.from({ length: 7 }, (_, index) => (
        <Skeleton key={index} className="h-5" style={{ width: `${92 - index * 6}%` }} />
      ))}
    </div>
  );
}

/** The value, settled: it only follows `value` after `delayMs` without change. */
function useSettled<T>(value: T, delayMs: number): T {
  const [settled, setSettled] = React.useState(value);
  React.useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), delayMs);
    return () => window.clearTimeout(timer);
  }, [value, delayMs]);
  return settled;
}

export function TransactionsExportDialog({
  years: accountingYears,
}: {
  /** The book's years, newest first, in its own calendar (from the core). */
  years: number[] | undefined;
}) {
  const { t } = useTranslation(["transactions", "common"]);
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [open, setOpen] = React.useState(false);
  const [kind, setKind] = React.useState<ExportKind>("transactions");
  const [excludedWallets, setExcludedWallets] = React.useState<Set<string>>(() => new Set());
  const [format, setFormat] = React.useState<ExportFormatId>("xlsx");
  const [sheetName, setSheetName] = React.useState<string | null>(null);
  const [exportBlock, setExportBlock] = React.useState<QuarantineBlock | null>(null);
  const years = React.useMemo(() => accountingYears ?? [], [accountingYears]);
  const [year, setYear] = React.useState<number | null>(null);
  const selectedYear =
    year !== null && years.includes(year) ? year : defaultExportYear(years);
  const walletsQuery = useDaemon<{ wallets: ExportWallet[] }>("ui.wallets.list", undefined, {
    enabled: open,
  });
  const wallets = walletsQuery.data?.data?.wallets ?? [];
  const selectedWallets = wallets.filter((wallet) => !excludedWallets.has(wallet.id));
  // Every wallet checked means "all wallets", which also covers wallets added
  // later; a partial selection names them.
  const allWalletsSelected = selectedWallets.length === wallets.length;
  const noWalletSelected = wallets.length > 0 && selectedWallets.length === 0;
  const walletArgs: Record<string, unknown> = allWalletsSelected
    ? {}
    : { wallets: selectedWallets.map((wallet) => wallet.id) };
  const formatSpec = exportFormat(format);

  const previewArgs: Record<string, unknown> = {
    export: kind === "compare" ? "" : PREVIEW_EXPORT[kind],
  };
  if (kind === "transactions") Object.assign(previewArgs, { format, ...walletArgs });
  if (kind === "yearHandoff") previewArgs.year = selectedYear;
  if (kind === "labels") Object.assign(previewArgs, walletArgs);
  // Toggling several wallets in a row asks for one preview, not one per click.
  const settledPreviewArgs = useSettled(previewArgs, PREVIEW_DEBOUNCE_MS);
  const usesWallets = kind !== "yearHandoff";
  const preview = useDaemon<ExportPreview>("ui.exports.preview", settledPreviewArgs, {
    enabled:
      open &&
      kind !== "compare" &&
      (kind !== "yearHandoff" || selectedYear !== null) &&
      !(usesWallets && noWalletSelected),
    retry: false,
    staleTime: 0,
    placeholderData: keepPreviousData,
  });
  const previewUpdating =
    preview.isFetching ||
    JSON.stringify(previewArgs) !== JSON.stringify(settledPreviewArgs);
  const previewData = preview.data?.data;
  const block = quarantineBlock(preview.error) ?? exportBlock;
  const previewFailed = Boolean(preview.error) && !block;

  const exportXlsx = useDaemonMutation<ExportResult>("ui.transactions.export_xlsx");
  const exportCsv = useDaemonMutation<ExportResult>("ui.transactions.export_csv");
  const exportPlatform = useDaemonMutation<ExportResult>(
    "ui.transactions.export_platform_csv",
  );
  const exportYearHandoff = useDaemonMutation<ExportResult>(
    "ui.reports.export_year_handoff",
  );
  const exportLabels = useDaemonMutation<ExportResult>("ui.metadata.bip329.export");
  const isExporting = [
    exportXlsx,
    exportCsv,
    exportPlatform,
    exportYearHandoff,
    exportLabels,
  ].some((mutation) => mutation.isPending);

  // A different choice is a different file: forget the previous export's
  // block and start on the first sheet again.
  React.useEffect(() => {
    setExportBlock(null);
    setSheetName(null);
  }, [kind, excludedWallets, format, selectedYear]);

  const sheets = previewData?.sheets ?? [];
  const activeSheet =
    sheets.find((sheet) => sheet.name === sheetName) ??
    sheets.find((sheet) => sheet.name === "Transactions") ??
    sheets[0];

  const notifyFailure = (error: unknown) => {
    addNotification({
      title: t("dashboard.export.failed"),
      body: errorMessage(error),
      tone: "error",
    });
  };

  const finish = async (
    payload: ExportResult | undefined,
    file: { defaultName: string; filter: { name: string; extensions: string[] } },
    titles: { done: string; doneWithWarnings: string; saveTitle: string },
    fallbackBody: string,
  ) => {
    setOpen(false);
    try {
      const { savedPath, copied } = await saveDaemonExport({
        exportPath: payload?.file ?? "",
        title: titles.saveTitle,
        defaultName: file.defaultName,
        filters: [file.filter],
      });
      const warnings = payload?.warnings ?? [];
      const flagged = payload?.reconciled === false || warnings.length > 0;
      addNotification({
        title: flagged ? titles.doneWithWarnings : titles.done,
        body: [
          copied
            ? t("dashboard.export.savedTo", { name: exportBasename(savedPath) })
            : fallbackBody,
          ...warnings,
        ].join("\n"),
        tone: flagged ? "warning" : "success",
      });
    } catch (error) {
      notifyFailure(error);
    }
  };

  const handleError = (error: unknown) => {
    const blocked = quarantineBlock(error);
    if (blocked) {
      setExportBlock(blocked);
      return;
    }
    notifyFailure(error);
  };

  const runExport = () => {
    setExportBlock(null);
    if (kind === "yearHandoff") {
      if (selectedYear === null) return;
      exportYearHandoff.mutate(
        { year: selectedYear },
        {
          onSuccess: (envelope) =>
            void finish(
              envelope?.data,
              {
                defaultName: `kassiber-year-handoff-${selectedYear}.xlsx`,
                filter: exportFormat("xlsx").filter,
              },
              {
                done: t("dashboard.export.yearHandoff.done", { year: selectedYear }),
                doneWithWarnings: t("dashboard.export.yearHandoff.doneWithWarnings", {
                  year: selectedYear,
                }),
                saveTitle: t("dashboard.export.yearHandoff.saveTitle"),
              },
              exportBasename(envelope?.data?.file ?? ""),
            ),
          onError: handleError,
        },
      );
      return;
    }
    if (kind === "labels") {
      exportLabels.mutate(
        { mode: "all", ...walletArgs },
        {
          onSuccess: (envelope) =>
            void finish(
              envelope?.data,
              {
                defaultName: "kassiber-labels.jsonl",
                filter: { name: "BIP 329 labels", extensions: ["jsonl"] },
              },
              {
                done: t("dashboard.export.labels.done"),
                doneWithWarnings: t("dashboard.export.labels.done"),
                saveTitle: t("dashboard.export.labels.saveTitle"),
              },
              exportBasename(envelope?.data?.file ?? ""),
            ),
          onError: handleError,
        },
      );
      return;
    }
    const onSuccess = (envelope: { data?: ExportResult } | undefined) =>
      void finish(
        envelope?.data,
        formatSpec,
        {
          done: t("dashboard.export.done"),
          doneWithWarnings: t("dashboard.export.doneWithWarnings"),
          saveTitle: t("dashboard.export.saveTitle"),
        },
        t("dashboard.export.rows", {
          count: typeof envelope?.data?.rows === "number" ? envelope.data.rows : 0,
        }),
      );
    if (formatSpec.daemon.kind === "platform") {
      exportPlatform.mutate(
        { platform: formatSpec.daemon.platform, ...walletArgs },
        { onSuccess, onError: handleError },
      );
      return;
    }
    (formatSpec.daemon.kind === "xlsx" ? exportXlsx : exportCsv).mutate(walletArgs, {
      onSuccess,
      onError: handleError,
    });
  };

  const openQuarantine = () => {
    setOpen(false);
    void navigate({ to: "/quarantine" });
  };

  const rowCount = previewData?.row_count ?? 0;
  // The name is known before the preview arrives (or when it is blocked).
  const fileName =
    previewData?.file_name ??
    (kind === "yearHandoff"
      ? `kassiber-year-handoff-${selectedYear ?? ""}.xlsx`
      : kind === "labels"
        ? "kassiber-labels.jsonl"
        : formatSpec.defaultName);
  const submitLabel = isExporting
    ? t("dashboard.export.exporting")
    : kind === "yearHandoff"
      ? t("dashboard.export.submitYear", { year: selectedYear ?? "" })
      : previewData
        ? t(kind === "labels" ? "dashboard.export.submitLabels" : "dashboard.export.submitRows", {
            count: rowCount,
            formatted: formatCount(rowCount),
          })
        : t("dashboard.export.submit");
  const walletLabel = allWalletsSelected
    ? t("dashboard.export.allWallets")
    : t("dashboard.export.someWallets", {
        selected: selectedWallets.length,
        total: wallets.length,
      });
  const importHint =
    kind === "transactions" && isToolFormat(formatSpec)
      ? t(`dashboard.export.importHint.${formatSpec.id}`)
      : kind === "yearHandoff"
        ? t("dashboard.export.importHint.yearHandoff")
        : null;

  const walletPicker = (
    <ExportWalletPicker
      wallets={wallets}
      loading={walletsQuery.isLoading}
      excluded={excludedWallets}
      onExcludedChange={setExcludedWallets}
    />
  );

  return (
    <Dialog open={open} onOpenChange={(next) => !isExporting && setOpen(next)}>
      <DialogTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className={pageHeaderActionClassName}
          aria-label={t("dashboard.export.label")}
          disabled={isExporting}
        >
          <Download className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">{t("dashboard.export.label")}</span>
        </Button>
      </DialogTrigger>
      <DialogContent
        // A workspace rather than a form: choices on the left, the file as it
        // will be written on the right. A fixed size keeps it still while the
        // preview changes.
        className="flex h-[min(52rem,var(--kb-dialog-max-h))] w-[min(calc(100vw-2rem),80rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:max-w-none"
      >
        <header className="flex shrink-0 flex-wrap items-center gap-x-6 gap-y-3 border-b px-5 py-3.5 pr-14">
          <DialogTitle className="text-base">{t("dashboard.export.title")}</DialogTitle>
          <DialogDescription className="sr-only">{t("dashboard.export.description")}</DialogDescription>
          <Tabs value={kind} onValueChange={(value) => setKind(value as ExportKind)}>
            <TabsList>
              <TabsTrigger value="transactions" className="px-3">
                <FileSpreadsheet aria-hidden="true" />
                {t("dashboard.export.tabs.transactions")}
              </TabsTrigger>
              <TabsTrigger value="yearHandoff" className="px-3">
                <CalendarCheck aria-hidden="true" />
                {t("dashboard.export.tabs.yearHandoff")}
              </TabsTrigger>
              <TabsTrigger value="labels" className="px-3">
                <Tags aria-hidden="true" />
                {t("dashboard.export.tabs.labels")}
              </TabsTrigger>
              <TabsTrigger value="compare" className="px-3">
                <ListChecks aria-hidden="true" />
                {t("dashboard.export.tabs.compare")}
              </TabsTrigger>
            </TabsList>
          </Tabs>
        </header>

        {kind === "compare" ? (
          <CrossCheckPanel
            open={open}
            years={years}
            defaultYear={selectedYear}
            onOpenTransaction={(ref) => {
              setOpen(false);
              void navigate({ to: "/transactions", search: { tx: ref } });
            }}
            onDone={() => setOpen(false)}
          />
        ) : (
          <>

          <div className="grid min-h-0 flex-1 overflow-y-auto lg:grid-cols-[20rem_minmax(0,1fr)] lg:overflow-hidden">
            <div className="flex min-h-0 flex-col gap-5 border-b bg-muted/25 p-5 lg:border-r lg:border-b-0">
              {kind === "transactions" ? (
                <>
                  <ExportFormatPicker value={format} onChange={setFormat} />
                </>
              ) : null}

              {kind === "yearHandoff" ? (
                <>
                  <div className="grid gap-1.5">
                    <span className="text-sm font-medium">{t("dashboard.export.year")}</span>
                    {years.length ? (
                      <YearPicker
                        years={years}
                        value={selectedYear}
                        onChange={(next) => next !== null && setYear(next)}
                        className="w-full"
                      />
                    ) : (
                      <p className="text-sm text-muted-foreground">{t("dashboard.export.noYears")}</p>
                    )}
                  </div>
                  <ul className="grid gap-2 text-sm">
                    {previewData && previewData.reconciled !== null ? (
                      <li className="flex items-center gap-2">
                        {previewData.reconciled ? (
                          <CircleCheck className="size-4 shrink-0 text-emerald-500" aria-hidden="true" />
                        ) : (
                          <AlertTriangle className="size-4 shrink-0 text-amber-500" aria-hidden="true" />
                        )}
                        {previewData.reconciled
                          ? t("dashboard.export.status.reconciled")
                          : t("dashboard.export.status.differences")}
                      </li>
                    ) : null}
                  </ul>
                </>
              ) : null}

              {kind === "transactions" || kind === "labels" ? walletPicker : null}

            </div>

            <div className="flex min-h-[22rem] min-w-0 flex-col gap-3 p-5">
              <div className="flex min-w-0 items-center gap-2.5 text-sm">
                {kind === "labels" ? (
                  <Tags className="size-4 shrink-0 text-primary" aria-hidden="true" />
                ) : kind === "transactions" ? (
                  <ExportFormatMark option={formatSpec} className="size-6" />
                ) : (
                  <FileSpreadsheet className="size-4 shrink-0 text-primary" aria-hidden="true" />
                )}
                <span className="min-w-0 truncate font-mono text-xs">{fileName}</span>
                {kind === "transactions" ? (
                  <span className="rounded-sm border px-1.5 py-px font-mono text-[10px] tracking-wide text-muted-foreground">
                    {formatSpec.extension}
                  </span>
                ) : null}
                {previewData && !block ? (
                  <span className="text-xs text-muted-foreground">
                    {sheets.length > 1
                      ? t("dashboard.export.fileSheets", { count: sheets.length })
                      : t("dashboard.export.fileRows", {
                          count: rowCount,
                          formatted: formatCount(rowCount),
                        })}
                  </span>
                ) : null}
                {previewUpdating && previewData ? (
                  <span className="ml-auto animate-pulse text-xs text-muted-foreground">
                    {t("dashboard.export.preview.updating")}
                  </span>
                ) : null}
              </div>
              {block ? (
                <div
                  role="alert"
                  className="grid gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm"
                >
                  <div className="flex items-start gap-2.5">
                    <ShieldAlert
                      className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
                      aria-hidden="true"
                    />
                    <div className="min-w-0">
                      <p className="font-medium">
                        {t("dashboard.export.blocked.title", { count: block.count })}
                      </p>
                      <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
                        {t("dashboard.export.blocked.body")}
                      </p>
                    </div>
                  </div>
                  <ul className="grid gap-1 text-xs">
                    {block.transactions.slice(0, QUARANTINE_PREVIEW).map((row) => (
                      <li
                        key={row.transaction_ref ?? row.transaction_id}
                        className="grid grid-cols-[auto_1fr_auto] items-baseline gap-x-3 rounded-md bg-background/60 px-2 py-1.5"
                      >
                        <span className="tabular-nums text-muted-foreground">
                          {shortDate(row.occurred_at)}
                        </span>
                        <span className="min-w-0 truncate">
                          <span className="font-medium">{row.wallet}</span>
                          <span className="ml-2 font-mono text-muted-foreground">
                            {shortTxid(row.transaction_id || row.transaction_ref)}
                          </span>
                        </span>
                        <span className="text-muted-foreground">{row.reason}</span>
                      </li>
                    ))}
                  </ul>
                  {block.count > QUARANTINE_PREVIEW ? (
                    <p className="text-xs text-muted-foreground">
                      {t("dashboard.export.blocked.more", {
                        count: block.count - QUARANTINE_PREVIEW,
                      })}
                    </p>
                  ) : null}
                  <div>
                    <Button type="button" size="sm" variant="outline" onClick={openQuarantine}>
                      <ArrowUpRight className="size-4" aria-hidden="true" />
                      {t("dashboard.export.blocked.review")}
                    </Button>
                  </div>
                </div>
              ) : previewFailed ? (
                <div className="grid flex-1 place-content-center gap-1 rounded-lg border border-dashed p-6 text-center">
                  <p className="text-sm font-medium">{t("dashboard.export.preview.failed")}</p>
                  <p className="max-w-md text-xs leading-5 text-muted-foreground">
                    {errorMessage(preview.error)}
                  </p>
                </div>
              ) : usesWallets && noWalletSelected ? (
                <div className="grid flex-1 place-content-center rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
                  {t("dashboard.export.selectWallet")}
                </div>
              ) : !previewData ? (
                <PreviewSkeleton />
              ) : (
                <>
                  {sheets.length > 1 ? (
                    <div role="tablist" aria-label={t("dashboard.export.preview.sheets")} className="flex flex-wrap gap-1">
                      {sheets.map((sheet) => {
                        const active = sheet === activeSheet;
                        return (
                          <button
                            key={sheet.name}
                            type="button"
                            role="tab"
                            aria-selected={active}
                            onClick={() => setSheetName(sheet.name)}
                            className={cn(
                              "inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                              active
                                ? "bg-foreground text-background"
                                : "text-muted-foreground hover:bg-muted hover:text-foreground",
                            )}
                          >
                            {sheet.name}
                            <span className={cn("tabular-nums", active ? "opacity-70" : "opacity-60")}>
                              {formatCount(sheet.total)}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  ) : null}
                  {activeSheet ? (
                    <PreviewTable sheet={activeSheet} emptyLabel={t("dashboard.export.preview.empty")} />
                  ) : null}
                  <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-xs text-muted-foreground">
                    <span>
                      {activeSheet && activeSheet.total > activeSheet.rows.length
                        ? t("dashboard.export.preview.showing", {
                            shown: activeSheet.rows.length,
                            count: activeSheet.total,
                            formatted: formatCount(activeSheet.total),
                          })
                        : t("dashboard.export.preview.complete")}
                    </span>
                    {importHint ? <span>{importHint}</span> : null}
                  </div>
                  {previewData.warnings.length ? (
                    <ul className="grid gap-1.5 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs leading-5">
                      {previewData.warnings.map((warning) => (
                        <li key={warning} className="flex gap-2">
                          <AlertTriangle className="mt-0.5 size-3.5 shrink-0 text-amber-500" aria-hidden="true" />
                          <span>{warning}</span>
                        </li>
                      ))}
                    </ul>
                  ) : null}
                </>
              )}
            </div>
          </div>

          <footer className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t px-5 py-3">
            <p className="min-w-0 truncate text-sm text-muted-foreground">
              {kind === "yearHandoff"
                ? t("dashboard.export.summary.yearHandoff", { year: selectedYear ?? "" })
                : kind === "labels"
                  ? t("dashboard.export.summary.labels", { wallet: walletLabel })
                  : t("dashboard.export.summary.transactions", {
                      wallet: walletLabel,
                      format: t(`dashboard.export.formats.${format}`),
                    })}
            </p>
            <div className="flex gap-2">
              <DialogClose asChild>
                <Button type="button" variant="outline" disabled={isExporting}>
                  {t("common:actions.cancel")}
                </Button>
              </DialogClose>
              <Button
                type="button"
                disabled={
                  isExporting ||
                  previewUpdating ||
                  (usesWallets && noWalletSelected) ||
                  Boolean(block) ||
                  previewFailed ||
                  (kind === "yearHandoff" && selectedYear === null)
                }
                onClick={runExport}
              >
                <Download className="size-4" aria-hidden="true" />
                {submitLabel}
              </Button>
            </div>
          </footer>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
