/**
 * Everything the Reports screen can write, one row per file with a button
 * per format. The tax report and the bookkeeping handoff are what a filer or
 * bookkeeper receives; the summary PDF and the audit package follow, each
 * with options folded under its row. The ledger's own exports live on
 * Transactions. The outcome of the last export (saved, saved with notes,
 * blocked, failed) sits at the top so it stays beside the files after its
 * notification goes away.
 */

import { useState, type ReactNode } from "react";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronDown,
  Download,
  ExternalLink,
  Loader2,
  ShieldCheck,
} from "lucide-react";

import { QuarantineExportBlockNotice } from "@/components/kb/quarantine/QuarantineExportBlockNotice";
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
import { NORMAL_HANDOFF_EXCLUSIONS } from "@/lib/handoffExports";
import { formatCount } from "@/lib/localeFormat";
import type { ReportExportStatus } from "@/lib/reportExportStatus";
import { cn } from "@/lib/utils";

import type {
  AuditPackageOptions,
  AuditPackageScope,
  ReportExportFormatId,
  ReportRegion,
} from "./reportModel";

export interface ReportWalletChoice {
  id: string;
  label: string;
  chain?: string;
}

export interface SourceFundsCaseChoice {
  id: string;
  label: string;
}

const AUDIT_PARTS: Array<{ key: keyof AuditPackageOptions["include"]; label: string }> = [
  { key: "copiedAttachments", label: "Copied files" },
  { key: "urlReferences", label: "URL references" },
  { key: "journalState", label: "Journal state" },
  { key: "reviewState", label: "Review state" },
  { key: "editHistory", label: "Edit history" },
];

function listWithOr(items: readonly string[]) {
  if (items.length < 2) return items.join("");
  return `${items.slice(0, -1).join(", ")}, or ${items[items.length - 1]}`;
}

