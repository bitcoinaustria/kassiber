import { TabsContent } from "@/components/ui/tabs";
import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { InfoHint, LedgerRow, formatSheetMoney } from "./TransactionDetailSheetParts";
import { blurClass } from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionLedgerTab({ ctx }: { ctx: TransactionDetailTabContext }) {
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
                            Net wallet impact
                            <InfoHint label="Net wallet impact">
                              The signed change to this wallet after principal
                              and fees. This is the bottom-line number for
                              accounting.
                            </InfoHint>
                          </div>
                          <div className="mt-1 text-2xl font-semibold tabular-nums">
                            {impactDirection === 0 && !feeBtc ? (
                              "See paired movement"
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
                          How it adds up
                        </div>
                        <LedgerRow
                          label="Principal"
                          value={
                            impactDirection === 0 ? (
                              <span className="text-muted-foreground">
                                Paired — see Linked tab
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
                          hint="Signed principal applied to this wallet."
                        />
                        {feeBtc > 0 ? (
                          <LedgerRow
                            label="Fee"
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
                            hint="Network or settlement fee subtracted from this wallet."
                          />
                        ) : null}
                        <LedgerRow
                          label="Net"
                          value={
                            <span
                              className={cn(
                                "font-semibold",
                                blurClass(hideSensitive),
                              )}
                            >
                              {impactDirection === 0 && !feeBtc
                                ? "See paired movement"
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
