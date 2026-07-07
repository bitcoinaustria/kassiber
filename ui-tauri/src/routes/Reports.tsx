/**
 * Reports — tax report package and export flow.
 *
 * Reports should read like a package workflow: check whether the journal state
 * is trusted, inspect the current daemon-provided audit rows, then export the
 * files that belong to the current book.
 */

import { Link } from "@tanstack/react-router";
import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  CalendarDays,
  ChevronDown,
  CheckCircle2,
  Download,
  ExternalLink,
  FileArchive,
  FileCheck2,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  KeyRound,
  Landmark,
  Loader2,
  PackageCheck,
  PieChart,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Sigma,
  WalletCards,
} from "lucide-react";

import { LightningProfitabilityPanel } from "@/components/lightning/LightningProfitabilityPanel";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
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
import {
  HANDOFF_EXPORT_MODES,
  NORMAL_HANDOFF_EXCLUSIONS,
  type HandoffExportMode,
} from "@/lib/handoffExports";
import {
  reportExportStatusForYear,
  type ReportExportStatus,
} from "@/lib/reportExportStatus";
import { reportYearFromSearch } from "@/lib/reportYear";
import { exportBasename } from "@/lib/exportFile";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenPanelClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import {
  JURISDICTIONS,
  type CapitalGainsReport,
  type CostBasisMethod,
  type DisposedLot,
  type KennzahlRow,
  type NeutralSwapLot,
} from "@/mocks/reports";
import { useUiStore } from "@/store/ui";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const METHOD_LABELS: Record<
  CostBasisMethod,
  { name: string; desc: string; fullName?: string }
> = {
    fifo: { name: "FIFO", desc: "First-in, first-out" },
    lifo: { name: "LIFO", desc: "Last-in, first-out" },
    hifo: { name: "HIFO", desc: "Highest-in, first-out" },
    lofo: { name: "LOFO", desc: "Lowest-in, first-out" },
    moving_average: {
      name: "Moving average",
      desc: "Average cost pool",
    },
    moving_average_at: {
      name: "ATM",
      fullName: "ATM - Austrian Tax Method (FIFO old stock & AVCO new stock)",
      desc: "FIFO old stock; AVCO new stock",
    },
  };

const AUSTRIAN_TAX_FIELD_COPY: Record<
  string,
  { label: string; form: string; note?: string }
> = {
  "172": { label: "Foreign ongoing crypto income", form: "E 1kv" },
  "174": { label: "Foreign realized crypto gains", form: "E 1kv" },
  "176": { label: "Foreign realized crypto losses", form: "E 1kv" },
  "801": {
    label: "Legacy holdings speculation gains",
    form: "E 1",
    note: "Outside E 1kv",
  },
};

const AUSTRIAN_KENNZAHL_PLACEHOLDER_ROWS: KennzahlRow[] = [
  {
    code: "172",
    label: AUSTRIAN_TAX_FIELD_COPY["172"].label,
    form: AUSTRIAN_TAX_FIELD_COPY["172"].form,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "174",
    label: AUSTRIAN_TAX_FIELD_COPY["174"].label,
    form: AUSTRIAN_TAX_FIELD_COPY["174"].form,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "176",
    label: AUSTRIAN_TAX_FIELD_COPY["176"].label,
    form: AUSTRIAN_TAX_FIELD_COPY["176"].form,
    amount: null,
    rowCount: 0,
    source: "pending",
  },
  {
    code: "801",
    label: AUSTRIAN_TAX_FIELD_COPY["801"].label,
    form: AUSTRIAN_TAX_FIELD_COPY["801"].form,
    amount: null,
    rowCount: 0,
    source: "pending",
    note: AUSTRIAN_TAX_FIELD_COPY["801"].note,
  },
];

type ReportExportFormatId =
  | "csv"
  | "pdf"
  | "xlsx"
  | "summary_pdf"
  | "audit_package"
  | "transactions_csv"
  | "transactions_xlsx";
type AuditPackageScope = "active_profile" | "source_funds_case";
type ReportTone = "good" | "warning" | "alert" | "neutral";
type ReportHref = "/journals" | "/quarantine" | "/transactions" | "/reports";

interface ReportExportResult {
  file?: string;
  dir?: string;
  manifest?: string;
  filename?: string;
  format?: string;
  scope?: string;
  bytes?: number;
  pages?: number;
  rows?: number;
  sheets?: string[];
  files?: Array<{
    file?: string;
    sheet?: string;
    bytes?: number;
    rows?: number;
  }>;
  summary_rows?: number;
  tax_year?: number;
  timeframe?: {
    start?: string;
    end?: string;
    label?: string;
  };
  wallets?: Array<{ id?: string; label?: string }>;
  snapshot?: boolean;
  transaction_count?: number;
  evidence_file_count?: number;
  url_reference_count?: number;
}

interface WalletListData {
  wallets: Array<{
    id?: string;
    label: string;
    kind?: string;
    chain?: string;
    transaction_count?: number;
  }>;
}

interface SourceFundsCaseRow {
  id: string;
  label?: string;
  target_external_id?: string;
  status?: string;
  created_at?: string;
}

