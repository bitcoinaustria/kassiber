/**
 * Reports — the tax year's result and the files that carry it.
 *
 * The page reads like a filing: pick the year, see whether the journal state
 * can be trusted, read the result and how it is built, and write the files.
 * Rows stay on Transactions, where the breakdown links: this page repeats no
 * ledger. Every figure comes from the daemon's capital-gains read, including
 * the indicative tax, so this page computes no tax of its own.
 */

import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Loader2,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { LightningProfitabilityPanel } from "@/components/lightning/LightningProfitabilityPanel";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { ExplanationBody } from "@/components/reports/CapitalGainExplanation";
import { InfoNote } from "@/components/reports/InfoNote";
import { ReportBreakdown } from "@/components/reports/ReportBreakdown";
import { ReportFilesPanel } from "@/components/reports/ReportFilesPanel";
import { ReportRules } from "@/components/reports/ReportRules";
import {
  formatReportPeriod,
  formatTaxRate,
  E1KV_FIRST_TAX_YEAR,
  reportBreakdown,
  reportE1kvApplies,
  reportRegion,
  reportTaxEstimate,
  reportTotals,
  reportUnassignedRows,
  signedFiat,
  type AuditPackageOptions,
  type ReportExportFormatId,
  type ReportTaxEstimate,
  type ReportTotals,
} from "@/components/reports/reportModel";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  canOpenExportedFiles,
  canSaveExportedFiles,
  openExportedFile,
  saveExportedFileAs,
} from "@/daemon/transport";
import { formatFiatAmount } from "@/lib/currency";
import { exportBasename } from "@/lib/exportFile";
import { normalizeFiatCompleteness } from "@/lib/fiatCompleteness";
import { formatCount } from "@/lib/localeFormat";
import { quarantineExportBlock } from "@/lib/quarantineExportBlock";
import {
  reportExportStatusForYear,
  type ReportExportStatus,
} from "@/lib/reportExportStatus";
import { reportYearFromSearch } from "@/lib/reportYear";
import {
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenPanelClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type { FiatCompleteness, OverviewSnapshot } from "@/mocks/seed";
import type { CapitalGainsReport } from "@/mocks/reports";
import { useUiStore } from "@/store/ui";

type ReportTone = "good" | "warning" | "alert" | "neutral";
type ReportHref = "/journals" | "/review";

interface ReportExportResult {
  reconciled?: boolean;
  warnings?: string[];
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

const DEFAULT_AUDIT_OPTIONS: AuditPackageOptions = {
  scope: "active_profile",
  caseId: "",
  include: {
    copiedAttachments: true,
    urlReferences: true,
    journalState: true,
    reviewState: true,
    editHistory: false,
  },
};

function basename(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function initialReportYearFromUrl() {
  if (typeof window === "undefined") return null;
  return reportYearFromSearch(window.location.search);
}

export function Reports() {
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const reference = new URLSearchParams(typeof window === "undefined" ? "" : window.location.search).get("result");
  if (reference) {
    let parsed;
    try { parsed = JSON.parse(reference); } catch { parsed = null; }
    return <div className={screenPanelClassName}><ExplanationBody reference={parsed} hideSensitive={hideSensitive} /></div>;
  }
  return <ReportOverview />;
}

function ReportOverview() {
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
  // Local read (no egress): carries custody-gap and missing-price signals the
  // capital-gains payload does not, so the page can say "provisional".
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const overviewFiat = overview.data?.data?.fiat;
  const completeness = overviewFiat
    ? normalizeFiatCompleteness(overviewFiat.completeness)
    : null;
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
        <div className="kb-surface p-(--kb-card-padding)">
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
        <div className="kb-surface p-(--kb-card-padding)">
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
      completeness={completeness}
      completenessPending={overview.isLoading}
    />
  );
}

interface ReportsViewProps {
  report: CapitalGainsReport;
  hideSensitive: boolean;
  selectedYear: number | null;
  onYearChange: (year: number) => void;
  wallets: WalletListData["wallets"];
  /** Book completeness from ui.overview.snapshot; `null` while not loaded. */
  completeness: FiatCompleteness | null;
  /** The completeness read is still loading, so it is neither known nor unknown. */
  completenessPending?: boolean;
}

function ReportsView({
  report,
  hideSensitive,
  selectedYear,
  onYearChange,
  wallets,
  completeness,
  completenessPending = false,
}: ReportsViewProps) {
  const navigate = useNavigate();
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
  const region = reportRegion(report.jurisdictionCode);
  const fiatCurrency = (report.fiatCurrency ?? "EUR").toUpperCase();
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
  const [audit, setAudit] = useState<AuditPackageOptions>(DEFAULT_AUDIT_OPTIONS);
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
  const exportYearHandoff =
    useDaemonMutation<ReportExportResult>("ui.reports.export_year_handoff");
  const activeProfileIsAustrian = region === "AT";
  const sourceFundsCasesQuery = useDaemon<SourceFundsCasesData>(
    "ui.source_funds.cases.list",
  );
  const sourceFundsCases = useMemo(
    () => sourceFundsCasesQuery.data?.data?.cases ?? [],
    [sourceFundsCasesQuery.data?.data?.cases],
  );
  const walletChoices = useMemo(
    () =>
      wallets
        .filter((wallet) => wallet.id || wallet.label)
        .map((wallet) => ({
          id: wallet.id ?? wallet.label,
          label: wallet.label,
          chain: wallet.chain,
        })),
    [wallets],
  );
  const showWalletPicker = walletChoices.length > 1;
  useEffect(() => {
    if (audit.scope !== "source_funds_case" || audit.caseId) return;
    const firstCase = sourceFundsCases[0];
    if (firstCase?.id) {
      setAudit((current) => ({ ...current, caseId: firstCase.id }));
    }
  }, [audit.caseId, audit.scope, sourceFundsCases]);
  useEffect(() => {
    setSummaryWalletIds((current) => {
      const allIds = walletChoices.map((wallet) => wallet.id);
      if (allIds.length <= 1) return allIds;
      const currentSet = new Set(current);
      const kept = allIds.filter((id) => currentSet.has(id));
      return kept.length ? kept : allIds;
    });
  }, [walletChoices]);

  const totals = reportTotals(report);
  const taxEstimate = reportTaxEstimate(report);
  const breakdown = reportBreakdown(report);
  // While another year loads, the report still describes the previous one.
  const e1kv =
    report.year === effectiveYear
      ? reportE1kvApplies(report)
      : effectiveYear >= E1KV_FIRST_TAX_YEAR;
  const readiness = buildReportReadiness(
    report,
    totals.rowCount,
    effectiveYear,
    completeness,
    completenessPending,
  );
  const provisional = reportFiguresProvisional(
    report,
    completeness,
    completenessPending,
  );
  // The notice names what blocks the figures, so the header does not repeat it.
  const showReadinessNotice =
    readiness.tone === "warning" || readiness.tone === "alert";
  const currentExportStatus = reportExportStatusForYear(
    exportStatus,
    effectiveYear,
  );
  const canOpenCurrentExport =
    currentExportStatus?.tone !== "error" &&
    canOpenExportPath(currentExportStatus?.openPath) &&
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
        : format === "year_handoff"
          ? exportYearHandoff
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
              include_copied_attachments: audit.include.copiedAttachments,
              include_url_references: audit.include.urlReferences,
              include_journal_state: audit.include.journalState,
              include_review_state: audit.include.reviewState,
              include_edit_history: audit.include.editHistory,
              ...(audit.scope === "source_funds_case" && audit.caseId
                ? { source_funds_case: audit.caseId }
                : {}),
            }
        : format === "year_handoff"
          ? { year: exportYear }
        : format === "pdf"
        ? activeProfileIsAustrian
          ? { year: exportYear }
          : {}
        : (format === "xlsx" || format === "csv") && activeProfileIsAustrian
          ? { year: exportYear }
          : {};

    // The promise settles even when this view has unmounted meanwhile (the
    // user moved on to another page), unlike per-call mutate callbacks, so the
    // save dialog and the notification still follow the export.
    void mutation.mutateAsync(args).then(
      async (envelope) => {
        const payload = envelope.data;
        const exportPath = payload?.file ?? payload?.dir ?? "";
        const filename = (payload?.filename ?? basename(exportPath)) || "report";
        const detail =
          payload?.scope === "audit_package"
            ? `${payload.transaction_count ?? 0} transaction${payload.transaction_count === 1 ? "" : "s"} · ${payload.evidence_file_count ?? 0} file${payload.evidence_file_count === 1 ? "" : "s"} · ${payload.url_reference_count ?? 0} link${payload.url_reference_count === 1 ? "" : "s"}`
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
            savedPath = (await saveExportedFileAs(exportPath)) ?? exportPath;
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
        // The handoff reports reconciliation differences and fallback
        // year-end rates instead of failing; say so next to the saved file,
        // not only in a notification that goes away.
        const warnings = payload?.warnings ?? [];
        const flagged = payload?.reconciled === false || warnings.length > 0;
        if (savedPath === exportPath || flagged) {
          setExportStatus({
            year: exportYear,
            tone: flagged ? "warning" : "success",
            message: flagged ? `${filename} saved with notes.` : `${filename} saved.`,
            ...(flagged && warnings.length ? { notes: warnings } : {}),
            path: savedPath,
            ...(savedPath === exportPath ? { openPath: exportPath } : {}),
          });
        }
        setSuccessfulExport({ format, year: exportYear });
        addNotification({
          title: flagged ? "Report exported with notes" : "Report export finished",
          body: [
            savedPath === exportPath
              ? detail
              : `${detail} · saved to ${exportBasename(savedPath)}`,
            ...warnings,
          ].join("\n"),
          tone: flagged ? "warning" : "success",
        });
      },
      (error: unknown) => {
        const message =
          error instanceof Error ? error.message : "Report export failed";
        // A quarantine block names the transactions in the way; the files
        // panel lists them with a link to review them.
        const quarantine = quarantineExportBlock(error);
        setExportStatus({
          year: exportYear,
          tone: "error",
          message,
          ...(quarantine ? { quarantine } : {}),
        });
        addNotification({
          title: quarantine
            ? "Report export blocked by quarantine"
            : "Report export failed",
          body: message,
          tone: "error",
        });
      },
    ).finally(() => setActiveExport(null));
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
    <div className={cn(screenShellClassName, "@container/reports")}>
      <ReportHeader
        selectedYear={effectiveYear}
        availableYears={availableYears}
        onYearChange={onYearChange}
        // A new year reloads the report and unmounts this view, so a running
        // export would lose its save dialog and notification.
        yearLocked={activeExport !== null}
        readiness={showReadinessNotice ? null : readiness}
      />

      <ReportMetricStrip
        hideSensitive={hideSensitive}
        fiatCurrency={fiatCurrency}
        totals={totals}
        taxEstimate={taxEstimate}
        taxRate={report.indicativeTax?.rate}
        provisional={provisional}
      />

      {showReadinessNotice ? <ReadinessNotice readiness={readiness} /> : null}

      <div className="grid items-start gap-(--kb-page-gap) @5xl/reports:grid-cols-[minmax(0,1fr)_22rem]">
        <div className="flex min-w-0 flex-col gap-(--kb-page-gap)">
          <ReportBreakdown
            region={region}
            groups={breakdown}
            year={effectiveYear}
            fiatCurrency={fiatCurrency}
            hideSensitive={hideSensitive}
          />
          <ReportRules
            region={region}
            method={report.method}
            longTermDays={report.longTermDays ?? null}
            taxRate={report.indicativeTax?.rate}
            e1kv={e1kv}
          />
        </div>
        <div className="min-w-0">
          <ReportFilesPanel
            year={effectiveYear}
            region={region}
            e1kv={e1kv}
            rowsBeforeFields={
              report.year === effectiveYear ? (report.rowsBeforeCryptoKennzahlen ?? 0) : 0
            }
            activeExport={activeExport}
            successfulExport={
              successfulExport?.year === effectiveYear
                ? successfulExport.format
                : null
            }
            exportStatus={currentExportStatus}
            openableExportPath={openableExportPath}
            openingExportPath={openingExportPath}
            onExport={handleExport}
            onOpenExport={handleOpenExport}
            onReviewQuarantine={() => void navigate({ to: "/review" })}
            wallets={walletChoices}
            selectedWalletIds={summaryWalletIds}
            onToggleWallet={(id) => {
              setSummaryWalletIds((current) => {
                const next = current.includes(id)
                  ? current.filter((item) => item !== id)
                  : [...current, id];
                return next.length ? next : current;
              });
            }}
            onSelectAllWallets={() =>
              setSummaryWalletIds(walletChoices.map((wallet) => wallet.id))
            }
            includeCover={summarySnapshot}
            onIncludeCoverChange={setSummarySnapshot}
            audit={audit}
            onAuditChange={setAudit}
            sourceFundsCases={sourceFundsCases.map((item) => ({
              id: item.id,
              label: item.label || item.target_external_id || item.id,
            }))}
          />
        </div>
      </div>

      <LightningProfitabilityPanel />
    </div>
  );
}


