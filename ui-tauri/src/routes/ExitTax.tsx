/**
 * Exit Tax — Wegzugsbesteuerung deemed-disposal estimate.
 *
 * Reads like a "what would I owe if I left now" simulator: pick a departure
 * date and destination class, see the estimated exit tax on Neubestand
 * (Altbestand is excluded), then export a Steuerberater handoff. The estimate
 * is computed by the daemon from the same processed journal state the
 * capital-gains report uses; this screen never does tax math itself.
 */

import { Link } from "@tanstack/react-router";
import { useMemo, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Clock,
  Download,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  Info,
  Landmark,
  Loader2,
  Plane,
  ShieldCheck,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DatePicker } from "@/components/ui/date-picker";
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
import {
  canOpenExportedFiles,
  canSaveExportedFiles,
  openExportedFile,
  saveExportedFileAs,
} from "@/daemon/transport";
import { saveFile } from "@/lib/filePicker";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type {
  ExitTaxDestination,
  ExitTaxLot,
  ExitTaxReport,
} from "@/mocks/exitTax";
import { useUiStore } from "@/store/ui";

type ExportFormat = "pdf" | "xlsx";

interface ExitTaxExportStatus {
  tone: "success" | "error";
  message: string;
  path?: string;
}

const DESTINATION_OPTIONS: Array<{ value: ExitTaxDestination; label: string }> = [
  { value: "eu_eea", label: "EU / EEA member state" },
  { value: "third_country", label: "Third country (non-EU/EEA)" },
];

function todayIso(): string {
  return new Date().toISOString().slice(0, 10);
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function ExitTax() {
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const [departureDate, setDepartureDate] = useState<string>(todayIso());
  const [destination, setDestination] = useState<ExitTaxDestination>("eu_eea");
  const [activeExport, setActiveExport] = useState<ExportFormat | null>(null);
  const [exportStatus, setExportStatus] = useState<ExitTaxExportStatus | null>(null);

  const preview = useDaemon<ExitTaxReport>("ui.reports.exit_tax_preview", {
    departure_date: departureDate,
    destination,
  });
  const exportPdf = useDaemonMutation("ui.reports.export_exit_tax_pdf");
  const exportXlsx = useDaemonMutation("ui.reports.export_exit_tax_xlsx");

  const report = preview.data?.data;
  const ccy = report?.fiatCurrency ?? "EUR";

  const fmtMoney = useMemo(() => {
    const formatter = new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: ccy,
      maximumFractionDigits: 2,
    });
    return (value: number | null | undefined) =>
      value == null ? "—" : formatter.format(value);
  }, [ccy]);

  const fmtBtc = (sats: number) => `${(sats / 1e8).toFixed(8)} BTC`;
  const sensitive = (hidden: boolean) => (hidden ? "sensitive" : "");

  const handleExport = (format: ExportFormat) => {
    if (activeExport) return;
    setExportStatus(null);
    setActiveExport(format);
    const mutation = format === "pdf" ? exportPdf : exportXlsx;
    mutation.mutate(
      { departure_date: departureDate, destination },
      {
        onSuccess: async (envelope) => {
          const payload = (envelope.data ?? {}) as {
            file?: string;
            filename?: string;
          };
          const exportPath = payload.file ?? "";
          const filename = payload.filename ?? basename(exportPath) ?? "exit-tax";
          let savedPath = exportPath;
          if (exportPath && canSaveExportedFiles()) {
            try {
              const destinationPath = await saveFile({
                title: "Save exit-tax handoff",
                defaultPath: filename,
              });
              if (destinationPath) {
                savedPath = await saveExportedFileAs(exportPath, destinationPath);
              }
            } catch (error) {
              setActiveExport(null);
              setExportStatus({
                tone: "error",
                message:
                  error instanceof Error ? error.message : "Could not save the export.",
                path: exportPath,
              });
              return;
            }
          }
          setActiveExport(null);
          setExportStatus({
            tone: "success",
            message: `Saved ${format.toUpperCase()} handoff — ${basename(savedPath)}`,
            path: savedPath,
          });
        },
        onError: (error: unknown) => {
          setActiveExport(null);
          setExportStatus({
            tone: "error",
            message: error instanceof Error ? error.message : "Export failed.",
          });
        },
      },
    );
  };

  return (
    <div className={screenShellClassName}>
      <header className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
        <div className="flex items-start gap-3">
          <div className="grid size-10 shrink-0 place-items-center rounded-xl border bg-card text-primary">
            <Plane className="size-5" />
          </div>
          <div>
            <h1 className="text-lg font-semibold tracking-tight">Exit tax</h1>
            <p className="max-w-prose text-sm text-muted-foreground">
              Estimate the Austrian Wegzugsbesteuerung on your Bitcoin if you give up tax
              residence — a deemed disposal at fair market value. Hand the draft to your
              Steuerberater.
            </p>
          </div>
        </div>
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            Departure date
            <DatePicker
              value={departureDate}
              onChange={(next) => setDepartureDate(next || todayIso())}
              className="w-[180px]"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs font-medium text-muted-foreground">
            Destination
            <Select
              value={destination}
              onValueChange={(value) => setDestination(value as ExitTaxDestination)}
            >
              <SelectTrigger className="h-9 w-[220px]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DESTINATION_OPTIONS.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    {option.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </label>
        </div>
      </header>

      {preview.isLoading ? (
        <ScreenSkeleton />
      ) : preview.isError ? (
        <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          <div className="flex items-center gap-2 font-medium">
            <AlertTriangle className="size-4" /> Could not load the exit-tax estimate
          </div>
          <p className="mt-1 text-destructive/90">
            {preview.error instanceof Error ? preview.error.message : "Unknown error."}
          </p>
        </div>
      ) : report ? (
        <ExitTaxBody
          report={report}
          fmtMoney={fmtMoney}
          fmtBtc={fmtBtc}
          sensitive={sensitive}
          hideSensitive={hideSensitive}
          activeExport={activeExport}
          exportStatus={exportStatus}
          onExport={handleExport}
        />
      ) : null}
    </div>
  );
}