interface SourceFundsCasesData {
  cases: SourceFundsCaseRow[];
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

function reportExportDefaultFilename(
  format: ReportExportFormatId,
  year: number,
  austrian: boolean,
) {
  if (format === "transactions_xlsx") return "kassiber-transactions.xlsx";
  if (format === "transactions_csv") return "kassiber-transactions.csv";
  if (format === "audit_package") return `kassiber-audit-package-${year}`;
  if (format === "summary_pdf") return `kassiber-summary-report-${year}.pdf`;
  if (austrian) {
    if (format === "pdf") return `kassiber-austrian-e1kv-${year}.pdf`;
    if (format === "xlsx") return `kassiber-austrian-e1kv-${year}.xlsx`;
    return `kassiber-austrian-e1kv-${year}-csv`;
  }
  if (format === "pdf") return "kassiber-report.pdf";
  if (format === "xlsx") return "kassiber-report.xlsx";
  return "kassiber-report.csv";
}

function reportExportSaveFilters(
  format: ReportExportFormatId,
  payload?: ReportExportResult,
) {
  if (format === "audit_package") return undefined;
  if (payload?.format === "csv" && payload.files?.length) return undefined;
  if (format === "transactions_xlsx") {
    return [{ name: "Excel workbook", extensions: ["xlsx"] }];
  }
  if (format === "transactions_csv") return [{ name: "CSV", extensions: ["csv"] }];
  if (format === "pdf" || format === "summary_pdf") {
    return [{ name: "PDF report", extensions: ["pdf"] }];
  }
  if (format === "xlsx") return [{ name: "Excel workbook", extensions: ["xlsx"] }];
  return [{ name: "CSV report", extensions: ["csv"] }];
}

function basename(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function initialReportYearFromUrl() {
  if (typeof window === "undefined") return null;
  return reportYearFromSearch(window.location.search);
}

export function Reports() {
  const [selectedYear, setSelectedYear] = useState<number | null>(
    initialReportYearFromUrl,
  );
  useEffect(() => {
    const syncYearFromUrl = () => {
      setSelectedYear(reportYearFromSearch(window.location.search));
    };
    window.addEventListener("popstate", syncYearFromUrl);
    return () => window.removeEventListener("popstate", syncYearFromUrl);
  }, []);
  const reportArgs = useMemo(
    () => (selectedYear !== null ? { year: selectedYear } : undefined),
    [selectedYear],
  );
  const { data, isLoading, isFetching, isError, error } =
    useDaemon<CapitalGainsReport>("ui.reports.capital_gains", reportArgs);
  const wallets = useDaemon<WalletListData>("ui.wallets.list");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const returnedReportYear = data?.data?.year;
  const reportYearMismatch =
    selectedYear !== null &&
    returnedReportYear !== undefined &&
    returnedReportYear !== selectedYear;

  if (isLoading || (reportYearMismatch && isFetching)) {
    return <ScreenSkeleton titleWidth="w-48" />;
  }

  if (reportYearMismatch) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-lg border bg-card p-4">
          <h2 className="text-base font-semibold">Reports unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            The daemon returned {returnedReportYear} while the selected tax year is{" "}
            {selectedYear}.
          </p>
        </div>
      </div>
    );
  }

  if (isError || data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-lg border bg-card p-4">
          <h2 className="text-base font-semibold">Reports unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ?? "The daemon did not return report data."}
          </p>
        </div>
      </div>
    );
  }

  return (
    <ReportsView
      report={data.data}
      hideSensitive={hideSensitive}
      selectedYear={selectedYear}
      onYearChange={setSelectedYear}
      wallets={wallets.data?.data?.wallets ?? []}
    />
  );
}

interface ReportsViewProps {
  report: CapitalGainsReport;
  hideSensitive: boolean;
  selectedYear: number | null;
  onYearChange: (year: number) => void;
  wallets: WalletListData["wallets"];
}

