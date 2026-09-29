import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Copy, ExternalLink, Eye } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { TabsContent } from "@/components/ui/tabs";
import { useDaemon } from "@/daemon/client";
import { transactionAnalysisSearch } from "@/lib/chainAnalysisNavigation";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { DirtyDot, InfoHint } from "./TransactionDetailSheetParts";
import { exchangeTransfer } from "./ExchangeTransferModel";
import { TransactionRecordFlow } from "./TransactionRecordFlow";
import { TransactionGraphTechnicalDetails } from "./TransactionGraphTechnicalDetails";
import { CommercialProvenancePanel } from "./TransactionDetailCommercialPanel";
import {
  blurClass,
  copyText,
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
  publicLookupCanAddToGraph,
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

/**
 * The transaction's coins: the glass graph and the input/output lists. Shown
 * beside the tabs rather than inside one, so the graph never scrolls away.
 */
export function TransactionFlowSection({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
  const { t: tPrivacy } = useTranslation("privacyMirror");
  const navigate = useNavigate();
  const setDeferredConnectionSetup = useUiStore(
    (state) => state.setDeferredConnectionSetup,
  );
  const {
    transaction,
    hideSensitive,
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
  const activeSwapLeg = selectedSwapLeg ?? exchangeTransfer(swapRoute)?.chainLeg ?? swapRoute?.currentLeg ?? null;
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
  const lookupButton =
    canPublicGraphLookup && !publicGraphLookup && publicLookupCanAddToGraph(activeGraphData) ? (
      <Button
        type="button"
        size="sm"
        variant="outline"
        className="h-8 text-xs"
        onClick={() => enablePublicGraphLookup?.()}
      >
        {t("graph.lookupOnChain")}
      </Button>
    ) : null;
  // A drawn graph carries the lookup beside its title; without one it leads.
  const drawn =
    Boolean(activeGraphData) &&
    (activeGraphData?.supportLevel === "full" || activeGraphData?.supportLevel === "partial") &&
    Boolean(activeGraphData?.inputs.length || activeGraphData?.outputs.length);
  return (
    <div className="space-y-3">
      <div className="kb-surface p-(--kb-card-padding)">
        {!drawn && lookupButton ? <div className="mb-3 flex justify-end">{lookupButton}</div> : null}
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
          headerAction={drawn ? lookupButton : null}
        />
      </div>
      <TransactionGraphTechnicalDetails
        graph={activeGraphData}
        hideSensitive={hideSensitive}
        action={
          analysisSearch.subject ? (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs"
              onClick={() => void navigate({ to: "/chain-analysis", search: analysisSearch })}
            >
              <Eye className="size-3.5" aria-hidden="true" />
              {tPrivacy("investigateTransaction")}
            </Button>
          ) : null
        }
      />
    </div>
  );
}

/** One label and value in the compact facts block. */
function Fact({
  label,
  hint,
  copyValue,
  hidden,
  dirty,
  mono,
  wide,
  children,
}: {
  label: string;
  hint?: ReactNode;
  copyValue?: string;
  hidden?: boolean;
  dirty?: boolean;
  mono?: boolean;
  wide?: boolean;
  children: ReactNode;
}) {
  const { t } = useTranslation("transactions");
  return (
    <div className={cn("min-w-0", wide && "col-span-2")}>
      <dt className="flex items-center gap-1 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        <span className="truncate">{label}</span>
        {hint ? <InfoHint label={t("infoHint.fieldMeaning", { label })}>{hint}</InfoHint> : null}
        <DirtyDot active={dirty} />
        {copyValue ? (
          <button
            type="button"
            className="ml-auto rounded-sm text-muted-foreground hover:text-foreground"
            aria-label={t("infoHint.copy", { label })}
            onClick={() => copyText(copyValue)}
          >
            <Copy className="size-3" aria-hidden="true" />
          </button>
        ) : null}
      </dt>
      <dd className={cn("mt-0.5 min-w-0 truncate text-sm font-medium", mono && "font-mono text-xs", blurClass(Boolean(hidden)))}>
        {children}
      </dd>
    </div>
  );
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
    commercialContext,
    commercialContextLoading,
    showSourceExternalId,
    tags,
    sourceLabel,
    explorer,
    openExplorer,
  } = ctx;
  return (
    <>
                  {/* Details — read-only source-of-record + book metadata */}
                  <TabsContent value="details" className="mt-4 space-y-4">
                    {/* One compact block: the header already names network and
                        counterparty, so they are not repeated here. */}
                    <dl className="kb-surface-inset grid grid-cols-2 gap-x-4 gap-y-3 p-3">
                      <Fact
                        label={t("details.transactionId")}
                        hint={t("details.transactionIdHint")}
                        copyValue={transactionDisplayId}
                        hidden={hideSensitive}
                        mono
                      >
                        {formatShortTxid(transactionDisplayId)}
                      </Fact>
                      <Fact label={t("details.priceAtTime")} hint={t("details.priceAtTimeHint")} hidden={hideSensitive}>
                        {localDraft.pricingSourceKind === "manual_override" && localDraft.manualPrice
                          ? t("details.manualPerBtc", {
                              price: localDraft.manualPrice,
                              currency: localDraft.manualCurrency,
                            })
                          : transaction.rate
                            ? t("details.perBtc", { value: currencyFormatter.format(transaction.rate) })
                            : t("details.priceMissing")}
                      </Fact>
                      <Fact label={t("sourceRecord.source")} hidden={hideSensitive}>
                        {sourceLabel}
                      </Fact>
                      <Fact label={t("details.explorer")}>
                        {explorer ? (
                          <button
                            type="button"
                            className="inline-flex max-w-full items-center gap-1 truncate text-left underline-offset-2 hover:underline"
                            onClick={openExplorer}
                          >
                            <span className="truncate">{explorer.label}</span>
                            <ExternalLink className="size-3 shrink-0" aria-hidden="true" />
                          </button>
                        ) : (
                          <span className="text-muted-foreground">{t("sourceRecord.noExplorer")}</span>
                        )}
                      </Fact>
                      <Fact label={t("details.label")} dirty={dirtyLabel}>
                        {localDraft.label}
                      </Fact>
                      <Fact label={t("details.tags")} dirty={dirtyTags} hidden={hideSensitive}>
                        {tags.length ? (
                          <span className="flex flex-wrap gap-1">
                            {tags.map((tag) => (
                              <Badge key={tag} variant="secondary" className="rounded-md">
                                {tag}
                              </Badge>
                            ))}
                          </span>
                        ) : (
                          <span className="text-muted-foreground">{t("details.tagsNone")}</span>
                        )}
                      </Fact>
                      <Fact label={t("details.included")} dirty={dirtyExcluded}>
                        {localDraft.excluded ? t("details.includedNo") : t("details.includedYes")}
                      </Fact>
                      {showSourceExternalId ? (
                        <Fact label={t("details.externalId")} hint={t("details.externalIdHint")} mono>
                          {formatShortTxid(transaction.txnId)}
                        </Fact>
                      ) : (
                        <Fact label={t("sourceRecord.kassiberRow")} copyValue={transaction.id} hidden={hideSensitive} mono>
                          {transaction.id}
                        </Fact>
                      )}
                      <Fact label={t("details.note")} hint={t("details.noteHint")} dirty={dirtyNote} hidden={hideSensitive} wide>
                        {localDraft.note ? (
                          <span className="line-clamp-2 whitespace-pre-line">{localDraft.note}</span>
                        ) : (
                          <span className="text-muted-foreground">{t("details.noteNone")}</span>
                        )}
                      </Fact>
                    </dl>
                    <CommercialProvenancePanel
                      context={commercialContext}
                      loading={commercialContextLoading}
                      hidden={hideSensitive}
                    />
                  </TabsContent>


    </>
  );
}
