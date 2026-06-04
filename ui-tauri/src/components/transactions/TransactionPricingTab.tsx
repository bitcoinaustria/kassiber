import { AlertTriangle, Save, SlidersHorizontal } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { TabsContent } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

import {
  DetailField,
  DirtyDot,
  InfoHint,
  formatRateAtTime,
} from "./TransactionDetailSheetParts";
import {
  blurClass,
  currencyFormatter,
  formatBtcAmount,
  pricingProviderLabel,
  pricingQualityLabel,
  pricingSourceLabel,
  transactionPricingOptions,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionPricingTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const {
    transaction,
    localDraft,
    dirtyPricing,
    hideSensitive,
    amountBtc,
    pricingValue,
    updateDraft,
    updateManualPrice,
    updateManualValue,
    manualPriceRef,
    hasCacheProvenance,
    isCoarsePricing,
    isProviderSamplePricing,
    isExactPricing,
    isPricingMissing,
    pricePoint,
    nowRate,
    onOpenMarketDataSettings,
    openMarketDataSettings,
    chooseExactManualPrice,
  } = ctx;
  return (
    <>
                  {/* Pricing — single workstation for pricing source + manual override */}
                  <TabsContent value="pricing" className="mt-4">
                    <div className="grid gap-4">
                      <div className="grid gap-3 md:grid-cols-4">
                        {transactionPricingOptions.map((option) => (
                          <button
                            key={option.value}
                            type="button"
                            className={cn(
                              "rounded-md border p-3 text-left transition-colors hover:bg-muted/50",
                              pricingValue === option.value &&
                                "border-primary bg-accent",
                            )}
                            onClick={() => {
                              updateDraft(
                                "pricingSourceKind",
                                option.sourceKind,
                              );
                              updateDraft("pricingQuality", option.quality);
                            }}
                          >
                            <div className="text-sm font-medium">
                              {option.label}
                            </div>
                            <div className="mt-1 text-xs text-muted-foreground">
                              {option.description}
                            </div>
                          </button>
                        ))}
                      </div>
                      <div className="grid gap-3 rounded-md border bg-muted/50 p-3">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div>
                            <div className="flex items-center gap-1.5 text-sm font-medium">
                              Manual price override
                              <DirtyDot active={dirtyPricing} />
                              <InfoHint label="Manual price override">
                                Saves a reviewed price source and exact manual
                                rate/value to the transaction row. Reprocess
                                journals after saving.
                              </InfoHint>
                            </div>
                            <div className="text-xs text-muted-foreground">
                              Calculated from the fixed amount:{" "}
                              {formatBtcAmount(amountBtc)}.
                            </div>
                          </div>
                          <Badge
                            variant="outline"
                            className={cn(
                              "rounded-md",
                              localDraft.pricingSourceKind === "manual_override"
                                ? "border-amber-600/30 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
                                : "text-muted-foreground",
                            )}
                          >
                            {pricingSourceLabel(
                              localDraft.pricingSourceKind,
                              localDraft.pricingQuality,
                            )}
                          </Badge>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[100px_1fr_1fr]">
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-currency">Currency</Label>
                            <Input
                              id="tx-manual-currency"
                              value={localDraft.manualCurrency}
                              onChange={(event) =>
                                updateDraft(
                                  "manualCurrency",
                                  event.target.value.toUpperCase(),
                                )
                              }
                              maxLength={3}
                            />
                          </div>
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-price">Price / BTC</Label>
                            <Input
                              id="tx-manual-price"
                              ref={manualPriceRef}
                              inputMode="decimal"
                              value={localDraft.manualPrice}
                              onChange={(event) =>
                                updateManualPrice(event.target.value)
                              }
                              placeholder="69453.46"
                            />
                          </div>
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-value">Total value</Label>
                            <Input
                              id="tx-manual-value"
                              inputMode="decimal"
                              value={localDraft.manualValue}
                              onChange={(event) =>
                                updateManualValue(event.target.value)
                              }
                              placeholder="17086.29"
                            />
                          </div>
                        </div>
                        <div className="grid gap-2">
                          <Label
                            htmlFor="tx-manual-source"
                            className="flex items-center gap-1.5"
                          >
                            Evidence / source
                            <InfoHint label="Evidence">
                              The proof for the price you typed — invoice
                              number, screenshot of an OTC quote, bank receipt,
                              or accountant note. Required for an auditable
                              manual override.
                            </InfoHint>
                          </Label>
                          <Input
                            id="tx-manual-source"
                            value={localDraft.manualSource}
                            className={blurClass(hideSensitive)}
                            onChange={(event) =>
                              updateDraft("manualSource", event.target.value)
                            }
                            placeholder="BTCPay invoice, bank receipt, accountant review"
                          />
                          <p className="text-[11px] text-muted-foreground">
                            Attach the actual file or URL via the{" "}
                            <span className="font-medium">Attachments</span>{" "}
                            panel on the right.
                          </p>
                        </div>
                      </div>
                      <div className="grid gap-3 sm:grid-cols-2">
                        <DetailField
                          label="Imported price"
                          value={
                            transaction.rate
                              ? `${currencyFormatter.format(transaction.rate)} / BTC`
                              : "None"
                          }
                          hidden={hideSensitive}
                          hint="The price that came in with the import. Kept here as audit reference even if you override it."
                        />
                        <DetailField
                          label="Spot now"
                          value={
                            nowRate
                              ? `${formatRateAtTime(nowRate)} / BTC`
                              : "Unknown"
                          }
                          hidden={hideSensitive}
                          hint="Current cached spot rate. Useful for sanity-checking a manual override."
                        />
                      </div>
                      {hasCacheProvenance ? (
                        <div
                          className={cn(
                            "rounded-md border bg-background p-3",
                            isCoarsePricing &&
                              "border-amber-500/40 bg-amber-500/10",
                          )}
                        >
                          <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                            <div className="flex items-center gap-1.5 text-[10px] font-medium uppercase text-muted-foreground">
                              {isCoarsePricing ? (
                                <AlertTriangle
                                  className="size-3 text-amber-600 dark:text-amber-400"
                                  aria-hidden="true"
                                />
                              ) : null}
                              Rate cache source
                            </div>
                            <Badge variant="outline" className="rounded-md">
                              {pricingQualityLabel(localDraft.pricingQuality)}
                            </Badge>
                          </div>
                          <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-3">
                            <div>
                              <span className="font-medium text-foreground">
                                Provider
                              </span>
                              <br />
                              {pricingProviderLabel(transaction.pricingProvider) ??
                                "Unknown"}
                            </div>
                            <div>
                              <span className="font-medium text-foreground">
                                Pair / granularity
                              </span>
                              <br />
                              {[transaction.pricingPair, transaction.pricingGranularity]
                                .filter(Boolean)
                                .join(" · ") || "Unknown"}
                            </div>
                            <div>
                              <span className="font-medium text-foreground">
                                {pricePoint.label}
                              </span>
                              <br />
                              {pricePoint.value}
                            </div>
                          </div>
                          {transaction.pricingMethod ? (
                            <p className="mt-2 text-xs text-muted-foreground">
                              Method: {transaction.pricingMethod}
                            </p>
                          ) : null}
                          {isCoarsePricing || isProviderSamplePricing ? (
                            <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border bg-background/70 p-3">
                              <div className="min-w-0 text-xs text-muted-foreground">
                                <div className="font-medium text-foreground">
                                  {isCoarsePricing
                                    ? "Fallback only"
                                    : "Provider sample"}
                                </div>
                                <p className="mt-1">
                                  {isCoarsePricing
                                    ? "Daily Kraken values are coarse coverage and stay reviewable until you confirm a better source."
                                    : "Minute market candles are sampled provider rates, not exchange execution evidence."}
                                </p>
                              </div>
                              <div className="flex shrink-0 flex-wrap gap-2">
                                <Button
                                  type="button"
                                  variant="outline"
                                  size="sm"
                                  onClick={chooseExactManualPrice}
                                >
                                  <Save className="size-3.5" aria-hidden="true" />
                                  Exact manual price
                                </Button>
                                {onOpenMarketDataSettings ? (
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    onClick={openMarketDataSettings}
                                  >
                                    <SlidersHorizontal
                                      className="size-3.5"
                                      aria-hidden="true"
                                    />
                                    Open rate settings
                                  </Button>
                                ) : null}
                              </div>
                            </div>
                          ) : null}
                        </div>
                      ) : null}
                      {isExactPricing && !hasCacheProvenance ? (
                        <div className="rounded-md border bg-emerald-500/10 p-3 text-xs text-muted-foreground">
                          <div className="font-medium text-foreground">
                            Exact pricing evidence
                          </div>
                          <p className="mt-1">
                            This row is marked exact because it comes from a
                            reviewed manual value or source-provided fiat record.
                          </p>
                        </div>
                      ) : null}
                      {isPricingMissing && !hasCacheProvenance ? (
                        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
                          <div className="flex items-start gap-2">
                            <AlertTriangle
                              className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
                              aria-hidden="true"
                            />
                            <div className="min-w-0 flex-1">
                              <div className="text-sm font-medium">
                                No cached spot yet
                              </div>
                              <p className="mt-1 text-xs text-muted-foreground">
                                Newer transactions can sit ahead of the local
                                rate cache. Import or fetch rates, or enter an
                                exact reviewed value.
                              </p>
                              <div className="mt-3 flex flex-wrap gap-2">
                                {onOpenMarketDataSettings ? (
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    onClick={openMarketDataSettings}
                                  >
                                    <SlidersHorizontal
                                      className="size-3.5"
                                      aria-hidden="true"
                                    />
                                    Open rate settings
                                  </Button>
                                ) : null}
                                <Button
                                  type="button"
                                  variant="outline"
                                  size="sm"
                                  onClick={chooseExactManualPrice}
                                >
                                  <Save className="size-3.5" aria-hidden="true" />
                                  Manual exact price
                                </Button>
                              </div>
                            </div>
                          </div>
                        </div>
                      ) : null}
                    </div>
                  </TabsContent>


    </>
  );
}
