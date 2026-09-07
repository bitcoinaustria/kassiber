import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Eye } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TabsContent } from "@/components/ui/tabs";
import { useDaemon } from "@/daemon/client";
import { transactionAnalysisSearch } from "@/lib/chainAnalysisNavigation";
import { transactionTypeLabel } from "@/lib/transactionTypeLabel";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import {
  DetailField,
  DirtyDot,
  LedgerRow,
  networkLabel,
} from "./TransactionDetailSheetParts";
import { TransactionRecordFlow } from "./TransactionRecordFlow";
import { TransactionGraphTechnicalDetails } from "./TransactionGraphTechnicalDetails";
import { CommercialProvenancePanel } from "./TransactionDetailCommercialPanel";
import {
  blurClass,
  currencyFormatter,
  formatShortTxid,
  SATS_PER_BTC,
} from "./model";
import type { TransactionDetailTabContext } from "./TransactionDetailTabContext";
import {
  TransactionGraphPanel,
  type TransactionGraphPayload,
  type TransactionGraphIssueTarget,
  type TransactionSwapRoute,
  type TransactionSwapRouteLegKey,
} from "./TransactionGraphTab";
import {
  preloadableSwapLegGraphLookupArgs,
  transactionGraphLookupReferenceArgs,
} from "./TransactionGraphLookup";
import {
  graphlessTradeKind,
  classifyRouteKind,
  classifyRouteOutRole,
  routeNetworkLabel,
} from "./TransactionGraphModel";

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
      network: routeNetworkLabel(pair.outAsset, pair.outWallet),
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
      network: routeNetworkLabel(pair.inAsset, pair.inWallet),
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

// The transactions-list pair row carries the same fields the graph payload's
// swapRoute does, so both go through the shared classifiers.
type PairRow = NonNullable<TransactionDetailTabContext["transaction"]["pair"]>;

function pairRouteArgs(pair: PairRow) {
  return {
    kind: pair.kind || pair.type,
    policy: pair.policy,
    outAsset: pair.outAsset,
    inAsset: pair.inAsset,
  };
}

function fallbackSwapOutRole(pair: PairRow) {
  return classifyRouteOutRole({
    kind: pair.kind || pair.type,
    outNetwork: routeNetworkLabel(pair.outAsset, pair.outWallet),
    inNetwork: routeNetworkLabel(pair.inAsset, pair.inWallet),
  });
}

function fallbackRouteKind(pair: PairRow) {
  return classifyRouteKind(pairRouteArgs(pair));
}

