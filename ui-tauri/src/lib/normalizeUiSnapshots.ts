import type {
  Connection,
  OverviewSnapshot,
  PortfolioPoint,
  TaxFreeBalanceSnapshot,
  Tx,
} from "@/mocks/seed";
import type {
  QuarantineAction,
  QuarantineAssumption,
  QuarantineCategory,
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

function normalizeQuarantineAssumption(value: unknown): QuarantineAssumption {
  const raw = isRecord(value) ? value : {};
  return {
    count: finiteNumber(raw.count),
    amount_msat: finiteNumber(raw.amount_msat),
    items: arrayOrEmpty<unknown>(raw.items)
      .filter(isRecord)
      .map((item) => ({
        transaction_id: nullableString(item.transaction_id) ?? "",
        occurred_at: nullableString(item.occurred_at),
        wallet: nullableString(item.wallet) ?? "",
        amount_msat: finiteNumber(item.amount_msat),
        external_id: nullableString(item.external_id) ?? "",
      }))
      .filter((item) => item.transaction_id),
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
  };
}

export function normalizeQuarantineSnapshot(value: unknown): QuarantineSnapshot {
  const raw = isRecord(value) ? value : {};
  const summary = isRecord(raw.summary) ? raw.summary : {};
  const byReason = arrayOrEmpty<unknown>(summary.by_reason).filter(
    (entry): entry is QuarantineReason => isRecord(entry),
  );
  const assumptions = isRecord(summary.assumptions) ? summary.assumptions : null;

  return {
    summary: {
      workspace: nullableString(summary.workspace),
      profile: nullableString(summary.profile),
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
      assumptions: assumptions
        ? {
            presumed_external_outbound: normalizeQuarantineAssumption(
              assumptions.presumed_external_outbound,
            ),
            unclassified_inbound: normalizeQuarantineAssumption(
              assumptions.unclassified_inbound,
            ),
          }
        : null,
    },
    items: arrayOrEmpty<QuarantineItem>(raw.items),
  };
}
