import { useTranslation } from "react-i18next";

import { TabsContent } from "@/components/ui/tabs";
import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { InfoHint, LedgerRow, formatSheetMoney } from "./TransactionDetailSheetParts";
import { blurClass } from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionLedgerTab({ ctx }: { ctx: TransactionDetailTabContext }) {
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
  return (
    <>
                  {/* Ledger — lead with Net wallet impact, breakdown below */}
                  <TabsContent value="ledger" className="mt-4 space-y-3">
                    <div className="rounded-md border bg-card p-4">
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <div>
                          <div className="flex items-center gap-1.5 text-xs uppercase text-muted-foreground">
                            {t("ledger.netWalletImpact")}
                            <InfoHint label={t("ledger.netWalletImpact")}>
                              {t("ledger.netWalletImpactHint")}
                            </InfoHint>
                          </div>
                          <div className="mt-1 text-2xl font-semibold tabular-nums">
                            {impactDirection === 0 && !feeBtc ? (
                              t("ledger.seePairedMovement")
                            ) : (
                              <span className={blurClass(hideSensitive)}>
                                {formatSheetMoney(
                                  netImpactEur,
                                  netImpactBtc,
                                  balanceCurrency,
                                  true,
                                )}
                              </span>
                            )}
                          </div>
                        </div>
                        <div className="flex rounded-md border bg-background p-0.5">
                          {(["btc", "eur"] satisfies Currency[]).map(
                            (value) => (
                              <button
                                key={value}
                                type="button"
                                aria-pressed={balanceCurrency === value}
                                onClick={() => setBalanceCurrency(value)}
                                className={cn(
                                  "h-7 min-w-10 rounded px-2 text-xs font-medium transition-colors",
                                  balanceCurrency === value
                                    ? "bg-primary text-primary-foreground"
                                    : "text-muted-foreground hover:text-foreground",
                                )}
                              >
                                {value === "btc" ? "BTC" : "EUR"}
                              </button>
                            ),
                          )}
                        </div>
                      </div>
                    </div>

                    {feeBtc > 0 || impactDirection === 0 ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          {t("ledger.howItAddsUp")}
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
                    ) : null}
                  </TabsContent>

    </>
  );
}
