import { useTranslation } from "react-i18next";

import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { InfoHint, LedgerRow, formatSheetMoney } from "./TransactionDetailSheetParts";
import { blurClass } from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

/**
 * Net-wallet-impact breakdown inside the Details tab. Rendered only when the
 * net differs from the headline amount (a fee applies) or the row's own sign
 * is misleading (paired/transfer legs) — for a plain no-fee receive the
 * header already states the amount, so repeating it here is noise.
 */
export function TransactionLedgerSection({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const {
    feeBtc,
    impactDirection,
    netImpactEur,
    netImpactBtc,
    balanceCurrency,
    setBalanceCurrency,
    principalImpactEur,
    principalImpactBtc,
    feeImpactEur,
    feeImpactBtc,
    hideSensitive,
  } = ctx;

  if (feeBtc <= 0 && impactDirection !== 0) return null;

  return (
    <div className="overflow-hidden rounded-md border">
      <div className="flex items-center justify-between gap-2 border-b bg-muted px-3 py-1">
        <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          {t("ledger.netWalletImpact")}
          <InfoHint label={t("ledger.netWalletImpact")}>
            {t("ledger.netWalletImpactHint")}
          </InfoHint>
        </div>
        <div className="flex rounded-md border bg-background p-0.5">
          {(["btc", "eur"] satisfies Currency[]).map((value) => (
            <button
              key={value}
              type="button"
              aria-pressed={balanceCurrency === value}
              onClick={() => setBalanceCurrency(value)}
              className={cn(
                "h-5 min-w-9 rounded px-1.5 text-[10px] font-medium transition-colors",
                balanceCurrency === value
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {value === "btc" ? "BTC" : "EUR"}
            </button>
          ))}
        </div>
      </div>
      <LedgerRow
        label={t("ledger.principal")}
        value={
          impactDirection === 0 ? (
            <span className="text-muted-foreground">
              {t("ledger.pairedSeeLinked")}
            </span>
          ) : (
            <span className={blurClass(hideSensitive)}>
              {formatSheetMoney(
                principalImpactEur,
                principalImpactBtc,
                balanceCurrency,
                true,
              )}
            </span>
          )
        }
        align="right"
        hint={t("ledger.principalHint")}
      />
      {feeBtc > 0 ? (
        <LedgerRow
          label={t("ledger.fee")}
          value={
            <span className={blurClass(hideSensitive)}>
              {formatSheetMoney(
                feeImpactEur,
                feeImpactBtc,
                balanceCurrency,
                true,
              )}
            </span>
          }
          align="right"
          hint={t("ledger.feeHint")}
        />
      ) : null}
      <LedgerRow
        label={t("ledger.net")}
        value={
          <span
            className={cn(
              "font-semibold",
              blurClass(hideSensitive),
            )}
          >
            {impactDirection === 0 && !feeBtc
              ? t("ledger.seePairedMovement")
              : formatSheetMoney(
                  netImpactEur,
                  netImpactBtc,
                  balanceCurrency,
                  true,
                )}
          </span>
        }
        align="right"
        muted
      />
    </div>
  );
}
