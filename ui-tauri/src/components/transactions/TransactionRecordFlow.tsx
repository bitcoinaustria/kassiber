import { ArrowRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { fiatFormatter } from "@/lib/currency";
import { formatAssetAmount, type Transaction } from "./model";

export function TransactionRecordFlow({ transaction, kind, hideSensitive }: {
  transaction: Transaction;
  kind: "buy" | "sell";
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const hidden = t("graph.hidden");
  const known = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);
  const btc = known(transaction.amountBtc) ? formatAssetAmount(Math.abs(transaction.amountBtc), transaction.asset || "BTC") : t("details.unknown");
  // A valuation or manual price is not evidence of the fiat counter-leg.
  const currency = transaction.fiatCurrency;
  const exactValue = transaction.pricingSourceKind === "exchange_execution" && transaction.pricingQuality === "exact"
    && known(transaction.amount) && currency && /^[A-Z]{3}$/.test(currency);
  const fiat = exactValue ? fiatFormatter(currency).format(Math.abs(transaction.amount!)) : t("recordFlow.valueMissing");
  const crypto = { label: t(kind === "sell" ? "recordFlow.sold" : "recordFlow.bought"), value: btc };
  const cash = { label: t("recordFlow.reportedValue"), value: fiat };
  const sides = kind === "sell" ? [crypto, cash] : [cash, crypto];
  return (
    <div className="space-y-3" data-testid="transaction-record-flow">
      <div className="flex items-center justify-between gap-3 text-sm">
        <span className="font-medium">{t(kind === "sell" ? "recordFlow.sale" : "recordFlow.purchase")}</span>
        <span className="truncate text-muted-foreground">{hideSensitive ? hidden : transaction.counterparty || transaction.wallet}</span>
      </div>
      <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3">
        {sides.map((side, index) => <div key={side.label} className="contents">
          {index === 1 ? <ArrowRight className="size-4 text-muted-foreground" aria-hidden="true" /> : null}
          <div className="min-w-0 rounded-md border bg-background p-3">
            <div className="text-xs text-muted-foreground">{side.label}</div>
            <div className="mt-1 break-words font-mono text-sm tabular-nums">{hideSensitive ? hidden : side.value}</div>
          </div>
        </div>)}
      </div>
      <div className="flex flex-wrap justify-between gap-2 text-xs text-muted-foreground">
        <span>{t("recordFlow.noChainTransaction")}</span>
        {known(transaction.feeBtc) && transaction.feeBtc !== 0 ? <span>
          {t("recordFlow.reportedFee")}: {hideSensitive ? hidden : formatAssetAmount(Math.abs(transaction.feeBtc), transaction.asset || "BTC")}
        </span> : null}
      </div>
    </div>
  );
}
