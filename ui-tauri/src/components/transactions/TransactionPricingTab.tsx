import type { ParseKeys } from "i18next";
import { AlertTriangle, Save, SlidersHorizontal } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

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
  const { t } = useTranslation(["transactions"]);
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
    suppressPricingCacheWarning,
    pricePoint,
    nowRate,
    onOpenMarketDataSettings,
    openMarketDataSettings,
    chooseExactManualPrice,
  } = ctx;
  const showMissingCacheWarning =
    isPricingMissing && !hasCacheProvenance && !suppressPricingCacheWarning;

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
                              {/* dynamic key */}
                              {t(option.label as ParseKeys<["transactions"]>)}
                            </div>
                            <div className="mt-1 text-xs text-muted-foreground">
                              {option.description
                                ? // dynamic key
                                  t(option.description as ParseKeys<["transactions"]>)
                                : null}
                            </div>
                          </button>
                        ))}
                      </div>
                      <div className="grid gap-3 rounded-md border bg-muted/50 p-3">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div>
                            <div className="flex items-center gap-1.5 text-sm font-medium">
                              {t("pricing.manualOverride")}
                              <DirtyDot active={dirtyPricing} />
                              <InfoHint label={t("pricing.manualOverride")}>
                                {t("pricing.manualOverrideHint")}
                              </InfoHint>
                            </div>
                            <div className="text-xs text-muted-foreground">
                              {t("pricing.calculatedFromAmount", {
                                amount: formatBtcAmount(amountBtc),
                              })}
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
                            {/* dynamic key */}
                            {t(
                              pricingSourceLabel(
                                localDraft.pricingSourceKind,
                                localDraft.pricingQuality,
                              ) as ParseKeys<["transactions"]>,
                            )}
                          </Badge>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[100px_1fr_1fr]">
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-currency">{t("pricing.currency")}</Label>
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
                            <Label htmlFor="tx-manual-price">{t("pricing.pricePerBtc")}</Label>
                            <Input
                              id="tx-manual-price"
                              ref={manualPriceRef}
                              inputMode="decimal"
                              value={localDraft.manualPrice}
                              onChange={(event) =>
                                updateManualPrice(event.target.value)
                              }
                              placeholder={t("pricing.manualPricePlaceholder")}
                            />
                          </div>
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-value">{t("pricing.totalValue")}</Label>
                            <Input
                              id="tx-manual-value"
                              inputMode="decimal"
                              value={localDraft.manualValue}
                              onChange={(event) =>
                                updateManualValue(event.target.value)
                              }
                              placeholder={t("pricing.manualValuePlaceholder")}
                            />
                          </div>
                        </div>
                        <div className="grid gap-2">
                          <Label
                            htmlFor="tx-manual-source"
                            className="flex items-center gap-1.5"
                          >
                            {t("pricing.evidenceSource")}
                            <InfoHint label={t("pricing.evidenceSource")}>
                              {t("pricing.evidenceHint")}
                            </InfoHint>
                          </Label>
                          <Input
                            id="tx-manual-source"
                            value={localDraft.manualSource}
                            className={blurClass(hideSensitive)}
                            onChange={(event) =>
                              updateDraft("manualSource", event.target.value)
                            }
                            placeholder={t("pricing.evidencePlaceholder")}
                          />
                          <p className="text-[11px] text-muted-foreground">
                            <Trans
                              i18nKey="pricing.attachHint"
                              ns="transactions"
                              components={[<span className="font-medium" />]}
                            />
                          </p>
                        </div>
                      </div>
                      <div className="grid gap-3 sm:grid-cols-2">
                        <DetailField
                          label={t("pricing.importedPrice")}
                          value={
                            transaction.rate
                              ? t("pricing.perBtc", {
                                  value: currencyFormatter.format(transaction.rate),
                                })
                              : t("pricing.importedPriceNone")
                          }
                          hidden={hideSensitive}
                          hint={t("pricing.importedPriceHint")}
                        />
                        <DetailField
                          label={t("pricing.spotNow")}
                          value={
                            nowRate
                              ? t("pricing.perBtc", {
                                  value: formatRateAtTime(nowRate),
                                })
                              : t("pricing.spotNowUnknown")
                          }
                          hidden={hideSensitive}
                          hint={t("pricing.spotNowHint")}
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
                              {t("pricing.rateCacheSource")}
                            </div>
                            <Badge variant="outline" className="rounded-md">
                              {t(pricingQualityLabel(localDraft.pricingQuality))}
                            </Badge>
                          </div>
                          <div className="grid gap-2 text-xs text-muted-foreground sm:grid-cols-3">
                            <div>
                              <span className="font-medium text-foreground">
                                {t("pricing.provider")}
                              </span>
                              <br />
                              {pricingProviderLabel(
                                transaction.pricingProvider,
                                t as (key: string) => string, // loose translator
                              ) ?? t("pricing.providerUnknown")}
                            </div>
                            <div>
                              <span className="font-medium text-foreground">
                                {t("pricing.pairGranularity")}
                              </span>
                              <br />
                              {[transaction.pricingPair, transaction.pricingGranularity]
                                .filter(Boolean)
                                .join(" · ") || t("pricing.pairGranularityUnknown")}
                            </div>
                            <div>
                              <span className="font-medium text-foreground">
                                {/* dynamic key */}
                                {t(pricePoint.label as ParseKeys<["transactions"]>)}
                              </span>
                              <br />
                              {pricePoint.value}
                            </div>
                          </div>
                          {transaction.pricingMethod ? (
                            <p className="mt-2 text-xs text-muted-foreground">
                              {t("pricing.method", {
                                method: transaction.pricingMethod,
                              })}
                            </p>
                          ) : null}
                          {isCoarsePricing || isProviderSamplePricing ? (
                            <div className="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-md border bg-background/70 p-3">
                              <div className="min-w-0 text-xs text-muted-foreground">
                                <div className="font-medium text-foreground">
                                  {isCoarsePricing
                                    ? t("pricing.fallbackOnly")
                                    : t("pricing.providerSample")}
                                </div>
                                <p className="mt-1">
                                  {isCoarsePricing
                                    ? t("pricing.fallbackOnlyBody")
                                    : t("pricing.providerSampleBody")}
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
                                  {t("pricing.exactManualPrice")}
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
                                    {t("pricing.openRateSettings")}
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
                            {t("pricing.exactEvidence")}
                          </div>
                          <p className="mt-1">
                            {t("pricing.exactEvidenceBody")}
                          </p>
                        </div>
                      ) : null}
                      {showMissingCacheWarning ? (
                        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
                          <div className="flex items-start gap-2">
                            <AlertTriangle
                              className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
                              aria-hidden="true"
                            />
                            <div className="min-w-0 flex-1">
                              <div className="text-sm font-medium">
                                {t("pricing.noCachedSpot")}
                              </div>
                              <p className="mt-1 text-xs text-muted-foreground">
                                {t("pricing.noCachedSpotBody")}
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
                                    {t("pricing.openRateSettings")}
                                  </Button>
                                ) : null}
                                <Button
                                  type="button"
                                  variant="outline"
                                  size="sm"
                                  onClick={chooseExactManualPrice}
                                >
                                  <Save className="size-3.5" aria-hidden="true" />
                                  {t("pricing.manualExactPrice")}
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
