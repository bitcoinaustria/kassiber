/**
 * Overview-only positions in legacy (non-Bitcoin) overlay assets.
 *
 * Reads `ui.reports.legacy_holdings`. Overlay assets never reach the journal
 * pipeline or any tax report; positions are valued at the last import-time
 * price. The panel renders NOTHING when there are no overlay rows — the
 * overwhelmingly common case — instead of an empty card.
 */
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { retryRetryableDaemonError, useDaemon } from "@/daemon/client";
import { MISSING_FIAT_LABEL, formatFiatAmount } from "@/lib/currency";
import { currentUiLocale } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import { blurClass } from "./model";

export interface LegacyHoldingsRow {
  asset: string;
  wallet: string;
  quantity: number;
  last_price: number | null;
  priced_at: string | null;
  market_value: number | null;
  fiat_currency: string;
  transaction_count: number;
  last_activity_at: string | null;
  tax_accounted: false;
}

export interface LegacyHoldingsData {
  rows: LegacyHoldingsRow[];
  tax_accounted: false;
  summary: { row_count: number; asset_count: number };
}

const formatQuantity = (value: number) =>
  new Intl.NumberFormat(currentUiLocale(), {
    maximumFractionDigits: 8,
  }).format(value);

const formatPricedAt = (value: string | null) => {
  if (!value) return null;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return null;
  return parsed.toLocaleDateString(currentUiLocale(), {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
};

export const LegacyHoldingsPanel = ({
  className,
  hideSensitive,
}: {
  className?: string;
  hideSensitive: boolean;
}) => {
  const { t } = useTranslation("overview");
  const query = useDaemon<LegacyHoldingsData>("ui.reports.legacy_holdings", undefined, {
    retry: retryRetryableDaemonError,
  });
  const rows = query.data?.data?.rows ?? [];
  // No overlay assets (or the report is unavailable): render nothing at all.
  if (rows.length === 0) return null;

  return (
    <div className={cn("overflow-hidden rounded-lg border bg-card", className)}>
      <div className="flex flex-wrap items-center gap-2 border-b px-3 py-2.5 sm:px-4">
        <span className="text-sm font-medium">{t("legacyHoldings.title")}</span>
        <Badge variant="outline" className="text-muted-foreground">
          {t("legacyHoldings.notTaxAccounted")}
        </Badge>
      </div>
      <p className="px-3 pt-2.5 text-xs text-muted-foreground sm:px-4">
        {t("legacyHoldings.explainer")}
      </p>
      <div className="overflow-x-auto p-3 sm:p-4">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("legacyHoldings.columns.asset")}</TableHead>
              <TableHead>{t("legacyHoldings.columns.wallet")}</TableHead>
              <TableHead className="text-right">
                {t("legacyHoldings.columns.quantity")}
              </TableHead>
              <TableHead className="text-right">
                {t("legacyHoldings.columns.lastPrice")}
              </TableHead>
              <TableHead className="text-right">
                {t("legacyHoldings.columns.value")}
              </TableHead>
              <TableHead className="text-right">
                {t("legacyHoldings.columns.pricedAt")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={`${row.asset}-${row.wallet}`}>
                <TableCell className="font-medium">{row.asset}</TableCell>
                <TableCell className="text-muted-foreground">
                  {row.wallet}
                </TableCell>
                <TableCell
                  className={cn(
                    "text-right tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatQuantity(row.quantity)}
                </TableCell>
                <TableCell className="text-right tabular-nums text-muted-foreground">
                  {row.last_price !== null
                    ? formatFiatAmount(row.last_price, row.fiat_currency)
                    : MISSING_FIAT_LABEL}
                </TableCell>
                <TableCell
                  className={cn(
                    "text-right tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {row.market_value !== null
                    ? formatFiatAmount(row.market_value, row.fiat_currency)
                    : MISSING_FIAT_LABEL}
                </TableCell>
                <TableCell className="text-right text-muted-foreground">
                  {formatPricedAt(row.priced_at) ?? "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
};