function ReportHeader({
  selectedYear,
  availableYears,
  onYearChange,
  yearLocked = false,
  readiness,
}: {
  selectedYear: number;
  availableYears: number[];
  onYearChange: (year: number) => void;
  yearLocked?: boolean;
  /** `null` while the readiness notice below states it. */
  readiness: ReportReadiness | null;
}) {
  const ReadinessIcon = readiness?.icon;
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
    <div className={pageHeaderClassName}>
      <div className="flex min-w-0 flex-wrap items-center gap-2 sm:gap-3">
        <span className="text-sm text-muted-foreground">Tax year</span>
        <Select value={String(selectedYear)} onValueChange={handleYearChange} disabled={yearLocked}>
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
        <span className="text-sm text-muted-foreground tabular-nums">
          {formatReportPeriod(selectedYear)}
        </span>
      </div>
      {readiness && ReadinessIcon ? (
        <div className={pageHeaderActionsClassName}>
          <span
            className={cn(
              "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
              readinessToneStyles[readiness.tone],
            )}
            title={readiness.detail}
          >
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readiness.title}
          </span>
        </div>
      ) : null}
    </div>
  );
}

function ReadinessNotice({ readiness }: { readiness: ReportReadiness }) {
  const Icon = readiness.icon;
  return (
    <div
      role="status"
      className={cn(
        "flex flex-col gap-3 rounded-lg border px-4 py-2.5 text-sm sm:flex-row sm:items-center",
        readiness.tone === "alert"
          ? "border-red-300 bg-red-50 text-red-900 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-200"
          : "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200",
      )}
    >
      <div className="flex flex-1 items-center gap-3">
        <Icon className="size-4 shrink-0" aria-hidden="true" />
        <p className="m-0">{readiness.detail}</p>
      </div>
      {readiness.action ? (
        <Button asChild size="sm" variant="outline" className="h-7 shrink-0">
          <Link to={readiness.action.href}>{readiness.action.label}</Link>
        </Button>
      ) : null}
    </div>
  );
}