export function TransactionDetailsTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const { t: tPrivacy } = useTranslation("privacyMirror");
  const navigate = useNavigate();
  const setDeferredConnectionSetup = useUiStore(
    (state) => state.setDeferredConnectionSetup,
  );
  const {
    transaction,
    localDraft,
    dirtyLabel,
    dirtyTags,
    dirtyNote,
    dirtyExcluded,
    transactionDisplayId,
    hideSensitive,
    commercialContext,
    commercialContextLoading,
    showSourceExternalId,
    tags,
    graphData,
    graphLoading,
    graphError,
    onOpenTransaction,
    publicGraphLookup = false,
    canPublicGraphLookup = false,
    enablePublicGraphLookup,
  } = ctx;
  const displayGraphData = graphWithPairFallbackRoute(graphData, transaction);
  const swapRoute = displayGraphData?.swapRoute ?? null;
  const [selectedSwapLeg, setSelectedSwapLeg] = useState<TransactionSwapRouteLegKey | null>(null);
  useEffect(() => {
    setSelectedSwapLeg(null);
  }, [transaction.id, swapRoute?.id]);
  const activeSwapLeg = selectedSwapLeg ?? swapRoute?.currentLeg ?? null;
  const currentGraphReferences = useMemo(
    () => [
      transaction.id,
      transaction.txnId,
      transaction.explorerId,
      displayGraphData?.transaction?.id,
      displayGraphData?.transaction?.txid,
      displayGraphData?.transaction?.externalId,
    ],
    [
      displayGraphData?.transaction?.externalId,
      displayGraphData?.transaction?.id,
      displayGraphData?.transaction?.txid,
      transaction.explorerId,
      transaction.id,
      transaction.txnId,
    ],
  );
  // The swap legs follow the sheet's own opt-in: preloading both legs of a
  // swap would otherwise multiply one unrequested lookup into three.
  const swapOutGraphArgs = useMemo(
    () =>
      preloadableSwapLegGraphLookupArgs(
        swapRoute,
        "out",
        currentGraphReferences,
        publicGraphLookup,
      ),
    [currentGraphReferences, publicGraphLookup, swapRoute],
  );
  const swapInGraphArgs = useMemo(
    () =>
      preloadableSwapLegGraphLookupArgs(
        swapRoute,
        "in",
        currentGraphReferences,
        publicGraphLookup,
      ),
    [currentGraphReferences, publicGraphLookup, swapRoute],
  );
  const swapOutGraphQuery = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    transactionGraphLookupReferenceArgs(
      swapOutGraphArgs.transaction,
      swapOutGraphArgs.allowPublicLookup,
    ),
    { enabled: Boolean(swapOutGraphArgs.transaction) },
  );
  const swapInGraphQuery = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    transactionGraphLookupReferenceArgs(
      swapInGraphArgs.transaction,
      swapInGraphArgs.allowPublicLookup,
    ),
    { enabled: Boolean(swapInGraphArgs.transaction) },
  );
  const activeSwapGraphQuery =
    activeSwapLeg === "out"
      ? swapOutGraphQuery
      : activeSwapLeg === "in"
        ? swapInGraphQuery
        : null;
  const activeSwapGraphData =
    activeSwapLeg === "out"
      ? swapOutGraphQuery.data?.data
      : activeSwapLeg === "in"
        ? swapInGraphQuery.data?.data
        : undefined;
  const resolveGraphIssue = (target: TransactionGraphIssueTarget) => {
    setDeferredConnectionSetup({
      sourceId: target,
      reason:
        target === "liquid"
          ? t("graph.backendSettingsReasonLiquid")
          : t("graph.backendSettingsReasonBitcoin"),
      backendKind: target,
    });
    void navigate({ to: "/settings", hash: target });
  };
  const activeSwapTransactionRef =
    activeSwapLeg === "out"
      ? swapOutGraphArgs.transaction
      : activeSwapLeg === "in"
        ? swapInGraphArgs.transaction
        : null;
  const activeGraphData =
    swapRoute && activeSwapLeg && activeSwapTransactionRef
      ? activeSwapGraphData
      : displayGraphData;
  const graphPanelLoading =
    swapRoute && activeSwapLeg && activeSwapTransactionRef
      ? (activeSwapGraphQuery?.isLoading ||
          (activeSwapGraphQuery?.isFetching && !activeSwapGraphData)) ??
        false
      : graphLoading;
  const graphPanelError =
    swapRoute && activeSwapLeg && activeSwapTransactionRef
      ? activeSwapGraphQuery?.error instanceof Error
        ? activeSwapGraphQuery.error.message
        : null
      : graphError;
  const tradeKind = !graphPanelLoading && !graphPanelError ? graphlessTradeKind(transaction, activeGraphData) : null;
  const analysisSearch = transactionAnalysisSearch(
    activeGraphData?.transaction ?? (activeSwapTransactionRef ? {} : transaction),
  );
  return (
    <>
                  {/* Details — read-only source-of-record + book metadata */}
                  <TabsContent value="details" className="mt-4 space-y-4">
                    <div className="grid gap-3 sm:grid-cols-2">
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
                    </div>
                    <div className="grid gap-3 lg:grid-cols-2">
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                          {t("details.sourceRecord")}
                        </div>
                        <LedgerRow
                          label={t("details.type")}
                          value={transactionTypeLabel(
                            t,
                            transaction.sourceType ?? transaction.direction,
                          )}
                        />
                        <LedgerRow
                          label={t("details.network")}
                          value={networkLabel(transaction)}
                        />
                        <LedgerRow
                          label={t("details.counterparty")}
                          value={
                            transaction.counterparty ? (
                              <span className={blurClass(hideSensitive)}>
                                {transaction.counterparty}
                              </span>
                            ) : (
                              <span className="text-muted-foreground">
                                {t("details.counterpartyNone")}
                              </span>
                            )
                          }
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
                        <div className="border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
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
                        <LedgerRow
                          label={t("details.note")}
                          value={
                            <span className="flex items-baseline gap-1.5">
                              {localDraft.note ? (
                                <span
                                  className={cn(
                                    "line-clamp-2 min-w-0 whitespace-pre-line",
                                    blurClass(hideSensitive),
                                  )}
                                >
                                  {localDraft.note}
                                </span>
                              ) : (
                                <span className="text-muted-foreground">
                                  {t("details.noteNone")}
                                </span>
                              )}
                              <DirtyDot active={dirtyNote} />
                            </span>
                          }
                          hint={t("details.noteHint")}
                        />
                      </div>
                    </div>
                    <div className="overflow-hidden rounded-md border">
                      <div className="flex flex-wrap items-center justify-between gap-2 border-b bg-muted px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                        {t(tradeKind ? "recordFlow.title" : "graph.sectionTitle")}
                        {canPublicGraphLookup && !publicGraphLookup ? (
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-6 text-2xs normal-case"
                            onClick={() => enablePublicGraphLookup?.()}
                          >
                            {t("graph.lookupOnChain")}
                          </Button>
                        ) : null}
                      </div>
                      <div className="p-3">
                        <TransactionGraphPanel
                          graphlessContent={tradeKind ? <TransactionRecordFlow transaction={transaction} kind={tradeKind} hideSensitive={hideSensitive} /> : undefined}
                          graph={activeGraphData}
                          loading={graphPanelLoading}
                          error={graphPanelError}
                          hideSensitive={hideSensitive}
                          selectedSwapLeg={activeSwapLeg}
                          onSelectSwapLeg={setSelectedSwapLeg}
                          onResolveIssue={resolveGraphIssue}
                          onOpenTransaction={onOpenTransaction}
                        />
                      </div>
                    </div>
                    {analysisSearch.subject ? <Button
                      variant="outline"
                      size="sm"
                      className="self-start"
                      onClick={() => void navigate({ to: "/chain-analysis", search: analysisSearch })}
                    >
                      <Eye className="size-3.5" aria-hidden="true" />
                      {tPrivacy("investigateTransaction")}
                    </Button> : null}
                    <TransactionGraphTechnicalDetails graph={activeGraphData} hideSensitive={hideSensitive} />
                    <CommercialProvenancePanel
                      context={commercialContext}
                      loading={commercialContextLoading}
                      hidden={hideSensitive}
                    />
                  </TabsContent>


    </>
  );
}
