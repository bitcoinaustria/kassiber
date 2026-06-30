import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { TabsContent } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon } from "@/daemon/client";
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
  formatBtcAmount,
  formatFee,
  formatShortTxid,
  SATS_PER_BTC,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";
import {
  TransactionGraphPanel,
  type TransactionGraphPayload,
  type TransactionSwapRoute,
  type TransactionSwapRouteLegKey,
} from "./TransactionGraphTab";

function graphWithPairFallbackRoute(
  graphData: TransactionGraphPayload | undefined,
  transaction: TransactionDetailTabContext["transaction"],
): TransactionGraphPayload | undefined {
  if (!graphData || graphData.swapRoute || !transaction.pair) return graphData;
  const pair = transaction.pair;
  const currentLeg =
    transaction.wallet && pair.outWallet && transaction.wallet === pair.outWallet
      ? "out"
      : transaction.wallet && pair.inWallet && transaction.wallet === pair.inWallet
        ? "in"
        : transaction.direction === "Send"
          ? "out"
          : "in";
  const currentReference = transaction.explorerId || transaction.txnId;
  const route: TransactionSwapRoute = {
    id: pair.id,
    kind: pair.kind || pair.type,
    routeKind: fallbackRouteKind(pair),
    policy: pair.policy,
    currentLeg,
    swapFeeBtc:
      typeof pair.feeSat === "number"
        ? Math.abs(pair.feeSat) / SATS_PER_BTC
        : null,
    swapFeeKind: pair.feeKind,
    out: {
      id: currentLeg === "out" ? transaction.id : undefined,
      externalId: currentLeg === "out" ? currentReference : undefined,
      txid: currentLeg === "out" ? currentReference : undefined,
      direction: "outbound",
      role: fallbackSwapOutRole(pair),
      asset: pair.outAsset,
      network: routeNetwork(pair.outAsset, pair.outWallet),
      amountBtc:
        typeof pair.outAmountSat === "number"
          ? Math.abs(pair.outAmountSat) / SATS_PER_BTC
          : null,
      wallet: { label: pair.outWallet },
      counterparty: transaction.counterparty,
    },
    in: {
      id: currentLeg === "in" ? transaction.id : undefined,
      externalId: currentLeg === "in" ? currentReference : undefined,
      txid: currentLeg === "in" ? currentReference : undefined,
      direction: "inbound",
      role: "receive",
      asset: pair.inAsset,
      network: routeNetwork(pair.inAsset, pair.inWallet),
      amountBtc:
        typeof pair.inAmountSat === "number"
          ? Math.abs(pair.inAmountSat) / SATS_PER_BTC
          : null,
      wallet: { label: pair.inWallet },
      counterparty: transaction.counterparty,
    },
  };
  return { ...graphData, swapRoute: route };
}

function fallbackSwapOutRole(pair: NonNullable<TransactionDetailTabContext["transaction"]["pair"]>) {
  if (fallbackRouteKind(pair) !== "swap") return "spend" as const;
  const kind = String(pair.kind || pair.type || "").toLowerCase();
  const outNetwork = routeNetwork(pair.outAsset, pair.outWallet);
  const inNetwork = routeNetwork(pair.inAsset, pair.inWallet);
  if (kind.includes("swap") && outNetwork === "Liquid" && outNetwork !== inNetwork) {
    return "consolidation" as const;
  }
  return "spend" as const;
}

function fallbackRouteKind(pair: NonNullable<TransactionDetailTabContext["transaction"]["pair"]>) {
  const kind = String(pair.kind || pair.type || "").toLowerCase();
  if (kind.includes("coinjoin") || kind.includes("whirlpool")) return "coinjoin";
  if (
    kind.includes("swap") ||
    kind.startsWith("peg-") ||
    String(pair.outAsset || "").toUpperCase() !== String(pair.inAsset || "").toUpperCase()
  ) {
    return "swap";
  }
  if (pair.policy === "carrying-value") return "transfer";
  return "pair";
}

