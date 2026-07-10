export interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
  code?: string;
  message?: string;
  reason?: string;
  hint?: string;
  details?: Record<string, unknown> | null;
  retryable?: boolean;
  imported?: number;
  updated?: number;
  unchanged?: number;
  records_fetched?: number;
  scripts_checked?: number;
  scripts_changed?: number;
  scripts_unchanged?: number;
  target_count?: number;
  elapsed_ms?: number;
  journal_invalidated?: boolean;
  utxos_skipped_unchanged?: boolean;
  utxos_refreshed?: boolean;
  force_full?: boolean;
}

export interface FreshnessSourceState {
  source_key: string;
  source_type: string;
  source_label: string;
  status:
    | "fresh"
    | "queued"
    | "syncing"
    | "paused"
    | "rate_limited"
    | "partially_stale"
    | "failed"
    | "blocking_reports"
    | string;
  stale_reason?: string | null;
  rate_limited_until?: string | null;
  last_error_message?: string | null;
  blocking_reports?: boolean;
}

export interface FreshnessJobSummary {
  job_type?: string;
  source_label?: string;
  status?: string;
  result?:
    | (Record<string, unknown> & {
        auto_pair?: FreshnessAutoPairSummary | null;
      })
    | null;
  error?: {
    code?: string;
    message?: string;
    hint?: string;
  } | null;
}

export interface FreshnessAutoPairSummary {
  enabled?: boolean;
  applied?: number;
  rules_applied?: number;
  bulk_exact_applied?: number;
  skipped_conflicts?: number;
  total_swap_fee_msat?: number;
  skipped?: boolean;
  error?: {
    code?: string;
    message?: string;
    retryable?: boolean;
  } | null;
  before?: FreshnessTransferCandidateCounts | null;
  remaining?: FreshnessTransferCandidateCounts | null;
}

export interface FreshnessTransferCandidateCounts {
  total?: number;
  exact?: number;
  strong?: number;
  conflicts?: number;
  rule_matches?: number;
}

export interface FreshnessRunData {
  results?: SyncResult[];
  enqueued?: FreshnessJobSummary[];
  completed?: FreshnessJobSummary[];
  sources?: FreshnessSourceState[];
  summary?: {
    failed?: number;
    blocking_reports?: number;
    rate_limited?: number;
  };
}

export function syncResultDetail(result: SyncResult | undefined): string | null {
  if (!result) return null;
  const primary = result.message || result.reason || result.code;
  return [primary, result.hint].filter(Boolean).join(" ") || null;
}

function formatNumber(value: number): string {
  return formatCount(value);
}

function syncResultObservability(result: SyncResult | undefined): string | null {
  if (!result || result.status !== "synced") return null;
  const parts: string[] = [];
  if (result.force_full) parts.push("full rescan");
  const imported = Number(result.imported ?? 0);
  const updated = Number(result.updated ?? 0);
  const unchanged = Number(result.unchanged ?? 0);
  if (imported || updated || unchanged) {
    const rowParts = [
      imported ? `${formatNumber(imported)} imported` : null,
      updated ? `${formatNumber(updated)} updated` : null,
      unchanged ? `${formatNumber(unchanged)} unchanged` : null,
    ].filter(Boolean);
    parts.push(rowParts.join(", "));
  } else if (typeof result.records_fetched === "number") {
    parts.push(`${formatNumber(result.records_fetched)} source rows`);
  }
  if (typeof result.scripts_checked === "number" && result.scripts_checked > 0) {
    parts.push(`${formatNumber(result.scripts_checked)} scripts checked`);
  } else if (typeof result.target_count === "number" && result.target_count > 0) {
    parts.push(`${formatNumber(result.target_count)} targets`);
  }
  if (result.utxos_skipped_unchanged) {
    parts.push("UTXOs unchanged");
  } else if (result.utxos_refreshed) {
    parts.push("UTXOs refreshed");
  }
  if (result.journal_invalidated === false) {
    parts.push("journals unchanged");
  } else if (result.journal_invalidated === true) {
    parts.push("journals marked stale");
  }
  if (typeof result.elapsed_ms === "number" && result.elapsed_ms >= 0) {
    parts.push(`${formatNumber(Math.max(0, Math.round(result.elapsed_ms)))} ms`);
  }
  return parts.join(" · ") || null;
}

