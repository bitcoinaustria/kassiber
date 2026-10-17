import { DaemonRequestError } from "@/daemon/client";

/**
 * The quarantined transactions that stop an export. The daemon names up to 20
 * of them in the error details (`require_no_quarantines` in the core).
 */
export interface QuarantineExportBlock {
  count: number;
  transactions: {
    transaction_id?: string;
    transaction_ref?: string;
    wallet?: string;
    occurred_at?: string;
    reason?: string;
  }[];
}

const QUARANTINE_EXPORT_CODES = new Set([
  "transaction_export_quarantined",
  "year_handoff_quarantined",
]);

/** The quarantine block behind a failed export or preview, or null for any other error. */
export function quarantineExportBlock(error: unknown): QuarantineExportBlock | null {
  if (!(error instanceof DaemonRequestError)) return null;
  const daemonError = error.envelope.error;
  if (!daemonError || !QUARANTINE_EXPORT_CODES.has(daemonError.code ?? "")) return null;
  const details = (daemonError.details ?? {}) as Partial<QuarantineExportBlock>;
  return {
    count: Number(details.count ?? 0),
    transactions: Array.isArray(details.transactions) ? details.transactions : [],
  };
}
