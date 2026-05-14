import type { NotificationProgress } from "@/store/ui";

export type WalletSyncProgress = {
  phase?: string;
  wallet?: string;
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

export function startingSyncProgress(): NotificationProgress {
  return {
    value: STARTING_SYNC_PROGRESS_VALUE,
    indeterminate: false,
    label: "Preparing wallet scan",
  };
}

export function formatSyncProgressBody(progress: WalletSyncProgress) {
  const wallet = progress.wallet ? `${progress.wallet}: ` : "";
  const { processed, total } = progressNumbers(progress);
  if (processed !== null && total !== null && total > 0) {
    return `${wallet}${processed.toLocaleString()} / ${total.toLocaleString()} transactions scanned.`;
  }
  if (processed !== null) {
    return `${wallet}${processed.toLocaleString()} transactions scanned.`;
  }
  return wallet
    ? `${wallet}refresh is running.`
    : "Kassiber is scanning configured watch-only sources.";
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
          ? `Scanning transactions: ${processed.toLocaleString()} / ${total.toLocaleString()}`
          : processed !== null
            ? `Scanning transactions: ${processed.toLocaleString()} scanned`
            : progress.wallet
              ? `Scanning ${progress.wallet}`
              : "Scanning configured sources",
    },
    value,
  };
}
