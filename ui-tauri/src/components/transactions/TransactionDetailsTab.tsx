import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ShieldAlert } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { TabsContent } from "@/components/ui/tabs";
import { useDaemon } from "@/daemon/client";
import {
  findPrivacyTransactionRow,
  formatPrivacyInt,
  privacyEvidenceTone,
  shortPrivacyId,
  type EvidenceLevel,
  type PrivacyMirrorPayload,
} from "@/lib/privacyMirror";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

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

function PrivacyEvidencePill({ level }: { level?: EvidenceLevel }) {
  const { t } = useTranslation("privacyMirror");
  const key = level || "unknown";
  const label =
    key === "exact"
      ? t("evidence.exact")
      : key === "derived"
        ? t("evidence.derived")
        : key === "unknown"
          ? t("evidence.unknown")
          : key;
  return (
    <Badge variant="outline" className={cn("rounded-md", privacyEvidenceTone(key))}>
      {label}
    </Badge>
  );
}

function TransactionPrivacyMirrorPanel({
  payload,
  loading,
  errorMessage,
  transactionRefs,
}: {
  payload?: PrivacyMirrorPayload;
  loading: boolean;
  errorMessage: string | null;
  transactionRefs: Array<string | null | undefined>;
}) {
  const { t } = useTranslation("privacyMirror");
  const row = findPrivacyTransactionRow(payload, transactionRefs);
  const tellKinds = row?.tell_kinds ?? [];
  const degraded = Boolean(errorMessage) || (!loading && !row);

  return (
    <div className="overflow-hidden rounded-md border" data-testid="transaction-privacy-mirror-panel">
      <div className="flex items-center justify-between gap-3 border-b bg-muted px-3 py-1.5">
        <div className="flex min-w-0 items-center gap-2">
          <ShieldAlert className="size-4 text-amber-600" aria-hidden="true" />
          <span className="truncate text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
            {t("detail.transactionTitle")}
          </span>
        </div>
        <PrivacyEvidencePill level={row?.evidence_level ?? (degraded ? "unknown" : "derived")} />
      </div>
      <div className="grid gap-3 p-3 sm:grid-cols-3">
        <div>
          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("detail.transactionTells")}
          </p>
          <p className="font-mono text-lg tabular-nums">
            {loading && !row ? "..." : formatPrivacyInt(row?.tell_count)}
          </p>
        </div>
        <div>
          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("detail.transactionPenalties")}
          </p>
          <p className="font-mono text-lg tabular-nums">
            {loading && !row ? "..." : formatPrivacyInt(row?.wallet_penalty_count)}
          </p>
        </div>
        <div className="min-w-0">
          <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("detail.transactionKinds")}
          </p>
          <p className="truncate text-sm">
            {tellKinds.length ? tellKinds.join(", ") : t("detail.none")}
          </p>
        </div>
      </div>
      <div className="border-t px-3 py-2 text-xs text-muted-foreground">
        {errorMessage
          ? t("detail.queryError", { message: errorMessage })
          : row
            ? t("detail.transactionMatched", { id: shortPrivacyId(row.txid) })
            : loading
              ? t("detail.loading")
              : t("detail.degraded")}
      </div>
    </div>
  );
}

export function TransactionDetailsTab({ ctx }: { ctx: TransactionDetailTabContext }) {
  const { t } = useTranslation("transactions");
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
  const swapOutGraphArgs = useMemo(
    () => preloadableSwapLegGraphLookupArgs(swapRoute, "out", currentGraphReferences),
    [currentGraphReferences, swapRoute],
  );
  const swapInGraphArgs = useMemo(
    () => preloadableSwapLegGraphLookupArgs(swapRoute, "in", currentGraphReferences),
    [currentGraphReferences, swapRoute],
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
  const privacyMirrorQuery = useDaemon<PrivacyMirrorPayload>("ui.reports.privacy_mirror");
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
  const privacyMirrorError =
    privacyMirrorQuery.error instanceof Error ? privacyMirrorQuery.error.message : null;
  const graphTx = activeGraphData?.transaction;
  const graphNetworkFeeBtc =
    typeof activeGraphData?.fee?.valueBtc === "number"
      ? activeGraphData.fee.valueBtc
      : typeof activeGraphData?.fee?.valueSats === "number"
        ? activeGraphData.fee.valueSats / SATS_PER_BTC
        : 0;
  const hiddenGraphValue = t("graph.hidden");
  const technicalRows = graphTx
    ? [
        [t("details.inputCount"), graphTx.inputCount ?? activeGraphData.inputs.length],
        [t("details.outputCount"), graphTx.outputCount ?? activeGraphData.outputs.length],
        [
          t("details.networkFee"),
          graphNetworkFeeBtc
            ? hideSensitive
              ? hiddenGraphValue
              : formatBtcAmount(graphNetworkFeeBtc)
            : t("details.unknown"),
        ],
        [
          t("details.feeRate"),
          graphTx.feeRateSatVb
            ? hideSensitive
              ? hiddenGraphValue
              : `${graphTx.feeRateSatVb} sat/vB`
            : t("details.unknown"),
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
                          onResolveIssue={resolveGraphIssue}
                        />
                      </div>
                    </div>
                    <TransactionPrivacyMirrorPanel
                      payload={privacyMirrorQuery.data?.data}
                      loading={privacyMirrorQuery.isLoading}
                      errorMessage={privacyMirrorError}
                      transactionRefs={currentGraphReferences}
                    />
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