function ReportsView({
  report,
  hideSensitive,
  selectedYear,
  onYearChange,
  wallets,
}: ReportsViewProps) {
  const year = report.year;
  const effectiveYear = selectedYear ?? year;
  const availableYears = Array.from(
    new Set([
      ...(report.availableYears?.length ? report.availableYears : [effectiveYear]),
      effectiveYear,
    ]),
  )
    .filter((item) => Number.isInteger(item))
    .sort((a, b) => b - a);
  const jurisdiction =
    JURISDICTIONS[report.jurisdictionCode] ?? JURISDICTIONS.AT;
  // The cost-basis method is the book's configured gains algorithm; the
  // report was computed with it, so the UI treats it as a fact, not a knob.
  const method = normalizeReportMethod(report.method, jurisdiction);
  const [exportStatus, setExportStatus] = useState<ReportExportStatus | null>(
    null,
  );
  const [activeExport, setActiveExport] =
    useState<ReportExportFormatId | null>(null);
  const [successfulExport, setSuccessfulExport] = useState<{
    format: ReportExportFormatId;
    year: number;
  } | null>(null);
  const [openingExportPath, setOpeningExportPath] = useState<string | null>(
    null,
  );
  const [summarySnapshot, setSummarySnapshot] = useState(true);
  const [summaryWalletIds, setSummaryWalletIds] = useState<string[]>([]);
  const [auditScope, setAuditScope] =
    useState<AuditPackageScope>("active_profile");
  const [auditCaseId, setAuditCaseId] = useState("");
  const [auditIncludeCopiedAttachments, setAuditIncludeCopiedAttachments] =
    useState(true);
  const [auditIncludeUrlReferences, setAuditIncludeUrlReferences] =
    useState(true);
  const [auditIncludeJournalState, setAuditIncludeJournalState] =
    useState(true);
  const [auditIncludeReviewState, setAuditIncludeReviewState] =
    useState(true);
  const [auditIncludeEditHistory, setAuditIncludeEditHistory] =
    useState(false);
  const addNotification = useUiStore((s) => s.addNotification);
  const exportCsv = useDaemonMutation<ReportExportResult>("ui.reports.export_csv");
  const exportXlsx = useDaemonMutation<ReportExportResult>("ui.reports.export_xlsx");
  const exportPdf = useDaemonMutation<ReportExportResult>("ui.reports.export_pdf");
  const exportAuditPackage =
    useDaemonMutation<ReportExportResult>("ui.reports.export_audit_package");
  const exportSummaryPdf =
    useDaemonMutation<ReportExportResult>("ui.reports.export_summary_pdf");
  const exportAustrianPdf =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_pdf");
  const exportAustrianXlsx =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_xlsx");
  const exportAustrianCsv =
    useDaemonMutation<ReportExportResult>("ui.reports.export_austrian_e1kv_csv");
  const exportTransactionsXlsx =
    useDaemonMutation<ReportExportResult>("ui.transactions.export_xlsx");
  const exportTransactionsCsv =
    useDaemonMutation<ReportExportResult>("ui.transactions.export_csv");
  const activeProfileIsAustrian = report.jurisdictionCode === "AT";
  const sourceFundsCasesQuery = useDaemon<SourceFundsCasesData>(
    "ui.source_funds.cases.list",
  );
  const sourceFundsCases = useMemo(
    () => sourceFundsCasesQuery.data?.data?.cases ?? [],
    [sourceFundsCasesQuery.data?.data?.cases],
  );
  const walletChoices = useMemo(
    () => wallets.filter((wallet) => wallet.id || wallet.label),
    [wallets],
  );
  const showWalletPicker = walletChoices.length > 1;
  useEffect(() => {
    if (auditScope !== "source_funds_case") return;
    if (auditCaseId) return;
    const firstCase = sourceFundsCases[0];
    if (firstCase?.id) {
      setAuditCaseId(firstCase.id);
    }
  }, [auditCaseId, auditScope, sourceFundsCases]);
  useEffect(() => {
    setSummaryWalletIds((current) => {
      const allIds = walletChoices.map((wallet) => wallet.id ?? wallet.label);
      if (allIds.length <= 1) return allIds;
      const currentSet = new Set(current);
      const kept = allIds.filter((id) => currentSet.has(id));
      return kept.length ? kept : allIds;
    });
  }, [walletChoices]);
  const kennzahlRows =
    report.kennzahlRows?.length
      ? report.kennzahlRows
      : activeProfileIsAustrian
        ? AUSTRIAN_KENNZAHL_PLACEHOLDER_ROWS
        : [];

  const lots = report.lots;
  const neutralSwapLots = report.neutralSwapLots ?? [];
  const totals = summarizeLots(lots);
  const estimatedTax = Math.max(totals.gain, 0) * jurisdiction.rate;
  const fmt = (n: number) =>
    n.toLocaleString(jurisdiction.locale, {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  const readiness = buildReportReadiness(report, lots, effectiveYear);
  const periodLabel = formatReportPeriod(effectiveYear, jurisdiction.locale);
  const currentExportStatus = reportExportStatusForYear(
    exportStatus,
    effectiveYear,
  );
  const canOpenCurrentExport =
    currentExportStatus?.tone === "success" &&
    canOpenExportPath(currentExportStatus.openPath) &&
    canOpenExportedFiles();
  const openableExportPath =
    canOpenCurrentExport && currentExportStatus?.openPath
      ? currentExportStatus.openPath
      : null;
  useEffect(() => {
    if (!successfulExport) return;
    const timeout = window.setTimeout(() => {
      setSuccessfulExport((current) =>
        current?.format === successfulExport.format &&
        current.year === successfulExport.year
          ? null
          : current,
      );
    }, 5000);
    return () => window.clearTimeout(timeout);
  }, [successfulExport]);

  const handleExport = (format: ReportExportFormatId) => {
    if (activeExport) return;
    const exportYear = effectiveYear;
    setExportStatus(null);
    setSuccessfulExport(null);
    setActiveExport(format);
    const mutation =
      format === "summary_pdf"
        ? exportSummaryPdf
        : format === "audit_package"
          ? exportAuditPackage
        : format === "transactions_xlsx"
          ? exportTransactionsXlsx
        : format === "transactions_csv"
          ? exportTransactionsCsv
        : format === "csv"
        ? activeProfileIsAustrian
          ? exportAustrianCsv
          : exportCsv
        : format === "xlsx"
          ? activeProfileIsAustrian
            ? exportAustrianXlsx
            : exportXlsx
          : activeProfileIsAustrian
            ? exportAustrianPdf
            : exportPdf;
    const args =
      format === "summary_pdf"
        ? {
            start: `${exportYear}-01-01T00:00:00Z`,
            end: `${exportYear}-12-31T23:59:59Z`,
            include_snapshot: summarySnapshot,
            ...(showWalletPicker ? { wallets: summaryWalletIds } : {}),
          }
        : format === "audit_package"
          ? {
              include_copied_attachments: auditIncludeCopiedAttachments,
              include_url_references: auditIncludeUrlReferences,
              include_journal_state: auditIncludeJournalState,
              include_review_state: auditIncludeReviewState,
              include_edit_history: auditIncludeEditHistory,
              ...(auditScope === "source_funds_case" && auditCaseId
                ? { source_funds_case: auditCaseId }
                : {}),
            }
        : format === "transactions_xlsx" || format === "transactions_csv"
          ? {}
        : format === "pdf"
        ? activeProfileIsAustrian
          ? { year: exportYear }
          : {}
        : (format === "xlsx" || format === "csv") && activeProfileIsAustrian
          ? { year: exportYear }
          : {};

    mutation.mutate(args, {
      onSuccess: async (envelope) => {
        const payload = envelope.data;
        const exportPath = payload?.file ?? payload?.dir ?? "";
        const filename = (payload?.filename ?? basename(exportPath)) || "report";
        const isTransactionExport =
          format === "transactions_xlsx" || format === "transactions_csv";
        const detail =
          payload?.scope === "audit_package"
            ? `${payload.transaction_count ?? 0} transaction${payload.transaction_count === 1 ? "" : "s"} · ${payload.evidence_file_count ?? 0} file${payload.evidence_file_count === 1 ? "" : "s"} · ${payload.url_reference_count ?? 0} link${payload.url_reference_count === 1 ? "" : "s"}`
          : isTransactionExport && payload?.rows !== undefined
            ? `${payload.rows} transaction${payload.rows === 1 ? "" : "s"}`
          : payload?.format === "pdf" && payload.pages
            ? `${payload.pages} page${payload.pages === 1 ? "" : "s"}`
            : payload?.format === "xlsx" && payload.sheets?.length
              ? `${payload.sheets.length} sheet${payload.sheets.length === 1 ? "" : "s"}`
              : payload?.format === "csv" && payload.files?.length
                ? `${payload.files.length} file${payload.files.length === 1 ? "" : "s"}`
              : payload?.format === "xlsx" && payload.rows !== undefined
                ? `${payload.rows} row${payload.rows === 1 ? "" : "s"}`
              : payload?.format === "csv" && payload.rows !== undefined
                ? `${payload.rows} row${payload.rows === 1 ? "" : "s"}`
                : "Export written";
        let savedPath = exportPath;
        if (exportPath && canSaveExportedFiles()) {
          try {
            const destination = await saveFile({
              title:
                payload?.scope === "audit_package"
                  ? "Save audit package"
                  : isTransactionExport
                    ? "Save transactions export"
                  : payload?.format === "csv" && payload.files?.length
                  ? "Save CSV bundle"
                  : "Save report export",
              defaultPath: reportExportDefaultFilename(
                format,
                exportYear,
                activeProfileIsAustrian,
              ),
              filters: reportExportSaveFilters(format, payload),
            });
            if (destination) {
              savedPath = await saveExportedFileAs(exportPath, destination);
            }
          } catch (error) {
            const message =
              error instanceof Error ? error.message : "Could not save report export";
            setExportStatus({
              year: exportYear,
              tone: "error",
              message,
              path: exportPath,
              openPath: exportPath,
            });
            addNotification({
              title: "Could not save report export",
              body: message,
              tone: "error",
            });
            return;
          }
        }
        if (savedPath === exportPath) {
          setExportStatus({
            year: exportYear,
            tone: "success",
            message: `${filename} saved.`,
            path: exportPath,
            openPath: exportPath,
          });
        }
        setSuccessfulExport({ format, year: exportYear });
        addNotification({
          title: isTransactionExport
            ? "Transactions exported"
            : "Report export finished",
          body:
            savedPath === exportPath
              ? detail
              : `${detail} · saved to ${exportBasename(savedPath)}`,
          tone: "success",
        });
      },
      onError: (error) => {
        const message =
          error instanceof Error ? error.message : "Report export failed";
        setExportStatus({ year: exportYear, tone: "error", message });
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
        selectedYear={effectiveYear}
        availableYears={availableYears}
        onYearChange={onYearChange}
        periodLabel={periodLabel}
        jurisdiction={jurisdiction}
        method={method}
        readiness={readiness}
      />

      <ReportMetricStrip
        hideSensitive={hideSensitive}
        jurisdiction={jurisdiction}
        lots={lots}
        totals={totals}
        estimatedTax={estimatedTax}
        year={effectiveYear}
        formatNumber={fmt}
      />

      <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="grid min-w-0 gap-3">
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
            method={method}
            jurisdiction={jurisdiction}
            hideSensitive={hideSensitive}
            formatNumber={fmt}
            year={effectiveYear}
          />
          {neutralSwapLots.length ? (
            <NeutralSwapAuditPanel
              swaps={neutralSwapLots}
              jurisdiction={jurisdiction}
              hideSensitive={hideSensitive}
              formatNumber={fmt}
            />
          ) : null}
          <HandoffScopePanel
            activeExport={activeExport}
            successfulExport={successfulExport}
            auditScope={auditScope}
            onAuditScopeChange={setAuditScope}
            auditCaseId={auditCaseId}
            onAuditCaseIdChange={setAuditCaseId}
            sourceFundsCases={sourceFundsCases}
            includeCopiedAttachments={auditIncludeCopiedAttachments}
            onIncludeCopiedAttachmentsChange={setAuditIncludeCopiedAttachments}
            includeUrlReferences={auditIncludeUrlReferences}
            onIncludeUrlReferencesChange={setAuditIncludeUrlReferences}
            includeJournalState={auditIncludeJournalState}
            onIncludeJournalStateChange={setAuditIncludeJournalState}
            includeReviewState={auditIncludeReviewState}
            onIncludeReviewStateChange={setAuditIncludeReviewState}
            includeEditHistory={auditIncludeEditHistory}
            onIncludeEditHistoryChange={setAuditIncludeEditHistory}
            onExport={handleExport}
          />
        </div>
        <div className="grid min-w-0 gap-3">
          <ReportFilesPanel
            year={effectiveYear}
            activeExport={activeExport}
            activeProfileIsAustrian={activeProfileIsAustrian}
            exportStatus={currentExportStatus}
            openableExportPath={openableExportPath}
            openingExportPath={openingExportPath}
            successfulExport={successfulExport}
            onExport={handleExport}
            onOpenExport={handleOpenExport}
          />
          <SummaryPdfPanel
            year={effectiveYear}
            wallets={walletChoices}
            selectedWalletIds={summaryWalletIds}
            showWalletPicker={showWalletPicker}
            includeSnapshot={summarySnapshot}
            loading={activeExport === "summary_pdf"}
            disabled={Boolean(activeExport)}
            success={
              successfulExport?.format === "summary_pdf" &&
              successfulExport.year === effectiveYear
            }
            onToggleSnapshot={setSummarySnapshot}
            onToggleWallet={(id) => {
              setSummaryWalletIds((current) => {
                const next = current.includes(id)
                  ? current.filter((item) => item !== id)
                  : [...current, id];
                return next.length ? next : current;
              });
            }}
            onExport={() => handleExport("summary_pdf")}
          />
        </div>
      </div>

      <LightningProfitabilityPanel />
    </div>
  );
}

function ReportPackageHeader({
  selectedYear,
  availableYears,
  onYearChange,
  periodLabel,
  jurisdiction,
  method,
  readiness,
}: {
  selectedYear: number;
  availableYears: number[];
  onYearChange: (year: number) => void;
  periodLabel: string;
  jurisdiction: (typeof JURISDICTIONS)[string];
  method: CostBasisMethod;
  readiness: ReportReadiness;
}) {
  const methodLabel = METHOD_LABELS[method] ?? METHOD_LABELS[jurisdiction.defaultMethod];
  const methodName = methodLabel.fullName ?? methodLabel.name;
  const ReadinessIcon = readiness.icon;
  const [rulesExpanded, setRulesExpanded] = useState(false);
  const handleYearChange = (value: string) => {
    const nextYear = Number(value);
    if (!Number.isInteger(nextYear)) return;
    if (typeof window !== "undefined") {
      const params = new URLSearchParams(window.location.search);
      params.set("year", String(nextYear));
      const query = params.toString();
      window.history.replaceState(
        null,
        "",
        query ? `${window.location.pathname}?${query}` : window.location.pathname,
      );
    }
    onYearChange(nextYear);
  };
  return (
    <div className="space-y-2">
      <div className={pageHeaderClassName}>
        <div className="flex min-w-0 flex-wrap items-center gap-2 sm:gap-3">
          <span className="text-sm font-semibold sm:text-base">Tax report</span>
          <Select value={String(selectedYear)} onValueChange={handleYearChange}>
            <SelectTrigger className="h-8 w-[88px] rounded-md text-xs" aria-label="Tax year">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {availableYears.map((availableYear) => (
                <SelectItem key={availableYear} value={String(availableYear)}>
                  {availableYear}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="hidden h-6 w-px bg-border sm:block" />
          <span className="inline-flex min-w-0 items-center gap-2 text-sm text-muted-foreground">
            <CalendarDays className="size-4 shrink-0" aria-hidden="true" />
            <span className="truncate">{periodLabel}</span>
          </span>
        </div>
        <div className={cn(pageHeaderActionsClassName, "min-w-0 lg:justify-end")}>
          <span
            className={cn(
              "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
              readinessToneStyles[readiness.tone],
            )}
          >
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readiness.title}
          </span>
          {readiness.action ? (
            <Button
              asChild
              size="sm"
              variant="outline"
              className={cn(pageHeaderActionClassName, "shrink-0 px-2 text-xs")}
            >
              <Link to={readiness.action.href}>{readiness.action.label}</Link>
            </Button>
          ) : null}
          <Button
            type="button"
            variant="outline"
            size="sm"
            className={cn(pageHeaderActionClassName, "max-w-full px-2 text-xs")}
            title={`Profile rules · ${jurisdiction.code} · ${methodName}`}
            aria-label={
              rulesExpanded ? "Collapse profile rules" : "Expand profile rules"
            }
            aria-expanded={rulesExpanded}
            onClick={() => setRulesExpanded((value) => !value)}
          >
            <ShieldAlert className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 truncate">
              Rules · {jurisdiction.code} · {methodLabel.name}
            </span>
            <ChevronDown
              className={cn(
                "size-4 shrink-0 text-muted-foreground transition-transform",
                rulesExpanded && "rotate-180",
              )}
              aria-hidden="true"
            />
          </Button>
        </div>
      </div>
      {rulesExpanded ? (
        <div className="rounded-lg border bg-card p-3">
          <ReportPolicyDetails
            jurisdiction={jurisdiction}
            methodName={methodName}
          />
        </div>
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
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4 lg:divide-x">
        {metrics.map((metric) => (
          <div
            key={metric.label}
            className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100"
          >
            <div className="pointer-events-none relative z-20 space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {metric.label}
            </p>
            <p className="min-w-0 text-lg leading-tight font-semibold tracking-tight tabular-nums sm:text-xl">
              {metric.value}
            </p>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              {metric.sub}
            </p>
            </div>
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
  const isEmptyField = (row: KennzahlRow) =>
    row.amount !== null && row.rowCount === 0 && Math.abs(row.amount) < 0.005;
  const visibleRows = rows;
  const rowGroups = visibleRows.reduce<Array<{ form: string; rows: KennzahlRow[] }>>(
    (groups, row) => {
      const fallbackForm = AUSTRIAN_TAX_FIELD_COPY[row.code]?.form ?? "E 1kv";
      const form = row.form || fallbackForm;
      const existing = groups.find((group) => group.form === form);
      if (existing) {
        existing.rows.push(row);
      } else {
        groups.push({ form, rows: [row] });
      }
      return groups;
    },
    [],
  );

  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-md border"
          >
            <Landmark className="size-4 text-muted-foreground" aria-hidden="true" />
          </span>
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
            {visibleRows.length} of {rows.length} fields
          </Badge>
        </div>
      </div>

      <div className="grid gap-2 px-3 pb-3 sm:grid-cols-2">
        {rowGroups.length ? (
          rowGroups.map((group) => (
            <Fragment key={group.form}>
              {rowGroups.length > 1 ? (
                <div className="pt-1 text-[10px] font-medium tracking-wide text-muted-foreground uppercase sm:col-span-2">
                  {group.form === "E 1kv"
                    ? "E 1kv filing fields"
                    : `${group.form} fields outside E 1kv`}
                </div>
              ) : null}
              {group.rows.map((row) => {
                const displayCopy = AUSTRIAN_TAX_FIELD_COPY[row.code];
                const displayLabel = displayCopy?.label ?? row.label;
                const displayNote = displayCopy?.note ?? row.note;
                const amount = row.amount;
                const isPending = amount === null;
                const isEmpty = isEmptyField(row);
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
            </Fragment>
          ))
        ) : (
          <div className="rounded-lg border bg-background/50 p-3 text-xs text-muted-foreground sm:col-span-2">
            No non-zero filing fields for this year.
          </div>
        )}
      </div>
    </div>
  );
}

function SummaryPdfPanel({
  year,
  wallets,
  selectedWalletIds,
  showWalletPicker,
  includeSnapshot,
  loading,
  disabled,
  success,
  onToggleSnapshot,
  onToggleWallet,
  onExport,
}: {
  year: number;
  wallets: WalletListData["wallets"];
  selectedWalletIds: string[];
  showWalletPicker: boolean;
  includeSnapshot: boolean;
  loading: boolean;
  disabled: boolean;
  success: boolean;
  onToggleSnapshot: (checked: boolean) => void;
  onToggleWallet: (id: string) => void;
  onExport: () => void;
}) {
  const selectedCount = showWalletPicker ? selectedWalletIds.length : wallets.length;
  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="flex items-start justify-between gap-3 p-3 pb-0">
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-md border"
          >
            <PieChart className="size-4 text-muted-foreground" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Summary snapshot
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              {year} · {selectedCount || "No"} wallet{selectedCount === 1 ? "" : "s"}
            </p>
          </div>
        </div>
        <Button
          type="button"
          size="sm"
          variant={success ? "default" : "outline"}
          className="size-8 shrink-0"
          disabled={disabled || (showWalletPicker && selectedWalletIds.length === 0)}
          aria-label="Export summary snapshot"
          onClick={onExport}
        >
          {loading ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : success ? (
            <CheckCircle2 className="size-4 animate-pulse" aria-hidden="true" />
          ) : (
            <Download className="size-4" aria-hidden="true" />
          )}
        </Button>
      </div>

      <div className="space-y-3 p-3">
        <div className="flex items-center justify-between gap-3 rounded-lg border bg-background/50 px-3 py-2">
          <div className="flex min-w-0 items-center gap-2">
            <CalendarDays className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="truncate text-xs text-muted-foreground">
              Include live cover snapshot
            </span>
          </div>
          <Switch checked={includeSnapshot} onCheckedChange={onToggleSnapshot} />
        </div>
        {showWalletPicker ? (
          <div className="rounded-lg border bg-background/50">
            <div className="flex items-center gap-2 border-b px-3 py-2 text-xs font-medium text-muted-foreground">
              <WalletCards className="size-4" aria-hidden="true" />
              Wallet scope
            </div>
            <div className="max-h-44 overflow-auto p-2">
              {wallets.map((wallet) => {
                const id = wallet.id ?? wallet.label;
                const checked = selectedWalletIds.includes(id);
                return (
                  <label
                    key={id}
                    className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted"
                  >
                    <Checkbox
                      checked={checked}
                      onCheckedChange={() => onToggleWallet(id)}
                    />
                    <span className="min-w-0 flex-1 truncate">{wallet.label}</span>
                    {wallet.chain ? (
                      <Badge variant="outline" className="rounded-md text-[10px]">
                        {wallet.chain}
                      </Badge>
                    ) : null}
                  </label>
                );
              })}
            </div>
          </div>
        ) : null}
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
  successfulExport,
  onExport,
  onOpenExport,
}: {
  year: number;
  activeExport: ReportExportFormatId | null;
  activeProfileIsAustrian: boolean;
  exportStatus: ReportExportStatus | null;
  openableExportPath: string | null;
  openingExportPath: string | null;
  successfulExport: { format: ReportExportFormatId; year: number } | null;
  onExport: (format: ReportExportFormatId) => void;
  onOpenExport: (path: string) => void;
}) {
  const [transactionLedgerExpanded, setTransactionLedgerExpanded] =
    useState(false);

  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="flex items-center justify-between gap-3 p-3 pb-0">
        <div className="flex items-center gap-2">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-md border"
          >
            <FolderOpen className="size-4 text-muted-foreground" />
          </span>
          <div>
            <h2 className="text-sm font-medium sm:text-base">
              Report package
            </h2>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              Filing exports
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-3 p-3">
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
            title="PDF filing report"
            detail={
              activeProfileIsAustrian
                ? `${year} · Austrian E 1kv`
                : "Complete report"
            }
            loading={activeExport === "pdf"}
            disabled={Boolean(activeExport)}
            success={
              successfulExport?.format === "pdf" &&
              successfulExport.year === year
            }
            onExport={onExport}
          />
          <ReportFileRow
            id="xlsx"
            icon={FileSpreadsheet}
            title="XLSX report workbook"
            detail={
              activeProfileIsAustrian
                ? `${year} · Multi-sheet Austrian workbook`
                : "Multi-sheet complete workbook"
            }
            loading={activeExport === "xlsx"}
            disabled={Boolean(activeExport)}
            success={
              successfulExport?.format === "xlsx" &&
              successfulExport.year === year
            }
            onExport={onExport}
          />
          <ReportFileRow
            id="csv"
            icon={FileArchive}
            title="CSV report bundle"
            detail={
              activeProfileIsAustrian
                ? `${year} · Austrian E 1kv CSV bundle`
                : "Complete report sections for spreadsheet review"
            }
            loading={activeExport === "csv"}
            disabled={Boolean(activeExport)}
            success={
              successfulExport?.format === "csv" &&
              successfulExport.year === year
            }
            onExport={onExport}
          />
        </div>
        <div className="overflow-hidden rounded-md border bg-background/50">
          <div className="flex items-center justify-between gap-3 px-3 py-2.5">
            <div className="min-w-0">
              <h3 className="truncate text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                Transaction ledger
              </h3>
              <p className="mt-0.5 truncate text-xs text-muted-foreground">
                Raw transactions with notes and references
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="size-8 shrink-0"
              aria-label={
                transactionLedgerExpanded
                  ? "Collapse transaction ledger exports"
                  : "Expand transaction ledger exports"
              }
              aria-expanded={transactionLedgerExpanded}
              onClick={() => setTransactionLedgerExpanded((value) => !value)}
            >
              <ChevronDown
                className={cn(
                  "size-4 text-muted-foreground transition-transform",
                  transactionLedgerExpanded && "rotate-180",
                )}
                aria-hidden="true"
              />
            </Button>
          </div>
          {transactionLedgerExpanded ? (
            <div className="divide-y border-t">
              <ReportFileRow
                id="transactions_xlsx"
                icon={FileSpreadsheet}
                title="Transactions XLSX"
                detail="Ledger rows for spreadsheet review"
                loading={activeExport === "transactions_xlsx"}
                disabled={Boolean(activeExport)}
                success={
                  successfulExport?.format === "transactions_xlsx" &&
                  successfulExport.year === year
                }
                onExport={onExport}
              />
              <ReportFileRow
                id="transactions_csv"
                icon={FileArchive}
                title="Transactions CSV"
                detail="Plain ledger export for external tools"
                loading={activeExport === "transactions_csv"}
                disabled={Boolean(activeExport)}
                success={
                  successfulExport?.format === "transactions_csv" &&
                  successfulExport.year === year
                }
                onExport={onExport}
              />
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function HandoffScopePanel({
  activeExport,
  successfulExport,
  auditScope,
  onAuditScopeChange,
  auditCaseId,
  onAuditCaseIdChange,
  sourceFundsCases,
  includeCopiedAttachments,
  onIncludeCopiedAttachmentsChange,
  includeUrlReferences,
  onIncludeUrlReferencesChange,
  includeJournalState,
  onIncludeJournalStateChange,
  includeReviewState,
  onIncludeReviewStateChange,
  includeEditHistory,
  onIncludeEditHistoryChange,
  onExport,
}: {
  activeExport: ReportExportFormatId | null;
  successfulExport: { format: ReportExportFormatId; year: number } | null;
  auditScope: AuditPackageScope;
  onAuditScopeChange: (scope: AuditPackageScope) => void;
  auditCaseId: string;
  onAuditCaseIdChange: (caseId: string) => void;
  sourceFundsCases: SourceFundsCaseRow[];
  includeCopiedAttachments: boolean;
  onIncludeCopiedAttachmentsChange: (value: boolean) => void;
  includeUrlReferences: boolean;
  onIncludeUrlReferencesChange: (value: boolean) => void;
  includeJournalState: boolean;
  onIncludeJournalStateChange: (value: boolean) => void;
  includeReviewState: boolean;
  onIncludeReviewStateChange: (value: boolean) => void;
  includeEditHistory: boolean;
  onIncludeEditHistoryChange: (value: boolean) => void;
  onExport: (format: ReportExportFormatId) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const auditExportDisabled =
    Boolean(activeExport) ||
    (auditScope === "source_funds_case" && !auditCaseId);

  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div className="flex items-center justify-between gap-3 p-3">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="flex size-7 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground"
            aria-hidden="true"
          >
            <ShieldCheck className="size-4 text-muted-foreground" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Auditor handoff
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              Audit exports and evidence boundaries
            </p>
          </div>
        </div>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className="size-8 shrink-0"
          aria-label={
            expanded ? "Collapse auditor handoff" : "Expand auditor handoff"
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
        <div className="space-y-3 p-3 pt-0">
          <div className="divide-y rounded-md border bg-background/50">
            {HANDOFF_EXPORT_MODES.map((mode) => (
              <HandoffModeRow
                key={mode.id}
                mode={mode}
                activeExport={activeExport}
                auditExportDisabled={auditExportDisabled}
                auditSuccess={successfulExport?.format === "audit_package"}
                onExport={onExport}
              />
            ))}
          </div>
          <div className="flex gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs leading-5 text-muted-foreground">
            <ShieldAlert className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <span>
              Normal handoffs never include {NORMAL_HANDOFF_EXCLUSIONS.join(", ")}.
            </span>
          </div>
          <AuditPackageControls
            scope={auditScope}
            onScopeChange={onAuditScopeChange}
            caseId={auditCaseId}
            onCaseIdChange={onAuditCaseIdChange}
            cases={sourceFundsCases}
            includeCopiedAttachments={includeCopiedAttachments}
            onIncludeCopiedAttachmentsChange={onIncludeCopiedAttachmentsChange}
            includeUrlReferences={includeUrlReferences}
            onIncludeUrlReferencesChange={onIncludeUrlReferencesChange}
            includeJournalState={includeJournalState}
            onIncludeJournalStateChange={onIncludeJournalStateChange}
            includeReviewState={includeReviewState}
            onIncludeReviewStateChange={onIncludeReviewStateChange}
            includeEditHistory={includeEditHistory}
            onIncludeEditHistoryChange={onIncludeEditHistoryChange}
          />
        </div>
      ) : null}
    </div>
  );
}

function HandoffModeRow({
  mode,
  activeExport,
  auditExportDisabled,
  auditSuccess,
  onExport,
}: {
  mode: HandoffExportMode;
  activeExport: ReportExportFormatId | null;
  auditExportDisabled: boolean;
  auditSuccess: boolean;
  onExport: (format: ReportExportFormatId) => void;
}) {
  const Icon =
    mode.id === "tax_advisor_report"
      ? FileCheck2
      : mode.id === "audit_package"
        ? PackageCheck
        : KeyRound;
  const badgeClass =
    mode.sensitivity === "External"
      ? "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
      : mode.sensitivity === "Trusted"
        ? "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300"
        : "border-destructive/35 bg-destructive/10 text-destructive";
  const availabilityLabel =
    mode.availability === "planned" ? "Planned" : "Separate approval";

  return (
    <div className="grid gap-2 px-3 py-2.5 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground ring-1 ring-inset ring-border">
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <div className="min-w-0 space-y-1">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <span className="min-w-0 truncate text-sm font-semibold">
            {mode.title}
          </span>
          <Badge variant="outline" className={cn("rounded-md", badgeClass)}>
            {mode.sensitivity}
          </Badge>
          {mode.availability !== "available" ? (
            <Badge variant="outline" className="rounded-md">
              {availabilityLabel}
            </Badge>
          ) : null}
        </div>
        <p className="truncate text-xs text-muted-foreground">
          {mode.summary}
        </p>
      </div>
      {mode.id === "audit_package" ? (
        <Button
          type="button"
          size="sm"
          variant={auditSuccess ? "default" : "outline"}
          className="size-8 shrink-0 justify-self-start sm:justify-self-end"
          disabled={auditExportDisabled}
          aria-label="Export audit package"
          onClick={() => onExport("audit_package")}
        >
          {activeExport === "audit_package" ? (
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          ) : auditSuccess ? (
            <CheckCircle2 className="size-4 animate-pulse" aria-hidden="true" />
          ) : (
            <Download className="size-4" aria-hidden="true" />
          )}
        </Button>
      ) : null}
    </div>
  );
}

function AuditPackageControls({
  scope,
  onScopeChange,
  caseId,
  onCaseIdChange,
  cases,
  includeCopiedAttachments,
  onIncludeCopiedAttachmentsChange,
  includeUrlReferences,
  onIncludeUrlReferencesChange,
  includeJournalState,
  onIncludeJournalStateChange,
  includeReviewState,
  onIncludeReviewStateChange,
  includeEditHistory,
  onIncludeEditHistoryChange,
}: {
  scope: AuditPackageScope;
  onScopeChange: (scope: AuditPackageScope) => void;
  caseId: string;
  onCaseIdChange: (caseId: string) => void;
  cases: SourceFundsCaseRow[];
  includeCopiedAttachments: boolean;
  onIncludeCopiedAttachmentsChange: (value: boolean) => void;
  includeUrlReferences: boolean;
  onIncludeUrlReferencesChange: (value: boolean) => void;
  includeJournalState: boolean;
  onIncludeJournalStateChange: (value: boolean) => void;
  includeReviewState: boolean;
  onIncludeReviewStateChange: (value: boolean) => void;
  includeEditHistory: boolean;
  onIncludeEditHistoryChange: (value: boolean) => void;
}) {
  return (
    <div className="rounded-lg border bg-background/50 p-3">
      <div className="mb-3 flex min-w-0 items-center gap-2">
        <PackageCheck className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        <div className="min-w-0">
          <h3 className="truncate text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Audit package options
          </h3>
        </div>
      </div>
      <div className="grid gap-3 md:grid-cols-[220px_minmax(0,1fr)]">
        <div className="space-y-1.5">
          <LabelText>Scope</LabelText>
          <Select value={scope} onValueChange={(value) => onScopeChange(value as AuditPackageScope)}>
            <SelectTrigger className="h-9">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="active_profile">Active book/profile</SelectItem>
              <SelectItem value="source_funds_case" disabled={!cases.length}>
                Saved source-funds case
              </SelectItem>
            </SelectContent>
          </Select>
          {scope === "source_funds_case" ? (
            <Select value={caseId} onValueChange={onCaseIdChange}>
              <SelectTrigger className="h-9">
                <SelectValue placeholder="Choose a saved case" />
              </SelectTrigger>
              <SelectContent>
                {cases.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    {item.label || item.target_external_id || item.id}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : null}
        </div>
        <div className="grid gap-2 sm:grid-cols-3">
          <AuditPackageCheckbox
            label="Copied files"
            checked={includeCopiedAttachments}
            onCheckedChange={onIncludeCopiedAttachmentsChange}
          />
          <AuditPackageCheckbox
            label="URL references"
            checked={includeUrlReferences}
            onCheckedChange={onIncludeUrlReferencesChange}
          />
          <AuditPackageCheckbox
            label="Journal state"
            checked={includeJournalState}
            onCheckedChange={onIncludeJournalStateChange}
          />
          <AuditPackageCheckbox
            label="Review state"
            checked={includeReviewState}
            onCheckedChange={onIncludeReviewStateChange}
          />
          <AuditPackageCheckbox
            label="Edit history"
            checked={includeEditHistory}
            onCheckedChange={onIncludeEditHistoryChange}
          />
        </div>
      </div>
    </div>
  );
}

function LabelText({ children }: { children: ReactNode }) {
  return (
    <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
      {children}
    </div>
  );
}

function AuditPackageCheckbox({
  label,
  checked,
  onCheckedChange,
}: {
  label: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex min-h-9 items-center gap-2 rounded-md border bg-card px-2 text-xs">
      <Checkbox
        checked={checked}
        onCheckedChange={(value) => onCheckedChange(value === true)}
      />
      <span>{label}</span>
    </label>
  );
}

function ExportNotice({
  exportStatus,
  openableExportPath,
  openingExportPath,
  onOpenExport,
}: {
  exportStatus: ReportExportStatus;
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
  success,
  onExport,
}: {
  id: ReportExportFormatId;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  title: string;
  detail: string;
  loading: boolean;
  disabled: boolean;
  success: boolean;
  onExport: (format: ReportExportFormatId) => void;
}) {
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground ring-1 ring-inset ring-border">
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
        variant={id === "pdf" || success ? "default" : "outline"}
        className="size-8 shrink-0"
        disabled={disabled}
        aria-label={`Export ${title}`}
        onClick={() => onExport(id)}
      >
        {loading ? (
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        ) : success ? (
          <CheckCircle2 className="size-4 animate-pulse" aria-hidden="true" />
        ) : (
          <Download className="size-4" aria-hidden="true" />
        )}
      </Button>
    </div>
  );
}

function ReportPolicyDetails({
  jurisdiction,
  methodName,
}: {
  jurisdiction: (typeof JURISDICTIONS)[string];
  methodName: string;
}) {
  return (
    <div className="mt-3 space-y-3">
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
              ? "Pre-1 Mar 2021 holdings only"
              : jurisdiction.longTermDays
              ? `${jurisdiction.longTermDays} days`
              : "Not applied"
          }
        />
        <ReportFactRow
          label="Cost-basis method"
          value={jurisdiction.methodNote ?? methodName}
        />
      </div>

      {!jurisdiction.methodLocked ? (
        <p className="rounded-lg border bg-background/50 px-3 py-2.5 text-xs text-muted-foreground">
          Lots on this page were computed with the book's configured cost-basis
          method ({methodName}). To change it, update the book's gains
          algorithm in profile settings, then reprocess journals. Switching it
          here would silently disagree with exported reports.
        </p>
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
  method,
  jurisdiction,
  hideSensitive,
  formatNumber,
  year,
}: {
  lots: DisposedLot[];
  totals: ReportTotals;
  method: CostBasisMethod;
  jurisdiction: (typeof JURISDICTIONS)[string];
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
  year: number;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div
        className={cn(
          "flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:justify-between",
          expanded && "border-b",
        )}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-md border"
          >
            <Sigma className="size-4 text-muted-foreground" />
          </span>
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
                  method={method}
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

function NeutralSwapAuditPanel({
  swaps,
  jurisdiction,
  hideSensitive,
  formatNumber,
}: {
  swaps: NeutralSwapLot[];
  jurisdiction: (typeof JURISDICTIONS)[string];
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
}) {
  const [expanded, setExpanded] = useState(false);
  const totalFeeSats = swaps.reduce((sum, swap) => sum + swap.feeSats, 0);

  return (
    <div className="min-w-0 overflow-hidden rounded-lg border bg-card">
      <div
        className={cn(
          "flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:justify-between",
          expanded && "border-b",
        )}
      >
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 items-center justify-center rounded-md border"
          >
            <RefreshCw className="size-4 text-muted-foreground" />
          </span>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-medium sm:text-base">
              Tax-neutral swap audit
            </h2>
            <p className="truncate text-[10px] text-muted-foreground sm:text-xs">
              Reviewed carrying-value movement and fee delta
            </p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="outline" className="rounded-md">
            {swaps.length} row{swaps.length === 1 ? "" : "s"}
          </Badge>
          <Badge variant="secondary" className="rounded-md">
            {totalFeeSats.toLocaleString("en-US")} sats fee
          </Badge>
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="size-8"
            aria-label={
              expanded
                ? "Collapse tax-neutral swap audit"
                : "Expand tax-neutral swap audit"
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

      {expanded ? (
        <div className="overflow-x-auto">
          <Table className="min-w-[880px]">
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead className="min-w-[112px]">Date</TableHead>
                <TableHead className="min-w-[180px]">From</TableHead>
                <TableHead className="text-right">Out sats</TableHead>
                <TableHead className="min-w-[180px]">To</TableHead>
                <TableHead className="text-right">In sats</TableHead>
                <TableHead className="text-right">Fee sats</TableHead>
                <TableHead className="text-right">
                  Carry basis {jurisdiction.ccy}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {swaps.map((swap, index) => (
                <TableRow key={`${swap.date}-${swap.pairId ?? index}`}>
                  <TableCell>{swap.date}</TableCell>
                  <TableCell>
                    <div className="min-w-0">
                      <p className="truncate font-medium">{swap.outWallet}</p>
                      <p className="text-xs text-muted-foreground">
                        {swap.outAsset}
                      </p>
                    </div>
                  </TableCell>
                  <TableCell
                    className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                  >
                    {swap.outSats.toLocaleString("en-US")}
                  </TableCell>
                  <TableCell>
                    <div className="min-w-0">
                      <p className="truncate font-medium">{swap.inWallet}</p>
                      <p className="text-xs text-muted-foreground">
                        {swap.inAsset}
                      </p>
                    </div>
                  </TableCell>
                  <TableCell
                    className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                  >
                    {swap.inSats.toLocaleString("en-US")}
                  </TableCell>
                  <TableCell
                    className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                  >
                    {swap.feeSats.toLocaleString("en-US")}
                  </TableCell>
                  <TableCell
                    className={cn("text-right tabular-nums", blurClass(hideSensitive))}
                  >
                    {formatNumber(swap.costEur)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : null}
    </div>
  );
}

interface ReportLotRowProps {
  lot: DisposedLot;
  method: CostBasisMethod;
  hideSensitive: boolean;
  formatNumber: (value: number) => string;
}

function ReportLotRow({
  lot,
  method,
  hideSensitive,
  formatNumber,
}: ReportLotRowProps) {
  const gain = lot.proceedsEur - lot.costEur;
  const isLong = lot.type === "LT";
  const acquiredLabel = lot.acquired || pooledAcquisitionLabel(method);

  return (
    <TableRow>
      <TableCell className="font-mono text-xs text-muted-foreground">
        {acquiredLabel}
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

function pooledAcquisitionLabel(method: CostBasisMethod) {
  return method === "moving_average" || method === "moving_average_at"
    ? "Pooled"
    : "Unknown";
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
      title: "Process ledger",
      detail: "Report totals need fresh journal processing before export.",
      tone: "warning",
      icon: RefreshCw,
      action: { label: "Open ledger", href: "/journals" },
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
    detail: "The ledger is current and the review queue is clear.",
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
