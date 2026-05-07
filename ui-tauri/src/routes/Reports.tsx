/**
 * Reports — tax report package and export flow.
 *
 * Reports should read like a package workflow: check whether the journal state
 * is trusted, inspect the current daemon-provided audit rows, then export the
 * files that belong to the current book.
 */

import { Link } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import {
  AlertTriangle,
  CalendarDays,
  ChevronDown,
  CheckCircle2,
  Download,
  ExternalLink,
  FileArchive,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  Landmark,
  Loader2,
  RefreshCw,
  ShieldAlert,
  Sigma,
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
import { canOpenExportedFiles, openExportedFile } from "@/daemon/transport";
import { screenPanelClassName, screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import {
  JURISDICTIONS,
  type CapitalGainsReport,
  type CostBasisMethod,
  type DisposedLot,
  type KennzahlRow,
} from "@/mocks/reports";
import { useUiStore } from "@/store/ui";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const COST_BASIS_METHODS: Array<{
  k: CostBasisMethod;
  name: string;
  desc: string;
}> = [
  { k: "fifo", name: "FIFO", desc: "First-in, first-out" },
  { k: "lifo", name: "LIFO", desc: "Last-in, first-out" },
  { k: "hifo", name: "HIFO", desc: "Highest-in, first-out" },
  { k: "lofo", name: "LOFO", desc: "Lowest-in, first-out" },
];

const METHOD_LABELS: Record<CostBasisMethod, { name: string; desc: string }> =
  {
    fifo: { name: "FIFO", desc: "First-in, first-out" },
    lifo: { name: "LIFO", desc: "Last-in, first-out" },
    hifo: { name: "HIFO", desc: "Highest-in, first-out" },
    lofo: { name: "LOFO", desc: "Lowest-in, first-out" },
    moving_average: {
      name: "Moving average",
      desc: "Average cost pool",
    },
    moving_average_at: {
      name: "Austria tax method",
      desc: "Old stock: FIFO; new stock: average cost",
    },
  };

const AUSTRIAN_TAX_FIELD_COPY: Record<string, { label: string; note?: string }> =
  {
    "172": { label: "Foreign recurring crypto income" },
    "174": { label: "Foreign realized crypto gains" },
    "176": { label: "Foreign realized crypto losses" },
    "801": {
      label: "Legacy holdings speculation gains",
      note: "Outside E 1kv",
    },
  };

const AUSTRIAN_KENNZAHL_PLACEHOLDER_ROWS: KennzahlRow[] = [
  {
    code: "172",
    label: AUSTRIAN_TAX_FIELD_COPY["172"].label,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "174",
    label: AUSTRIAN_TAX_FIELD_COPY["174"].label,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "176",
    label: AUSTRIAN_TAX_FIELD_COPY["176"].label,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "801",
    label: AUSTRIAN_TAX_FIELD_COPY["801"].label,
    amount: null,
    rowCount: 0,
    source: "pending",
    note: AUSTRIAN_TAX_FIELD_COPY["801"].note,
  },
];

type ReportExportFormatId = "csv" | "pdf" | "xlsx";
type ReportTone = "good" | "warning" | "alert" | "neutral";
type ReportHref = "/journals" | "/quarantine" | "/transactions" | "/reports";

interface ReportExportResult {
  file?: string;
  filename?: string;
  format?: string;
  scope?: string;
  bytes?: number;
  pages?: number;
  rows?: number;
  summary_rows?: number;
  tax_year?: number;
}

interface ReportReadiness {
  title: string;
  detail: string;
  tone: ReportTone;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  action?: {
    label: string;
    href: ReportHref;
  };
}

export function Reports() {
  const { data, isLoading } = useDaemon<CapitalGainsReport>(
    "ui.reports.capital_gains",
  );
  const hideSensitive = useUiStore((s) => s.hideSensitive);

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading reports...
      </div>
    );
  }

  if (data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">Reports unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {data?.error?.message ?? "The daemon did not return report data."}
          </p>
        </div>
      </div>
    );
  }

  return <ReportsView report={data.data} hideSensitive={hideSensitive} />;
}

