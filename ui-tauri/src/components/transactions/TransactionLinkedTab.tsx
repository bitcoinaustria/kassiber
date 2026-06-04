import { FileText, Repeat2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TabsContent } from "@/components/ui/tabs";

import { LedgerRow } from "./TransactionDetailSheetParts";
import { blurClass, currencyFormatter, formatBtcAmount } from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";

export function TransactionLinkedTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const {
    pair,
    onUnpair,
    isUnpairing,
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
                            Paired movement
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
                            {isUnpairing ? "Unpairing..." : "Unpair"}
                          </Button>
                        </div>
                        <LedgerRow
                          label="Out wallet"
                          value={pair.outWallet ?? "Unknown"}
                          align="right"
                        />
                        <LedgerRow
                          label="Out amount"
                          value={`${Math.abs(
                            (pair.outAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.outAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label="In wallet"
                          value={pair.inWallet ?? "Unknown"}
                          align="right"
                        />
                        <LedgerRow
                          label="In amount"
                          value={`${Math.abs(
                            (pair.inAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.inAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label="Pair fee"
                          value={
                            pair.feeSat
                              ? formatBtcAmount(
                                  Math.abs(pair.feeSat / 100_000_000),
                                )
                              : "-"
                          }
                          align="right"
                          muted
                          hint="Signed fee computed at pair time."
                        />
                        {pair.kind ? (
                          <LedgerRow
                            label="Pair kind"
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
                          No paired movement
                        </div>
                        <p className="mt-1 text-xs text-muted-foreground">
                          When this tx is the other leg of a transfer, swap, or
                          peg-in/out, the matched leg shows up here. You can
                          also create a manual pair from the swap candidate
                          queue.
                        </p>
                      </div>
                    )}

                    <div className="overflow-hidden rounded-md border">
                      <div className="flex items-center gap-2 border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        <FileText
                          className="size-3"
                          aria-hidden="true"
                        />
                        Journal entries
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
                          No journal entries for this row yet. Process journals
                          after metadata or pricing changes.
                        </div>
                      )}
                    </div>
                  </TabsContent>


    </>
  );
}
