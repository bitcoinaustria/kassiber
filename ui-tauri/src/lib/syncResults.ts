export interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
  code?: string;
  message?: string;
  reason?: string;
  hint?: string;
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
      .join(", ") || "No wallet changes returned.";
  const firstError = results.find((result) => result.status === "error");
  const detail = syncResultDetail(firstError);
  return firstError && detail ? `${summary}: ${firstError.wallet}: ${detail}` : summary;
}
