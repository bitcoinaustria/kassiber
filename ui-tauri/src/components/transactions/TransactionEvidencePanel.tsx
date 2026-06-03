import {
  AlertTriangle,
  CheckCircle2,
  FileText,
  Layers,
  Link2,
  ShieldCheck,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export type AuditEvidenceWarning = {
  code: string;
  severity: "blocker" | "warning" | "info" | string;
  message: string;
  action?: string;
};

export type AuditEvidenceAttachment = {
  id: string;
  attachment_type: "file" | "url" | string;
  label: string;
  media_type?: string;
  size_bytes?: number | null;
  sha256?: string;
  exists?: boolean | null;
  url_host?: string;
  copied_from_attachment_id?: string;
  copied_from_transaction_id?: string;
};

export type AuditEvidenceSource = {
  id: string;
  source_type: string;
  label: string;
  review_state: string;
  attachments?: AuditEvidenceAttachment[];
};

export type AuditEvidenceLink = {
  id: string;
  link_type: string;
  state: string;
  confidence: string;
  method: string;
  asset?: string;
  allocation_amount?: number | null;
  allocation_policy?: string;
  explanation?: string;
  attachments?: AuditEvidenceAttachment[];
  from_source?: AuditEvidenceSource | null;
};

export type AuditEvidenceTransactionSummary = {
  transaction: {
    id: string;
    external_id?: string;
    asset?: string;
  };
  readiness: {
    status: "ready" | "warning" | "blocked" | string;
    warnings: AuditEvidenceWarning[];
  };
  direct_attachments: AuditEvidenceAttachment[];
  source_funds_links: AuditEvidenceLink[];
};

export type AuditEvidenceSummaryData = {
  transactions: AuditEvidenceTransactionSummary[];
  summary?: {
    ready_count?: number;
    blocked_count?: number;
    warning_count?: number;
    transaction_count?: number;
  };
};

function statusCopy(status: string) {
  if (status === "ready") return "Audit ready";
  if (status === "warning") return "Review warnings";
  return "Blocked";
}

function statusTone(status: string) {
  if (status === "ready") {
    return "border-emerald-500/35 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
  }
  if (status === "warning") {
    return "border-amber-500/35 bg-amber-500/10 text-amber-700 dark:text-amber-300";
  }
  return "border-destructive/35 bg-destructive/10 text-destructive";
}

function warningTone(severity: string) {
  if (severity === "info") return "text-muted-foreground";
  if (severity === "warning") return "text-amber-700 dark:text-amber-300";
  return "text-destructive";
}

function attachmentIcon(type: string) {
  return type === "url" ? Link2 : FileText;
}

function EvidenceAttachmentRow({
  attachment,
  hideSensitive,
}: {
  attachment: AuditEvidenceAttachment;
  hideSensitive: boolean;
}) {
  const Icon = attachmentIcon(attachment.attachment_type);
  const detail =
    attachment.attachment_type === "url"
      ? attachment.url_host || "URL reference"
      : [
          attachment.media_type || "file",
          attachment.sha256 ? `sha256 ${attachment.sha256.slice(0, 8)}` : "",
          attachment.exists === false ? "missing file" : "",
        ]
          .filter(Boolean)
          .join(" · ");
  return (
    <li className="flex min-w-0 items-center gap-2 rounded-md border bg-background px-2 py-1.5">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
        <Icon className="size-3.5" aria-hidden="true" />
      </span>
      <span className="min-w-0 flex-1">
        <span className={cn("block truncate text-xs font-medium", blurClass(hideSensitive))}>
          {attachment.label}
        </span>
        {detail ? (
          <span className="block truncate text-[10px] text-muted-foreground">
            {detail}
          </span>
        ) : null}
      </span>
      {attachment.copied_from_attachment_id ? (
        <Badge variant="secondary" className="shrink-0 rounded-md">
          reused
        </Badge>
      ) : null}
    </li>
  );
}

export function TransactionEvidenceReadinessPanel({
  summary,
  hideSensitive,
}: {
  summary?: AuditEvidenceTransactionSummary;
  hideSensitive: boolean;
}) {
  const readiness = summary?.readiness;
  const status = readiness?.status ?? "blocked";
  const directAttachments = summary?.direct_attachments ?? [];
  const links = summary?.source_funds_links ?? [];
  const blockers = readiness?.warnings.filter((item) => item.severity === "blocker") ?? [];
  const warnings = readiness?.warnings.filter((item) => item.severity !== "blocker") ?? [];
  const StatusIcon = status === "ready" ? CheckCircle2 : AlertTriangle;

  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-3 flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2 text-sm font-semibold">
          <ShieldCheck className="size-4 text-muted-foreground" aria-hidden="true" />
          Evidence readiness
        </div>
        <Badge variant="outline" className={cn("shrink-0 rounded-md", statusTone(status))}>
          <StatusIcon className="mr-1 size-3" aria-hidden="true" />
          {statusCopy(status)}
        </Badge>
      </div>

      {summary ? (
        <div className="space-y-3">
          <div>
            <div className="mb-1.5 flex items-center gap-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              <FileText className="size-3" aria-hidden="true" />
              Direct attachments
            </div>
            {directAttachments.length ? (
              <ul className="space-y-1.5">
                {directAttachments.map((attachment) => (
                  <EvidenceAttachmentRow
                    key={attachment.id}
                    attachment={attachment}
                    hideSensitive={hideSensitive}
                  />
                ))}
              </ul>
            ) : (
              <p className="rounded-md border border-dashed bg-muted/40 px-2 py-2 text-xs text-muted-foreground">
                No direct receipt, note, file, or URL reference is attached.
              </p>
            )}
          </div>

          <div>
            <div className="mb-1.5 flex items-center gap-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              <Layers className="size-3" aria-hidden="true" />
              Source-funds review
            </div>
            {links.length ? (
              <ul className="space-y-2">
                {links.map((link) => (
                  <li key={link.id} className="rounded-md border bg-background px-2 py-2">
                    <div className="flex min-w-0 items-start justify-between gap-2">
                      <span className="min-w-0">
                        <span className="block truncate text-xs font-semibold">
                          {link.link_type} · {link.state}
                        </span>
                        <span className="block truncate text-[10px] text-muted-foreground">
                          {link.confidence} · {link.method}
                          {link.allocation_amount
                            ? ` · ${link.allocation_amount.toFixed(8)} ${link.asset ?? "BTC"}`
                            : ""}
                          {link.allocation_policy ? ` · ${link.allocation_policy}` : ""}
                        </span>
                      </span>
                      <Badge variant="outline" className="shrink-0 rounded-md text-[10px]">
                        {link.state}
                      </Badge>
                    </div>
                    {link.explanation ? (
                      <p className={cn("mt-1 text-[10px] leading-4 text-muted-foreground", blurClass(hideSensitive))}>
                        {link.explanation}
                      </p>
                    ) : null}
                    {link.from_source ? (
                      <p className="mt-1 text-[10px] text-muted-foreground">
                        Root source:{" "}
                        <span className={blurClass(hideSensitive)}>
                          {link.from_source.label}
                        </span>{" "}
                        · {link.from_source.source_type} · {link.from_source.review_state}
                      </p>
                    ) : null}
                    {[...(link.attachments ?? []), ...(link.from_source?.attachments ?? [])].length ? (
                      <ul className="mt-2 space-y-1">
                        {[...(link.attachments ?? []), ...(link.from_source?.attachments ?? [])].map((attachment) => (
                          <EvidenceAttachmentRow
                            key={`${link.id}-${attachment.id}`}
                            attachment={attachment}
                            hideSensitive={hideSensitive}
                          />
                        ))}
                      </ul>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="rounded-md border border-dashed bg-muted/40 px-2 py-2 text-xs text-muted-foreground">
                No source-funds link is connected to this transaction yet.
              </p>
            )}
          </div>

          {blockers.length || warnings.length ? (
            <div className="space-y-1.5">
              {[...blockers, ...warnings].map((item) => (
                <div
                  key={`${item.code}-${item.message}`}
                  className={cn("rounded-md border bg-background px-2 py-1.5 text-xs", warningTone(item.severity))}
                >
                  <div className="font-medium">{item.message}</div>
                  {item.action ? (
                    <div className="mt-0.5 text-[10px] opacity-80">{item.action}</div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Evidence summary is loading from the local database.
        </p>
      )}
    </div>
  );
}
