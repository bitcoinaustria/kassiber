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
                        label="Transaction ID"
                        value={formatShortTxid(transactionDisplayId)}
                        copyValue={transactionDisplayId}
                        hidden={hideSensitive}
                        hint="Canonical on-chain identifier or import row id, depending on the source."
                      />
                      <DetailField
                        label="Price at time"
                        value={
                          localDraft.pricingSourceKind === "manual_override" &&
                          localDraft.manualPrice
                            ? `${localDraft.manualPrice} ${localDraft.manualCurrency}/BTC`
                            : transaction.rate
                              ? `${currencyFormatter.format(transaction.rate)} / BTC`
                              : "Missing"
                        }
                        hidden={hideSensitive}
                        hint="BTC/fiat rate used to value this tx at the time it occurred."
                      />
                      <DetailField
                        label="Fee"
                        value={
                          feeBtc ? (
                            <CurrencyToggleText
                              className={blurClass(hideSensitive)}
                            >
                              {formatFee(transaction, currency)}
                            </CurrencyToggleText>
                          ) : (
                            "None"
                          )
                        }
                        hidden={hideSensitive}
                        hint="Network or settlement fee paid for this transaction."
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
                          Source record
                        </div>
                        <LedgerRow
                          label="Type"
                          value={
                            transaction.sourceType ?? transaction.direction
                          }
                        />
                        <LedgerRow
                          label="Network"
                          value={networkLabel(transaction)}
                        />
                        <LedgerRow
                          label="Counterparty"
                          value={transaction.counterparty}
                        />
                        {showSourceExternalId ? (
                          <LedgerRow
                            label="External id"
                            value={formatShortTxid(transaction.txnId)}
                            hint="Wallet/exchange internal id. Different from the on-chain Transaction ID for off-chain sources."
                          />
                        ) : null}
                      </div>
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          Book metadata
                        </div>
                        <LedgerRow
                          label="Label"
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.label}
                              <DirtyDot active={dirtyLabel} />
                            </span>
                          }
                        />
                        <LedgerRow
                          label="Tags"
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
                                None
                                <DirtyDot active={dirtyTags} />
                              </span>
                            )
                          }
                        />
                        <LedgerRow
                          label="Included"
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.excluded ? "Excluded" : "Included"}
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
                        Note
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
                        placeholder="Receipt, invoice, counterparty, or review context"
                      />
                    </div>
                  </TabsContent>


    </>
  );
}
