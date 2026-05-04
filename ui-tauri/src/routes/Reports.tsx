/**
 * Reports — capital-gains export flow.
 *
 * The screen still uses fixture data through the mock daemon, but the
 * layout now follows the shared shadcn dashboard language used by Overview
 * and Transactions.
 */

import { useState, type ReactNode } from "react";
import {
  CheckCircle2,
  ExternalLink,
  FileSpreadsheet,
  FileText,
  Loader2,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  JURISDICTIONS,
  type CapitalGainsReport,
  type CostBasisMethod,
  type DisposedLot,
} from "@/mocks/reports";

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

const REPORTING_YEARS = [2023, 2024, 2025, 2026] as const;
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
      desc: "FIFO old stock · AVCO new stock",
    },
  };

function normalizeReportMethod(
  method: CostBasisMethod | undefined,
  jurisdiction: (typeof JURISDICTIONS)[string],
): CostBasisMethod {
  if (jurisdiction.methodLocked) {
    return jurisdiction.defaultMethod;
  }
  return method && METHOD_LABELS[method] ? method : jurisdiction.defaultMethod;
}

type ReportExportFormatId = "csv" | "pdf" | "xlsx";

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
      <div className="w-full bg-background p-3 sm:p-4 md:p-6">
        <Card>
          <CardHeader>
            <CardTitle>Reports unavailable</CardTitle>
            <CardDescription>
              {data?.error?.message ?? "The daemon did not return report data."}
            </CardDescription>
          </CardHeader>
        </Card>
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
  const [year, setYear] = useState(report.year);
  const j = JURISDICTIONS[report.jurisdictionCode] ?? JURISDICTIONS.AT;
  const [method, setMethod] = useState<CostBasisMethod>(
    normalizeReportMethod(report.method, j),
  );
  const [exportStatus, setExportStatus] = useState<{
    tone: "success" | "error";
    message: string;
    path?: string;
  } | null>(null);
  const addNotification = useUiStore((s) => s.addNotification);
  const exportCsv =
    useDaemonMutation<ReportExportResult>("ui.reports.export_capital_gains_csv");
  const exportPdf = useDaemonMutation<ReportExportResult>("ui.reports.export_pdf");
  const exportAustrianPdf =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_pdf");
  const exportAustrianXlsx =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_xlsx");
  const [activeExport, setActiveExport] =
    useState<ReportExportFormatId | null>(null);
  const [openingExportPath, setOpeningExportPath] = useState<string | null>(
    null,
  );
  const activeProfileIsAustrian = report.jurisdictionCode === "AT";

  const fmt = (n: number) =>
    n.toLocaleString(j.locale, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });

  const lots = report.lots;
  const totals = lots.reduce(
    (a, l) => ({
      sats: a.sats + l.sats,
      cost: a.cost + l.costEur,
      proceeds: a.proceeds + l.proceedsEur,
      gain: a.gain + (l.proceedsEur - l.costEur),
    }),
    { sats: 0, cost: 0, proceeds: 0, gain: 0 },
  );
  const kest = totals.gain * j.rate;
  const canOpenCurrentExport =
    exportStatus?.tone === "success" &&
    canOpenExportPath(exportStatus.path) &&
    canOpenExportedFiles();
  const openableExportPath =
    canOpenCurrentExport && exportStatus?.path ? exportStatus.path : null;

  const handleExport = (format: ReportExportFormatId) => {
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
    const args =
      format === "xlsx" || (format === "pdf" && activeProfileIsAustrian)
        ? { year }
        : undefined;
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
        const message = `${filename} saved to the managed exports folder.`;
        setExportStatus({ tone: "success", message, path: file });
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
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,360px)_minmax(0,1fr)]">
        <ReportControls
          year={year}
          setYear={setYear}
          jurisdiction={j}
          method={method}
          setMethod={setMethod}
          rateLabel={j.rateLabel}
          policy={j.policy}
        />

        <div className="min-w-0 space-y-4">
          {report.status?.needsJournals && (
            <Card className="border-amber-200 bg-amber-50 text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-100">
              <CardHeader>
                <CardTitle>Journals need processing</CardTitle>
                <CardDescription className="text-amber-800 dark:text-amber-200">
                  Recent transaction changes are not reflected in trusted report
                  totals yet.
                </CardDescription>
              </CardHeader>
            </Card>
          )}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-4">
            <ReportMetricCard
              label="Proceeds"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.proceeds)}
                </span>
              }
              sub={`${lots.length} disposals`}
            />
            <ReportMetricCard
              label="Cost Basis"
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(totals.cost)}
                </span>
              }
              sub={METHOD_LABELS[method].name}
            />
            <ReportMetricCard
              label="Net Gain"
              value={
                <span
                  className={cn(
                    totals.gain >= 0 ? "text-emerald-600" : "text-red-600",
                    blurClass(hideSensitive),
                  )}
                >
                  {signedMoney(j.ccy, totals.gain, fmt)}
                </span>
              }
              sub={`${year} tax year`}
            />
            <ReportMetricCard
              label={j.rateLabel}
              value={
                <span className={blurClass(hideSensitive)}>
                  {j.ccy} {fmt(kest)}
                </span>
              }
              sub="Estimated liability"
            />
          </div>

          <Card>
            <CardHeader className="border-b">
              <CardTitle>Disposed lots · {year}</CardTitle>
              <CardDescription>
                Disposal lots for the books' jurisdiction and rp2 method.
              </CardDescription>
            </CardHeader>
            <CardContent className="p-4">
              <div className="overflow-x-auto rounded-md border">
                <Table className="min-w-[760px]">
                  <TableHeader>
                    <TableRow className="bg-muted/50 hover:bg-muted/50">
                      <TableHead className="min-w-[112px]">Acquired</TableHead>
                      <TableHead className="min-w-[112px]">Disposed</TableHead>
                      <TableHead>Holding</TableHead>
                      <TableHead className="text-right">Sats</TableHead>
                      <TableHead className="text-right">
                        Cost {j.ccy}
                      </TableHead>
                      <TableHead className="text-right">
                        Proceeds {j.ccy}
                      </TableHead>
                      <TableHead className="text-right">
                        Gain {j.ccy}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {lots.map((lot, index) => (
                      <ReportLotRow
                        key={`${lot.acquired}-${lot.disposed}-${index}`}
                        lot={lot}
                        hideSensitive={hideSensitive}
                      />
                    ))}
                    <TableRow className="bg-muted/30 font-medium">
                      <TableCell colSpan={3}>Total</TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {totals.sats.toLocaleString("en-US")}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {totals.cost.toFixed(2)}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          blurClass(hideSensitive),
                        )}
                      >
                        {totals.proceeds.toFixed(2)}
                      </TableCell>
                      <TableCell
                        className={cn(
                          "text-right tabular-nums",
                          totals.gain >= 0
                            ? "text-emerald-600"
                            : "text-red-600",
                          blurClass(hideSensitive),
                        )}
                      >
                        {signedNumber(totals.gain)}
                      </TableCell>
                    </TableRow>
                  </TableBody>
                </Table>
              </div>
            </CardContent>
          </Card>

          {exportStatus ? (
            <div
              className={cn(
                "rounded-md border px-3 py-2 text-sm",
                exportStatus.tone === "error"
                  ? "border-destructive/35 bg-destructive/10 text-destructive"
                  : "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
              )}
            >
              <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                <div className="flex min-w-0 items-start gap-2">
                  {exportStatus.tone === "success" ? (
                    <CheckCircle2
                      className="mt-0.5 size-4 shrink-0"
                      aria-hidden="true"
                    />
                  ) : null}
                  <span className="min-w-0">
                    {exportStatus.message}
                    {exportStatus.path ? (
                      <span className="mt-1 block break-all font-mono text-xs opacity-80">
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
                    className="self-start bg-background text-foreground"
                    disabled={openingExportPath === openableExportPath}
                    onClick={() => handleOpenExport(openableExportPath)}
                  >
                    {openingExportPath === openableExportPath ? (
                      <Loader2
                        className="size-4 animate-spin"
                        aria-hidden="true"
                      />
                    ) : (
                      <ExternalLink className="size-4" aria-hidden="true" />
                    )}
                    Open
                  </Button>
                ) : null}
              </div>
            </div>
          ) : null}

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
            <ReportExportFormat
              icon={FileSpreadsheet}
              id="csv"
              name="CSV"
              sub="Spreadsheet"
              detail="Capital gains · UTF-8"
              loading={activeExport === "csv"}
              onClick={handleExport}
            />
            <ReportExportFormat
              icon={FileText}
              id="pdf"
              name="PDF"
              sub="Human-readable"
              detail={
                activeProfileIsAustrian
                  ? `${year} · Austrian E 1kv`
                  : "Complete report"
              }
              primary
              loading={activeExport === "pdf"}
              onClick={handleExport}
            />
            <ReportExportFormat
              icon={FileSpreadsheet}
              id="xlsx"
              name="XLSX"
              sub="Spreadsheet"
              detail={
                activeProfileIsAustrian
                  ? `${year} · Multi-sheet workbook`
                  : "Austrian books only"
              }
              disabled={!activeProfileIsAustrian}
              loading={activeExport === "xlsx"}
              onClick={handleExport}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

