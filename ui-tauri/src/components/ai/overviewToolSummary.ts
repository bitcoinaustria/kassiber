import type { TFunction } from "i18next";

import { formatFiatAmount } from "@/lib/currency";
import { normalizeFiatCompleteness } from "@/lib/fiatCompleteness";

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * One-line summary of a `ui.overview.snapshot` tool result. Realized YTD comes
 * from the same journal as the cost basis, so while `fiat.completeness` is not
 * complete it is qualified instead of reading as an exact figure.
 */
export function overviewSnapshotSummary(
  data: Record<string, unknown>,
  t: TFunction<"assistant">,
): string {
  const connections = Array.isArray(data.connections) ? data.connections : [];
  const txs = Array.isArray(data.txs) ? data.txs : [];
  const fiat = asRecord(data.fiat);
  const completeness = normalizeFiatCompleteness(fiat?.completeness);
  const basisIncomplete = !completeness.costBasisComplete;
  // priceEur falls back to a transaction rate when no EUR market rate exists.
  const eurRateMissing =
    completeness.marketRateMissing &&
    String(fiat?.fiatCurrency ?? "EUR").toUpperCase() === "EUR";
  const priceEur =
    typeof data.priceEur === "number" && !eurRateMissing
      ? t("tool.overviewSnapshot.priceEur", {
          value: formatFiatAmount(data.priceEur, "EUR"),
        })
      : eurRateMissing
        ? t("tool.overviewSnapshot.noMarketRate")
        : null;
  const realizedYtd =
    typeof fiat?.eurRealizedYTD === "number"
      ? t(
          basisIncomplete
            ? "tool.overviewSnapshot.realizedYtdIncomplete"
            : "tool.overviewSnapshot.realizedYtd",
          { value: formatFiatAmount(fiat.eurRealizedYTD, "EUR") },
        )
      : null;
  const basisNote = basisIncomplete
    ? t(
        completeness.state === "stale"
          ? "tool.overviewSnapshot.basisStale"
          : "tool.overviewSnapshot.basisIncomplete",
      )
    : null;
  return [
    t("tool.overviewSnapshot.connections", { count: connections.length }),
    t("tool.overviewSnapshot.recentTransactions", { count: txs.length }),
    priceEur,
    realizedYtd,
    basisNote,
  ]
    .filter((part): part is string => Boolean(part))
    .join("; ");
}
