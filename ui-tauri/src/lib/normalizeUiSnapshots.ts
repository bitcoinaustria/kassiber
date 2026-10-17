import type {
  Connection,
  OverviewSnapshot,
  PortfolioPoint,
  TaxFreeBalanceSnapshot,
  Tx,
} from "@/mocks/seed";
import type {
  QuarantineAction,
  QuarantineCategory,
  QuarantineScope,
  QuarantineEvidence,
  QuarantineFreshness,
  QuarantineGroup,
  QuarantineItem,
  QuarantineReason,
  QuarantineSnapshot,
} from "@/components/kb/quarantine/types";
import { normalizeFiatCompleteness } from "@/lib/fiatCompleteness";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function arrayOrEmpty<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

function finiteNumber(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function nullableString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function normalizeNumberArray(value: unknown): number[] {
  return arrayOrEmpty<unknown>(value).filter(
    (entry): entry is number => typeof entry === "number" && Number.isFinite(entry),
  );
}

function normalizeBalanceSummary(
  value: unknown,
): OverviewSnapshot["balanceSummary"] {
  if (!isRecord(value)) return undefined;
  if (typeof value.totalBtc !== "number" || !Number.isFinite(value.totalBtc)) {
    return undefined;
  }
  return {
    totalBtc: value.totalBtc,
    status: typeof value.status === "string" ? value.status : "current",
    source: typeof value.source === "string" ? value.source : "mixed",
    needsJournals: value.needsJournals === true,
    quarantines: finiteNumber(value.quarantines),
    chainWalletCount: finiteNumber(value.chainWalletCount),
    bookWalletCount: finiteNumber(value.bookWalletCount),
    transactionWalletCount: finiteNumber(value.transactionWalletCount),
    duplicateOutpointAdjustmentBtc: finiteNumber(
      value.duplicateOutpointAdjustmentBtc,
    ),
  };
}

export function normalizeOverviewSnapshot(value: unknown): OverviewSnapshot {
  const raw = isRecord(value) ? value : {};
  const fiat = isRecord(raw.fiat) ? raw.fiat : {};
  const status = isRecord(raw.status) ? raw.status : null;
  const balanceSummary = normalizeBalanceSummary(raw.balanceSummary);

  return {
    priceEur: finiteNumber(raw.priceEur),
    priceUsd: finiteNumber(raw.priceUsd),
    marketRate: isRecord(raw.marketRate)
      ? (raw.marketRate as unknown as OverviewSnapshot["marketRate"])
      : undefined,
    connections: arrayOrEmpty<Connection>(raw.connections),
    activityTxs: arrayOrEmpty<Tx>(raw.activityTxs),
    txs: arrayOrEmpty<Tx>(raw.txs),
    balanceSeries: normalizeNumberArray(raw.balanceSeries),
    portfolioSeries: Array.isArray(raw.portfolioSeries)
      ? (raw.portfolioSeries as PortfolioPoint[])
      : undefined,
    fiat: {
      fiatCurrency: nullableString(fiat.fiatCurrency),
      eurBalance: finiteNumber(fiat.eurBalance),
      eurCostBasis: finiteNumber(fiat.eurCostBasis),
      eurUnrealized: finiteNumber(fiat.eurUnrealized),
      eurRealizedYTD: finiteNumber(fiat.eurRealizedYTD),
      // Missing/unknown completeness never reads as complete; see
      // UNKNOWN_FIAT_COMPLETENESS.
      completeness: normalizeFiatCompleteness(fiat.completeness),
    },
    ...(balanceSummary ? { balanceSummary } : {}),
    taxFreeBalance: isRecord(raw.taxFreeBalance)
      ? (raw.taxFreeBalance as unknown as TaxFreeBalanceSnapshot)
      : null,
    status: status
      ? {
          workspace: nullableString(status.workspace),
          profile: nullableString(status.profile),
          transactionCount:
            typeof status.transactionCount === "number"
              ? status.transactionCount
              : undefined,
          needsJournals: status.needsJournals === true,
          quarantines: finiteNumber(status.quarantines),
        }
      : undefined,
  };
}

function normalizeQuarantineFreshness(value: unknown): QuarantineFreshness | null {
  if (!isRecord(value)) return null;
  const lastError = isRecord(value.last_error) ? value.last_error : null;
  return {
    needs_processing: value.needs_processing === true,
    status: nullableString(value.status),
    last_processed_at: nullableString(value.last_processed_at),
    last_error: lastError
      ? {
          code: nullableString(lastError.code) ?? "journal_refresh_failed",
          message: nullableString(lastError.message) ?? "",
          at: nullableString(lastError.at),
        }
      : null,
  };
}

function normalizeQuarantineGroup(value: Record<string, unknown>): QuarantineGroup {
  return {
    key: nullableString(value.key) ?? "",
    category: (nullableString(value.category) ?? "needs_decision") as QuarantineCategory,
    reason: nullableString(value.reason) ?? "",
    root_transaction_id: nullableString(value.root_transaction_id),
    root_occurred_at: nullableString(value.root_occurred_at),
    root_wallet: nullableString(value.root_wallet),
    root_external_id: nullableString(value.root_external_id),
    root_amount_msat:
      typeof value.root_amount_msat === "number" ? value.root_amount_msat : null,
    root_direction: nullableString(value.root_direction),
    root_asset: nullableString(value.root_asset),
    count: finiteNumber(value.count),
    downstream_count: finiteNumber(value.downstream_count),
    blocks_reports: value.blocks_reports === true,
    wallets: arrayOrEmpty<unknown>(value.wallets).filter(
      (entry): entry is string => typeof entry === "string",
    ),
    earliest_occurred_at: nullableString(value.earliest_occurred_at),
    evidence: isRecord(value.evidence) ? (value.evidence as QuarantineEvidence) : {},
    actions: arrayOrEmpty<unknown>(value.actions).filter(isRecord) as unknown as QuarantineAction[],
    root_transaction_ids: arrayOrEmpty<unknown>(value.root_transaction_ids).filter(
      (entry): entry is string => typeof entry === "string",
    ),
    root_count: finiteNumber(value.root_count),
  };
}

export function normalizeQuarantineSnapshot(value: unknown): QuarantineSnapshot {
  const raw = isRecord(value) ? value : {};
  const summary = isRecord(raw.summary) ? raw.summary : {};
  const byReason = arrayOrEmpty<unknown>(summary.by_reason).filter(
    (entry): entry is QuarantineReason => isRecord(entry),
  );

  return {
    summary: {
      workspace: nullableString(summary.workspace),
      profile: nullableString(summary.profile),
      workspace_id: nullableString(summary.workspace_id),
      profile_id: nullableString(summary.profile_id),
      count: finiteNumber(summary.count),
      by_reason: byReason,
      limit: finiteNumber(summary.limit),
      offset: finiteNumber(summary.offset),
      // An older daemon sends no freshness; `null` means "unknown", never
      // "current", so the page does not claim the list is up to date.
      freshness: normalizeQuarantineFreshness(summary.freshness),
      blocking_count: finiteNumber(summary.blocking_count),
      reports_blocked: summary.reports_blocked === true,
      by_category: arrayOrEmpty<unknown>(summary.by_category)
        .filter(isRecord)
        .map((entry) => ({
          category: (nullableString(entry.category) ?? "needs_decision") as QuarantineCategory,
          count: finiteNumber(entry.count),
        })),
      groups: arrayOrEmpty<unknown>(summary.groups)
        .filter(isRecord)
        .map(normalizeQuarantineGroup),
      group_count: finiteNumber(summary.group_count),
      ...(typeof summary.attention_count === "number"
        ? {
            attention_count: finiteNumber(summary.attention_count),
            waiting_count: finiteNumber(summary.waiting_count),
            scope_count: finiteNumber(summary.scope_count),
            scope: (["attention", "waiting", "all"].includes(String(summary.scope))
              ? summary.scope
              : "all") as QuarantineScope,
          }
        : {}),
    },
    items: arrayOrEmpty<QuarantineItem>(raw.items),
  };
}
