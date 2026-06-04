export interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
  code?: string;
  message?: string;
  reason?: string;
  hint?: string;
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
  error?: {
    code?: string;
    message?: string;
    hint?: string;
  } | null;
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
    return `${wallet} refreshed.`;
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
    sources.some((source) => Boolean(source.blocking_reports) || source.status === "failed") ||
    Boolean((summary?.failed ?? 0) > 0 || (summary?.blocking_reports ?? 0) > 0)
  );
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
  const parts = [
    done ? `${done} completed` : null,
    rateLimited ? `${rateLimited} cooling down` : null,
    failed ? `${failed} needs attention` : null,
  ].filter(Boolean);
  const summary = parts.join(", ") || "No source changes returned.";
  const firstProblem = completed.find((job) => job.status && job.status !== "done");
  const detail = firstProblem?.error?.message || firstProblem?.error?.hint;
  return firstProblem && detail
    ? `${summary}: ${firstProblem.source_label ?? "Source"}: ${detail}`
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
