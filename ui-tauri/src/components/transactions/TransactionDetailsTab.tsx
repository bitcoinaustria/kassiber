import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { TabsContent } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

import {
  DetailField,
  DirtyDot,
  LedgerRow,
  networkLabel,
} from "./TransactionDetailSheetParts";
import { CommercialProvenancePanel } from "./TransactionDetailCommercialPanel";
import {
  blurClass,
  currencyFormatter,
  formatFee,
  formatShortTxid,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionDetailsTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const {
    transaction,
    localDraft,
    dirtyLabel,
    dirtyTags,
    dirtyNote,
    dirtyExcluded,
    transactionDisplayId,
    hideSensitive,
    feeBtc,
    commercialContext,
    commercialContextLoading,
    showSourceExternalId,
    updateDraft,
    tags,
    currency,
  } = ctx;
  return (
    <>
                  {/* Details — read-only source-of-record + book metadata */}
                  <TabsContent value="details" className="mt-4 space-y-4">
                    <div className="grid gap-3 sm:grid-cols-3">
                      <DetailField
                        label={t("details.transactionId")}
                        value={formatShortTxid(transactionDisplayId)}
                        copyValue={transactionDisplayId}
                        hidden={hideSensitive}
                        hint={t("details.transactionIdHint")}
                      />
                      <DetailField
                        label={t("details.priceAtTime")}
                        value={
                          localDraft.pricingSourceKind === "manual_override" &&
                          localDraft.manualPrice
                            ? t("details.manualPerBtc", {
                                price: localDraft.manualPrice,
                                currency: localDraft.manualCurrency,
                              })
                            : transaction.rate
                              ? t("details.perBtc", {
                                  value: currencyFormatter.format(transaction.rate),
                                })
                              : t("details.priceMissing")
                        }
                        hidden={hideSensitive}
                        hint={t("details.priceAtTimeHint")}
                      />
                      <DetailField
                        label={t("details.fee")}
                        value={
                          feeBtc ? (
                            <CurrencyToggleText
                              className={blurClass(hideSensitive)}
                            >
                              {formatFee(transaction, currency)}
                            </CurrencyToggleText>
                          ) : (
                            t("details.feeNone")
                          )
                        }
                        hidden={hideSensitive}
                        hint={t("details.feeHint")}
                      />
                    </div>
                    <CommercialProvenancePanel
                      context={commercialContext}
                      loading={commercialContextLoading}
                      hidden={hideSensitive}
                    />
                    <div className="grid gap-3 lg:grid-cols-2">
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          {t("details.sourceRecord")}
                        </div>
                        <LedgerRow
                          label={t("details.type")}
                          value={
                            transaction.sourceType ?? transaction.direction
                          }
                        />
                        <LedgerRow
                          label={t("details.network")}
                          value={networkLabel(transaction)}
                        />
                        <LedgerRow
                          label={t("details.counterparty")}
                          value={transaction.counterparty}
                        />
                        {showSourceExternalId ? (
                          <LedgerRow
                            label={t("details.externalId")}
                            value={formatShortTxid(transaction.txnId)}
                            hint={t("details.externalIdHint")}
                          />
                        ) : null}
                      </div>
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          {t("details.bookMetadata")}
                        </div>
                        <LedgerRow
                          label={t("details.label")}
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.label}
                              <DirtyDot active={dirtyLabel} />
                            </span>
                          }
                        />
                        <LedgerRow
                          label={t("details.tags")}
                          value={
                            tags.length ? (
                              <div
                                className={cn(
                                  "flex flex-wrap items-center gap-1",
                                  blurClass(hideSensitive),
                                )}
                              >
                                {tags.map((tag) => (
                                  <Badge
                                    key={tag}
                                    variant="secondary"
                                    className="rounded-md"
                                  >
                                    {tag}
                                  </Badge>
                                ))}
                                {dirtyTags ? <DirtyDot active /> : null}
                              </div>
                            ) : (
                              <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                                {t("details.tagsNone")}
                                <DirtyDot active={dirtyTags} />
                              </span>
                            )
                          }
                        />
                        <LedgerRow
                          label={t("details.included")}
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.excluded
                                ? t("details.includedNo")
                                : t("details.includedYes")}
                              <DirtyDot active={dirtyExcluded} />
                            </span>
                          }
                        />
                      </div>
                    </div>
                    <div className="grid gap-2">
                      <Label
                        htmlFor="tx-detail-note"
                        className="flex items-center gap-1.5"
                      >
                        {t("details.note")}
                        <DirtyDot active={dirtyNote} />
                      </Label>
                      <Textarea
                        id="tx-detail-note"
                        value={localDraft.note}
                        onChange={(event) =>
                          updateDraft("note", event.target.value)
                        }
                        className={cn(
                          "min-h-24 resize-none",
                          blurClass(hideSensitive),
                        )}
                        placeholder={t("details.notePlaceholder")}
                      />
                    </div>
                  </TabsContent>


    </>
  );
}