export function ReportFilesPanel({
  year,
  region,
  e1kv = true,
  rowsBeforeFields = 0,
  activeExport,
  successfulExport,
  exportStatus,
  openableExportPath,
  openingExportPath,
  onExport,
  onOpenExport,
  onReviewQuarantine,
  wallets,
  selectedWalletIds,
  onToggleWallet,
  onSelectAllWallets,
  includeCover,
  onIncludeCoverChange,
  audit,
  onAuditChange,
  sourceFundsCases,
}: {
  year: number;
  region: ReportRegion;
  /** `false` for an Austrian year before 2022, which has no crypto Kennzahlen. */
  e1kv?: boolean;
  /** Rows dated before 1 March 2022 in a 2022 book; the core refuses its E 1kv. */
  rowsBeforeFields?: number;
  activeExport: ReportExportFormatId | null;
  successfulExport: ReportExportFormatId | null;
  exportStatus: ReportExportStatus | null;
  openableExportPath: string | null;
  openingExportPath: string | null;
  onExport: (format: ReportExportFormatId) => void;
  onOpenExport: (path: string) => void;
  onReviewQuarantine: () => void;
  /** Wallets the summary can cover; the picker shows for two or more. */
  wallets: ReportWalletChoice[];
  selectedWalletIds: string[];
  onToggleWallet: (id: string) => void;
  onSelectAllWallets: () => void;
  includeCover: boolean;
  onIncludeCoverChange: (value: boolean) => void;
  audit: AuditPackageOptions;
  onAuditChange: (next: AuditPackageOptions) => void;
  sourceFundsCases: SourceFundsCaseChoice[];
}) {
  const [openOptions, setOpenOptions] = useState<"summary" | "audit" | null>(
    null,
  );
  const busy = activeExport !== null;
  const e1kvRefused = region === "AT" && (!e1kv || rowsBeforeFields > 0);
  const showWalletPicker = wallets.length > 1;
  const selectedWallets = showWalletPicker
    ? selectedWalletIds.length
    : wallets.length;
  const includedParts = AUDIT_PARTS.filter((part) => audit.include[part.key]);
  const auditCaseMissing = audit.scope === "source_funds_case" && !audit.caseId;

  const format = (id: ReportExportFormatId, label: string, title: string, disabled = false) => (
    <FormatButton
      key={id}
      label={label}
      title={title}
      loading={activeExport === id}
      success={successfulExport === id}
      disabled={busy || disabled}
      onClick={() => onExport(id)}
    />
  );

  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="report-files-title">
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5 sm:px-4">
        <h2 id="report-files-title" className="m-0 text-sm font-medium">
          Files
        </h2>
        <span
          className="inline-flex items-center gap-1 text-xs text-muted-foreground"
          title={`No file includes ${listWithOr(NORMAL_HANDOFF_EXCLUSIONS)}.`}
        >
          <ShieldCheck className="size-3.5" aria-hidden="true" />
          No keys or credentials
        </span>
      </div>

      {exportStatus?.quarantine ? (
        <div className="border-b p-3">
          <QuarantineExportBlockNotice
            block={exportStatus.quarantine}
            onReview={onReviewQuarantine}
          />
        </div>
      ) : exportStatus ? (
        <ExportStatus
          status={exportStatus}
          openableExportPath={openableExportPath}
          openingExportPath={openingExportPath}
          onOpenExport={onOpenExport}
        />
      ) : null}

      <ul className="m-0 list-none divide-y p-0">
        <FileRow
          title="Tax report"
          detail={
            region !== "AT"
              ? "Every year in the book"
              : !e1kv
                ? "No crypto Kennzahlen before 2022 · the bookkeeping handoff lists every row"
                : rowsBeforeFields > 0
                  ? `No E 1kv ${year}: rows before 1 March · the bookkeeping handoff lists every row`
                  : `E 1kv ${year} with every row`
          }
          formats={[
            // The core refuses an E 1kv for a year without crypto Kennzahlen
            // or with rows dated before them.
            format("pdf", "PDF", "Tax report", e1kvRefused),
            format("xlsx", "XLSX", "Tax report", e1kvRefused),
            format("csv", "CSV", "Tax report", e1kvRefused),
          ]}
        />
        <FileRow
          title="Bookkeeping handoff"
          detail={`EA / Jahresabschluss ${year}`}
          formats={[format("year_handoff", "XLSX", "Bookkeeping handoff")]}
        />
        <FileRow
          title="Summary"
          detail={`Portfolio ${year}, no tax tables`}
          formats={[
            format(
              "summary_pdf",
              "PDF",
              "Summary",
              showWalletPicker && selectedWalletIds.length === 0,
            ),
          ]}
          optionsLabel={[
            showWalletPicker && selectedWallets < wallets.length
              ? `${formatCount(selectedWallets)} of ${formatCount(wallets.length)} wallets`
              : `${wallets.length === 1 ? "1 wallet" : "All wallets"}`,
            includeCover ? "balance cover page" : "no cover page",
          ].join(" · ")}
          optionsOpen={openOptions === "summary"}
          onToggleOptions={() =>
            setOpenOptions((current) => (current === "summary" ? null : "summary"))
          }
          options={
            <div className="grid gap-3">
              <label className="flex items-center justify-between gap-3 text-xs">
                <span>Cover page with today's balances</span>
                <Switch
                  checked={includeCover}
                  onCheckedChange={onIncludeCoverChange}
                  aria-label="Cover page with today's balances"
                />
              </label>
              {showWalletPicker ? (
                <div className="grid gap-1">
                  <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
                    <span>Wallets</span>
                    {selectedWallets < wallets.length ? (
                      <button
                        type="button"
                        className="text-xs text-foreground underline-offset-2 hover:underline"
                        onClick={onSelectAllWallets}
                      >
                        Select all
                      </button>
                    ) : null}
                  </div>
                  <ul className="m-0 grid list-none gap-0.5 p-0">
                    {wallets.map((wallet) => (
                      <li key={wallet.id}>
                        <label className="flex cursor-pointer items-center gap-2 rounded-md px-1 py-1 text-xs hover:bg-muted">
                          <Checkbox
                            checked={selectedWalletIds.includes(wallet.id)}
                            onCheckedChange={() => onToggleWallet(wallet.id)}
                          />
                          <span className="min-w-0 flex-1 truncate">{wallet.label}</span>
                          {wallet.chain ? (
                            <span className="text-2xs text-muted-foreground">
                              {wallet.chain}
                            </span>
                          ) : null}
                        </label>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </div>
          }
        />
        <FileRow
          title="Audit package"
          detail="Book evidence for an auditor"
          formats={[format("audit_package", "Export", "Audit package", auditCaseMissing)]}
          optionsLabel={[
            audit.scope === "source_funds_case" ? "One source-of-funds case" : "Whole book",
            `${formatCount(includedParts.length)} of ${formatCount(AUDIT_PARTS.length)} parts`,
          ].join(" · ")}
          optionsOpen={openOptions === "audit"}
          onToggleOptions={() =>
            setOpenOptions((current) => (current === "audit" ? null : "audit"))
          }
          options={
            <div className="grid gap-3">
              <div className="grid gap-1.5">
                <span className="text-xs text-muted-foreground">Scope</span>
                <Select
                  value={audit.scope}
                  onValueChange={(value) =>
                    onAuditChange({ ...audit, scope: value as AuditPackageScope })
                  }
                >
                  <SelectTrigger className="h-8 text-xs" aria-label="Audit package scope">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="active_profile">Whole book</SelectItem>
                    <SelectItem
                      value="source_funds_case"
                      disabled={!sourceFundsCases.length}
                    >
                      A saved source-of-funds case
                    </SelectItem>
                  </SelectContent>
                </Select>
                {audit.scope === "source_funds_case" ? (
                  <Select
                    value={audit.caseId}
                    onValueChange={(caseId) => onAuditChange({ ...audit, caseId })}
                  >
                    <SelectTrigger className="h-8 text-xs" aria-label="Source-of-funds case">
                      <SelectValue placeholder="Choose a saved case" />
                    </SelectTrigger>
                    <SelectContent>
                      {sourceFundsCases.map((item) => (
                        <SelectItem key={item.id} value={item.id}>
                          {item.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : null}
              </div>
              <div className="grid gap-1">
                <span className="text-xs text-muted-foreground">Include</span>
                <ul className="m-0 grid list-none gap-0.5 p-0">
                  {AUDIT_PARTS.map((part) => (
                    <li key={part.key}>
                      <label className="flex cursor-pointer items-center gap-2 rounded-md px-1 py-1 text-xs hover:bg-muted">
                        <Checkbox
                          checked={audit.include[part.key]}
                          onCheckedChange={(value) =>
                            onAuditChange({
                              ...audit,
                              include: { ...audit.include, [part.key]: value === true },
                            })
                          }
                        />
                        {part.label}
                      </label>
                    </li>
                  ))}
                </ul>
              </div>
            </div>
          }
        />
      </ul>

    </section>
  );
}

function FileRow({
  title,
  detail,
  formats,
  optionsLabel,
  optionsOpen = false,
  onToggleOptions,
  options,
}: {
  title: string;
  detail: string;
  formats: ReactNode;
  optionsLabel?: string;
  optionsOpen?: boolean;
  onToggleOptions?: () => void;
  options?: ReactNode;
}) {
  return (
    <li className="px-3 py-2.5 sm:px-4">
      <p className="m-0 text-sm font-medium">{title}</p>
      <p className="m-0 text-xs text-muted-foreground">{detail}</p>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <div className="flex flex-wrap gap-1.5">{formats}</div>
        {options && optionsLabel ? (
          <button
            type="button"
            className="inline-flex min-w-0 items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            aria-expanded={optionsOpen}
            onClick={onToggleOptions}
          >
            <span className="truncate">{optionsLabel}</span>
            <ChevronDown
              className={cn(
                "size-3.5 shrink-0 transition-transform",
                optionsOpen && "rotate-180",
              )}
              aria-hidden="true"
            />
          </button>
        ) : null}
      </div>
      {options && optionsOpen ? (
        <div className="mt-2 rounded-md bg-muted/40 p-2.5">{options}</div>
      ) : null}
    </li>
  );
}

function FormatButton({
  label,
  title,
  loading,
  success,
  disabled,
  onClick,
}: {
  label: string;
  title: string;
  loading: boolean;
  success: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      type="button"
      variant="outline"
      size="sm"
      className="h-7 gap-1.5 px-2 text-xs"
      disabled={disabled}
      aria-label={label === "Export" ? `Export ${title}` : `Export ${title} as ${label}`}
      onClick={onClick}
    >
      {loading ? (
        <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
      ) : success ? (
        <Check className="size-3.5 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
      ) : (
        <Download className="size-3.5" aria-hidden="true" />
      )}
      {label}
    </Button>
  );
}

function ExportStatus({
  status,
  openableExportPath,
  openingExportPath,
  onOpenExport,
}: {
  status: ReportExportStatus;
  openableExportPath: string | null;
  openingExportPath: string | null;
  onOpenExport: (path: string) => void;
}) {
  const Icon = status.tone === "success" ? CheckCircle2 : AlertTriangle;
  return (
    <div
      role={status.tone === "error" ? "alert" : "status"}
      className={cn(
        "flex flex-wrap items-start gap-x-3 gap-y-2 border-b px-3 py-2.5 text-sm sm:px-4",
        status.tone === "error"
          ? "bg-destructive/10 text-destructive"
          : status.tone === "warning"
            ? "bg-amber-500/10 text-amber-800 dark:text-amber-200"
            : "bg-emerald-500/10 text-emerald-800 dark:text-emerald-200",
      )}
    >
      <Icon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="m-0">{status.message}</p>
        {status.notes?.length ? (
          <ul className="m-0 mt-1 grid list-none gap-0.5 p-0 text-xs">
            {status.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        ) : null}
        {status.path ? (
          <p className="m-0 mt-1 truncate font-mono text-xs opacity-80" title={status.path}>
            {status.path}
          </p>
        ) : null}
      </div>
      {openableExportPath ? (
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-7 gap-1.5 bg-background px-2 text-xs text-foreground"
          disabled={openingExportPath === openableExportPath}
          onClick={() => onOpenExport(openableExportPath)}
        >
          {openingExportPath === openableExportPath ? (
            <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          ) : (
            <ExternalLink className="size-3.5" aria-hidden="true" />
          )}
          Open
        </Button>
      ) : null}
    </div>
  );
}