interface ReportsViewProps {
  report: CapitalGainsReport;
  hideSensitive: boolean;
}

function ReportsView({ report, hideSensitive }: ReportsViewProps) {
  const year = report.year;
  const jurisdiction =
    JURISDICTIONS[report.jurisdictionCode] ?? JURISDICTIONS.AT;
  const [method, setMethod] = useState<CostBasisMethod>(
    normalizeReportMethod(report.method, jurisdiction),
  );
  const [exportStatus, setExportStatus] = useState<{
    tone: "success" | "error";
    message: string;
    path?: string;
  } | null>(null);
  const [activeExport, setActiveExport] =
    useState<ReportExportFormatId | null>(null);
  const [openingExportPath, setOpeningExportPath] = useState<string | null>(
    null,
  );
  const addNotification = useUiStore((s) => s.addNotification);
  const exportCsv =
    useDaemonMutation<ReportExportResult>("ui.reports.export_capital_gains_csv");
  const exportPdf = useDaemonMutation<ReportExportResult>("ui.reports.export_pdf");
  const exportAustrianPdf =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_pdf");
  const exportAustrianXlsx =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_xlsx");
  const activeProfileIsAustrian = report.jurisdictionCode === "AT";
  const kennzahlRows =
    report.kennzahlRows?.length
      ? report.kennzahlRows
      : activeProfileIsAustrian
        ? AUSTRIAN_KENNZAHL_PLACEHOLDER_ROWS
        : [];

  const lots = report.lots;
  const totals = summarizeLots(lots);
  const estimatedTax = Math.max(totals.gain, 0) * jurisdiction.rate;
  const fmt = (n: number) =>
    n.toLocaleString(jurisdiction.locale, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  const methodLabel = METHOD_LABELS[method] ?? METHOD_LABELS[jurisdiction.defaultMethod];
  const readiness = buildReportReadiness(report, lots, year);
  const periodLabel = formatReportPeriod(year, jurisdiction.locale);
  const canOpenCurrentExport =
    exportStatus?.tone === "success" &&
    canOpenExportPath(exportStatus.path) &&
    canOpenExportedFiles();
  const openableExportPath =
    canOpenCurrentExport && exportStatus?.path ? exportStatus.path : null;

  const handleExport = (format: ReportExportFormatId) => {
    if (activeExport) return;
    setExportStatus(null);
    setActiveExport(format);
    const mutation =
      format === "csv"
        ? exportCsv
        : format === "xlsx"
          ? exportAustrianXlsx
          : activeProfileIsAustrian
            ? exportAustrianPdf
            : exportPdf;
    const args = { year };

    mutation.mutate(args, {
      onSuccess: (envelope) => {
        const payload = envelope.data;
        const file = payload?.file ?? "";
        const filename = payload?.filename ?? file.split("/").pop() ?? "report";
        const detail =
          payload?.format === "pdf" && payload.pages
            ? `${payload.pages} page${payload.pages === 1 ? "" : "s"}`
            : payload?.format === "xlsx" && payload.rows !== undefined
              ? `${payload.rows} row${payload.rows === 1 ? "" : "s"}`
              : payload?.format === "csv" && payload.rows !== undefined
                ? `${payload.rows} row${payload.rows === 1 ? "" : "s"}`
                : "Export written";
        setExportStatus({
          tone: "success",
          message: `${filename} saved to the managed exports folder.`,
          path: file,
        });
        addNotification({
          title: "Report export finished",
          body: detail,
          tone: "success",
        });
      },
      onError: (error) => {
        const message =
          error instanceof Error ? error.message : "Report export failed";
        setExportStatus({ tone: "error", message });
        addNotification({
          title: "Report export failed",
          body: message,
          tone: "error",
        });
      },
      onSettled: () => setActiveExport(null),
    });
  };

  const handleOpenExport = (path: string) => {
    setOpeningExportPath(path);
    void openExportedFile(path)
      .then(() => {
        addNotification({
          title: "Report opened",
          body: "Opened with the system default app.",
          tone: "success",
        });
      })
      .catch((error) => {
        const message =
          error instanceof Error ? error.message : "Could not open report";
        addNotification({
          title: "Could not open report",
          body: message,
          tone: "error",
        });
      })
      .finally(() => setOpeningExportPath(null));
  };

  return (
    <div className={screenShellClassName}>
      <ReportPackageHeader
        year={year}
        periodLabel={periodLabel}
        jurisdiction={jurisdiction}
        methodLabel={methodLabel}
      />

      <ReportReadinessStrip readiness={readiness} />

      <ReportMetricStrip
        hideSensitive={hideSensitive}
        jurisdiction={jurisdiction}
        lots={lots}
        totals={totals}
        estimatedTax={estimatedTax}
        year={year}
        formatNumber={fmt}
      />

      <div className="grid grid-cols-1 items-start gap-3 sm:gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(340px,380px)] 2xl:grid-cols-[minmax(0,1fr)_400px]">
        <div className="grid min-w-0 gap-3 sm:gap-4">
          {kennzahlRows.length ? (
            <KennzahlOverviewPanel
              rows={kennzahlRows}
              jurisdiction={jurisdiction}
              hideSensitive={hideSensitive}
              formatNumber={fmt}
            />
          ) : null}
          <LotAuditPanel
            lots={lots}
            totals={totals}
            jurisdiction={jurisdiction}
            hideSensitive={hideSensitive}
            formatNumber={fmt}
            year={year}
          />
        </div>
        <div className="grid min-w-0 gap-3 sm:gap-4">
          <ReportFilesPanel
            year={year}
            activeExport={activeExport}
            activeProfileIsAustrian={activeProfileIsAustrian}
            exportStatus={exportStatus}
            openableExportPath={openableExportPath}
            openingExportPath={openingExportPath}
            onExport={handleExport}
            onOpenExport={handleOpenExport}
          />
          <ReportPolicyPanel
            jurisdiction={jurisdiction}
            method={method}
            setMethod={setMethod}
          />
        </div>
      </div>
    </div>
  );
}

function ReportPackageHeader({
  year,
  periodLabel,
  jurisdiction,
  methodLabel,
}: {
  year: number;
  periodLabel: string;
  jurisdiction: (typeof JURISDICTIONS)[string];
  methodLabel: { name: string; desc: string };
}) {
  return (
    <div className="rounded-xl border bg-card px-3 py-3 sm:px-4">
      <div className="flex min-w-0 flex-wrap items-center gap-2 sm:gap-3">
        <span className="text-base font-semibold sm:text-lg">Tax report</span>
        <Badge variant="outline" className="rounded-md">
          {year}
        </Badge>
        <span className="hidden h-6 w-px bg-border sm:block" />
        <span className="inline-flex min-w-0 items-center gap-2 text-sm text-muted-foreground">
          <CalendarDays className="size-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{periodLabel}</span>
        </span>
        <Badge variant="outline" className="rounded-md">
          {jurisdiction.code} · {jurisdiction.name}
        </Badge>
        <Badge variant="outline" className="hidden rounded-md xl:inline-flex">
          {methodLabel.name}
        </Badge>
      </div>
    </div>
  );
}

function ReportReadinessStrip({ readiness }: { readiness: ReportReadiness }) {
  const Icon = readiness.icon;

  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        <span
          className={cn(
            "inline-flex h-8 shrink-0 items-center gap-2 rounded-md border px-2.5 text-sm font-medium",
            readinessToneStyles[readiness.tone],
          )}
        >
          <Icon className="size-4" aria-hidden="true" />
          {readiness.title}
        </span>
        <span className="min-w-0 truncate text-xs text-muted-foreground sm:text-sm">
          {readiness.detail}
        </span>
      </div>
      {readiness.action ? (
        <Button asChild size="sm" variant="outline" className="h-8 self-start sm:self-auto">
          <Link to={readiness.action.href}>{readiness.action.label}</Link>
        </Button>
      ) : null}
    </div>
  );
}

