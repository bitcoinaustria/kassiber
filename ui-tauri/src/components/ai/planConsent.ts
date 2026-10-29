/**
 * Consent previews for plan-bound assistant writes (attachment and document
 * import, and import rollback). The daemon recomputes the plan when it asks for consent
 * and sends it as `arguments_preview`; the card renders that, never figures
 * the model supplied.
 */

export const PLAN_BOUND_CONSENT_TOOLS = new Set([
  "ui.wallets.import_attachment.apply",
  "ui.wallets.import_document.apply",
  "ui.imports.rollback",
]);

export type PlanPreviewStatus = "ready" | "blocked" | "stale" | "unavailable";

export interface PlanBlocker {
  code: string;
  message: string;
  desktopOnly: boolean;
}

export interface ImportPlanTotal {
  asset: string;
  direction: string;
  rows: number;
  amountMsat: number | null;
  feeMsat: number | null;
}

export interface ImportPlanPreview {
  kind: "import";
  status: PlanPreviewStatus;
  code?: string;
  targetLabel: string;
  createsWallet: boolean;
  completenessClass: string;
  rows: { toImport: number; alreadyPresent: number; updatesExisting: number };
  /** Null when the provider's agent access withholds the file's values. */
  dateRange: { first: string | null; last: string | null } | null;
  totals: ImportPlanTotal[];
  overlap: { inDateRange: number; inOtherWallets: number };
  warnings: string[];
  blockers: PlanBlocker[];
}

export interface RollbackPlanPreview {
  kind: "rollback";
  status: PlanPreviewStatus;
  code?: string;
  transactionsToDelete: number;
  importedAt: string | null;
  sourceFormat: string | null;
  alsoRemoved: { table: string; count: number }[];
  blockers: PlanBlocker[];
}

export type PlanConsentPreview = ImportPlanPreview | RollbackPlanPreview;

const record = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
const count = (value: unknown): number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : 0;
const text = (value: unknown): string => (typeof value === "string" ? value : "");
const integerOrNull = (value: unknown): number | null =>
  typeof value === "number" && Number.isInteger(value) ? value : null;

function status(value: unknown): PlanPreviewStatus {
  return value === "ready" || value === "blocked" || value === "stale" ? value : "unavailable";
}

function blockers(value: unknown): PlanBlocker[] {
  return (Array.isArray(value) ? value : []).flatMap((item) => {
    const blocker = record(item);
    if (!blocker || typeof blocker.code !== "string") return [];
    return [{
      code: blocker.code,
      message: text(blocker.message),
      desktopOnly: record(blocker.remediation)?.desktop_only === true,
    }];
  });
}

/** Read the daemon's consent preview; null when it is not a plan preview. */
export function planConsentPreview(
  name: string,
  preview: Record<string, unknown>,
): PlanConsentPreview | null {
  if (!PLAN_BOUND_CONSENT_TOOLS.has(name)) return null;
  const plan = record(preview.plan) ?? {};
  const base = { status: status(preview.status), code: text(preview.code) || undefined };
  if (name === "ui.imports.rollback") {
    const batch = record(plan.batch) ?? {};
    const removed = record(plan.also_removed) ?? {};
    return {
      kind: "rollback",
      ...base,
      transactionsToDelete: count(plan.transactions_to_delete),
      importedAt: text(batch.imported_at) || null,
      sourceFormat: text(batch.source_format) || null,
      alsoRemoved: Object.entries(removed)
        .map(([table, value]) => ({ table, count: count(value) }))
        .filter((item) => item.count > 0),
      blockers: blockers(plan.blockers),
    };
  }
  const target = record(plan.target) ?? {};
  const rows = record(plan.rows) ?? {};
  const range = record(plan.date_range);
  const overlap = record(plan.overlap) ?? {};
  return {
    kind: "import",
    ...base,
    targetLabel: text(target.label),
    createsWallet: target.action === "create",
    completenessClass: text(target.completeness_class),
    rows: {
      toImport: count(rows.to_import),
      alreadyPresent: count(rows.already_present),
      updatesExisting: count(rows.would_update_existing),
    },
    dateRange: range ? { first: text(range.first) || null, last: text(range.last) || null } : null,
    totals: (Array.isArray(plan.totals) ? plan.totals : []).flatMap((item) => {
      const total = record(item);
      if (!total || typeof total.asset !== "string") return [];
      return [{
        asset: total.asset,
        direction: text(total.direction),
        rows: count(total.rows),
        amountMsat: integerOrNull(total.amount_msat),
        feeMsat: integerOrNull(total.fee_msat),
      }];
    }),
    overlap: {
      inDateRange: count(overlap.existing_rows_in_date_range),
      inOtherWallets: count(overlap.external_ids_in_other_wallets),
    },
    warnings: (Array.isArray(plan.warnings) ? plan.warnings : []).filter(
      (item): item is string => typeof item === "string",
    ),
    blockers: blockers(plan.blockers),
  };
}

export function planApprovalAvailable(preview: PlanConsentPreview | null): boolean {
  return preview?.status === "ready";
}