function ReportMetricStrip({
  hideSensitive,
  fiatCurrency,
  totals,
  taxEstimate,
  taxRate,
  provisional = false,
}: {
  hideSensitive: boolean;
  fiatCurrency: string;
  totals: ReportTotals;
  /** The core's indicative tax; `null` for books it does not cover. */
  taxEstimate: ReportTaxEstimate | null;
  taxRate?: string;
  /** Journals stale, quarantine, custody gap, or missing prices. */
  provisional?: boolean;
}) {
  const sensitive = hideSensitive ? "sensitive" : "";
  const money = (value: number) => formatFiatAmount(value, fiatCurrency);
  // The gain sums every row; say which parts are income or stay off the return.
  const gainParts = [
    totals.income > 0
      ? `${hideSensitive ? "—" : money(totals.income)} income`
      : null,
    Math.abs(totals.taxFreeGain) >= 0.005
      ? `${hideSensitive ? "—" : signedFiat(totals.taxFreeGain, fiatCurrency)} tax-free (Altbestand)`
      : null,
  ].filter((part): part is string => part !== null);
  const metrics: Array<{
    label: string;
    value: ReactNode;
    sub?: string;
    /** Caveats shown on hover beside the label. */
    notes?: string[];
    /** Depends on the journal basis and must carry the provisional badge. */
    basisDependent?: boolean;
  }> = [
    {
      label: "Proceeds",
      value: <span className={sensitive}>{money(totals.proceeds)}</span>,
    },
    {
      label: "Cost basis",
      value: <span className={sensitive}>{money(totals.cost)}</span>,
      basisDependent: true,
    },
    {
      label: "Gain / loss",
      value: (
        <span
          className={cn(
            totals.gain > 0
              ? "text-emerald-600 dark:text-emerald-400"
              : totals.gain < 0
                ? "text-red-600 dark:text-red-400"
                : "",
            sensitive,
          )}
        >
          {signedFiat(totals.gain, fiatCurrency)}
        </span>
      ),
      sub: gainParts.length ? `Includes ${gainParts.join(" · ")}` : undefined,
      basisDependent: true,
    },
    ...(taxEstimate
      ? [
          {
            label: `Indicative tax · ${formatTaxRate(taxRate)}`,
            value: (
              <span className={sensitive}>
                {taxEstimate.amount === null ? "—" : money(taxEstimate.amount)}
              </span>
            ),
            sub:
              Math.abs(taxEstimate.progressiveAmount) >= 0.005
                ? "KZ 172 + 174 − 176, without KZ 801"
                : "KZ 172 + 174 − 176",
            notes: taxEstimate.notes,
            basisDependent: true,
          },
        ]
      : []),
  ];
  return (
    <div className="kb-surface min-w-0 overflow-hidden">
      {/* The page's width, not the window's, sets the columns; the 1px gaps
          over the border colour draw the dividers in every layout. */}
      <div
        className={cn(
          "grid grid-cols-1 gap-px bg-border/70",
          metrics.length === 4
            ? "@xl/reports:grid-cols-2 @5xl/reports:grid-cols-4"
            : "@2xl/reports:grid-cols-3",
        )}
      >
        {metrics.map((metric) => (
          <div
            key={metric.label}
            className="group relative isolate overflow-hidden bg-card p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100"
          >
            <div className="pointer-events-none relative z-20 space-y-1.5">
            <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              {metric.label}
              {metric.notes?.length ? (
                <InfoNote
                  label={`About ${metric.label}`}
                  notes={metric.notes}
                  className="pointer-events-auto"
                />
              ) : null}
              {provisional && metric.basisDependent ? (
                <span
                  className="rounded-full border border-amber-500/40 px-1.5 text-2xs font-medium text-amber-700 dark:text-amber-300"
                  title="Quarantined rows, custody gaps, missing prices, or stale journals are not reflected yet."
                >
                  Provisional
                </span>
              ) : null}
            </p>
            <p className="min-w-0 text-lg leading-tight font-semibold tracking-tight tabular-nums sm:text-xl">
              {metric.value}
            </p>
            {metric.sub ? (
              <p className="text-2xs text-muted-foreground sm:text-xs">
                {metric.sub}
              </p>
            ) : null}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function reportFiguresProvisional(
  report: CapitalGainsReport,
  completeness: FiatCompleteness | null,
  completenessPending = false,
) {
  return (
    Boolean(report.status?.needsJournals) ||
    (report.status?.quarantines ?? 0) > 0 ||
    // Rows in no field may still belong on the return.
    reportUnassignedRows(report) > 0 ||
    // Unknown completeness (older daemon, failed read) is never final; a
    // read still loading is not unknown yet.
    (!completenessPending && !completeness?.costBasisComplete)
  );
}

function buildReportReadiness(
  report: CapitalGainsReport,
  rowCount: number,
  year: number,
  completeness: FiatCompleteness | null = null,
  completenessPending = false,
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
      detail: `${formatCount(quarantines)} quarantined item${
        quarantines === 1 ? "" : "s"
      } should be resolved before filing.`,
      tone: "alert",
      icon: ShieldAlert,
      action: { label: "Review", href: "/review" },
    };
  }

  if (completenessPending) {
    return {
      title: "Checking completeness",
      detail: "Kassiber is still reading whether the cost basis is complete.",
      tone: "neutral",
      icon: Loader2,
    };
  }

  if (completeness?.reasons.includes("custody_unresolved")) {
    return {
      title: "Custody gaps open",
      detail: "Unresolved custody gaps block final cost basis; figures are provisional.",
      tone: "alert",
      icon: ShieldAlert,
      action: { label: "Open ledger", href: "/journals" },
    };
  }

  if (completeness?.reasons.includes("missing_prices")) {
    const count = completeness.missingPriceCount;
    return {
      title: "Prices missing",
      detail: `${formatCount(count)} transaction${count === 1 ? " has" : "s have"} no price; figures are provisional.`,
      tone: "warning",
      icon: AlertTriangle,
      action: { label: "Review", href: "/review" },
    };
  }

  if (!completeness?.costBasisComplete) {
    return {
      title: "Completeness unknown",
      detail: "Kassiber could not confirm that the cost basis is complete; figures are provisional.",
      tone: "warning",
      icon: AlertTriangle,
      action: { label: "Open ledger", href: "/journals" },
    };
  }

  const austrian = reportRegion(report.jurisdictionCode) === "AT";
  const unassigned = reportUnassignedRows(report);
  if (austrian && !reportE1kvApplies(report)) {
    return {
      title: `No crypto fields before ${E1KV_FIRST_TAX_YEAR}`,
      // The notice shows only the detail, so it names the rule itself.
      detail: `Austria's crypto Kennzahlen (KZ 172, 174, 176) start on 1 March ${E1KV_FIRST_TAX_YEAR}, so ${year} states only KZ 801 and no 27.5 % estimate.${
        unassigned > 0
          ? ` ${formatCount(unassigned)} row${unassigned === 1 ? " has" : "s have"} no field that year; have a Steuerberater state ${unassigned === 1 ? "it" : "them"}.`
          : ""
      }`,
      tone: "warning",
      icon: AlertTriangle,
    };
  }

  const beforeFields = report.rowsBeforeCryptoKennzahlen ?? 0;
  if (austrian && beforeFields > 0) {
    return {
      title: "Rows before 1 March 2022",
      detail: `${formatCount(beforeFields)} row${beforeFields === 1 ? "" : "s"} from January or February ${year} followed the old rules unless you opted for the new ones. ${beforeFields === 1 ? "It has" : "They have"} no field, so there is no E 1kv or estimate for ${year}; have a Steuerberater state ${beforeFields === 1 ? "it" : "them"}.`,
      tone: "warning",
      icon: AlertTriangle,
    };
  }

  if (unassigned > 0) {
    return {
      title: "Rows without a field",
      detail: `${formatCount(unassigned)} row${unassigned === 1 ? " is" : "s are"} in no field of the return; check ${unassigned === 1 ? "it" : "them"} before filing.`,
      tone: "warning",
      icon: AlertTriangle,
    };
  }

  if (!rowCount && !hasFilingRows) {
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