function ReportMetricStrip({
  hideSensitive,
  jurisdiction,
  lots,
  totals,
  estimatedTax,
  year,
  formatNumber,
}: {
  hideSensitive: boolean;
  jurisdiction: (typeof JURISDICTIONS)[string];
  lots: DisposedLot[];
  totals: ReportTotals;
  estimatedTax: number;
  year: number;
  formatNumber: (value: number) => string;
}) {
  const metrics: Array<{
    label: string;
    value: ReactNode;
    sub: string;
  }> = [
    {
      label: "Proceeds",
      value: (
        <span className={blurClass(hideSensitive)}>
          {formatMoney(jurisdiction.ccy, totals.proceeds, formatNumber)}
        </span>
      ),
      sub: `${lots.length} disposal${lots.length === 1 ? "" : "s"}`,
    },
    {
      label: "Cost basis",
      value: (
        <span className={blurClass(hideSensitive)}>
          {formatMoney(jurisdiction.ccy, totals.cost, formatNumber)}
        </span>
      ),
      sub: "Applied to disposed rows",
    },
    {
      label: "Gain / loss",
      value: (
        <span
          className={cn(
            totals.gain >= 0
              ? "text-emerald-600 dark:text-emerald-400"
              : "text-red-600 dark:text-red-400",
            blurClass(hideSensitive),
          )}
        >
          {signedMoney(jurisdiction.ccy, totals.gain, formatNumber)}
        </span>
      ),
      sub: `${year} tax year`,
    },
    {
      label: jurisdiction.rateLabel,
      value: (
        <span className={blurClass(hideSensitive)}>
          {formatMoney(jurisdiction.ccy, estimatedTax, formatNumber)}
        </span>
      ),
      sub: totals.gain > 0 ? "Estimated liability" : "No positive gain",
    },
  ];

  return (
    <div className="rounded-xl border bg-card">
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4 lg:divide-x">
        {metrics.map((metric) => (
          <div key={metric.label} className="space-y-2.5 p-3 sm:p-4">
            <p className="text-xs font-medium text-muted-foreground sm:text-sm">
              {metric.label}
            </p>
            <p className="min-w-0 text-xl leading-tight font-semibold tracking-tight tabular-nums sm:text-2xl">
              {metric.value}
            </p>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              {metric.sub}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}

function KennzahlOverviewPanel({
  rows,
  jurisdiction,
  hideSensitive,
  formatNumber,
}: {
  rows: KennzahlRow[];
  jurisdiction: (typeof JURISDICTIONS)[string];
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
}) {
  const hasMockRows = rows.some((row) => row.source === "mock");
  const hasPendingRows = rows.some((row) => row.source === "pending");
  const sourceLabel = hasPendingRows ? "Pending" : hasMockRows ? "Preview" : "Daemon";

  return (
    <div className="rounded-xl border bg-card">
      <div className="flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-start sm:justify-between sm:px-5">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-8 shrink-0"
            aria-label="Tax fields overview"
          >
            <Landmark className="size-4 text-muted-foreground" aria-hidden="true" />
          </Button>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Tax fields overview
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              {jurisdiction.code} filing fields from the export taxonomy
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="outline" className="rounded-md">
            {sourceLabel}
          </Badge>
          <Badge variant="outline" className="rounded-md">
            {rows.length} field{rows.length === 1 ? "" : "s"}
          </Badge>
        </div>
      </div>

      <div className="grid gap-2 px-4 pb-4 sm:grid-cols-2 sm:px-5">
        {rows.map((row) => {
          const displayCopy = AUSTRIAN_TAX_FIELD_COPY[row.code];
          const displayLabel = displayCopy?.label ?? row.label;
          const displayNote = displayCopy?.note ?? row.note;
          const amount = row.amount;
          const isPending = amount === null;
          const isEmpty =
            amount !== null && row.rowCount === 0 && Math.abs(amount) < 0.005;
          return (
            <div
              key={row.code}
              className={cn(
                "rounded-lg border bg-background/50 p-3",
                isEmpty && "text-muted-foreground",
              )}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex min-w-0 items-center gap-2">
                    <Badge variant="outline" className="rounded-md">
                      Field {row.code}
                    </Badge>
                    <span className="text-[10px] text-muted-foreground sm:text-xs">
                      {row.rowCount} row{row.rowCount === 1 ? "" : "s"}
                    </span>
                  </div>
                  <p className="mt-2 text-xs leading-5 text-muted-foreground">
                    {displayLabel}
                  </p>
                  {displayNote ? (
                    <p className="mt-1 text-[10px] text-muted-foreground">
                      {displayNote}
                    </p>
                  ) : null}
                </div>
                <div
                  className={cn(
                    "shrink-0 text-right text-sm font-semibold tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {isPending
                    ? "—"
                    : formatMoney(jurisdiction.ccy, amount, formatNumber)}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ReportFilesPanel({
  year,
  activeExport,
  activeProfileIsAustrian,
  exportStatus,
  openableExportPath,
  openingExportPath,
  onExport,
  onOpenExport,
}: {
  year: number;
  activeExport: ReportExportFormatId | null;
  activeProfileIsAustrian: boolean;
  exportStatus: {
    tone: "success" | "error";
    message: string;
    path?: string;
  } | null;
  openableExportPath: string | null;
  openingExportPath: string | null;
  onExport: (format: ReportExportFormatId) => void;
  onOpenExport: (path: string) => void;
}) {
  return (
    <div className="rounded-xl border bg-card">
      <div className="flex items-center justify-between gap-3 px-4 pt-4 sm:px-5">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-8 shrink-0"
            aria-label="Report files"
          >
            <FolderOpen className="size-4 text-muted-foreground" />
          </Button>
          <div>
            <h2 className="text-sm font-medium sm:text-base">Report files</h2>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              Export from the managed local report package
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-3 px-4 pt-3 pb-4 sm:px-5">
        {exportStatus ? (
          <ExportNotice
            exportStatus={exportStatus}
            openableExportPath={openableExportPath}
            openingExportPath={openingExportPath}
            onOpenExport={onOpenExport}
          />
        ) : null}
        <div className="divide-y rounded-lg border bg-background/50">
          <ReportFileRow
            id="pdf"
            icon={FileText}
            title="PDF report"
            detail={
              activeProfileIsAustrian
                ? `${year} · Austrian E 1kv`
                : "Complete report"
            }
            loading={activeExport === "pdf"}
            disabled={Boolean(activeExport)}
            onExport={onExport}
          />
          <ReportFileRow
            id="xlsx"
            icon={FileSpreadsheet}
            title="XLSX workbook"
            detail={
              activeProfileIsAustrian
                ? `${year} · Multi-sheet Austrian workbook`
                : "Available for Austrian books"
            }
            loading={activeExport === "xlsx"}
            disabled={Boolean(activeExport) || !activeProfileIsAustrian}
            onExport={onExport}
          />
          <ReportFileRow
            id="csv"
            icon={FileArchive}
            title="Capital gains CSV"
            detail="UTF-8 rows for spreadsheet review"
            loading={activeExport === "csv"}
            disabled={Boolean(activeExport)}
            onExport={onExport}
          />
        </div>
      </div>
    </div>
  );
}

function ExportNotice({
  exportStatus,
  openableExportPath,
  openingExportPath,
  onOpenExport,
}: {
  exportStatus: {
    tone: "success" | "error";
    message: string;
    path?: string;
  };
  openableExportPath: string | null;
  openingExportPath: string | null;
  onOpenExport: (path: string) => void;
}) {
  return (
    <div
      className={cn(
        "rounded-lg border px-3 py-2 text-sm",
        exportStatus.tone === "error"
          ? "border-destructive/35 bg-destructive/10 text-destructive"
          : "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
      )}
    >
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 items-start gap-2">
          {exportStatus.tone === "success" ? (
            <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          ) : (
            <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          )}
          <span className="min-w-0">
            {exportStatus.message}
            {exportStatus.path ? (
              <span className="mt-1 block truncate font-mono text-xs opacity-80">
                {exportStatus.path}
              </span>
            ) : null}
          </span>
        </div>
        {openableExportPath ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-8 self-start bg-background text-foreground"
            disabled={openingExportPath === openableExportPath}
            onClick={() => onOpenExport(openableExportPath)}
          >
            {openingExportPath === openableExportPath ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <ExternalLink className="size-4" aria-hidden="true" />
            )}
            Open
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function ReportFileRow({
  id,
  icon: Icon,
  title,
  detail,
  loading,
  disabled,
  onExport,
}: {
  id: ReportExportFormatId;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  title: string;
  detail: string;
  loading: boolean;
  disabled: boolean;
  onExport: (format: ReportExportFormatId) => void;
}) {
  return (
    <div className="flex items-center gap-3 px-3 py-3">
      <span className="flex size-8 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground ring-1 ring-inset ring-border">
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate text-sm font-semibold">{title}</span>
        </span>
        <span className="mt-0.5 block truncate text-xs text-muted-foreground">
          {detail}
        </span>
      </span>
      <Button
        type="button"
        size="sm"
        variant={id === "pdf" ? "default" : "outline"}
        className="h-8 shrink-0 gap-2"
        disabled={disabled}
        onClick={() => onExport(id)}
      >
        {loading ? (
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        ) : (
          <Download className="size-4" aria-hidden="true" />
        )}
        <span className="hidden sm:inline">Export</span>
      </Button>
    </div>
  );
}

function ReportPolicyPanel({
  jurisdiction,
  method,
  setMethod,
}: {
  jurisdiction: (typeof JURISDICTIONS)[string];
  method: CostBasisMethod;
  setMethod: (method: CostBasisMethod) => void;
}) {
  const methodLabel = METHOD_LABELS[method] ?? METHOD_LABELS[jurisdiction.defaultMethod];
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="rounded-xl border bg-card">
      <div className="flex items-center justify-between gap-3 px-4 py-4 sm:px-5">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-8 shrink-0"
            aria-label="Profile rules"
          >
            <ShieldAlert className="size-4 text-muted-foreground" />
          </Button>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Profile rules
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              {jurisdiction.code} · {methodLabel.name}
            </p>
          </div>
        </div>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className="size-8 shrink-0"
          aria-label={
            expanded ? "Collapse profile rules" : "Expand profile rules"
          }
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          <ChevronDown
            className={cn(
              "size-4 text-muted-foreground transition-transform",
              expanded && "rotate-180",
            )}
            aria-hidden="true"
          />
        </Button>
      </div>

      {expanded ? (
        <div className="space-y-3 px-4 pb-4 sm:px-5">
          <div className="divide-y rounded-lg border bg-background/50">
            <ReportFactRow
              label="Jurisdiction"
              value={`${jurisdiction.code} · ${jurisdiction.name}`}
            />
            <ReportFactRow label="Policy" value={jurisdiction.policy} />
            <ReportFactRow label="Tax rate" value={jurisdiction.rateLabel} />
            <ReportFactRow
              label="Internal transfers"
              value={jurisdiction.internalsNonTaxable ? "Non-taxable" : "Taxable"}
            />
            <ReportFactRow
              label="Long-term rule"
              value={
                jurisdiction.code === "AT"
                  ? "Pre-2022 holdings only"
                  : jurisdiction.longTermDays
                  ? `${jurisdiction.longTermDays} days`
                  : "Not applied"
              }
            />
            <ReportFactRow
              label="Cost-basis method"
              value={jurisdiction.methodNote ?? methodLabel.name}
            />
          </div>

          {!jurisdiction.methodLocked ? (
            <div className="grid gap-2">
              {COST_BASIS_METHODS.map(({ k, name, desc }) => {
                const active = method === k;
                return (
                  <button
                    key={k}
                    type="button"
                    onClick={() => setMethod(k)}
                    className={cn(
                      "rounded-lg border p-3 text-left transition-colors",
                      active
                        ? "border-primary bg-primary/5"
                        : "bg-background hover:bg-muted/50",
                    )}
                  >
                    <span className="flex items-center gap-2">
                      <span
                        className={cn(
                          "size-2 rounded-full",
                          active ? "bg-primary" : "bg-muted-foreground/40",
                        )}
                      />
                      <span className="font-medium">{name}</span>
                    </span>
                    <span className="mt-1 block text-xs text-muted-foreground">
                      {desc}
                    </span>
                  </button>
                );
              })}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function ReportFactRow({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="grid grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] items-start gap-3 px-3 py-2.5 text-sm">
      <span className="min-w-0 truncate text-muted-foreground">{label}</span>
      <span className="min-w-0 text-right leading-snug font-medium break-words">
        {value}
      </span>
    </div>
  );
}

function LotAuditPanel({
  lots,
  totals,
  jurisdiction,
  hideSensitive,
  formatNumber,
  year,
}: {
  lots: DisposedLot[];
  totals: ReportTotals;
  jurisdiction: (typeof JURISDICTIONS)[string];
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
  year: number;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="rounded-xl border bg-card">
      <div
        className={cn(
          "flex flex-col gap-3 px-4 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-5",
          expanded && "border-b",
        )}
      >
        <div className="flex min-w-0 items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-8 shrink-0"
            aria-label="Disposed lot audit"
          >
            <Sigma className="size-4 text-muted-foreground" />
          </Button>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Disposed lot audit
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              Acquisition, disposal, proceeds, basis, and gain
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="outline" className="rounded-md">
            {lots.length} row{lots.length === 1 ? "" : "s"}
          </Badge>
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="size-8"
            aria-label={
              expanded ? "Collapse disposed lot audit" : "Expand disposed lot audit"
            }
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            <ChevronDown
              className={cn(
                "size-4 text-muted-foreground transition-transform",
                expanded && "rotate-180",
              )}
              aria-hidden="true"
            />
          </Button>
        </div>
      </div>

      {expanded && lots.length ? (
        <div className="overflow-x-auto">
          <Table className="min-w-[760px]">
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead className="min-w-[112px]">Acquired</TableHead>
                <TableHead className="min-w-[112px]">Disposed</TableHead>
                <TableHead>Holding</TableHead>
                <TableHead className="text-right">Sats</TableHead>
                <TableHead className="text-right">
                  Cost {jurisdiction.ccy}
                </TableHead>
                <TableHead className="text-right">
                  Proceeds {jurisdiction.ccy}
                </TableHead>
                <TableHead className="text-right">
                  Gain {jurisdiction.ccy}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {lots.map((lot, index) => (
                <ReportLotRow
                  key={`${lot.acquired}-${lot.disposed}-${index}`}
                  lot={lot}
                  hideSensitive={hideSensitive}
                  formatNumber={formatNumber}
                />
              ))}
              <TableRow className="bg-muted/30 font-medium">
                <TableCell colSpan={3}>Total</TableCell>
                <TableCell
                  className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                >
                  {totals.sats.toLocaleString("en-US")}
                </TableCell>
                <TableCell
                  className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                >
                  {formatNumber(totals.cost)}
                </TableCell>
                <TableCell
                  className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                >
                  {formatNumber(totals.proceeds)}
                </TableCell>
                <TableCell
                  className={cn(
                    "text-right tabular-nums",
                    totals.gain >= 0
                      ? "text-emerald-600 dark:text-emerald-400"
                      : "text-red-600 dark:text-red-400",
                    blurClass(hideSensitive),
                  )}
                >
                  {signedNumber(totals.gain, formatNumber)}
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </div>
      ) : expanded ? (
        <div className="flex min-h-32 items-center justify-center px-4 py-8 text-center text-sm text-muted-foreground">
          No disposal lots were returned for {year}.
        </div>
      ) : null}
    </div>
  );
}

interface ReportLotRowProps {
  lot: DisposedLot;
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
}

function ReportLotRow({ lot, hideSensitive, formatNumber }: ReportLotRowProps) {
  const gain = lot.proceedsEur - lot.costEur;
  const isLong = lot.type === "LT";

  return (
    <TableRow>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {lot.acquired || "n/a"}
      </TableCell>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {lot.disposed}
      </TableCell>
      <TableCell>
        <span
          className={cn(
            "inline-flex rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
            isLong
              ? "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20"
              : "bg-muted text-muted-foreground ring-border",
          )}
        >
          {isLong ? "> 1Y" : "< 1Y"}
        </span>
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {lot.sats.toLocaleString("en-US")}
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {formatNumber(lot.costEur)}
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {formatNumber(lot.proceedsEur)}
      </TableCell>
      <TableCell
        className={cn(
          "text-right tabular-nums",
          gain >= 0
            ? "text-emerald-600 dark:text-emerald-400"
            : "text-red-600 dark:text-red-400",
          blurClass(hideSensitive),
        )}
      >
        {signedNumber(gain, formatNumber)}
      </TableCell>
    </TableRow>
  );
}

type ReportTotals = {
  sats: number;
  cost: number;
  proceeds: number;
  gain: number;
};

function summarizeLots(lots: DisposedLot[]): ReportTotals {
  return lots.reduce(
    (acc, lot) => ({
      sats: acc.sats + lot.sats,
      cost: acc.cost + lot.costEur,
      proceeds: acc.proceeds + lot.proceedsEur,
      gain: acc.gain + (lot.proceedsEur - lot.costEur),
    }),
    { sats: 0, cost: 0, proceeds: 0, gain: 0 },
  );
}

function normalizeReportMethod(
  method: CostBasisMethod | undefined,
  jurisdiction: (typeof JURISDICTIONS)[string],
): CostBasisMethod {
  if (jurisdiction.methodLocked) {
    return jurisdiction.defaultMethod;
  }
  return method && METHOD_LABELS[method] ? method : jurisdiction.defaultMethod;
}

function formatReportPeriod(year: number, locale: string) {
  const from = new Date(Date.UTC(year, 0, 1)).toLocaleDateString(locale);
  const to = new Date(Date.UTC(year, 11, 31)).toLocaleDateString(locale);
  return `${from} - ${to}`;
}

function buildReportReadiness(
  report: CapitalGainsReport,
  lots: DisposedLot[],
  year: number,
): ReportReadiness {
  const needsJournals = Boolean(report.status?.needsJournals);
  const quarantines = report.status?.quarantines ?? 0;
  const hasFilingRows = Boolean(
    report.kennzahlRows?.some(
      (row) => row.rowCount > 0 || Math.abs(row.amount ?? 0) > 0.005,
    ),
  );

  if (needsJournals) {
    return {
      title: "Reprocess journals",
      detail: "Report totals need a fresh journal state before export.",
      tone: "warning",
      icon: RefreshCw,
      action: { label: "Open journals", href: "/journals" },
    };
  }

  if (quarantines > 0) {
    return {
      title: "Review queue open",
      detail: `${quarantines} quarantined item${
        quarantines === 1 ? "" : "s"
      } should be resolved before filing.`,
      tone: "alert",
      icon: ShieldAlert,
      action: { label: "Review queue", href: "/quarantine" },
    };
  }

  if (!lots.length && !hasFilingRows) {
    return {
      title: "No taxable rows",
      detail: `No disposal or filing-field rows are currently included in the ${year} package.`,
      tone: "neutral",
      icon: AlertTriangle,
    };
  }

  return {
    title: "Ready for export",
    detail: "Journals are current and the review queue is clear.",
    tone: "good",
    icon: CheckCircle2,
  };
}

function signedNumber(value: number, format: (value: number) => string) {
  return `${value >= 0 ? "+" : "-"} ${format(Math.abs(value))}`;
}

function signedMoney(
  currency: string,
  value: number,
  format: (value: number) => string,
) {
  return `${value >= 0 ? "+" : "-"} ${currency} ${format(Math.abs(value))}`;
}

function formatMoney(
  currency: string,
  value: number,
  format: (value: number) => string,
) {
  return `${currency} ${format(value)}`;
}

function canOpenExportPath(path?: string) {
  return Boolean(path && (path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path)));
}

const readinessToneStyles: Record<ReportTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};
