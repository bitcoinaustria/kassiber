import type {
  ActiveMaintenanceProgress,
  NotificationProgress,
  NotificationTone,
} from "@/store/ui";

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
};

export const BOOK_REFRESH_PROGRESS_ID = "book-refresh";
export const STARTING_SYNC_PROGRESS_VALUE = 5;

function clampProgress(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

function progressNumbers(progress: WalletSyncProgress) {
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
  importing: "Importing transactions",
  import: "Importing transactions",
  rate_coverage: "Checking market-rate coverage",
  journal_refresh: "Refreshing journals",
  done: "Refresh complete",
  error: "Refresh needs attention",
};

const PHASE_PROGRESS_FRACTIONS: Record<string, number> = {
  discovery: 0.12,
  backend_fetch: 0.46,
  decode_enrich: 0.62,
  importing: 0.78,
  import: 0.78,
  rate_coverage: 0.86,
  journal_refresh: 0.94,
  done: 1,
  error: 1,
};

export function syncPhaseLabel(phase: string | undefined, fallback: string) {
  if (!phase) return fallback;
  return PHASE_LABELS[phase] ?? phase.replaceAll("_", " ");
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

function rowProgressLabel(progress: WalletSyncProgress) {
  const { processed, total } = progressNumbers(progress);
  if (processed !== null && total !== null && total > 0) {
    return `${processed.toLocaleString()} / ${total.toLocaleString()} rows scanned`;
  }
  if (processed !== null) {
    return `${processed.toLocaleString()} rows scanned`;
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
    imported !== null ? `${imported.toLocaleString()} imported` : null,
    skipped !== null ? `${skipped.toLocaleString()} unchanged` : null,
  ].filter(Boolean);
  return parts.join(" · ");
}

function jobProgressLabel(progress: WalletSyncProgress) {
  const { index, total } = jobNumbers(progress);
  if (index === null || total === null) return null;
  const current = Math.min(index, total).toLocaleString();
  return `Source ${current} of ${total.toLocaleString()}`;
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
  if (processed !== null && total !== null && total > 0) {
    const rows = `${processed.toLocaleString()} / ${total.toLocaleString()}`;
    return `${prefix}${phase}; ${rows} rows scanned.`;
  }
  if (processed !== null) {
    return `${prefix}${phase}; ${processed.toLocaleString()} rows scanned.`;
  }
  return prefix ? `${prefix}${phase}.` : `${phase}.`;
}

export function syncProgressNotification(
  progress: WalletSyncProgress,
  previousValue: number = STARTING_SYNC_PROGRESS_VALUE,
): { body: string; progress: NotificationProgress; value: number } {
  const { processed, total } = progressNumbers(progress);
  const scanningPhase = syncPhaseLabel(progress.phase, "Scanning transactions");
  const refreshPhase = syncPhaseLabel(progress.phase, "Refreshing");
  const value = computeProgressValue(progress, previousValue);

  return {
    body: formatSyncProgressBody(progress),
    progress: {
      value,
      indeterminate: false,
      label:
        processed !== null && total !== null && total > 0
          ? `${scanningPhase}: ${processed.toLocaleString()} / ${total.toLocaleString()}`
          : processed !== null
            ? `${scanningPhase}: ${processed.toLocaleString()} scanned`
            : sourceLabel(progress)
              ? `${refreshPhase}: ${sourceLabel(progress)}`
              : syncPhaseLabel(progress.phase, "Refreshing configured sources"),
    },
    value,
  };
}

function activeSyncTitle(progress: WalletSyncProgress) {
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
    rowProgressLabel(progress),
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
