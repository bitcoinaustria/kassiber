import { keepPreviousData } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, ArrowUpRight, Download, ShieldAlert } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

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
import { DaemonRequestError, useDaemon, useDaemonMutation } from "@/daemon/client";
import { exportBasename, saveDaemonExport } from "@/lib/exportFile";
import { currentUiLocale, formatCount } from "@/lib/localeFormat";
import { pageHeaderActionClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { ExportFormatMark, ExportFormatPicker } from "./ExportFormatPicker";
import { ExportPeriodPicker } from "./ExportPeriodPicker";
import { periodArgs, type ExportPeriod, type ResolvedPeriod } from "./exportPeriod";
import { ExportWalletPicker, type ExportWallet } from "./ExportWalletPicker";
import { exportFormat, isToolFormat, type ExportFormatId } from "./exportFormats";

interface ExportResult {
  file?: string;
  rows?: number;
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
  range: ResolvedPeriod | null;
}

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
const QUARANTINE_CODES = new Set(["transaction_export_quarantined"]);
const QUARANTINE_PREVIEW = 5;

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
                <td className="sticky left-0 border-r bg-muted/60 px-2 py-1.5 text-right text-muted-foreground tabular-nums">
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
              <td colSpan={sheet.columns.length + 1} className="px-3 py-10 text-center text-muted-foreground">
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

/**
 * The transaction export: one file, chosen by format, period and wallets,
 * previewed exactly as it will be written. Year-end bookkeeping lives with the
 * other annual files on Reports; checking another tool's export lives on
 * Reconcile.
 */
export function TransactionsExportDialog({ bookTimezone }: { bookTimezone: string | undefined }) {
  const { t } = useTranslation(["transactions", "common"]);
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [open, setOpen] = React.useState(false);
  const [excludedWallets, setExcludedWallets] = React.useState<Set<string>>(() => new Set());
  const [format, setFormat] = React.useState<ExportFormatId>("xlsx");
  const [period, setPeriod] = React.useState<ExportPeriod>({ preset: "all" });
  const [exportBlock, setExportBlock] = React.useState<QuarantineBlock | null>(null);
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
  const isLabels = formatSpec.daemon.kind === "labels";
  const rangeArgs = formatSpec.dated ? periodArgs(period) : {};

  const previewArgs: Record<string, unknown> = isLabels
    ? { export: "labels", ...walletArgs }
    : { export: "transactions", format, ...walletArgs, ...rangeArgs };
  // Toggling several wallets in a row asks for one preview, not one per click.
  const settledPreviewArgs = useSettled(previewArgs, PREVIEW_DEBOUNCE_MS);
  const preview = useDaemon<ExportPreview>("ui.exports.preview", settledPreviewArgs, {
    enabled: open && !noWalletSelected,
    retry: false,
    staleTime: 0,
    placeholderData: keepPreviousData,
  });
  const previewUpdating =
    preview.isFetching || JSON.stringify(previewArgs) !== JSON.stringify(settledPreviewArgs);
  const previewData = preview.data?.data;
  const block = quarantineBlock(preview.error) ?? exportBlock;
  const previewFailed = Boolean(preview.error) && !block;

  const exportXlsx = useDaemonMutation<ExportResult>("ui.transactions.export_xlsx");
  const exportCsv = useDaemonMutation<ExportResult>("ui.transactions.export_csv");
  const exportPlatform = useDaemonMutation<ExportResult>("ui.transactions.export_platform_csv");
  const exportLabels = useDaemonMutation<ExportResult>("ui.metadata.bip329.export");
  const isExporting = [exportXlsx, exportCsv, exportPlatform, exportLabels].some(
    (mutation) => mutation.isPending,
  );

  // A different choice is a different file: forget the previous export's block.
  React.useEffect(() => {
    setExportBlock(null);
  }, [excludedWallets, format, period]);

  const sheet = previewData?.sheets[0];
  const rowCount = previewData?.row_count ?? 0;
  const fileName = previewData?.file_name ?? formatSpec.defaultName;

  const handleError = (error: unknown) => {
    const blocked = quarantineBlock(error);
    if (blocked) {
      setExportBlock(blocked);
      return;
    }
    addNotification({ title: t("dashboard.export.failed"), body: errorMessage(error), tone: "error" });
  };

  const finish = async (payload: ExportResult | undefined) => {
    setOpen(false);
    try {
      const { savedPath, copied } = await saveDaemonExport({
        exportPath: payload?.file ?? "",
        title: isLabels ? t("dashboard.export.labels.saveTitle") : t("dashboard.export.saveTitle"),
        defaultName: fileName,
        filters: [formatSpec.filter],
      });
      const warnings = payload?.warnings ?? [];
      addNotification({
        title: isLabels
          ? t("dashboard.export.labels.done")
          : warnings.length
            ? t("dashboard.export.doneWithWarnings")
            : t("dashboard.export.done"),
        body: [
          copied
            ? t("dashboard.export.savedTo", { name: exportBasename(savedPath) })
            : isLabels
              ? exportBasename(payload?.file ?? "")
              : t("dashboard.export.rows", { count: payload?.rows ?? 0 }),
          ...warnings,
        ].join("\n"),
        tone: warnings.length ? "warning" : "success",
      });
    } catch (error) {
      addNotification({ title: t("dashboard.export.failed"), body: errorMessage(error), tone: "error" });
    }
  };

  const runExport = () => {
    setExportBlock(null);
    const callbacks = {
      onSuccess: (envelope: { data?: ExportResult } | undefined) => void finish(envelope?.data),
      onError: handleError,
    };
    const daemon = formatSpec.daemon;
    if (daemon.kind === "labels") {
      exportLabels.mutate({ mode: "all", ...walletArgs }, callbacks);
    } else if (daemon.kind === "platform") {
      exportPlatform.mutate({ platform: daemon.platform, ...walletArgs, ...rangeArgs }, callbacks);
    } else {
      (daemon.kind === "xlsx" ? exportXlsx : exportCsv).mutate({ ...walletArgs, ...rangeArgs }, callbacks);
    }
  };

  const openQuarantine = () => {
    setOpen(false);
    void navigate({ to: "/quarantine" });
  };

  const submitLabel = isExporting
    ? t("dashboard.export.exporting")
    : previewData
      ? t(isLabels ? "dashboard.export.submitLabels" : "dashboard.export.submitRows", {
          count: rowCount,
          formatted: formatCount(rowCount),
        })
      : t("dashboard.export.submit");
  const walletLabel = allWalletsSelected
    ? t("dashboard.export.allWallets")
    : t("dashboard.export.someWallets", { selected: selectedWallets.length, total: wallets.length });
  const periodLabel =
    period.preset === "custom" && previewData?.range
      ? `${previewData.range.first_day} – ${previewData.range.last_day}`
      : t(`dashboard.export.period.presets.${period.preset}`);
  const importHint = isToolFormat(formatSpec) ? t(`dashboard.export.importHint.${formatSpec.id}`) : null;

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
        <header className="flex shrink-0 items-center gap-3 border-b px-5 py-3.5 pr-14">
          <DialogTitle className="text-base">{t("dashboard.export.title")}</DialogTitle>
          <DialogDescription className="sr-only">{t("dashboard.export.description")}</DialogDescription>
        </header>

        <div className="grid min-h-0 flex-1 overflow-y-auto lg:grid-cols-[20rem_minmax(0,1fr)] lg:overflow-hidden">
          <div className="flex min-h-0 flex-col gap-5 overflow-y-auto border-b bg-muted/25 p-5 lg:border-r lg:border-b-0">
            <ExportFormatPicker value={format} onChange={setFormat} />
            {formatSpec.dated ? (
              <ExportPeriodPicker
                value={period}
                onChange={setPeriod}
                bookTimezone={bookTimezone ?? "UTC"}
                resolved={previewData?.range ?? null}
              />
            ) : (
              <p className="text-xs leading-5 text-muted-foreground">
                {t("dashboard.export.period.labelsAllTime")}
              </p>
            )}
            <ExportWalletPicker
              wallets={wallets}
              loading={walletsQuery.isLoading}
              excluded={excludedWallets}
              onExcludedChange={setExcludedWallets}
            />
          </div>

          <div className="flex min-h-[22rem] min-w-0 flex-col gap-3 p-5">
            <div className="flex min-w-0 items-center gap-2.5 text-sm">
              <ExportFormatMark option={formatSpec} className="size-6" />
              <span className="min-w-0 truncate font-mono text-xs">{fileName}</span>
              <span className="rounded-sm border px-1.5 py-px font-mono text-[10px] tracking-wide text-muted-foreground">
                {formatSpec.extension}
              </span>
              {previewData && !block ? (
                <span className="text-xs text-muted-foreground">
                  {t("dashboard.export.fileRows", { count: rowCount, formatted: formatCount(rowCount) })}
                </span>
              ) : null}
              {previewUpdating && previewData ? (
                <span className="ml-auto animate-pulse text-xs text-muted-foreground">
                  {t("dashboard.export.preview.updating")}
                </span>
              ) : null}
            </div>
            {block ? (
              <div role="alert" className="grid gap-3 rounded-lg border border-amber-500/40 bg-amber-500/10 p-4 text-sm">
                <div className="flex items-start gap-2.5">
                  <ShieldAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
                  <div className="min-w-0">
                    <p className="font-medium">{t("dashboard.export.blocked.title", { count: block.count })}</p>
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
                      <span className="text-muted-foreground tabular-nums">{shortDate(row.occurred_at)}</span>
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
                    {t("dashboard.export.blocked.more", { count: block.count - QUARANTINE_PREVIEW })}
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
                <p className="max-w-md text-xs leading-5 text-muted-foreground">{errorMessage(preview.error)}</p>
              </div>
            ) : noWalletSelected ? (
              <div className="grid flex-1 place-content-center rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
                {t("dashboard.export.selectWallet")}
              </div>
            ) : !previewData || !sheet ? (
              <PreviewSkeleton />
            ) : (
              <>
                <PreviewTable sheet={sheet} emptyLabel={t("dashboard.export.preview.empty")} />
                <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  <span>
                    {sheet.total > sheet.rows.length
                      ? t("dashboard.export.preview.showing", {
                          shown: sheet.rows.length,
                          count: sheet.total,
                          formatted: formatCount(sheet.total),
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
            {isLabels
              ? t("dashboard.export.summary.labels", { wallet: walletLabel })
              : t("dashboard.export.summary.transactions", {
                  wallet: walletLabel,
                  format: t(`dashboard.export.formats.${format}`),
                })}
            {formatSpec.dated ? ` · ${periodLabel}` : null}
          </p>
          <div className="flex gap-2">
            <DialogClose asChild>
              <Button type="button" variant="outline" disabled={isExporting}>
                {t("common:actions.cancel")}
              </Button>
            </DialogClose>
            <Button
              type="button"
              disabled={isExporting || previewUpdating || noWalletSelected || Boolean(block) || previewFailed}
              onClick={runExport}
            >
              <Download className="size-4" aria-hidden="true" />
              {submitLabel}
            </Button>
          </div>
        </footer>
      </DialogContent>
    </Dialog>
  );
}
