import type { ParseKeys } from "i18next";
import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { TabsContent } from "@/components/ui/tabs";

import { DirtyDot, InfoHint, LedgerRow } from "./TransactionDetailSheetParts";
import {
  austrianSelectionValue,
  austrianTaxClassificationForValue,
  austrianTaxClassificationOptions,
  blurClass,
  currencyFormatter,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";
import {
  summarizeTransactionTaxEffect,
  type TaxTranslationKey,
} from "./TransactionTaxModel";

export function TransactionTaxTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation(["transactions"]);
  const {
    transaction,
    localDraft,
    dirty,
    dirtyExcluded,
    dirtyReviewTax,
    flow,
    taxNarrative,
    hideSensitive,
    updateDraft,
    journalEvents,
    isBasisQuarantine,
  } = ctx;
  const taxEffect = summarizeTransactionTaxEffect(journalEvents, flow);
  const showBasisQuarantineGuidance = Boolean(
    isBasisQuarantine && !localDraft.excluded,
  );
  const taxEffectValue = (
    value: number | null,
    fallbackKey?: TaxTranslationKey,
  ) => {
    if (value === null) return t(fallbackKey ?? "tax.journalPending");
    return (
      <span className={blurClass(hideSensitive)}>
        {currencyFormatter.format(value)}
      </span>
    );
  };
  return (
    <>
                  {/* Tax — owns Austrian classification, taxable, excluded; ends with gain/loss */}
                  <TabsContent value="tax" className="mt-4 space-y-3">
                    <div className="rounded-md border bg-muted/50 p-3 text-sm leading-relaxed">
                      <div className="mb-1 flex items-center gap-1.5 text-xs font-semibold uppercase text-muted-foreground">
                        {t("tax.plainEnglish")}
                        <InfoHint label={t("tax.plainEnglish")}>
                          {t("tax.plainEnglishHint")}
                        </InfoHint>
                      </div>
                      <p className={blurClass(hideSensitive)}>{taxNarrative}</p>
                    </div>
                    {showBasisQuarantineGuidance ? (
                      <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
                        <div className="flex items-start gap-2">
                          <AlertTriangle
                            className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
                            aria-hidden="true"
                          />
                          <div className="min-w-0">
                            <div className="font-medium">
                              {t("tax.basisBlockerTitle")}
                            </div>
                            <p className="mt-1 text-xs text-muted-foreground">
                              {t("tax.basisBlockerBody", {
                                asset: transaction.asset ?? "asset",
                              })}
                            </p>
                          </div>
                        </div>
                      </div>
                    ) : null}
                    <div className="rounded-md border bg-background p-3">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
                          {t("tax.handling")}
                          <DirtyDot active={dirtyExcluded || dirtyReviewTax} />
                        </h3>
                        <Badge
                          variant={
                            localDraft.taxable && !localDraft.excluded
                              ? "default"
                              : "outline"
                          }
                        >
                          {localDraft.excluded
                            ? t("taxable.excluded")
                            : localDraft.taxable
                              ? t("taxable.taxable")
                              : t("taxable.notTaxable")}
                        </Badge>
                      </div>
                      <div className="grid gap-3 xl:grid-cols-[minmax(220px,0.9fr)_minmax(0,1fr)_minmax(0,1fr)]">
                        <div className="grid gap-2">
                          <Label
                            htmlFor="tx-tax-treatment"
                            className="flex items-center gap-1.5"
                          >
                            {t("tax.austrianCategory")}
                            <DirtyDot active={dirty.atRegime || dirty.atCategory} />
                            <InfoHint label={t("tax.austrianCategory")}>
                              {t("tax.austrianCategoryHint")}
                            </InfoHint>
                          </Label>
                          <Select
                            value={austrianSelectionValue(
                              localDraft.atRegime,
                              localDraft.atCategory,
                            )}
                            onValueChange={(value) => {
                              const option =
                                austrianTaxClassificationForValue(value);
                              updateDraft("atRegime", option.atRegime);
                              updateDraft("atCategory", option.atCategory);
                              updateDraft("taxable", option.taxable);
                            }}
                          >
                            <SelectTrigger id="tx-tax-treatment">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {austrianTaxClassificationOptions.map(
                                (option) => (
                                  <SelectItem
                                    key={option.value}
                                    value={option.value}
                                  >
                                    {/* dynamic key */}
                                    {t(option.label as ParseKeys<["transactions"]>)}
                                  </SelectItem>
                                ),
                              )}
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label
                              htmlFor="tx-taxable"
                              className="flex items-center gap-1.5"
                            >
                              {t("tax.taxable")}
                              <DirtyDot active={dirty.taxable} />
                            </Label>
                            <p className="text-xs text-muted-foreground">
                              {t("tax.taxableHint")}
                            </p>
                          </div>
                          <Switch
                            id="tx-taxable"
                            checked={localDraft.taxable}
                            onCheckedChange={(checked) =>
                              updateDraft("taxable", checked)
                            }
                          />
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label
                              htmlFor="tx-excluded"
                              className="flex items-center gap-1.5"
                            >
                              {t("tax.excluded")}
                              <DirtyDot active={dirtyExcluded} />
                              <span className="text-xs font-normal text-muted-foreground">
                                (<kbd className="rounded border bg-muted px-1">e</kbd>)
                              </span>
                            </Label>
                            <p className="text-xs text-muted-foreground">
                              {t("tax.excludedHint")}
                            </p>
                          </div>
                          <Switch
                            id="tx-excluded"
                            checked={localDraft.excluded}
                            onCheckedChange={(checked) =>
                              updateDraft("excluded", checked)
                            }
                          />
                        </div>
                      </div>
                    </div>
                    <div className="overflow-hidden rounded-md border">
                      <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        {t("tax.projectedEffect")}
                      </div>
                      <LedgerRow
                        label={t(taxEffect.costBasisLabelKey)}
                        value={taxEffectValue(
                          taxEffect.costBasisEur,
                          taxEffect.costBasisFallbackKey,
                        )}
                        align="right"
                        hint={t("tax.costBasisHint")}
                      />
                      <LedgerRow
                        label={t(taxEffect.proceedsLabelKey)}
                        value={taxEffectValue(
                          taxEffect.proceedsEur,
                          taxEffect.proceedsFallbackKey,
                        )}
                        align="right"
                        hint={t("tax.proceedsHint")}
                      />
                      <LedgerRow
                        label={t(taxEffect.gainLossLabelKey)}
                        value={taxEffectValue(
                          taxEffect.gainLossEur,
                          taxEffect.gainLossFallbackKey,
                        )}
                        align="right"
                        muted={
                          taxEffect.state !== "disposal" &&
                          taxEffect.state !== "income"
                        }
                        hint={t("tax.gainLossHint")}
                      />
                      {localDraft.pricingSourceKind === "manual_override" ? (
                        <LedgerRow
                          label={t("tax.priceEvidence")}
                          value={
                            <span className={blurClass(hideSensitive)}>
                              {localDraft.manualSource || t("tax.sourceMissing")}
                            </span>
                          }
                          align="right"
                          muted
                        />
                      ) : null}
                    </div>
                  </TabsContent>


    </>
  );
}
