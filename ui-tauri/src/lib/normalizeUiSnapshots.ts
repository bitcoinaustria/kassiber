import type {
  Connection,
  OverviewSnapshot,
  PortfolioPoint,
  TaxFreeBalanceSnapshot,
  Tx,
} from "@/mocks/seed";
import type {
  QuarantineItem,
  QuarantineReason,
  QuarantineSnapshot,
} from "@/components/kb/quarantine/types";

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

export function normalizeOverviewSnapshot(value: unknown): OverviewSnapshot {
  const raw = isRecord(value) ? value : {};
  const fiat = isRecord(raw.fiat) ? raw.fiat : {};
  const status = isRecord(raw.status) ? raw.status : null;

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
    },
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
      count: finiteNumber(summary.count),
      by_reason: byReason,
      limit: finiteNumber(summary.limit),
    },
    items: arrayOrEmpty<QuarantineItem>(raw.items),
  };
}
