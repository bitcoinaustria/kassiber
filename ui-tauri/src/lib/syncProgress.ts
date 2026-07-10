import type {
  ActiveMaintenanceProgress,
  NotificationProgress,
  NotificationTone,
} from "@/store/ui";
import { formatCount } from "@/lib/localeFormat";

export type WalletSyncProgress = {
  job_id?: string;
  job_type?: string;
  job_index?: number;
  job_total?: number;
  phase?: string;
  wallet?: string;
  source_label?: string;
  source_type?: string;
  processed?: number;
  total?: number;
  imported?: number;
  skipped?: number;
  retained_targets?: number;
  gap_limit?: number;
  unused_streak?: number;
  branch_index?: number;
  // Emitted with phase "rate_limited" while a backend 429/503 backoff is waiting,
  // so the UI shows "rate limited, retrying" instead of a frozen progress bar.
  retry_attempt?: number;
  retry_max?: number;
  wait_seconds?: number;
};

export const BOOK_REFRESH_PROGRESS_ID = "book-refresh";
export const STARTING_SYNC_PROGRESS_VALUE = 5;

function clampProgress(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

function progressNumbers(progress: WalletSyncProgress) {
  if (progress.phase === "discovery") {
    return {
      processed:
        typeof progress.processed === "number" ? progress.processed : null,
      total: null,
    };
  }
  return {
    processed:
      typeof progress.processed === "number" ? progress.processed : null,
    total: typeof progress.total === "number" ? progress.total : null,
  };
}

function jobNumbers(progress: WalletSyncProgress) {
  return {
    index:
      typeof progress.job_index === "number" && progress.job_index > 0
        ? Math.floor(progress.job_index)
        : null,
    total:
      typeof progress.job_total === "number" && progress.job_total > 0
        ? Math.floor(progress.job_total)
        : null,
  };
}

const PHASE_LABELS: Record<string, string> = {
  discovery: "Discovering wallet history",
  backend_fetch: "Fetching source history",
  decode_enrich: "Decoding and enriching transactions",
  import: "Importing transactions",
  importing: "Importing transactions",
  rate_coverage: "Checking market-rate coverage",
  auto_pair: "Pairing swaps and transfers",
  journal_refresh: "Refreshing journals",
  rate_limited: "Waiting out rate limit",
  done: "Refresh complete",
  error: "Refresh needs attention",
};

const PHASE_PROGRESS_FRACTIONS: Record<string, number> = {
  discovery: 0.12,
  backend_fetch: 0.46,
  decode_enrich: 0.62,
  import: 0.78,
  importing: 0.78,
  rate_coverage: 0.86,
  auto_pair: 0.91,
  journal_refresh: 0.94,
  done: 1,
  error: 1,
};

export interface SyncMilestone {
  phase: string;
  label: string;
  /** Cumulative progress fraction (0–1) at which this phase is considered done. */
  fraction: number;
}

/**
 * Ordered checklist for the first-sync experience. Derived from the same phase
 * label/fraction maps that drive the progress bar so the milestone copy and the
 * percentage can never drift apart.
 */
export const FIRST_SYNC_MILESTONES: readonly SyncMilestone[] = [
  "discovery",
  "backend_fetch",
  "decode_enrich",
  "import",
  "rate_coverage",
  "auto_pair",
  "journal_refresh",
].map((phase) => ({
  phase,
  label: PHASE_LABELS[phase] ?? phase,
  fraction: PHASE_PROGRESS_FRACTIONS[phase] ?? 1,
}));

/**
 * Index of the milestone the progress bar is currently in: the first phase the
 * `fraction` (0–1) hasn't passed yet. Uses `<=` so a phase stays active while
 * the bar is still within it (a phase at exactly its threshold is the current
 * one, not already done). Returns `FIRST_SYNC_MILESTONES.length` when every
 * phase is complete; with no determinate value the first milestone is active.
 */
export function firstSyncActiveMilestoneIndex(
  fraction: number,
  isDeterminate: boolean,
): number {
  if (!isDeterminate) return 0;
  const firstPending = FIRST_SYNC_MILESTONES.findIndex(
    (milestone) => fraction <= milestone.fraction,
  );
  return firstPending === -1 ? FIRST_SYNC_MILESTONES.length : firstPending;
}

export function syncPhaseLabel(phase: string | undefined, fallback: string) {
  if (!phase) return fallback;
  return PHASE_LABELS[phase] ?? phase.replaceAll("_", " ");
}

export function syncProgressPhaseLabel(
  phase: string | undefined,
  fallback: string,
) {
  return syncPhaseLabel(phase, fallback);
}

function phaseProgressFraction(phase: string | undefined) {
  if (!phase) return null;
  return PHASE_PROGRESS_FRACTIONS[phase] ?? null;
}

function sourceLabel(progress: WalletSyncProgress) {
  return progress.wallet || progress.source_label || "";
}

function sourceTypeLabel(progress: WalletSyncProgress) {
  switch (progress.source_type) {
    case "onchain_wallet":
      return "Wallet source";
    case "btcpay_wallet":
      return "BTCPay wallet";
    case "btcpay_provenance":
      return "BTCPay provenance";
    case "market_rates":
      return "Market rates";
    case "journals":
      return "Journals";
    default:
      return null;
  }
}

function computeProgressValue(
  progress: WalletSyncProgress,
  previousValue: number,
) {
  // A rate-limit backoff is a wait, not forward progress — hold the bar steady
  // (rather than nudging it) so the "waiting" state reads as paused, not stalled.
  if (progress.phase === "rate_limited") {
    return clampProgress(previousValue);
  }
  const { processed, total } = progressNumbers(progress);
  const { index, total: jobTotal } = jobNumbers(progress);
  if (processed !== null && total !== null && total > 0) {
    const rowFraction = clampProgress((processed / total) * 100) / 100;
    if (index !== null && jobTotal !== null) {
      return clampProgress(((index - 1 + rowFraction) / jobTotal) * 100);
    }
    return clampProgress(rowFraction * 100);
  }

  const phaseFraction = phaseProgressFraction(progress.phase);
  if (phaseFraction !== null) {
    if (index !== null && jobTotal !== null) {
      return clampProgress(((index - 1 + phaseFraction) / jobTotal) * 100);
    }
    return clampProgress(Math.max(previousValue, phaseFraction * 100));
  }

  if (processed !== null) {
    return Math.min(85, Math.max(previousValue, previousValue + 8));
  }
  return Math.min(85, previousValue + 10);
}

function progressLabel(progress: WalletSyncProgress) {
  const source = sourceLabel(progress);
  const phase = syncPhaseLabel(progress.phase, "Refreshing configured sources");
  const { processed, total } = progressNumbers(progress);
  const prefix = source ? `${source}: ` : "";

  if (processed !== null && total !== null && total > 0) {
    return `${prefix}${phase} · ${formatCount(processed)} / ${formatCount(total)}`;
  }
  if (processed !== null) {
    return `${prefix}${phase} · ${formatCount(processed)} ${progressUnit(progress)}`;
  }
  return `${prefix}${phase}`;
}

function progressUnit(progress: WalletSyncProgress) {
  if (progress.phase === "discovery") {
    return "addresses probed";
  }
  if (progress.phase === "backend_fetch") {
    return "scan targets checked";
  }
  if (progress.phase === "decode_enrich") {
    return "transactions scanned";
  }
  return "rows scanned";
}

function gapLimitLabel(progress: WalletSyncProgress) {
  if (progress.phase !== "discovery") return null;
  const gapLimit =
    typeof progress.gap_limit === "number" && progress.gap_limit > 0
      ? Math.floor(progress.gap_limit)
      : null;
  if (gapLimit === null) return null;
  const unused =
    typeof progress.unused_streak === "number" && progress.unused_streak >= 0
      ? Math.floor(progress.unused_streak)
      : null;
  if (unused !== null) {
    return `Unused gap ${formatCount(Math.min(unused, gapLimit))} / ${formatCount(gapLimit)}`;
  }
  return `Stops after ${formatCount(gapLimit)} consecutive unused addresses`;
}

function retainedTargetLabel(progress: WalletSyncProgress) {
  if (progress.phase !== "discovery") return null;
  const retained =
    typeof progress.retained_targets === "number" && progress.retained_targets >= 0
      ? Math.floor(progress.retained_targets)
      : null;
  if (retained === null) return null;
  return `${formatCount(retained)} targets selected so far`;
}

function rowProgressLabel(progress: WalletSyncProgress) {
  const { processed, total } = progressNumbers(progress);
  const unit = progressUnit(progress);
  if (processed !== null && total !== null && total > 0) {
    return `${formatCount(processed)} / ${formatCount(total)} ${unit}`;
  }
  if (processed !== null) {
    return `${formatCount(processed)} ${unit}`;
  }
  return null;
}

function importOutcomeLabel(progress: WalletSyncProgress) {
  const imported =
    typeof progress.imported === "number" ? progress.imported : null;
  const skipped =
    typeof progress.skipped === "number" ? progress.skipped : null;
  if (imported === null && skipped === null) return null;
  const parts = [
    imported !== null ? `${formatCount(imported)} imported` : null,
    skipped !== null ? `${formatCount(skipped)} unchanged` : null,
  ].filter(Boolean);
  return parts.join(" · ");
}

function jobProgressLabel(progress: WalletSyncProgress) {
  const { index, total } = jobNumbers(progress);
  if (index === null || total === null) return null;
  const current = formatCount(Math.min(index, total));
  return `Source ${current} of ${formatCount(total)}`;
}

function backoffLabel(progress: WalletSyncProgress) {
  if (progress.phase !== "rate_limited") return null;
  const attempt =
    typeof progress.retry_attempt === "number" ? progress.retry_attempt : null;
  const max = typeof progress.retry_max === "number" ? progress.retry_max : null;
  const wait =
    typeof progress.wait_seconds === "number" ? progress.wait_seconds : null;
  const retry =
    attempt !== null && max !== null ? `retry ${attempt}/${max}` : "retrying";
  const waitText = wait !== null ? ` in ${wait}s` : "";
  return `Rate limited — ${retry}${waitText}`;
}

export function startingSyncProgress(): NotificationProgress {
  return {
    value: STARTING_SYNC_PROGRESS_VALUE,
    indeterminate: false,
    label: "Preparing source refresh",
  };
}

export function formatSyncProgressBody(progress: WalletSyncProgress) {
  const source = sourceLabel(progress);
  const prefix = source ? `${source}: ` : "";
  const phase = syncPhaseLabel(progress.phase, "refresh is running");
  const { processed, total } = progressNumbers(progress);
  const unit = progressUnit(progress);
  const gap = gapLimitLabel(progress);
  if (processed !== null && total !== null && total > 0) {
    const rows = `${formatCount(processed)} / ${formatCount(total)}`;
    return `${prefix}${phase}; ${rows} ${unit}.`;
  }
  if (processed !== null) {
    const suffix = gap ? ` ${gap}.` : "";
    return `${prefix}${phase}; ${formatCount(processed)} ${unit}.${suffix}`;
  }
  return prefix ? `${prefix}${phase}.` : `${phase}.`;
}

export function syncProgressNotification(
  progress: WalletSyncProgress,
  previousValue: number = STARTING_SYNC_PROGRESS_VALUE,
): { body: string; progress: NotificationProgress; value: number } {
  const value = computeProgressValue(progress, previousValue);
  const indeterminate = progress.phase === "discovery";

  return {
    body: formatSyncProgressBody(progress),
    progress: {
      value,
      indeterminate,
      label: progressLabel(progress),
    },
    value,
  };
}

function activeSyncTitle(progress: WalletSyncProgress) {
  if (progress.phase === "rate_limited") {
    return "Waiting out rate limit";
  }
  if (progress.phase === "auto_pair") {
    return "Pairing swaps and transfers";
  }
  if (
    progress.source_type === "journals" ||
    progress.phase === "journal_refresh"
  ) {
    return "Refreshing journals";
  }
  if (
    progress.source_type === "market_rates" ||
    progress.phase === "rate_coverage"
  ) {
    return "Checking market rates";
  }
  return "Refreshing book";
}

export function activeSyncMaintenanceProgress(
  progress: WalletSyncProgress,
  previousValue: number = STARTING_SYNC_PROGRESS_VALUE,
  options: {
    id?: string;
    tone?: NotificationTone;
    startedAt?: string;
    updatedAt?: string;
  } = {},
): ActiveMaintenanceProgress {
  const notification = syncProgressNotification(progress, previousValue);
  const phase = syncPhaseLabel(
    progress.phase,
    "Refreshing configured sources",
  );
  const source = sourceLabel(progress);
  const sourceType = sourceTypeLabel(progress);
  const details = [
    jobProgressLabel(progress),
    source ? source : sourceType,
    backoffLabel(progress),
    rowProgressLabel(progress),
    gapLimitLabel(progress),
    retainedTargetLabel(progress),
    importOutcomeLabel(progress),
  ].filter((item): item is string => Boolean(item));
  const now = options.updatedAt ?? new Date().toISOString();
  return {
    id: options.id ?? BOOK_REFRESH_PROGRESS_ID,
    title: activeSyncTitle(progress),
    body: source ? `${source}: ${phase}.` : notification.body,
    tone: options.tone ?? "warning",
    progress: notification.progress,
    details,
    active: true,
    startedAt: options.startedAt ?? now,
    updatedAt: now,
  };
}