interface BodyProps {
  report: ExitTaxReport;
  fmtMoney: (value: number | null | undefined) => string;
  fmtBtc: (sats: number) => string;
  sensitive: (hidden: boolean) => string;
  hideSensitive: boolean;
  activeExport: ExportFormat | null;
  exportStatus: ExitTaxExportStatus | null;
  onExport: (format: ExportFormat) => void;
}

function ExitTaxBody({
  report,
  fmtMoney,
  fmtBtc,
  sensitive,
  hideSensitive,
  activeExport,
  exportStatus,
  onExport,
}: BodyProps) {
  const totals = report.totals;
  const deferred = totals.collectionTiming === "deferred";
  const incomplete = report.status.needsJournals || report.status.quarantines > 0;
  // Alt/Neu is Austrian terminology; a generic profile gets neutral labels.
  const isAt = report.jurisdictionCode === "AT";

  return (
    <div className="grid gap-4">
      {incomplete ? (
        <div className="flex items-start gap-3 rounded-xl border border-amber-500/30 bg-amber-500/10 p-4 text-sm">
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
          <div>
            <p className="font-medium">This estimate is incomplete.</p>
            <p className="text-muted-foreground">
              {report.status.needsJournals
                ? "Journals are not processed yet."
                : `${report.status.quarantines} quarantined transaction(s) are excluded.`}{" "}
              Resolve them in the{" "}
              <Link to="/quarantine" className="underline underline-offset-2">
                quarantine queue
              </Link>{" "}
              before relying on the number.
            </p>
          </div>
        </div>
      ) : null}

      {/* Headline liability */}
      <div className="grid gap-4 rounded-xl border bg-card p-5 sm:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)] sm:items-center">
        <div>
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            Estimated exit tax
          </p>
          <p
            className={cn(
              "mt-1 text-4xl font-semibold tracking-tight tabular-nums",
              sensitive(hideSensitive),
            )}
          >
            {fmtMoney(totals.estimatedTax)}
          </p>
          <p className="mt-1 text-sm text-muted-foreground">
            {totals.estimatedTaxRate != null
              ? `${(totals.estimatedTaxRate * 100).toFixed(1)}% of `
              : "on "}
            <span className={sensitive(hideSensitive)}>{fmtMoney(totals.taxableGain)}</span>{" "}
            taxable {isAt ? "Neubestand " : ""}gain
          </p>
        </div>
        <dl className="grid grid-cols-2 gap-3 text-sm">
          <div>
            <dt className="text-xs text-muted-foreground">{isAt ? "Neubestand gain" : "Taxable gain"}</dt>
            <dd className={cn("font-medium tabular-nums", sensitive(hideSensitive))}>
              {fmtMoney(totals.neuGain)}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">{isAt ? "Altbestand (excluded)" : "Excluded holdings"}</dt>
            <dd className={cn("font-medium tabular-nums", sensitive(hideSensitive))}>
              {fmtMoney(totals.altMarketValue)}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Method</dt>
            <dd className="font-medium">{report.method}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Jurisdiction</dt>
            <dd className="font-medium">{report.jurisdictionCode}</dd>
          </div>
        </dl>
      </div>

      {/* Collection-timing banner */}
      <div
        className={cn(
          "flex items-start gap-3 rounded-xl border p-4 text-sm",
          deferred
            ? "border-sky-500/30 bg-sky-500/10"
            : "border-amber-500/30 bg-amber-500/10",
        )}
      >
        {deferred ? (
          <Clock className="mt-0.5 size-4 shrink-0 text-sky-600 dark:text-sky-400" />
        ) : (
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
        )}
        <div>
          <p className="font-medium">
            {deferred
              ? "Assessed but deferred until you sell"
              : "Due immediately on departure"}
          </p>
          <p className="text-muted-foreground">
            {deferred
              ? "For an EU/EEA move the tax is assessed but, on application, not collected until you actually dispose of the assets (Nichtfestsetzung, § 27 Abs 6 Z 1 lit a)."
              : "For a third-country (non-EU/EEA) move the deemed-disposal tax falls due at departure."}
          </p>
        </div>
      </div>

      <div className="grid items-start gap-4 2xl:grid-cols-[minmax(0,1fr)_360px]">
        <div className="grid min-w-0 gap-4">
          {/* Deemed disposal lots */}
          <section className="min-w-0 overflow-hidden rounded-xl border bg-card">
            <div className="flex items-center gap-2 px-4 pt-4 sm:px-5">
              <Landmark className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-medium">Deemed disposal at fair market value</h2>
            </div>
            <div className="overflow-x-auto px-1 pb-2 sm:px-2">
              <Table className="min-w-[680px]">
                <TableHeader>
                  <TableRow className="bg-muted/50">
                    <TableHead>Asset</TableHead>
                    <TableHead>Regime</TableHead>
                    <TableHead className="text-right">Quantity</TableHead>
                    <TableHead className="text-right">Cost basis</TableHead>
                    <TableHead className="text-right">Market value</TableHead>
                    <TableHead className="text-right">Gain</TableHead>
                    <TableHead className="text-right">Kennzahl</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {report.lots.map((lot, index) => (
                    <LotRow
                      key={`${lot.asset}-${lot.regime}-${index}`}
                      lot={lot}
                      isAt={isAt}
                      fmtMoney={fmtMoney}
                      fmtBtc={fmtBtc}
                      sensitive={sensitive}
                      hideSensitive={hideSensitive}
                    />
                  ))}
                </TableBody>
              </Table>
            </div>
          </section>

          {/* Wallet holdings */}
          {report.walletHoldings.length ? (
            <section className="min-w-0 overflow-hidden rounded-xl border bg-card">
              <div className="flex items-center gap-2 px-4 pt-4 sm:px-5">
                <Info className="size-4 text-muted-foreground" />
                <h2 className="text-sm font-medium">Wallet holdings (context)</h2>
              </div>
              <div className="overflow-x-auto px-1 pb-2 sm:px-2">
                <Table className="min-w-[480px]">
                  <TableHeader>
                    <TableRow className="bg-muted/50">
                      <TableHead>Wallet</TableHead>
                      <TableHead>Asset</TableHead>
                      <TableHead className="text-right">Quantity</TableHead>
                      <TableHead className="text-right">Market value</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {report.walletHoldings.map((holding, index) => (
                      <TableRow key={`${holding.wallet}-${holding.asset}-${index}`}>
                        <TableCell className="font-medium">{holding.wallet}</TableCell>
                        <TableCell>{holding.asset}</TableCell>
                        <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
                          {fmtBtc(holding.quantitySats)}
                        </TableCell>
                        <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
                          {fmtMoney(holding.marketValue)}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            </section>
          ) : null}
        </div>

        <div className="grid min-w-0 gap-4">
          {/* Export handoff */}
          <section className="rounded-xl border bg-card p-4 sm:p-5">
            <div className="flex items-center gap-2">
              <Download className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-medium">Steuerberater handoff</h2>
            </div>
            <p className="mt-1 text-xs text-muted-foreground">
              Export the estimate for your tax adviser to review and stamp.
            </p>
            <div className="mt-3 flex flex-col gap-2">
              <Button
                variant="default"
                disabled={Boolean(activeExport)}
                onClick={() => onExport("pdf")}
              >
                {activeExport === "pdf" ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <FileText className="size-4" />
                )}
                Export PDF
              </Button>
              <Button
                variant="outline"
                disabled={Boolean(activeExport)}
                onClick={() => onExport("xlsx")}
              >
                {activeExport === "xlsx" ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <FileSpreadsheet className="size-4" />
                )}
                Export XLSX
              </Button>
            </div>
            {exportStatus ? (
              <div
                className={cn(
                  "mt-3 flex items-start gap-2 rounded-lg border p-2 text-xs",
                  exportStatus.tone === "success"
                    ? "border-emerald-500/30 bg-emerald-500/10"
                    : "border-destructive/30 bg-destructive/5 text-destructive",
                )}
              >
                {exportStatus.tone === "success" ? (
                  <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                ) : (
                  <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                )}
                <div className="min-w-0">
                  <p className="break-words">{exportStatus.message}</p>
                  {exportStatus.tone === "success" &&
                  exportStatus.path &&
                  canOpenExportedFiles() ? (
                    <button
                      type="button"
                      className="mt-1 inline-flex items-center gap-1 underline underline-offset-2"
                      onClick={() => {
                        if (exportStatus.path) void openExportedFile(exportStatus.path);
                      }}
                    >
                      <FolderOpen className="size-3" /> Open
                    </button>
                  ) : null}
                </div>
              </div>
            ) : null}
          </section>

          {/* FMV source */}
          <section className="rounded-xl border bg-card p-4 sm:p-5">
            <h2 className="text-sm font-medium">Fair market value</h2>
            <ul className="mt-2 grid gap-1 text-xs text-muted-foreground">
              {report.fmvSource.map((source) => (
                <li key={source.asset} className="flex items-center justify-between gap-2">
                  <span>{source.pair}</span>
                  <span className="tabular-nums">
                    {source.rate != null ? fmtMoney(source.rate) : "—"}
                    <Badge variant="outline" className="ml-2">
                      {source.source}
                    </Badge>
                  </span>
                </li>
              ))}
            </ul>
          </section>

          {/* Assumptions & review gate */}
          <section className="rounded-xl border bg-card p-4 sm:p-5">
            <div className="flex items-center gap-2">
              <ShieldCheck className="size-4 text-muted-foreground" />
              <h2 className="text-sm font-medium">Assumptions &amp; review</h2>
            </div>
            <ul className="mt-2 list-disc space-y-1 pl-4 text-xs text-muted-foreground">
              {report.assumptions.map((note, index) => (
                <li key={index}>{note}</li>
              ))}
            </ul>
            <p className="mt-3 rounded-lg border border-dashed bg-muted/30 p-2 text-xs text-muted-foreground">
              {report.reviewGate}
            </p>
          </section>
        </div>
      </div>
    </div>
  );
}

interface LotRowProps {
  lot: ExitTaxLot;
  isAt: boolean;
  fmtMoney: (value: number | null | undefined) => string;
  fmtBtc: (sats: number) => string;
  sensitive: (hidden: boolean) => string;
  hideSensitive: boolean;
}

function LotRow({ lot, isAt, fmtMoney, fmtBtc, sensitive, hideSensitive }: LotRowProps) {
  return (
    <TableRow>
      <TableCell className="font-medium">{lot.asset}</TableCell>
      <TableCell>
        {lot.regime === "neu" ? (
          <Badge className="bg-primary/15 text-primary hover:bg-primary/15">
            {isAt ? "Neubestand · taxable" : "Taxable"}
          </Badge>
        ) : (
          <Badge variant="outline" className="text-muted-foreground">
            {isAt ? "Altbestand · tax-free" : "Tax-free"}
          </Badge>
        )}
      </TableCell>
      <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
        {fmtBtc(lot.quantitySats)}
      </TableCell>
      <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
        {lot.regime === "alt" ? "—" : fmtMoney(lot.costBasis)}
      </TableCell>
      <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
        {fmtMoney(lot.marketValue)}
      </TableCell>
      <TableCell className={cn("text-right tabular-nums", sensitive(hideSensitive))}>
        {lot.regime === "alt" ? "excluded" : fmtMoney(lot.gain)}
      </TableCell>
      <TableCell className="text-right text-muted-foreground tabular-nums">
        {lot.kennzahl ?? "—"}
      </TableCell>
    </TableRow>
  );
}

export default ExitTax;