export function describeWalletSyncResult(
  result: SyncResult | undefined,
  walletLabel: string,
): string {
  const wallet = result?.wallet || walletLabel;
  const status = result?.status ?? "synced";
  const detail = syncResultDetail(result);

  if (status === "error") {
    return detail ? `${wallet} refresh failed: ${detail}` : `${wallet} refresh failed.`;
  }
  if (status === "skipped") {
    return detail ? `${wallet} refresh skipped: ${detail}` : `${wallet} refresh skipped.`;
  }
  if (status === "synced") {
    const observations = syncResultObservability(result);
    return observations ? `${wallet} refreshed: ${observations}.` : `${wallet} refreshed.`;
  }
  if (status === "queued") {
    return `${wallet} refresh queued.`;
  }
  if (status === "rate_limited") {
    return detail ? `${wallet} refresh cooling down: ${detail}` : `${wallet} refresh cooling down.`;
  }
  if (status === "partially_stale") {
    return detail ? `${wallet} partially refreshed: ${detail}` : `${wallet} partially refreshed.`;
  }
  return `${wallet} refresh ${status}.`;
}

export function summarizeSyncResults(results: SyncResult[]): string {
  const synced = results.filter((result) => result.status === "synced").length;
  const skipped = results.filter((result) => result.status === "skipped").length;
  const errors = results.filter((result) => result.status === "error").length;
  const summary =
    [
      synced ? `${synced} refreshed` : null,
      skipped ? `${skipped} skipped` : null,
      errors ? `${errors} failed` : null,
    ]
      .filter(Boolean)
      .join(", ") || "No source changes returned.";
  const firstError = results.find((result) => result.status === "error");
  const detail = syncResultDetail(firstError);
  return firstError && detail ? `${summary}: ${firstError.wallet}: ${detail}` : summary;
}

export function syncResultsAreTrustedForReports(results: SyncResult[]): boolean {
  return !results.some((result) =>
    ["error", "failed", "blocking_reports"].includes(result.status),
  );
}

export function freshnessRunNeedsAttention(data: FreshnessRunData | null | undefined): boolean {
  const completed = data?.completed ?? [];
  const sources = data?.sources ?? [];
  const summary = data?.summary;
  return (
    completed.some((job) => ["error", "cancelled"].includes(job.status ?? "")) ||
    completed.some(
      (job) => job.job_type === "journal_refresh" && autoPairNeedsAttention(job),
    ) ||
    sources.some((source) => Boolean(source.blocking_reports) || source.status === "failed") ||
    Boolean((summary?.failed ?? 0) > 0 || (summary?.blocking_reports ?? 0) > 0)
  );
}

function positiveInteger(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) return 0;
  return Math.floor(value);
}

export function freshnessRunQuarantineCount(
  data: FreshnessRunData | null | undefined,
): number {
  return (data?.completed ?? []).reduce((total, job) => {
    if (job.job_type !== "journal_refresh") return total;
    const result = job.result;
    if (!result || typeof result !== "object") return total;
    return (
      total +
      Math.max(
        positiveInteger(result.quarantined),
        positiveInteger(result.quarantine_count),
      )
    );
  }, 0);
}

function autoPairSummary(job: FreshnessJobSummary): FreshnessAutoPairSummary | null {
  const result = job.result;
  const summary = result?.auto_pair;
  if (!summary || typeof summary !== "object") return null;
  return summary;
}

