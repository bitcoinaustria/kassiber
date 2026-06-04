import type { NotificationProgress } from "@/store/ui";

export type WalletSyncProgress = {
  phase?: string;
  wallet?: string;
  source_label?: string;
  source_type?: string;
  processed?: number;
  total?: number;
  imported?: number;
  skipped?: number;
};

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

const PHASE_LABELS: Record<string, string> = {
  discovery: "Discovering wallet history",
  backend_fetch: "Fetching source history",
  decode_enrich: "Decoding and enriching transactions",
  import: "Importing transactions",
  rate_coverage: "Checking market-rate coverage",
  journal_refresh: "Refreshing journals",
  done: "Refresh complete",
  error: "Refresh needs attention",
};

function phaseLabel(phase: string | undefined, fallback: string) {
  if (!phase) return fallback;
  return PHASE_LABELS[phase] ?? phase.replaceAll("_", " ");
}

function sourceLabel(progress: WalletSyncProgress) {
  return progress.wallet || progress.source_label || "";
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
  const phase = phaseLabel(progress.phase, "refresh is running");
  const { processed, total } = progressNumbers(progress);
  if (processed !== null && total !== null && total > 0) {
    return `${prefix}${phase}; ${processed.toLocaleString()} / ${total.toLocaleString()} rows scanned.`;
  }
  if (processed !== null) {
    return `${prefix}${phase}; ${processed.toLocaleString()} rows scanned.`;
  }
  return prefix
    ? `${prefix}${phase}.`
    : `${phase}.`;
}

export function syncProgressNotification(
  progress: WalletSyncProgress,
  previousValue: number = STARTING_SYNC_PROGRESS_VALUE,
): { body: string; progress: NotificationProgress; value: number } {
  const { processed, total } = progressNumbers(progress);
  const value =
    processed !== null && total !== null && total > 0
      ? clampProgress((processed / total) * 100)
      : Math.min(85, previousValue + 10);

  return {
    body: formatSyncProgressBody(progress),
    progress: {
      value,
      indeterminate: false,
      label:
        processed !== null && total !== null && total > 0
          ? `${phaseLabel(progress.phase, "Scanning transactions")}: ${processed.toLocaleString()} / ${total.toLocaleString()}`
          : processed !== null
            ? `${phaseLabel(progress.phase, "Scanning transactions")}: ${processed.toLocaleString()} scanned`
            : sourceLabel(progress)
              ? `${phaseLabel(progress.phase, "Refreshing")} ${sourceLabel(progress)}`
              : phaseLabel(progress.phase, "Refreshing configured sources"),
    },
    value,
  };
}