interface ReportControlsProps {
  year: number;
  setYear: (year: number) => void;
  jurisdiction: (typeof JURISDICTIONS)[string];
  method: CostBasisMethod;
  setMethod: (method: CostBasisMethod) => void;
  rateLabel: string;
  policy: string;
}

function ReportControls({
  year,
  setYear,
  jurisdiction,
  method,
  setMethod,
  rateLabel,
  policy,
}: ReportControlsProps) {
  return (
    <Card className="h-fit">
      <CardHeader className="border-b">
        <CardTitle>Report setup</CardTitle>
        <CardDescription>{policy}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <section className="space-y-3">
          <Label>Jurisdiction</Label>
          <div
            aria-readonly="true"
            className="flex min-h-9 items-center justify-between rounded-md border bg-muted/35 px-3 py-2 text-sm"
          >
            <span className="font-medium">
              {jurisdiction.code} · {jurisdiction.name}
            </span>
            <span className="rounded-md bg-background px-2 py-0.5 text-xs text-muted-foreground">
              Books
            </span>
          </div>
        </section>

        <section className="space-y-3">
          <Label>Reporting period</Label>
          <Select
            value={String(year)}
            onValueChange={(value) => setYear(Number(value))}
          >
            <SelectTrigger className="w-full">
              <SelectValue placeholder="Select tax year" />
            </SelectTrigger>
            <SelectContent>
              {REPORTING_YEARS.map((y) => (
                <SelectItem key={y} value={String(y)}>
                  {y}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="grid grid-cols-2 gap-2">
            <div className="space-y-1.5">
              <Label htmlFor="report-from">From</Label>
              <Input id="report-from" value={`${year}-01-01`} readOnly />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="report-to">To</Label>
              <Input id="report-to" value={`${year}-12-31`} readOnly />
            </div>
          </div>
        </section>

        <section className="space-y-3">
          <Label>Cost-basis method</Label>
          {jurisdiction.methodLocked ? (
            <div
              aria-readonly="true"
              className="rounded-md border bg-muted/35 p-3 text-left"
            >
              <div className="flex items-start justify-between gap-3">
                <span>
                  <span className="flex items-center gap-2">
                    <span className="size-2 rounded-full bg-primary" />
                    <span className="font-medium">
                      {METHOD_LABELS[method].name}
                    </span>
                  </span>
                  <span className="mt-1 block text-xs text-muted-foreground">
                    {jurisdiction.methodNote ?? METHOD_LABELS[method].desc}
                  </span>
                </span>
                <span className="shrink-0 rounded-md bg-background px-2 py-0.5 text-xs text-muted-foreground">
                  Books
                </span>
              </div>
            </div>
          ) : (
            <div className="grid gap-2">
              {COST_BASIS_METHODS.map(({ k, name, desc }) => {
                const active = method === k;
                return (
                  <button
                    key={k}
                    type="button"
                    onClick={() => setMethod(k)}
                    className={cn(
                      "rounded-md border p-3 text-left transition-colors",
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
          )}
        </section>

        <section className="space-y-3">
          <Label>Policy</Label>
          <div className="grid gap-2">
            <ReportToggleRow label="Treat internal transfers as non-taxable" def />
            <ReportToggleRow label={`Apply ${rateLabel} flat rate`} def />
            <ReportToggleRow
              label="Fees follow journals: network, Lightning, swaps"
              def
              disabled
            />
            <ReportToggleRow label="Aggregate lots per UTXO set" />
          </div>
        </section>
      </CardContent>
    </Card>
  );
}

interface ReportLotRowProps {
  lot: DisposedLot;
  hideSensitive: boolean;
}

function ReportLotRow({ lot, hideSensitive }: ReportLotRowProps) {
  const gain = lot.proceedsEur - lot.costEur;
  const isLong = lot.type === "LT";
  return (
    <TableRow>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {lot.acquired}
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
        {lot.costEur.toFixed(2)}
      </TableCell>
      <TableCell
        className={cn("text-right tabular-nums", blurClass(hideSensitive))}
      >
        {lot.proceedsEur.toFixed(2)}
      </TableCell>
      <TableCell
        className={cn(
          "text-right tabular-nums",
          gain >= 0 ? "text-emerald-600" : "text-red-600",
          blurClass(hideSensitive),
        )}
      >
        {signedNumber(gain)}
      </TableCell>
    </TableRow>
  );
}

function signedNumber(value: number) {
  return `${value >= 0 ? "+" : "-"} ${Math.abs(value).toFixed(2)}`;
}

function signedMoney(
  currency: string,
  value: number,
  format: (value: number) => string,
) {
  return `${value >= 0 ? "+" : "-"} ${currency} ${format(Math.abs(value))}`;
}

function canOpenExportPath(path?: string) {
  return Boolean(path && (path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path)));
}

interface ReportMetricCardProps {
  label: string;
  value: ReactNode;
  sub: string;
}

function ReportMetricCard({ label, value, sub }: ReportMetricCardProps) {
  return (
    <Card className="min-w-0 gap-3 py-5">
      <CardContent className="min-w-0 space-y-3">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        <p className="min-w-0 whitespace-nowrap text-xl leading-tight font-semibold tracking-tight tabular-nums sm:text-2xl">
          {value}
        </p>
        <p className="text-xs text-muted-foreground">{sub}</p>
      </CardContent>
    </Card>
  );
}

interface ReportToggleRowProps {
  label: string;
  def?: boolean;
  disabled?: boolean;
}

function ReportToggleRow({ label, def, disabled }: ReportToggleRowProps) {
  const [on, setOn] = useState(!!def);
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={() => {
        if (!disabled) setOn((v) => !v);
      }}
      className="flex items-center justify-between gap-3 rounded-md border bg-background px-3 py-2 text-left text-sm transition-colors hover:bg-muted/50 disabled:cursor-not-allowed disabled:bg-muted/35 disabled:text-muted-foreground disabled:hover:bg-muted/35"
    >
      <span>{label}</span>
      <span
        className={cn(
          "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors",
          on ? (disabled ? "bg-primary/60" : "bg-primary") : "bg-muted",
        )}
      >
        <span
          className={cn(
            "size-4 rounded-full bg-background shadow transition-transform",
            on ? "translate-x-4" : "translate-x-0.5",
          )}
        />
      </span>
    </button>
  );
}

interface ReportExportFormatProps {
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  id: ReportExportFormatId;
  name: string;
  sub: string;
  detail: string;
  primary?: boolean;
  loading?: boolean;
  disabled?: boolean;
  onClick: (format: ReportExportFormatId) => void;
}

function ReportExportFormat({
  icon: Icon,
  id,
  name,
  sub,
  detail,
  primary,
  loading = false,
  disabled = false,
  onClick,
}: ReportExportFormatProps) {
  return (
    <Button
      type="button"
      variant={primary ? "default" : "outline"}
      className="h-auto min-h-20 min-w-0 justify-start gap-3 whitespace-normal p-4 text-left"
      disabled={disabled || loading}
      onClick={() => onClick(id)}
    >
      {loading ? (
        <Loader2 className="size-5 shrink-0 animate-spin" aria-hidden="true" />
      ) : (
        <Icon className="size-5 shrink-0" aria-hidden="true" />
      )}
      <span className="grid min-w-0 gap-1">
        <span className="font-medium">{name}</span>
        <span
          className={cn(
            "whitespace-normal text-xs leading-snug font-normal break-words",
            primary ? "text-primary-foreground/75" : "text-muted-foreground",
          )}
        >
          {sub} · {detail}
        </span>
      </span>
    </Button>
  );
}
