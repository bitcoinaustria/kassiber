import { ArrowRight, Building2, Wallet } from "lucide-react";
import { useTranslation } from "react-i18next";
import { formatAssetAmount, formatShortTxid } from "./model";
import type { TransactionSwapRoute } from "./TransactionGraphModel";
import { exchangeTransfer } from "./ExchangeTransferModel";

export function ExchangeTransferSummary({ route, hideSensitive }: {
  route: TransactionSwapRoute;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const transfer = exchangeTransfer(route);
  if (!transfer) return null;
  const hidden = t("graph.hidden");
  const exchange = route[transfer.exchangeLeg];
  const chain = route[transfer.chainLeg];
  const txid = chain.txid && /^[a-f\d]{64}$/i.test(chain.txid) ? chain.txid : null;
  const fee = exchange.feeBtc;
  const reference = exchange.externalId?.trim();
  const distinctReference = reference && reference.toLowerCase() !== txid?.toLowerCase()
    ? reference : null;
  return (
    <div className="space-y-3" data-testid="exchange-transfer-summary">
      <div className="text-sm font-medium">{t(`exchangeTransfer.${transfer.direction}`)}</div>
      <div className="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3">
        {(["out", "in"] as const).map((side, index) => {
          const leg = route[side];
          const isExchange = side === transfer.exchangeLeg;
          const Icon = isExchange ? Building2 : Wallet;
          return <div key={side} className="contents">
            {index === 1 ? <ArrowRight className="size-4 text-muted-foreground" aria-hidden="true" /> : null}
            <div className="min-w-0 rounded-md border bg-background p-3">
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Icon className="size-4 shrink-0" aria-hidden="true" />
                {t(isExchange ? "exchangeTransfer.account" : "exchangeTransfer.wallet")}
              </div>
              <div className="mt-2 break-words text-sm font-medium">{hideSensitive ? hidden : leg.wallet?.label || t("graph.swapRouteUnknownLeg")}</div>
              <div className="mt-1 break-words font-mono text-sm tabular-nums">
                {hideSensitive ? hidden : typeof leg.amountBtc === "number" && Number.isFinite(leg.amountBtc)
                  ? formatAssetAmount(leg.amountBtc, leg.asset || "BTC") : t("details.unknown")}
              </div>
            </div>
          </div>;
        })}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 text-xs text-muted-foreground">
        <div className="min-w-0 space-y-1">
          <div>{t("exchangeTransfer.chainRecord")}{txid ? <> · <span className="font-mono">{hideSensitive ? hidden : formatShortTxid(txid)}</span></> : null}</div>
          {typeof fee === "number" && Number.isFinite(fee) && fee > 0 ? <div>
            {t("exchangeTransfer.fee")}: {hideSensitive ? hidden : formatAssetAmount(fee, exchange.asset || "BTC")}
          </div> : null}
        </div>
      </div>
      {distinctReference ? <div className="break-all text-xs text-muted-foreground">
        {t("exchangeTransfer.reference")}: <span className="font-mono">{hideSensitive ? hidden : distinctReference}</span>
      </div> : null}
    </div>
  );
}