function routeNetwork(asset?: string | null, wallet?: string | null) {
  const assetText = String(asset || "").toUpperCase();
  const walletText = String(wallet || "").toLowerCase();
  if (assetText === "LBTC" || assetText === "L-BTC" || walletText.includes("liquid")) {
    return "Liquid";
  }
  if (assetText === "BTC") return "Bitcoin";
  return asset || undefined;
}

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
    graphData,
    graphLoading,
    graphError,
  } = ctx;
  const displayGraphData = graphWithPairFallbackRoute(graphData, transaction);
  const swapRoute = displayGraphData?.swapRoute ?? null;
  const [selectedSwapLeg, setSelectedSwapLeg] = useState<TransactionSwapRouteLegKey | null>(null);
  useEffect(() => {
    setSelectedSwapLeg(null);
  }, [transaction.id, swapRoute?.id]);
  const activeSwapLeg = selectedSwapLeg ?? swapRoute?.currentLeg ?? null;
  const selectedSwapTransactionRef = useMemo(() => {
    if (!swapRoute || !activeSwapLeg || activeSwapLeg === swapRoute.currentLeg) {
      return null;
    }
    const leg = swapRoute[activeSwapLeg];
    return leg.id || leg.txid || leg.externalId || null;
  }, [activeSwapLeg, swapRoute]);
  const selectedSwapGraphQuery = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    { transaction: selectedSwapTransactionRef ?? "" },
    { enabled: Boolean(selectedSwapTransactionRef) },
  );
  const selectedSwapGraphData = selectedSwapGraphQuery.data?.data;
  const activeGraphData = selectedSwapTransactionRef
    ? selectedSwapGraphData
    : displayGraphData;
  const graphPanelLoading = selectedSwapTransactionRef
    ? selectedSwapGraphQuery.isLoading ||
      (selectedSwapGraphQuery.isFetching && !selectedSwapGraphData)
    : graphLoading;
  const graphPanelError = selectedSwapTransactionRef
    ? selectedSwapGraphQuery.error instanceof Error
      ? selectedSwapGraphQuery.error.message
      : null
    : graphError;
  const graphTx = activeGraphData?.transaction;
  const graphNetworkFeeBtc =
    typeof activeGraphData?.fee?.valueBtc === "number"
      ? activeGraphData.fee.valueBtc
      : typeof activeGraphData?.fee?.valueSats === "number"
        ? activeGraphData.fee.valueSats / SATS_PER_BTC
        : 0;
  const technicalRows = graphTx
    ? [
        [t("details.inputCount"), graphTx.inputCount ?? activeGraphData.inputs.length],
        [t("details.outputCount"), graphTx.outputCount ?? activeGraphData.outputs.length],
        [
          t("details.networkFee"),
          graphNetworkFeeBtc ? formatBtcAmount(graphNetworkFeeBtc) : t("details.unknown"),
        ],
        [
          t("details.feeRate"),
          graphTx.feeRateSatVb ? `${graphTx.feeRateSatVb} sat/vB` : t("details.unknown"),
        ],
        [t("details.version"), graphTx.version ?? t("details.unknown")],
        [t("details.locktime"), graphTx.locktime ?? t("details.unknown")],
        [t("details.size"), graphTx.size ? `${graphTx.size} B` : t("details.unknown")],
        [t("details.vsize"), graphTx.vsize ? `${graphTx.vsize} vB` : t("details.unknown")],
        [t("details.weight"), graphTx.weight ? `${graphTx.weight} WU` : t("details.unknown")],
      ]
    : [];
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
                          ) : graphNetworkFeeBtc ? (
                            <CurrencyToggleText
                              className={blurClass(hideSensitive)}
                            >
                              {t("details.senderPaidNetworkFee", {
                                value: formatBtcAmount(graphNetworkFeeBtc),
                              })}
                            </CurrencyToggleText>
                          ) : (
                            t("details.feeNone")
                          )
                        }
                        hidden={hideSensitive}
                        hint={t("details.feeHint")}
                      />
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
                    <div className="overflow-hidden rounded-md border">
                      <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        {t("graph.sectionTitle")}
                      </div>
                      <div className="p-3">
                        <TransactionGraphPanel
                          graph={activeGraphData}
                          loading={graphPanelLoading}
                          error={graphPanelError}
                          hideSensitive={hideSensitive}
                          selectedSwapLeg={activeSwapLeg}
                          onSelectSwapLeg={setSelectedSwapLeg}
                        />
                      </div>
                    </div>
                    {technicalRows.length ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          {t("details.technical")}
                        </div>
                        <div className="grid sm:grid-cols-2">
                          {technicalRows.map(([label, value]) => (
                            <LedgerRow
                              key={String(label)}
                              label={String(label)}
                              value={value}
                            />
                          ))}
                        </div>
                      </div>
                    ) : null}
                    <CommercialProvenancePanel
                      context={commercialContext}
                      loading={commercialContextLoading}
                      hidden={hideSensitive}
                    />
                  </TabsContent>


    </>
  );
}
