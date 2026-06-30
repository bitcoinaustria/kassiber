import { Coins, FileText, Repeat2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TabsContent } from "@/components/ui/tabs";

import { LedgerRow } from "./TransactionDetailSheetParts";
import { blurClass, currencyFormatter, formatBtcAmount } from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

function loanRoleLabelKey(role: string): string {
  if (role === "collateral_release") return "table.row.collateral.returnedAccounting";
  if (role === "loan_principal_received") {
    return "table.row.collateral.principalReceivedAccounting";
  }
  if (role === "loan_principal_repaid") {
    return "table.row.collateral.principalRepaidAccounting";
  }
  return "table.row.collateral.collateralAccounting";
}

function shortTransactionId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 8)}...${id.slice(-4)}` : id;
}

export function TransactionLinkedTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const {
    transaction,
    pair,
    loanMark,
    linkedLoanMarks,
    loanLinkCandidates,
    onUnpair,
    isUnpairing,
    onLinkLoan,
    isLoanLinking,
    journalEvents,
    hideSensitive,
  } = ctx;
  return (
    <>
                  {/* Linked — pairs, source-of-funds, journal entries */}
                  <TabsContent value="linked" className="mt-4 space-y-3">
                    {pair ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="flex items-center justify-between border-b bg-muted px-3 py-1.5">
                          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                            <Repeat2
                              className="size-3"
                              aria-hidden="true"
                            />
                            {t("linked.pairedMovement")}
                            {pair.policy ? (
                              <Badge
                                variant="outline"
                                className="rounded-md text-[10px]"
                              >
                                {pair.policy}
                              </Badge>
                            ) : null}
                          </div>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-7 px-2 text-xs text-muted-foreground"
                            disabled={!onUnpair || isUnpairing}
                            onClick={() => onUnpair?.(pair.id)}
                          >
                            {isUnpairing ? t("linked.unpairing") : t("linked.unpair")}
                          </Button>
                        </div>
                        <LedgerRow
                          label={t("linked.outWallet")}
                          value={pair.outWallet ?? t("linked.unknown")}
                          align="right"
                        />
                        <LedgerRow
                          label={t("linked.outAmount")}
                          value={`${Math.abs(
                            (pair.outAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.outAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label={t("linked.inWallet")}
                          value={pair.inWallet ?? t("linked.unknown")}
                          align="right"
                        />
                        <LedgerRow
                          label={t("linked.inAmount")}
                          value={`${Math.abs(
                            (pair.inAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.inAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label={t("linked.pairFee")}
                          value={
                            pair.feeSat
                              ? formatBtcAmount(
                                  Math.abs(pair.feeSat / 100_000_000),
                                )
                              : "-"
                          }
                          align="right"
                          muted
                          hint={t("linked.pairFeeHint")}
                        />
                        {pair.kind ? (
                          <LedgerRow
                            label={t("linked.pairKind")}
                            value={pair.kind}
                            align="right"
                            muted
                          />
                        ) : null}
                      </div>
                    ) : (
                      <div className="rounded-md border border-dashed bg-muted/40 p-4 text-sm">
                        <div className="flex items-center gap-2 font-medium">
                          <Repeat2
                            className="size-4 text-muted-foreground"
                            aria-hidden="true"
                          />
                          {t("linked.noPairedMovement")}
                        </div>
                        <p className="mt-1 text-xs text-muted-foreground">
                          {t("linked.noPairedMovementBody")}
                        </p>
                      </div>
                    )}

                    {loanMark ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="flex items-center justify-between border-b bg-muted px-3 py-1.5">
                          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                            <Coins className="size-3" aria-hidden="true" />
                            {t("linked.loanLegs")}
                          </div>
                          {loanMark.loan_id ? (
                            <Badge variant="outline" className="rounded-md text-[10px]">
                              {t("linked.loanId", { id: shortTransactionId(loanMark.loan_id) })}
                            </Badge>
                          ) : null}
                        </div>
                        <LedgerRow
                          label={t("linked.loanThisLeg")}
                          value={(t as (key: string) => string)(loanRoleLabelKey(loanMark.role))}
                          align="right"
                        />
                        {linkedLoanMarks.length ? (
                          linkedLoanMarks.map((mark) => (
                            <div
                              key={mark.transaction_id}
                              className="flex items-center justify-between gap-3 border-t px-3 py-2 text-xs"
                            >
                              <div className="min-w-0">
                                <p className="truncate font-medium">
                                  {(t as (key: string) => string)(loanRoleLabelKey(mark.role))}
                                </p>
                                <p className="truncate text-muted-foreground">
                                  {mark.description || shortTransactionId(mark.transaction_id)}
                                  {mark.occurred_at ? ` · ${mark.occurred_at}` : ""}
                                </p>
                              </div>
                              <Badge variant="outline" className="shrink-0 rounded-md text-[10px]">
                                {t("linked.loanLinked")}
                              </Badge>
                            </div>
                          ))
                        ) : (
                          <div className="border-t px-3 py-2 text-xs text-muted-foreground">
                            {t("linked.noLoanLinks")}
                          </div>
                        )}
                        {loanLinkCandidates.length ? (
                          <div className="border-t bg-muted/30 px-3 py-2">
                            <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                              {t("linked.loanLinkCandidates")}
                            </p>
                            <div className="space-y-1.5">
                              {loanLinkCandidates.map((mark) => (
                                <div
                                  key={mark.transaction_id}
                                  className="flex items-center justify-between gap-3 rounded-md border bg-background px-2 py-1.5 text-xs"
                                >
                                  <div className="min-w-0">
                                    <p className="truncate font-medium">
                                      {(t as (key: string) => string)(
                                        loanRoleLabelKey(mark.role),
                                      )}
                                    </p>
                                    <p className="truncate text-muted-foreground">
                                      {mark.description || shortTransactionId(mark.transaction_id)}
                                      {mark.occurred_at ? ` · ${mark.occurred_at}` : ""}
                                    </p>
                                  </div>
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    className="h-7 shrink-0 px-2 text-xs"
                                    disabled={!onLinkLoan || isLoanLinking}
                                    onClick={() => {
                                      void onLinkLoan?.(transaction, mark.transaction_id);
                                    }}
                                  >
                                    {isLoanLinking ? t("linked.loanLinking") : t("linked.loanLink")}
                                  </Button>
                                </div>
                              ))}
                            </div>
                          </div>
                        ) : null}
                      </div>
                    ) : null}

                    <div className="overflow-hidden rounded-md border">
                      <div className="flex items-center gap-2 border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        <FileText
                          className="size-3"
                          aria-hidden="true"
                        />
                        {t("linked.journalEntries")}
                      </div>
                      {journalEvents.length ? (
                        journalEvents.map((entry) => (
                          <LedgerRow
                            key={entry.id}
                            label={`${entry.entryType}${entry.atCategory ? ` · ${entry.atCategory}` : ""}`}
                            value={
                              <span className={blurClass(hideSensitive)}>
                                {entry.quantity.toFixed(8)} {entry.asset} ·{" "}
                                {currencyFormatter.format(entry.fiatValueEur)}
                              </span>
                            }
                            align="right"
                            hint={entry.description || undefined}
                          />
                        ))
                      ) : (
                        <div className="p-3 text-xs text-muted-foreground">
                          {t("linked.noJournalEntries")}
                        </div>
                      )}
                    </div>
                  </TabsContent>


    </>
  );
}