function autoPairNeedsAttention(job: FreshnessJobSummary): boolean {
  const summary = autoPairSummary(job);
  return Boolean(summary?.skipped || summary?.error);
}

export function freshnessRunAutoPairCount(
  data: FreshnessRunData | null | undefined,
): number {
  return (data?.completed ?? []).reduce((total, job) => {
    if (job.job_type !== "journal_refresh") return total;
    return total + positiveInteger(autoPairSummary(job)?.applied);
  }, 0);
}

export function freshnessRunTransferReviewCount(
  data: FreshnessRunData | null | undefined,
): number {
  return (data?.completed ?? []).reduce((total, job) => {
    if (job.job_type !== "journal_refresh") return total;
    return total + positiveInteger(autoPairSummary(job)?.remaining?.total);
  }, 0);
}

export function summarizeFreshnessRun(data: FreshnessRunData | null | undefined): string {
  const completed = data?.completed ?? [];
  if (!completed.length) {
    const enqueued = data?.enqueued?.length ?? 0;
    if (enqueued) return `${enqueued} source${enqueued === 1 ? "" : "s"} queued.`;
    return summarizeSyncResults(data?.results ?? []);
  }

  const done = completed.filter((job) => job.status === "done").length;
  const rateLimited = completed.filter((job) => job.status === "rate_limited").length;
  const failed = completed.filter((job) => ["error", "cancelled"].includes(job.status ?? "")).length;
  const quarantineCount = freshnessRunQuarantineCount(data);
  const autoPaired = freshnessRunAutoPairCount(data);
  const transferReviewCount = freshnessRunTransferReviewCount(data);
  const autoPairProblem = completed.find(
    (job) => job.job_type === "journal_refresh" && autoPairNeedsAttention(job),
  );
  const parts = [
    done ? `${done} completed` : null,
    rateLimited ? `${rateLimited} cooling down` : null,
    failed ? `${failed} needs attention` : null,
    autoPairProblem ? "automatic pairing skipped" : null,
    autoPaired ? `${autoPaired} pair${autoPaired === 1 ? "" : "s"} applied` : null,
    transferReviewCount
      ? `${transferReviewCount} swap/transfer candidate${transferReviewCount === 1 ? "" : "s"} to review`
      : null,
    quarantineCount
      ? `${quarantineCount} quarantined transaction${quarantineCount === 1 ? "" : "s"}`
      : null,
  ].filter(Boolean);
  const summary = parts.join(", ") || "No source changes returned.";
  const firstProblem = completed.find((job) => job.status && job.status !== "done");
  const autoPairDetail = autoPairProblem
    ? autoPairSummary(autoPairProblem)?.error?.message
    : null;
  const detail = firstProblem?.error?.message || firstProblem?.error?.hint;
  return firstProblem && detail
    ? `${summary}: ${firstProblem.source_label ?? "Source"}: ${detail}`
    : autoPairDetail
      ? `${summary}: ${autoPairDetail}`
    : summary;
}

export function describeFreshnessSourceState(source: FreshnessSourceState): string {
  const label = source.source_label || source.source_key;
  if (source.status === "fresh") return `${label} is fresh.`;
  if (source.status === "queued") return `${label} is queued.`;
  if (source.status === "syncing") return `${label} is syncing.`;
  if (source.status === "paused") return `${label} is paused.`;
  if (source.status === "rate_limited") {
    return source.rate_limited_until
      ? `${label} is rate limited until ${source.rate_limited_until}.`
      : `${label} is rate limited.`;
  }
  if (source.status === "partially_stale") return `${label} is partially stale.`;
  if (source.status === "blocking_reports") return `${label} is blocking reports.`;
  if (source.status === "failed") {
    return source.last_error_message
      ? `${label} failed: ${source.last_error_message}`
      : `${label} failed.`;
  }
  return `${label} is ${source.status}.`;
}
import { formatCount } from "@/lib/localeFormat";
