/**
 * Swap-matching review queue.
 *
 * Drives the ``ui.transfers.suggest`` daemon kind to surface candidate
 * pairings the matcher believes form one reviewed movement. Bitcoin rail swaps
 * (Lightning ↔ Liquid, Liquid ↔ on-chain BTC, etc.) live with the
 * carrying-value Bitcoin-movement queue; other cross-asset swaps live with
 * swaps. Each row exposes inline kind / policy
 * controls + per-row Pair / Dismiss actions wired to
 * ``ui.transfers.pair`` and ``ui.transfers.dismiss``.
 *
 * Heavy-user UX hooks already wired in this commit:
 *  - Status pill header with counts (total / exact / strong / conflicts).
 *  - Filter chips that pin confidence, method, and asset pair.
 *  - Conflict-cluster grouping renders a shared ⚠ banner; bulk-pair
 *    intentionally skips clustered candidates (the user must
 *    disambiguate first).
 *  - "What actually left your custody" — the computed
 *    ``swap_fee_msat`` is the headline number on every card.
 *
 * Bulk + preview + undo land in commit 12, rules + saved-view chips in
 * commit 13, and keyboard shortcuts in commit 14.
 */

import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  Check,
  Eye,
  History as HistoryIcon,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Settings as SettingsIcon,
  Sparkles,
  Star,
  Trash2,
  Undo2,
  Unlink,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useKeymap, type Keybinding } from "@/lib/keymap";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

const PAIR_KIND_OPTIONS = ["manual", "peg-in", "peg-out", "submarine-swap", "swap-refund"] as const;
const PAIR_POLICY_OPTIONS = ["carrying-value", "taxable"] as const;
// Stable `value` (machine code, used in filter payloads + lookups) paired with a
// `labelKey` into the `review` namespace; labels are resolved at render.
const CONFIDENCE_OPTIONS = [
  { value: "all", labelKey: "swap.confidence.any" },
  { value: "exact", labelKey: "swap.confidence.exact" },
  { value: "strong", labelKey: "swap.confidence.strong" },
] as const;
const METHOD_OPTIONS = [
  { value: "all", labelKey: "swap.method.any" },
  { value: "payment_hash", labelKey: "swap.method.paymentHash" },
  { value: "heuristic", labelKey: "swap.method.heuristic" },
  { value: "htlc_refund", labelKey: "swap.method.htlcRefund" },
] as const;

type CandidateMethod = "payment_hash" | "heuristic" | "htlc_refund";
// Per-method i18n keys for the two render sites (the status-cell tooltip and the
// detail sheet). The Record forces every method (including any future one) to
// define both labels, so the type checker flags a missing translation instead
// of silently labelling it as a heuristic match.
const METHOD_LABEL_KEYS = {
  payment_hash: {
    matched: "swap.detail.matchedByPaymentHash",
    rationale: "swap.detail.rationalePaymentHash",
  },
  heuristic: {
    matched: "swap.detail.matchedByTimeAmount",
    rationale: "swap.detail.rationaleHeuristic",
  },
  htlc_refund: {
    matched: "swap.detail.matchedByRefundLink",
    rationale: "swap.detail.rationaleHtlcRefund",
  },
} as const satisfies Record<
  CandidateMethod,
  { matched: string; rationale: string }
>;
const ROUTE_PAIR_OPTIONS = [
  { value: "all", labelKey: "swap.route.any" },
  { value: "LNBTC-LBTC", labelKey: "swap.route.lnToLiquid" },
  { value: "LBTC-LNBTC", labelKey: "swap.route.liquidToLn" },
  { value: "LBTC-BTC", labelKey: "swap.route.liquidToOnchain" },
  { value: "LNBTC-BTC", labelKey: "swap.route.lnToOnchain" },
  { value: "BTC-LBTC", labelKey: "swap.route.onchainToLiquid" },
] as const;
const ROUTE_PAIR_VALUES = new Set<string>(ROUTE_PAIR_OPTIONS.map((option) => option.value));

type PairKind = (typeof PAIR_KIND_OPTIONS)[number];
type PairPolicy = (typeof PAIR_POLICY_OPTIONS)[number];
type SwapRail = "onchain" | "lightning" | "liquid";
const BITCOIN_LAYER_TRANSITION_KINDS = new Set<PairKind>([
  "peg-in",
  "peg-out",
  "submarine-swap",
  "swap-refund",
]);

interface SwapCandidate {
  out_id: string;
  in_id: string;
  out_asset: string;
  in_asset: string;
  out_amount_msat: number;
  out_amount: number;
  in_amount_msat: number;
  in_amount: number;
  out_wallet_id: string;
  in_wallet_id: string;
  out_wallet_label: string;
  in_wallet_label: string;
  out_wallet_kind: string;
  in_wallet_kind: string;
  out_occurred_at: string;
  in_occurred_at: string;
  confidence: "exact" | "strong";
  method: CandidateMethod;
  swap_fee_msat: number;
  swap_fee: number;
  swap_fee_kind: string;
  default_kind: PairKind;
  default_policy: PairPolicy;
  candidate_type?: "transfer" | "swap";
  conflict_set_id: string;
  /** Cluster cardinality over the full unfiltered candidate set; > 1 means
   * this candidate shares a leg with others (possibly hidden by filters). */
  conflict_size: number;
  rule_match?: {
    rule_id: string;
    rule_name: string | null;
    kind: PairKind;
    policy: PairPolicy;
  };
}

interface SuggestEnvelope {
  candidates: SwapCandidate[];
  counts: {
    total: number;
    exact: number;
    strong: number;
    conflicts: number;
    rule_matches?: number;
  };
}

const btcFmt = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 8,
  minimumFractionDigits: 8,
});

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

function formatBtc(value: number) {
  return `₿${btcFmt.format(value)}`;
}

function formatSats(msat: number) {
  return `${Math.round(msat / 1000).toLocaleString()} sats`;
}

function formatTimestamp(value: string) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "short",
    timeStyle: "short",
  }).format(date);
}

function compactRecordId(value: string) {
  if (!value) return "—";
  if (value.length <= 22) return value;
  return `${value.slice(0, 12)}…${value.slice(-6)}`;
}

/** Accepts any leg-fee shape (candidate or persisted pair) — only the swapped
 *  outbound magnitude and the fee delta matter for the percentage. */
function feePercent(fee: { swap_fee_msat: number; out_amount_msat: number }) {
  if (!fee.out_amount_msat) return 0;
  return (Math.abs(fee.swap_fee_msat) / fee.out_amount_msat) * 100;
}

function candidatePairType(candidate: SwapCandidate) {
  if (candidate.out_asset.toUpperCase() === candidate.in_asset.toUpperCase()) {
    return "transfer";
  }
  if (
    candidate.candidate_type === "transfer" ||
    isBitcoinLayerTransitionKind(candidate.default_kind)
  ) {
    return "layer-transition";
  }
  return "swap";
}

function candidateLabelKey(candidate: SwapCandidate) {
  const type = candidatePairType(candidate);
  if (type === "transfer") return "swap.detail.candidateLabelTransfer";
  if (type === "layer-transition") return "swap.detail.candidateLabelLayerTransition";
  return "swap.detail.candidateLabelSwap";
}

function candidateFeeLabelKey(candidate: SwapCandidate) {
  const type = candidatePairType(candidate);
  if (type === "swap") return "swap.detail.swapFee";
  if (type === "layer-transition") return "swap.detail.layerTransitionFee";
  return "swap.detail.transferFee";
}

interface BulkPairResult {
  applied: Array<{ id: string; swap_fee_msat?: number | null }>;
  summary: {
    count: number;
    skipped_conflicts: number;
    total_swap_fee_msat: number;
  };
}

interface SavedView {
  id: string;
  surface: string;
  name: string;
  filter: {
    confidence?: string;
    method?: string;
    asset_pair?: string;
    route_pair?: string;
    [key: string]: unknown;
  };
}

interface SavedViewsEnvelope {
  views: SavedView[];
}

interface SwapRule {
  id: string;
  name: string | null;
  predicate: Record<string, unknown>;
  kind: PairKind;
  policy: PairPolicy;
  enabled: boolean;
}

interface RulesEnvelope {
  rules: SwapRule[];
}

/** One leg of a persisted pair as returned by ``ui.transfers.list``. */
interface PairLeg {
  transaction_id: string;
  external_id: string;
  wallet: string;
  wallet_kind: string;
  asset: string;
  occurred_at: string;
  amount: number;
  amount_msat: number;
  full_amount?: number;
  full_amount_msat?: number;
}

/** An already-paired swap/transfer (the inverse of a {@link SwapCandidate}). */
interface PairedSwap {
  id: string;
  out_transaction_id: string;
  in_transaction_id: string;
  kind: PairKind;
  policy: PairPolicy;
  notes: string | null;
  swap_fee_msat: number | null;
  swap_fee_kind: string | null;
  confidence_at_pair: "exact" | "strong" | null;
  pair_source: string | null;
  out_amount: number | null;
  deleted_at: string | null;
  created_at: string;
  out: PairLeg;
  in: PairLeg;
}

interface PairsEnvelope {
  pairs: PairedSwap[];
}

/** msat → BTC (1 BTC = 100_000_000_000 msat); pairs carry only the msat fee. */
const MSAT_PER_BTC = 100_000_000_000;

// Persisted ``pair_source`` codes (handlers.py `_PAIR_SOURCE_VALUES`) → i18n
// keys. Returns a literal key (so the typed `t()` accepts it) or null for an
// unknown/legacy source, which the caller renders verbatim.
function pairSourceLabelKey(source: string | null) {
  switch (source) {
    case "manual":
      return "swap.paired.sourceManual" as const;
    case "bulk_exact":
      return "swap.paired.sourceBulkExact" as const;
    case "bulk_selected":
      return "swap.paired.sourceBulkSelected" as const;
    case "rule_auto":
      return "swap.paired.sourceRuleAuto" as const;
    default:
      return null;
  }
}

const UNDO_WINDOW_MS = 20_000;

type PairingReviewMode = "swaps" | "transfers";

/** Same-asset pairs are pure transfers; known peg/submarine/refund kinds are
 *  Bitcoin swaps with carrying-value treatment, so they stay in the Bitcoin
 *  movement tab even when the ledger asset codes differ (BTC vs LBTC). */
function pairIsSameAsset(pair: PairedSwap) {
  return pair.out.asset.toUpperCase() === pair.in.asset.toUpperCase();
}

function isBitcoinLayerTransitionKind(kind: PairKind | string | null | undefined) {
  return BITCOIN_LAYER_TRANSITION_KINDS.has(kind as PairKind);
}

function pairPresentationType(pair: PairedSwap) {
  if (pairIsSameAsset(pair)) return "transfer";
  if (isBitcoinLayerTransitionKind(pair.kind)) return "layer-transition";
  return "swap";
}

function pairReviewMode(pair: PairedSwap): PairingReviewMode {
  return pairPresentationType(pair) === "swap" ? "swaps" : "transfers";
}

const RAIL_DETAILS: Record<
  SwapRail,
  {
    label: string;
    shortLabel: string;
    icon: string;
    className: string;
  }
> = {
  onchain: {
    label: "On-chain",
    shortLabel: "BTC",
    icon: bitcoinIcon,
    className:
      "border-orange-200 bg-orange-50 text-orange-800 dark:border-orange-400/30 dark:bg-orange-950/40 dark:text-orange-100",
  },
  lightning: {
    label: "Lightning",
    shortLabel: "LN",
    icon: lightningIcon,
    className:
      "border-violet-200 bg-violet-50 text-violet-800 dark:border-violet-400/30 dark:bg-violet-950/40 dark:text-violet-100",
  },
  liquid: {
    label: "Liquid",
    shortLabel: "Liquid",
    icon: liquidIcon,
    className:
      "border-cyan-200 bg-cyan-50 text-cyan-800 dark:border-cyan-400/30 dark:bg-cyan-950/40 dark:text-cyan-100",
  },
};

function railForLeg(
  asset: string | null | undefined,
  walletKind: string | null | undefined,
): SwapRail {
  // Tolerate a daemon that predates the enriched pair payload (no wallet_kind):
  // fall through to the on-chain default instead of throwing on .toLowerCase().
  const assetKey = (asset ?? "").toUpperCase();
  const kindKey = (walletKind ?? "").toLowerCase();
  if (assetKey === "LBTC" || kindKey.includes("liquid")) return "liquid";
  if (
    kindKey.includes("phoenix") ||
    kindKey.includes("lightning") ||
    kindKey.includes("coreln") ||
    kindKey.includes("core-ln") ||
    kindKey === "lnd" ||
    kindKey === "nwc"
  ) {
    return "lightning";
  }
  return "onchain";
}

type PairingReviewTab = "swaps" | "transfers";
type PairingView = "review" | "paired";

export function SwapMatching() {
  const { t } = useTranslation("review");
  const [activeTab, setActiveTab] = useState<PairingReviewTab>("transfers");
  // Swaps/Transfers is the only tab strip. The settled "History" list isn't a
  // second tab — it opens from a History card in the review-queue metrics and
  // returns via a back control. The view is shared across both tabs.
  const [view, setView] = useState<PairingView>("review");
  const showHistory = () => setView("paired");
  const showReview = () => setView("review");

  return (
    <div className={screenShellClassName}>
      <Tabs
        value={activeTab}
        onValueChange={(value) => setActiveTab(value as PairingReviewTab)}
        className="space-y-3"
      >
        <TabsList className="w-full justify-start overflow-x-auto sm:w-fit">
          <TabsTrigger value="transfers">{t("swap.tabs.transfers")}</TabsTrigger>
          <TabsTrigger value="swaps">{t("swap.tabs.swaps")}</TabsTrigger>
        </TabsList>
        <TabsContent value="transfers" className="mt-0">
          {activeTab === "transfers" ? (
            view === "review" ? (
              <PairingReview mode="transfers" onShowHistory={showHistory} />
            ) : (
              <PairedSwaps mode="transfers" onBackToReview={showReview} />
            )
          ) : null}
        </TabsContent>
        <TabsContent value="swaps" className="mt-0">
          {activeTab === "swaps" ? (
            view === "review" ? (
              <PairingReview mode="swaps" onShowHistory={showHistory} />
            ) : (
              <PairedSwaps mode="swaps" onBackToReview={showReview} />
            )
          ) : null}
        </TabsContent>
      </Tabs>
    </div>
  );
}

/** A persisted pair's fee, shaped for {@link SwapFeeText} / {@link feePercent}. */
function pairFee(pair: PairedSwap) {
  const swapFeeMsat = pair.swap_fee_msat ?? 0;
  return {
    swap_fee: swapFeeMsat / MSAT_PER_BTC,
    swap_fee_msat: swapFeeMsat,
    out_amount_msat: pair.out.amount_msat,
  };
}

/**
 * Already-paired swaps / transfers for the active rail. Same row layout as the
 * review queue — reusing {@link SwapLegInline} and {@link SwapFeeText} — but
 * read-from-the-ledger instead of select-to-pair. Each row can be opened to
 * edit its kind / policy (``ui.transfers.update``) or unpaired
 * (``ui.transfers.unpair``).
 */
function PairedSwaps({
  mode,
  onBackToReview,
}: {
  mode: PairingReviewMode;
  onBackToReview: () => void;
}) {
  const { t } = useTranslation(["review", "common"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const { data, isLoading, isError, error } =
    useDaemon<PairsEnvelope>("ui.transfers.list");
  const unpairMutation = useDaemonMutation<unknown>("ui.transfers.unpair");
  const updateMutation = useDaemonMutation<unknown>("ui.transfers.update");

  const [detailPair, setDetailPair] = useState<PairedSwap | null>(null);
  const [unpairTarget, setUnpairTarget] = useState<PairedSwap | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const pairs = useMemo(
    () =>
      (data?.data?.pairs ?? []).filter(
        (pair) => pairReviewMode(pair) === mode,
      ),
    [data, mode],
  );

  const title =
    mode === "transfers"
      ? t("swap.paired.transfersTitle")
      : t("swap.paired.swapsTitle");
  const description =
    mode === "transfers"
      ? t("swap.paired.transfersDescription")
      : t("swap.paired.swapsDescription");
  const emptyText =
    mode === "transfers"
      ? t("swap.paired.transfersEmpty")
      : t("swap.paired.swapsEmpty");

  const handleSave = useCallback(
    async (pair: PairedSwap, kind: PairKind, policy: PairPolicy) => {
      setActionError(null);
      try {
        await updateMutation.mutateAsync({ pair_id: pair.id, kind, policy });
        setDetailPair(null);
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
      }
    },
    [updateMutation],
  );

  const handleUnpair = useCallback(async () => {
    if (!unpairTarget) return;
    setActionError(null);
    try {
      await unpairMutation.mutateAsync({ pair_id: unpairTarget.id });
      setUnpairTarget(null);
      setDetailPair(null);
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
  }, [unpairMutation, unpairTarget]);

  const busy = unpairMutation.isPending || updateMutation.isPending;

  return (
    <div className="min-w-0">
      <div className="overflow-hidden rounded-xl border bg-card">
        <header className="flex flex-col gap-3 px-3 py-3 sm:flex-row sm:items-start sm:justify-between sm:px-4">
          <div className="min-w-0 space-y-1">
            <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
              {t("swap.paired.label")}
            </p>
            <h1 className="text-base font-semibold">{title}</h1>
            <p className="max-w-3xl text-sm text-muted-foreground">{description}</p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="h-9 shrink-0"
            onClick={onBackToReview}
          >
            <ArrowLeft className="size-3.5" aria-hidden="true" />
            <span>{t("swap.paired.backToReview")}</span>
          </Button>
        </header>

        {actionError ? (
          <div className="border-t px-3 py-3 sm:px-6">
            <div className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
              {actionError}
            </div>
          </div>
        ) : null}

        {isLoading ? (
          <div className="flex items-center gap-2 border-t px-6 py-8 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> {t("swap.paired.loading")}
          </div>
        ) : isError ? (
          <div className="border-t px-6 py-6">
            <div className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm">
              {t("swap.paired.loadFailed", { error: String(error) })}
            </div>
          </div>
        ) : pairs.length === 0 ? (
          <div className="border-t px-6 py-8">
            <div className="rounded border border-dashed border-muted-foreground/40 p-6 text-center text-sm text-muted-foreground">
              {emptyText}
            </div>
          </div>
        ) : (
          <div className="overflow-x-auto border-t">
            <Table className="min-w-[1200px] w-full table-fixed">
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead className="w-[180px] text-xs font-medium text-muted-foreground">
                    {t("swap.paired.pairing")}
                  </TableHead>
                  <TableHead className="w-[360px] text-xs font-medium text-muted-foreground">
                    {t("swap.table.outgoing")}
                  </TableHead>
                  <TableHead className="w-[44px] text-center"></TableHead>
                  <TableHead className="w-[360px] text-xs font-medium text-muted-foreground">
                    {t("swap.table.incoming")}
                  </TableHead>
                  <TableHead className="w-[160px] text-right text-xs font-medium text-muted-foreground">
                    {t("swap.table.feeDelta")}
                  </TableHead>
                  <TableHead className="w-[44px]"></TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {pairs.map((pair) => (
                  <TableRow
                    key={pair.id}
                    className="cursor-pointer align-middle hover:bg-muted/35"
                    onClick={() => {
                      setActionError(null);
                      setDetailPair(pair);
                    }}
                  >
                    <TableCell className="whitespace-normal align-top">
                      <PairingCell pair={pair} />
                    </TableCell>
                    <TableCell className="whitespace-nowrap">
                      <SwapLegInline
                        direction="out"
                        asset={pair.out.asset}
                        amount={pair.out.amount}
                        wallet={pair.out.wallet}
                        walletKind={pair.out.wallet_kind}
                        timestamp={pair.out.occurred_at}
                        txId={pair.out.transaction_id}
                        hideSensitive={hideSensitive}
                      />
                    </TableCell>
                    <TableCell className="text-center text-muted-foreground">
                      <ArrowRight className="mx-auto mt-1 size-4" aria-hidden="true" />
                    </TableCell>
                    <TableCell className="whitespace-nowrap">
                      <SwapLegInline
                        direction="in"
                        asset={pair.in.asset}
                        amount={pair.in.amount}
                        wallet={pair.in.wallet}
                        walletKind={pair.in.wallet_kind}
                        timestamp={pair.in.occurred_at}
                        txId={pair.in.transaction_id}
                        hideSensitive={hideSensitive}
                      />
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-right">
                      <SwapFeeText candidate={pairFee(pair)} hideSensitive={hideSensitive} />
                    </TableCell>
                    <TableCell>
                      <PairedRowMenu
                        pair={pair}
                        onEdit={() => {
                          setActionError(null);
                          setDetailPair(pair);
                        }}
                        onUnpair={() => {
                          setActionError(null);
                          setUnpairTarget(pair);
                        }}
                        disabled={busy}
                      />
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}

        {!isLoading && !isError && pairs.length > 0 ? (
          <div className="flex items-center border-t px-3 py-3 text-xs text-muted-foreground sm:px-6">
            <span>{t("swap.paired.count", { count: pairs.length })}</span>
          </div>
        ) : null}
      </div>

      <PairedDetailSheet
        pair={detailPair}
        onOpenChange={(open) => {
          if (!open) setDetailPair(null);
        }}
        onSave={handleSave}
        onUnpair={(pair) => {
          setActionError(null);
          setUnpairTarget(pair);
        }}
        saving={updateMutation.isPending}
        hideSensitive={hideSensitive}
      />

      <Dialog
        open={unpairTarget !== null}
        onOpenChange={(open) => {
          if (!open) setUnpairTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("swap.paired.unpairTitle")}</DialogTitle>
            <DialogDescription>{t("swap.paired.unpairBody")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setUnpairTarget(null)}
              disabled={unpairMutation.isPending}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleUnpair()}
              disabled={unpairMutation.isPending}
            >
              {unpairMutation.isPending ? (
                <Loader2 className="mr-1 size-4 animate-spin" />
              ) : null}
              {t("swap.paired.unpairConfirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

/** The "pairing" summary cell: kind + policy + how/when it was matched. */
function PairingCell({ pair }: { pair: PairedSwap }) {
  const { t } = useTranslation("review");
  const sourceKey = pairSourceLabelKey(pair.pair_source);
  const sourceLabel = sourceKey ? t(sourceKey) : pair.pair_source;
  const confidenceLabel = pair.confidence_at_pair
    ? pair.confidence_at_pair === "exact"
      ? t("swap.metric.exact")
      : t("swap.metric.strong")
    : null;
  return (
    <div className="space-y-1">
      <Badge variant="outline" className="text-[11px]">
        {pair.kind}
      </Badge>
      <div className="text-xs text-muted-foreground">{pair.policy}</div>
      {sourceLabel || confidenceLabel ? (
        <div className="text-[11px] text-muted-foreground/80">
          {sourceLabel}
          {sourceLabel && confidenceLabel ? " · " : null}
          {confidenceLabel}
        </div>
      ) : null}
    </div>
  );
}

interface PairedRowMenuProps {
  pair: PairedSwap;
  onEdit: () => void;
  onUnpair: () => void;
  disabled: boolean;
}

function PairedRowMenu({ pair, onEdit, onUnpair, disabled }: PairedRowMenuProps) {
  const { t } = useTranslation("review");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="size-8 text-muted-foreground hover:text-foreground"
          aria-label={t("swap.paired.rowMenuAria", { id: pair.id })}
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={onEdit}>
          <Pencil className="mr-2 size-4" aria-hidden="true" />
          {t("swap.paired.edit")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="text-destructive"
          disabled={disabled}
          onSelect={onUnpair}
        >
          <Unlink className="mr-2 size-4" aria-hidden="true" />
          {t("swap.paired.unpair")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

interface PairedDetailSheetProps {
  pair: PairedSwap | null;
  onOpenChange: (open: boolean) => void;
  onSave: (pair: PairedSwap, kind: PairKind, policy: PairPolicy) => void;
  onUnpair: (pair: PairedSwap) => void;
  saving: boolean;
  hideSensitive: boolean;
}

function PairedDetailSheet({
  pair,
  onOpenChange,
  onSave,
  onUnpair,
  saving,
  hideSensitive,
}: PairedDetailSheetProps) {
  const { t } = useTranslation("review");
  const [kind, setKind] = useState<PairKind>("manual");
  const [policy, setPolicy] = useState<PairPolicy>("carrying-value");
  useEffect(() => {
    if (pair) {
      setKind(pair.kind);
      setPolicy(pair.policy);
    }
  }, [pair]);
  const dirty = pair ? kind !== pair.kind || policy !== pair.policy : false;
  const sameAsset = pair ? pairIsSameAsset(pair) : false;
  const presentationType = pair ? pairPresentationType(pair) : "transfer";
  const sourceKey = pairSourceLabelKey(pair?.pair_source ?? null);
  return (
    <Sheet open={Boolean(pair)} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto p-0 sm:max-w-2xl">
        {pair ? (
          <>
            <SheetHeader className="border-b p-4 sm:p-6">
              <SheetTitle>
                {t(
                  presentationType === "transfer"
                    ? "swap.detail.candidateLabelTransfer"
                    : presentationType === "layer-transition"
                      ? "swap.detail.candidateLabelLayerTransition"
                      : "swap.detail.candidateLabelSwap",
                )}
              </SheetTitle>
              <SheetDescription>
                <span className={blurClass(hideSensitive)}>
                  {t("swap.detail.delta", {
                    delta: formatSats(pair.swap_fee_msat ?? 0),
                    percent: feePercent(pairFee(pair)).toFixed(2),
                  })}
                </span>
              </SheetDescription>
            </SheetHeader>
            <div className="space-y-4 p-4 sm:p-6">
              <div className="grid gap-4 md:grid-cols-2">
                <SwapLegDetails
                  title={t("swap.detail.outgoing")}
                  asset={pair.out.asset}
                  amount={pair.out.amount}
                  amountMsat={pair.out.amount_msat}
                  wallet={pair.out.wallet}
                  walletKind={pair.out.wallet_kind}
                  timestamp={pair.out.occurred_at}
                  txId={pair.out.transaction_id}
                  hideSensitive={hideSensitive}
                />
                <SwapLegDetails
                  title={t("swap.detail.incoming")}
                  asset={pair.in.asset}
                  amount={pair.in.amount}
                  amountMsat={pair.in.amount_msat}
                  wallet={pair.in.wallet}
                  walletKind={pair.in.wallet_kind}
                  timestamp={pair.in.occurred_at}
                  txId={pair.in.transaction_id}
                  hideSensitive={hideSensitive}
                />
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>{t("swap.detail.kind")}</Label>
                  <Select value={kind} onValueChange={(value) => setKind(value as PairKind)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PAIR_KIND_OPTIONS.map((option) => (
                        <SelectItem key={option} value={option}>
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label>{t("swap.detail.policy")}</Label>
                  <Select
                    value={policy}
                    onValueChange={(value) => setPolicy(value as PairPolicy)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PAIR_POLICY_OPTIONS.map((option) => (
                        <SelectItem
                          key={option}
                          value={option}
                          disabled={sameAsset && option === "taxable"}
                        >
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {sameAsset ? (
                    <p className="text-[11px] text-muted-foreground">
                      {t("swap.paired.sameAssetTaxableHint")}
                    </p>
                  ) : null}
                </div>
              </div>

              <div className="rounded-lg border bg-muted/20 p-3 text-sm">
                <dl className="space-y-1 text-xs">
                  <DetailRow
                    label={t("swap.paired.source")}
                    value={sourceKey ? t(sourceKey) : pair.pair_source ?? "—"}
                  />
                  <DetailRow
                    label={t("swap.paired.created")}
                    value={formatTimestamp(pair.created_at)}
                  />
                  <DetailRow
                    label={t(
                      presentationType === "swap"
                        ? "swap.detail.swapFee"
                        : presentationType === "layer-transition"
                          ? "swap.detail.layerTransitionFee"
                          : "swap.detail.transferFee",
                    )}
                    value={
                      <span className={blurClass(hideSensitive)}>
                        {t("swap.detail.feeLine", {
                          fee: formatSats(pair.swap_fee_msat ?? 0),
                          percent: feePercent(pairFee(pair)).toFixed(2),
                        })}
                      </span>
                    }
                  />
                  <DetailRow
                    label={t("swap.paired.notes")}
                    value={
                      pair.notes ? (
                        <span className={blurClass(hideSensitive)}>{pair.notes}</span>
                      ) : (
                        t("swap.paired.noNotes")
                      )
                    }
                  />
                </dl>
                <p className="mt-2 text-xs text-muted-foreground">
                  {t("swap.detail.deltasNote")}
                </p>
                {presentationType === "layer-transition" ? (
                  <p className="mt-2 text-xs text-muted-foreground">
                    {t("swap.detail.layerTransitionOwnershipHint")}
                  </p>
                ) : null}
              </div>
            </div>
            <SheetFooter className="border-t p-4 sm:flex-row sm:items-center sm:justify-between sm:px-6">
              <Button variant="outline" onClick={() => onUnpair(pair)}>
                <Unlink className="mr-2 size-4" aria-hidden="true" />
                {t("swap.paired.unpair")}
              </Button>
              <Button onClick={() => onSave(pair, kind, policy)} disabled={!dirty || saving}>
                {saving ? <Loader2 className="mr-1 size-4 animate-spin" /> : null}
                {t("swap.paired.save")}
              </Button>
            </SheetFooter>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

function PairingReview({
  mode,
  onShowHistory,
}: {
  mode: PairingReviewMode;
  onShowHistory: () => void;
}) {
  const { t } = useTranslation(["review", "common"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const candidateType = mode === "transfers" ? "transfer" : "swap";
  const savedViewSurface =
    mode === "transfers" ? "transfer_candidates" : "swap_candidates";
  const routeFilterEnabled = true;
  const pageTitle =
    mode === "transfers" ? t("swap.page.transfersTitle") : t("swap.page.swapsTitle");
  const pageDescription =
    mode === "transfers"
      ? t("swap.page.transfersDescription")
      : t("swap.page.swapsDescription");
  const emptyText =
    mode === "transfers" ? t("swap.page.transfersEmpty") : t("swap.page.swapsEmpty");
  const [confidence, setConfidence] = useState<string>("all");
  const [method, setMethod] = useState<string>("all");
  const [routePair, setRoutePair] = useState<string>("all");
  const [overrides, setOverrides] = useState<
    Record<string, { kind?: PairKind; policy?: PairPolicy }>
  >({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [bulkKind, setBulkKind] = useState<PairKind | null>(null);
  const [bulkPolicy, setBulkPolicy] = useState<PairPolicy | null>(null);
  const [previewState, setPreviewState] = useState<
    | { mode: "exact"; candidates: SwapCandidate[] }
    | { mode: "rules"; candidates: SwapCandidate[] }
    | { mode: "selected"; candidates: SwapCandidate[] }
    | null
  >(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [undoState, setUndoState] = useState<{
    pairIds: string[];
    summary: BulkPairResult["summary"];
    deadline: number;
  } | null>(null);
  const undoTimerRef = useRef<number | null>(null);

  useEffect(() => {
    if (!undoState) return;
    const remaining = undoState.deadline - Date.now();
    if (remaining <= 0) {
      setUndoState(null);
      return;
    }
    undoTimerRef.current = window.setTimeout(() => {
      setUndoState(null);
    }, remaining);
    return () => {
      if (undoTimerRef.current !== null) {
        window.clearTimeout(undoTimerRef.current);
        undoTimerRef.current = null;
      }
    };
  }, [undoState]);

  const args = useMemo(() => {
    const next: Record<string, unknown> = { candidate_type: candidateType };
    if (confidence !== "all") next.confidence = confidence;
    if (method !== "all") next.method = method;
    if (routeFilterEnabled && routePair !== "all") next.route_pair = routePair;
    return next;
  }, [candidateType, confidence, method, routeFilterEnabled, routePair]);

  const { data, isLoading, isError, error, refetch, isFetching } =
    useDaemon<SuggestEnvelope>("ui.transfers.suggest", args);

  // Count of already-paired pairs for this rail — powers the History card.
  // Shares the cached query with the paired (History) view, so it's one fetch.
  const pairedListQuery = useDaemon<PairsEnvelope>("ui.transfers.list");
  const historyCount = useMemo(
    () =>
      (pairedListQuery.data?.data?.pairs ?? []).filter(
        (pair) => pairReviewMode(pair) === mode,
      ).length,
    [pairedListQuery.data, mode],
  );

  const pairMutation = useDaemonMutation<unknown>("ui.transfers.pair");
  const dismissMutation = useDaemonMutation<unknown>("ui.transfers.dismiss");
  const bulkPairMutation = useDaemonMutation<BulkPairResult>("ui.transfers.bulk_pair");
  const unpairMutation = useDaemonMutation<unknown>("ui.transfers.unpair");

  const savedViewsQuery = useDaemon<SavedViewsEnvelope>("ui.saved_views.list", {
    surface: savedViewSurface,
  });
  const savedViewCreate = useDaemonMutation<SavedView>("ui.saved_views.create");
  const savedViewDelete = useDaemonMutation<unknown>("ui.saved_views.delete");
  const rulesQuery = useDaemon<RulesEnvelope>("ui.transfers.rules.list");
  const ruleCreate = useDaemonMutation<SwapRule>("ui.transfers.rules.create");
  const ruleDelete = useDaemonMutation<unknown>("ui.transfers.rules.delete");
  const ruleSetEnabled = useDaemonMutation<SwapRule>("ui.transfers.rules.set_enabled");
  const ruleApply = useDaemonMutation<BulkPairResult>("ui.transfers.rules.apply");

  const [saveViewOpen, setSaveViewOpen] = useState(false);
  const [saveViewName, setSaveViewName] = useState("");
  const [createRuleOpen, setCreateRuleOpen] = useState(false);
  const [rulesExpanded, setRulesExpanded] = useState(false);
  const [cursorIndex, setCursorIndex] = useState(0);
  const [helpOpen, setHelpOpen] = useState(false);
  const [detailCandidate, setDetailCandidate] = useState<SwapCandidate | null>(null);
  const savedViews = savedViewsQuery.data?.data?.views ?? [];
  const rules = rulesQuery.data?.data?.rules ?? [];
  const enabledRuleCount = rules.filter((rule) => rule.enabled).length;

  const filterIsDirty =
    confidence !== "all" ||
    method !== "all" ||
    (routeFilterEnabled && routePair !== "all");

  const applySavedView = (view: SavedView) => {
    setConfidence(typeof view.filter.confidence === "string" ? view.filter.confidence : "all");
    setMethod(typeof view.filter.method === "string" ? view.filter.method : "all");
    if (!routeFilterEnabled) {
      setRoutePair("all");
      return;
    }
    const savedRoutePair =
      typeof view.filter.route_pair === "string" && ROUTE_PAIR_VALUES.has(view.filter.route_pair)
        ? view.filter.route_pair
        : typeof view.filter.asset_pair === "string" && ROUTE_PAIR_VALUES.has(view.filter.asset_pair)
          ? view.filter.asset_pair
        : "all";
    setRoutePair(savedRoutePair);
  };

  const commitSaveView = async () => {
    const name = saveViewName.trim();
    if (!name) return;
    const filterPayload: Record<string, unknown> = {};
    if (confidence !== "all") filterPayload.confidence = confidence;
    if (method !== "all") filterPayload.method = method;
    if (routeFilterEnabled && routePair !== "all") filterPayload.route_pair = routePair;
    try {
      await savedViewCreate.mutateAsync({
        surface: savedViewSurface,
        name,
        filter: filterPayload,
      });
      setSaveViewName("");
      setSaveViewOpen(false);
      void savedViewsQuery.refetch();
    } catch {
      // Conflict surfaces as a mutation error; leave dialog open so user can rename.
    }
  };

  const deleteSavedView = async (view: SavedView) => {
    await savedViewDelete.mutateAsync({ view_id: view.id });
    void savedViewsQuery.refetch();
  };

  const toggleRule = async (rule: SwapRule) => {
    await ruleSetEnabled.mutateAsync({ rule_id: rule.id, enabled: !rule.enabled });
    void rulesQuery.refetch();
  };

  const deleteRule = async (rule: SwapRule) => {
    await ruleDelete.mutateAsync({ rule_id: rule.id });
    void rulesQuery.refetch();
  };

  const candidates = useMemo(
    () => data?.data?.candidates ?? [],
    [data?.data?.candidates],
  );
  const counts = data?.data?.counts ?? { total: 0, exact: 0, strong: 0, conflicts: 0 };

  // Count of cluster members visible under the current filters/tab.
  // conflict_size is stamped server-side over the full candidate set, so
  // visibleClusterCounts < conflict_size means siblings are hidden here.
  const visibleClusterCounts = useMemo(() => {
    const sizes: Record<string, number> = {};
    for (const candidate of candidates) {
      sizes[candidate.conflict_set_id] = (sizes[candidate.conflict_set_id] ?? 0) + 1;
    }
    return sizes;
  }, [candidates]);

  const candidateKey = (c: SwapCandidate) => `${c.out_id}->${c.in_id}`;

  const exactSolo = useMemo(
    () =>
      candidates.filter(
        (c) => c.confidence === "exact" && c.conflict_size <= 1,
      ),
    [candidates],
  );

  const ruleSolo = useMemo(
    () =>
      candidates.filter(
        (c) => c.rule_match && c.conflict_size <= 1,
      ),
    [candidates],
  );

  const selectableCandidates = useMemo(
    () => candidates.filter((c) => c.conflict_size <= 1),
    [candidates],
  );

  const selectableCandidatesByKey = useMemo(() => {
    const map: Record<string, SwapCandidate> = {};
    for (const candidate of selectableCandidates) {
      map[candidateKey(candidate)] = candidate;
    }
    return map;
  }, [selectableCandidates]);

  const selectedCandidates = useMemo(
    () =>
      Array.from(selected)
        .map((key) => selectableCandidatesByKey[key])
        .filter((c): c is SwapCandidate => Boolean(c)),
    [selected, selectableCandidatesByKey],
  );
  const selectedCandidateCount = selectedCandidates.length;

  useEffect(() => {
    setSelected((prev) => {
      if (prev.size === 0) return prev;
      const next = new Set<string>();
      for (const key of prev) {
        if (selectableCandidatesByKey[key]) next.add(key);
      }
      return next.size === prev.size ? prev : next;
    });
  }, [selectableCandidatesByKey]);

  const toggleSelected = useCallback((key: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    }), []);

  const handleSelectAll = useCallback(() => {
    const eligibleKeys = selectableCandidates.map(candidateKey);
    setSelected((prev) => {
      const allEligibleSelected =
        eligibleKeys.length > 0 && eligibleKeys.every((key) => prev.has(key));
      return allEligibleSelected ? new Set() : new Set(eligibleKeys);
    });
  }, [selectableCandidates]);

  const handlePair = useCallback(async (candidate: SwapCandidate) => {
    const key = candidateKey(candidate);
    const override = overrides[key] ?? {};
    await pairMutation.mutateAsync({
      tx_out: candidate.out_id,
      tx_in: candidate.in_id,
      kind: override.kind ?? candidate.default_kind,
      policy: override.policy ?? candidate.default_policy,
      pair_source: "manual",
      confidence_at_pair: candidate.confidence,
    });
    void refetch();
  }, [overrides, pairMutation, refetch]);

  const handleDismiss = useCallback(async (candidate: SwapCandidate) => {
    await dismissMutation.mutateAsync({
      tx_out: candidate.out_id,
      tx_in: candidate.in_id,
      reason: t("swap.dismissReason"),
    });
    void refetch();
  }, [dismissMutation, refetch, t]);

  const openExactPreview = useCallback(() => {
    setPreviewError(null);
    setPreviewState({ mode: "exact", candidates: exactSolo });
  }, [exactSolo]);

  const openRulesPreview = () => {
    setPreviewError(null);
    setPreviewState({ mode: "rules", candidates: ruleSolo });
  };

  const openSelectedPreview = () => {
    setPreviewError(null);
    setPreviewState({ mode: "selected", candidates: selectedCandidates });
  };

  const commitBulk = async () => {
    if (!previewState) return;
    setPreviewError(null);
    try {
      if (previewState.mode === "exact" || previewState.mode === "rules") {
        const envelope =
          previewState.mode === "exact"
            ? await bulkPairMutation.mutateAsync({ ...args, confidence: "exact" })
            : await ruleApply.mutateAsync(args);
        const result = envelope.data;
        if (!result || result.summary.count === 0) {
          await refetch();
          setPreviewError(t("swap.preview.noPairsExact"));
          return;
        }
        setUndoState({
          pairIds: result.applied.map((p) => p.id),
          summary: result.summary,
          deadline: Date.now() + UNDO_WINDOW_MS,
        });
      } else {
        const applied: string[] = [];
        let totalFee = 0;
        for (const candidate of previewState.candidates) {
          const key = candidateKey(candidate);
          const override = overrides[key] ?? {};
          const envelope = await pairMutation.mutateAsync({
            tx_out: candidate.out_id,
            tx_in: candidate.in_id,
            kind: override.kind ?? bulkKind ?? candidate.default_kind,
            policy: override.policy ?? bulkPolicy ?? candidate.default_policy,
            pair_source: "bulk_selected",
            confidence_at_pair: candidate.confidence,
          });
          const created = envelope.data as { id?: string; swap_fee_msat?: number } | undefined;
          if (created?.id) applied.push(created.id);
          if (typeof created?.swap_fee_msat === "number") totalFee += created.swap_fee_msat;
        }
        if (applied.length === 0) {
          await refetch();
          setPreviewError(t("swap.preview.noPairsSelected"));
          return;
        }
        setUndoState({
          pairIds: applied,
          summary: {
            count: applied.length,
            skipped_conflicts: 0,
            total_swap_fee_msat: totalFee,
          },
          deadline: Date.now() + UNDO_WINDOW_MS,
        });
      }
      setPreviewState(null);
      setSelected(new Set());
      await refetch();
    } catch (error) {
      setPreviewError(
        error instanceof Error ? error.message : t("swap.preview.pairFailed"),
      );
    }
  };

  const cancelUndo = () => {
    setUndoState(null);
  };

  const performUndo = useCallback(async () => {
    if (!undoState) return;
    const pairIds = undoState.pairIds;
    setUndoState(null);
    for (const pairId of pairIds) {
      try {
        await unpairMutation.mutateAsync({ pair_id: pairId });
      } catch {
        // Swallow per-row failures; the next refetch surfaces the actual state.
      }
    }
    void refetch();
  }, [refetch, undoState, unpairMutation]);

  useEffect(() => {
    if (cursorIndex >= candidates.length) {
      setCursorIndex(Math.max(0, candidates.length - 1));
    }
  }, [candidates.length, cursorIndex]);

  const cursorCandidate = candidates[cursorIndex];
  const cursorKey = cursorCandidate ? candidateKey(cursorCandidate) : null;

  const bindings = useMemo<Keybinding[]>(() => {
    return [
      {
        keys: ["?", "Shift+?"],
        description: t("swap.keymap.showShortcuts"),
        category: t("swap.keymap.categoryHelp"),
        handler: () => setHelpOpen(true),
      },
      {
        keys: "Escape",
        description: t("swap.keymap.clearSelection"),
        category: t("swap.keymap.categorySelection"),
        handler: () => {
          if (helpOpen) setHelpOpen(false);
          else if (detailCandidate) setDetailCandidate(null);
          else if (previewState) setPreviewState(null);
          else if (selected.size > 0) setSelected(new Set());
        },
      },
      {
        keys: ["j", "ArrowDown"],
        description: t("swap.keymap.cursorDown"),
        category: t("swap.keymap.categoryNavigation"),
        handler: () => {
          if (candidates.length === 0) return;
          setCursorIndex((idx) => Math.min(candidates.length - 1, idx + 1));
        },
      },
      {
        keys: ["k", "ArrowUp"],
        description: t("swap.keymap.cursorUp"),
        category: t("swap.keymap.categoryNavigation"),
        handler: () => {
          if (candidates.length === 0) return;
          setCursorIndex((idx) => Math.max(0, idx - 1));
        },
      },
      {
        keys: " ",
        description: t("swap.keymap.toggleSelection"),
        category: t("swap.keymap.categorySelection"),
        handler: () => {
          if (!cursorCandidate) return;
          if (cursorCandidate.conflict_size > 1) return;
          toggleSelected(candidateKey(cursorCandidate));
        },
      },
      {
        keys: "a",
        description: t("swap.keymap.selectAll"),
        category: t("swap.keymap.categorySelection"),
        handler: () => handleSelectAll(),
      },
      {
        keys: "p",
        description: t("swap.keymap.pairCurrent"),
        category: t("swap.keymap.categoryActions"),
        handler: () => {
          if (cursorCandidate) void handlePair(cursorCandidate);
        },
      },
      {
        keys: "d",
        description: t("swap.keymap.dismissCurrent"),
        category: t("swap.keymap.categoryActions"),
        handler: () => {
          if (cursorCandidate) void handleDismiss(cursorCandidate);
        },
      },
      {
        keys: "e",
        description: t("swap.keymap.applyExactPreview"),
        category: t("swap.keymap.categoryActions"),
        handler: () => {
          if (exactSolo.length > 0) openExactPreview();
        },
      },
      {
        keys: "u",
        description: t("swap.keymap.undoLast"),
        category: t("swap.keymap.categoryActions"),
        handler: () => {
          if (undoState) void performUndo();
        },
      },
      {
        keys: "r",
        description: t("swap.keymap.refresh"),
        category: t("swap.keymap.categoryNavigation"),
        handler: () => void refetch(),
      },
    ];
  }, [
    candidates,
    cursorCandidate,
    exactSolo,
    detailCandidate,
    helpOpen,
    handleDismiss,
    handlePair,
    handleSelectAll,
    openExactPreview,
    performUndo,
    previewState,
    refetch,
    selected,
    toggleSelected,
    undoState,
    t,
  ]);

  useKeymap(bindings);

  const bulkCommitPending =
    bulkPairMutation.isPending || pairMutation.isPending || ruleApply.isPending;

  return (
    <div className="min-w-0">
      <Collapsible open={rulesExpanded} onOpenChange={setRulesExpanded}>
        <div className="overflow-hidden rounded-xl border bg-card">
          <header className="flex flex-col gap-2.5 px-3 py-3 sm:flex-row sm:items-start sm:justify-between sm:px-4">
            <div className="min-w-0">
              <p className="text-[10px] font-medium tracking-[0.18em] text-muted-foreground uppercase">
                {t("swap.queueLabel")}
              </p>
              <div className="mt-0.5 flex flex-wrap items-center gap-2">
                <h1 className="text-base font-semibold">
                  {pageTitle}
                </h1>
              </div>
              <p className="max-w-3xl text-sm text-muted-foreground">
                {pageDescription}
              </p>
              {savedViews.length > 0 ? (
                <div className="flex flex-wrap items-center gap-1 text-xs">
                  <Star className="size-3.5 text-muted-foreground" aria-hidden="true" />
                  {savedViews.map((view) => (
                    <span
                      key={view.id}
                      className="inline-flex items-center gap-1 rounded-full border border-input bg-background px-2 py-0.5"
                    >
                      <button
                        className="font-medium text-foreground/90 hover:text-foreground"
                        onClick={() => applySavedView(view)}
                      >
                        {view.name}
                      </button>
                      <button
                        aria-label={t("swap.savedView.deleteAria", { name: view.name })}
                        onClick={() => void deleteSavedView(view)}
                      >
                        <X className="size-3" />
                      </button>
                    </span>
                  ))}
                </div>
              ) : null}
            </div>

            <div className="flex shrink-0 flex-wrap items-center gap-2">
              {exactSolo.length > 0 ? (
                <Button
                  size="sm"
                  className="h-9 whitespace-nowrap"
                  onClick={openExactPreview}
                  disabled={bulkPairMutation.isPending}
                >
                  <Sparkles className="size-4" />
                  <span>{t("swap.header.applyExact", { count: exactSolo.length })}</span>
                </Button>
              ) : null}
              <Button
                variant="outline"
                size="sm"
                className="h-9"
                onClick={() => void refetch()}
                disabled={isFetching}
              >
                {isFetching ? <Loader2 className="size-4 animate-spin" /> : null}
                <span className="ml-1">{t("common:actions.refresh")}</span>
              </Button>
              <CollapsibleTrigger asChild>
                <Button variant="outline" size="sm" className="h-9">
                  <SettingsIcon className="size-3.5" />
                  <span>{t("swap.header.rules", { enabled: enabledRuleCount, total: rules.length })}</span>
                </Button>
              </CollapsibleTrigger>
            </div>
          </header>

          <div className="grid grid-cols-2 divide-x-0 divide-y divide-border border-t sm:grid-cols-5 sm:divide-x sm:divide-y-0">
            <SwapQueueMetric
              label={t("swap.metric.candidates")}
              ariaLabel={t("swap.metric.showAllAria", { label: t("swap.metric.candidates") })}
              value={counts.total}
              tone={counts.total ? "neutral" : "good"}
              active={!filterIsDirty}
              onClick={() => {
                setConfidence("all");
                setMethod("all");
                if (routeFilterEnabled) setRoutePair("all");
              }}
            />
            <SwapQueueMetric
              label={t("swap.metric.exact")}
              ariaLabel={t("swap.metric.filterAria", { label: t("swap.metric.exact") })}
              value={counts.exact}
              tone={counts.exact ? "good" : "neutral"}
              active={confidence === "exact"}
              onClick={() => setConfidence(confidence === "exact" ? "all" : "exact")}
            />
            <SwapQueueMetric
              label={t("swap.metric.strong")}
              ariaLabel={t("swap.metric.filterAria", { label: t("swap.metric.strong") })}
              value={counts.strong}
              tone={counts.strong ? "warning" : "neutral"}
              active={confidence === "strong"}
              onClick={() => setConfidence(confidence === "strong" ? "all" : "strong")}
            />
            <SwapQueueMetric
              label={t("swap.metric.conflicts")}
              value={counts.conflicts}
              tone={counts.conflicts ? "alert" : "neutral"}
            />
            <SwapQueueMetric
              label={t("swap.metric.history")}
              ariaLabel={t("swap.metric.historyAria")}
              value={historyCount}
              icon={<HistoryIcon className="size-3.5" aria-hidden="true" />}
              onClick={onShowHistory}
            />
          </div>

          <div className="grid gap-2 border-t px-2 py-3 text-sm xl:grid-cols-[minmax(0,1fr)_auto] xl:items-center">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <label className="flex shrink-0 items-center gap-2">
                <Checkbox
                  checked={selectedCandidateCount > 0}
                  onCheckedChange={handleSelectAll}
                />
                <span className="text-xs text-muted-foreground">
                  {t("swap.filters.select")}
                </span>
              </label>
              <span className="shrink-0 text-xs text-muted-foreground">
                {t("swap.filters.visible", { count: candidates.length })}
              </span>
              <Select value={confidence} onValueChange={setConfidence}>
                <SelectTrigger className="h-8 w-40 shrink-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CONFIDENCE_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {t(option.labelKey)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Select value={method} onValueChange={setMethod}>
                <SelectTrigger className="h-8 w-36 shrink-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {METHOD_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {t(option.labelKey)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {routeFilterEnabled ? (
                <Select value={routePair} onValueChange={setRoutePair}>
                  <SelectTrigger className="h-8 w-44 shrink-0" aria-label={t("swap.filters.routeAria")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {ROUTE_PAIR_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {t(option.labelKey)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : null}
              {filterIsDirty ? (
                <>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 shrink-0 px-2"
                    onClick={() => setSaveViewOpen(true)}
                  >
                    <Star className="size-3.5" />
                    <span>{t("swap.filters.save")}</span>
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 shrink-0 px-2"
                    onClick={() => {
                      setConfidence("all");
                      setMethod("all");
                      if (routeFilterEnabled) setRoutePair("all");
                    }}
                  >
                    {t("swap.filters.clear")}
                  </Button>
                </>
              ) : null}
            </div>
            <div className="flex shrink-0 items-center justify-end gap-2">
              {ruleSolo.length > 0 ? (
                <Button
                  size="sm"
                  variant="outline"
                  className="h-8 whitespace-nowrap"
                  onClick={openRulesPreview}
                  disabled={ruleApply.isPending}
                >
                  <Sparkles className="size-4" />
                  <span>{t("swap.filters.applyRule", { count: ruleSolo.length })}</span>
                </Button>
              ) : null}
            </div>
          </div>

          <CollapsibleContent>
            <div className="space-y-2 border-t bg-muted/10 p-3 text-xs sm:px-6">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">{t("swap.rules.title")}</span>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 text-xs"
                  onClick={() => setCreateRuleOpen(true)}
                >
                  <Plus className="size-3" />
                  <span>{t("swap.rules.new")}</span>
                </Button>
              </div>
              {rules.length === 0 ? (
                <p className="text-muted-foreground">
                  {t("swap.rules.empty")}
                </p>
              ) : (
                rules.map((rule) => (
                  <div
                    key={rule.id}
                    className="flex flex-wrap items-center gap-2 rounded border border-border/60 bg-background px-2 py-1"
                  >
                    <span className="font-medium">{rule.name ?? t("swap.rules.unnamed")}</span>
                    <code className="rounded bg-muted px-1 text-[10px]">
                      {Object.entries(rule.predicate)
                        .filter(([, v]) => v !== null && v !== "")
                        .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
                        .join(" · ") || t("swap.rules.anyCandidate")}
                    </code>
                    <Badge variant="outline" className="text-[10px]">
                      {rule.kind} · {rule.policy}
                    </Badge>
                    <div className="ml-auto flex items-center gap-2">
                      <Switch
                        checked={rule.enabled}
                        onCheckedChange={() => void toggleRule(rule)}
                        aria-label={t("swap.rules.toggleAria")}
                      />
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-6 px-1"
                        onClick={() => void deleteRule(rule)}
                        aria-label={t("swap.rules.deleteAria")}
                      >
                        <Trash2 className="size-3" />
                      </Button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </CollapsibleContent>

          {isLoading ? (
            <div className="flex items-center gap-2 border-t px-6 py-8 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" /> {t("swap.table.loading")}
            </div>
          ) : isError ? (
            <div className="border-t px-6 py-6">
              <div className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm">
                {t("swap.table.loadFailed", { error: String(error) })}
              </div>
            </div>
          ) : candidates.length === 0 ? (
            <div className="border-t px-6 py-8">
              <div className="rounded border border-dashed border-muted-foreground/40 p-6 text-center text-sm text-muted-foreground">
                {emptyText}
              </div>
            </div>
          ) : (
            <div className="overflow-x-auto border-t">
              <Table className="min-w-[1180px] w-full table-fixed">
                <TableHeader>
                  <TableRow className="bg-muted/50 hover:bg-muted/50">
                    <TableHead className="w-[42px]"></TableHead>
                    <TableHead className="w-[140px] text-xs font-medium text-muted-foreground">
                      {t("swap.table.status")}
                    </TableHead>
                    <TableHead className="w-[380px] text-xs font-medium text-muted-foreground">
                      {t("swap.table.outgoing")}
                    </TableHead>
                    <TableHead className="w-[44px] text-center"></TableHead>
                    <TableHead className="w-[380px] text-xs font-medium text-muted-foreground">
                      {t("swap.table.incoming")}
                    </TableHead>
                    <TableHead className="w-[160px] text-right text-xs font-medium text-muted-foreground">
                      {t("swap.table.feeDelta")}
                    </TableHead>
                    <TableHead className="w-[44px]"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {candidates.map((candidate) => {
                    const key = candidateKey(candidate);
                    const conflicted = candidate.conflict_size > 1;
                    const hiddenSiblings =
                      candidate.conflict_size -
                      (visibleClusterCounts[candidate.conflict_set_id] ?? 1);
                    return (
                      <TableRow
                        key={key}
                        className={cn(
                          "cursor-pointer align-middle hover:bg-muted/35",
                          conflicted ? "bg-rose-50/40 dark:bg-rose-950/20" : null,
                          cursorKey === key ? "bg-muted/60" : null,
                        )}
                        onClick={() => setDetailCandidate(candidate)}
                      >
                        <TableCell>
                          <Checkbox
                            aria-label={t("swap.table.selectAria")}
                            disabled={conflicted}
                            checked={!conflicted && selected.has(key)}
                            onClick={(event) => event.stopPropagation()}
                            onCheckedChange={() => toggleSelected(key)}
                          />
                        </TableCell>
                        <TableCell className="whitespace-normal">
                          <SwapStatusCell
                            candidate={candidate}
                            conflicted={conflicted}
                            hiddenSiblings={hiddenSiblings}
                          />
                        </TableCell>
                        <TableCell className="whitespace-nowrap">
                          <SwapLegInline
                            direction="out"
                            asset={candidate.out_asset}
                            amount={candidate.out_amount}
                            wallet={candidate.out_wallet_label}
                            walletKind={candidate.out_wallet_kind}
                            timestamp={candidate.out_occurred_at}
                            txId={candidate.out_id}
                            hideSensitive={hideSensitive}
                          />
                        </TableCell>
                        <TableCell className="text-center text-muted-foreground">
                          <ArrowRight className="mx-auto mt-1 size-4" aria-hidden="true" />
                        </TableCell>
                        <TableCell className="whitespace-nowrap">
                          <SwapLegInline
                            direction="in"
                            asset={candidate.in_asset}
                            amount={candidate.in_amount}
                            wallet={candidate.in_wallet_label}
                            walletKind={candidate.in_wallet_kind}
                            timestamp={candidate.in_occurred_at}
                            txId={candidate.in_id}
                            hideSensitive={hideSensitive}
                          />
                        </TableCell>
                        <TableCell className="whitespace-nowrap text-right">
                          <SwapFeeText candidate={candidate} hideSensitive={hideSensitive} />
                        </TableCell>
                        <TableCell>
                          <SwapRowMenu
                            candidate={candidate}
                            onOpen={() => setDetailCandidate(candidate)}
                            onPair={() => void handlePair(candidate)}
                            onDismiss={() => void handleDismiss(candidate)}
                            pairDisabled={pairMutation.isPending}
                            dismissDisabled={dismissMutation.isPending || pairMutation.isPending}
                          />
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          )}

          {selectedCandidateCount > 0 ? (
            <div className="flex flex-wrap items-center gap-3 border-t bg-muted/25 px-3 py-3 text-sm sm:px-6">
              <span className="shrink-0 text-xs font-medium text-foreground">
                {t("swap.bulk.selected", { count: selectedCandidateCount })}
              </span>
              <label className="flex min-w-[16rem] items-center gap-2 text-xs text-muted-foreground">
                {t("swap.bulk.kind")}
                <Select
                  value={bulkKind ?? "default"}
                  onValueChange={(v) => setBulkKind(v === "default" ? null : v as PairKind)}
                >
                  <SelectTrigger className="h-8 w-52">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">{t("swap.bulk.candidateDefault")}</SelectItem>
                    {PAIR_KIND_OPTIONS.map((option) => (
                      <SelectItem key={option} value={option}>
                        {option}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label className="flex min-w-[16rem] items-center gap-2 text-xs text-muted-foreground">
                {t("swap.bulk.policy")}
                <Select
                  value={bulkPolicy ?? "default"}
                  onValueChange={(v) => setBulkPolicy(v === "default" ? null : v as PairPolicy)}
                >
                  <SelectTrigger className="h-8 w-52">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">{t("swap.bulk.candidateDefault")}</SelectItem>
                    {PAIR_POLICY_OPTIONS.map((option) => (
                      <SelectItem key={option} value={option}>
                        {option}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <Button
                size="sm"
                className="ml-auto h-8"
                onClick={openSelectedPreview}
                disabled={pairMutation.isPending}
              >
                {t("swap.bulk.pairSelected")}
              </Button>
            </div>
          ) : null}

          <div className="flex items-center border-t px-3 py-3 text-xs text-muted-foreground sm:px-6">
            <span>
              {t("swap.showing", {
                from: candidates.length === 0 ? 0 : 1,
                to: candidates.length,
                total: counts.total,
              })}
            </span>
          </div>
        </div>
      </Collapsible>

      <SwapCandidateDetailSheet
        candidate={detailCandidate}
        override={detailCandidate ? overrides[candidateKey(detailCandidate)] ?? {} : {}}
        onOpenChange={(open) => {
          if (!open) setDetailCandidate(null);
        }}
        onKindChange={(candidate, value) =>
          setOverrides((prev) => ({
            ...prev,
            [candidateKey(candidate)]: { ...prev[candidateKey(candidate)], kind: value },
          }))
        }
        onPolicyChange={(candidate, value) =>
          setOverrides((prev) => ({
            ...prev,
            [candidateKey(candidate)]: { ...prev[candidateKey(candidate)], policy: value },
          }))
        }
        onPair={(candidate) => {
          setDetailCandidate(null);
          void handlePair(candidate);
        }}
        onDismiss={(candidate) => {
          setDetailCandidate(null);
          void handleDismiss(candidate);
        }}
        pairDisabled={pairMutation.isPending}
        dismissDisabled={dismissMutation.isPending || pairMutation.isPending}
        hideSensitive={hideSensitive}
      />

      <Dialog
        open={previewState !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPreviewError(null);
            setPreviewState(null);
          }
        }}
      >
        <DialogContent className="grid max-h-[85vh] w-[calc(100vw-2rem)] max-w-2xl grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0">
          <DialogHeader className="border-b px-5 py-4 pr-12">
            <DialogTitle>
              {previewState?.mode === "exact"
                ? t("swap.preview.exactTitle")
                : previewState?.mode === "rules"
                  ? t("swap.preview.rulesTitle")
                  : t("swap.preview.selectedTitle")}
            </DialogTitle>
            <DialogDescription className={blurClass(hideSensitive)}>
              {previewState
                ? (() => {
                    const summary = previewSummary(previewState.candidates);
                    return t(summary.key, summary.params);
                  })()
                : null}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 overflow-y-auto px-5 py-4">
            {previewError ? (
              <div className="mb-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">
                {previewError}
              </div>
            ) : null}
            <div className="rounded-md border border-border/60 text-sm">
              {previewState?.candidates.map((candidate) => (
                <div
                  key={`${candidate.out_id}->${candidate.in_id}`}
                  className="grid gap-1 border-b border-border/40 p-2.5 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
                >
                  <span className={cn("min-w-0 truncate text-xs", blurClass(hideSensitive))}>
                    {displayAssetLabel(candidate.out_asset)} {formatBtc(candidate.out_amount)} →{" "}
                    {displayAssetLabel(candidate.in_asset)} {formatBtc(candidate.in_amount)}
                  </span>
                  <span className={cn("text-xs text-muted-foreground", blurClass(hideSensitive))}>
                    {t("swap.preview.feeLine", { fee: formatSats(candidate.swap_fee_msat) })}
                  </span>
                </div>
              ))}
            </div>
          </div>
          <DialogFooter className="border-t px-5 py-4">
            <Button
              variant="outline"
              onClick={() => {
                setPreviewError(null);
                setPreviewState(null);
              }}
            >
              {t("common:actions.cancel")}
            </Button>
            <Button
              onClick={() => void commitBulk()}
              disabled={bulkCommitPending}
            >
              {bulkCommitPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              <span className="ml-1">
                {t("swap.preview.pairCount", { count: previewState?.candidates.length ?? 0 })}
              </span>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <KeymapHelpDialog
        open={helpOpen}
        onClose={() => setHelpOpen(false)}
        bindings={bindings}
      />

      <SaveViewDialog
        open={saveViewOpen}
        name={saveViewName}
        onNameChange={setSaveViewName}
        onCancel={() => {
          setSaveViewOpen(false);
          setSaveViewName("");
        }}
        onSave={commitSaveView}
        isSaving={savedViewCreate.isPending}
      />

      <CreateRuleDialog
        open={createRuleOpen}
        onClose={() => setCreateRuleOpen(false)}
        onCreate={async (payload) => {
          await ruleCreate.mutateAsync({ ...payload });
          void rulesQuery.refetch();
          setCreateRuleOpen(false);
        }}
        isCreating={ruleCreate.isPending}
      />

      {undoState ? (
        <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex justify-center">
          <div className="pointer-events-auto flex items-center gap-3 rounded-full bg-zinc-900 px-4 py-2 text-sm text-zinc-50 shadow-lg dark:bg-zinc-100 dark:text-zinc-900">
            <span>
              {t("swap.undo.paired", { count: undoState.summary.count })}
              {undoState.summary.total_swap_fee_msat ? (
                <span className={blurClass(hideSensitive)}>
                  {t("swap.undo.feesSuffix", {
                    fees: formatSats(undoState.summary.total_swap_fee_msat),
                  })}
                </span>
              ) : null}
            </span>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-inherit hover:bg-zinc-700 dark:hover:bg-zinc-300"
              onClick={() => void performUndo()}
              disabled={unpairMutation.isPending}
            >
              <Undo2 className="size-3.5" />
              <span className="ml-1">{t("swap.undo.undo")}</span>
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-1 text-inherit hover:bg-zinc-700 dark:hover:bg-zinc-300"
              onClick={cancelUndo}
              aria-label={t("swap.undo.dismissAria")}
            >
              <X className="size-3.5" />
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function SwapQueueMetric({
  label,
  ariaLabel,
  value,
  tone = "neutral",
  active = false,
  onClick,
  icon,
}: {
  label: string;
  ariaLabel?: string;
  value: number;
  tone?: "neutral" | "good" | "warning" | "alert";
  active?: boolean;
  onClick?: () => void;
  icon?: ReactNode;
}) {
  const toneClass = {
    neutral: "text-muted-foreground",
    good: "text-emerald-700 dark:text-emerald-300",
    warning: "text-amber-700 dark:text-amber-300",
    alert: "text-rose-700 dark:text-rose-300",
  }[tone];
  const className = cn(
    "min-w-0 space-y-2 p-3 text-left sm:p-4",
    onClick &&
      "relative w-full cursor-pointer transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
    active && "bg-primary/5 ring-1 ring-primary/30 ring-inset",
  );
  const content = (
    <>
      <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        {icon}
        {label}
      </p>
      <p className={cn("text-xl font-semibold tabular-nums", active ? "text-primary" : toneClass)}>
        {value.toLocaleString("en-US")}
      </p>
    </>
  );
  if (!onClick) {
    return <div className={className}>{content}</div>;
  }
  return (
    <button
      type="button"
      className={className}
      onClick={onClick}
      aria-pressed={active}
      aria-label={ariaLabel ?? label}
    >
      {content}
    </button>
  );
}

interface SwapLegInlineProps {
  direction: "out" | "in";
  asset: string;
  amount: number;
  wallet: string;
  walletKind: string;
  timestamp: string;
  txId: string;
  hideSensitive: boolean;
}

function SwapLegInline({
  direction,
  asset,
  amount,
  wallet,
  walletKind,
  timestamp,
  txId,
  hideSensitive,
}: SwapLegInlineProps) {
  const rail = railForLeg(asset, walletKind);
  const walletName = displayWalletName(wallet, walletKind);
  return (
    <div className="grid min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_auto] items-start gap-2">
      <RailIcon rail={rail} size="compact" />
      <div className="min-w-0 text-xs text-muted-foreground">
        <div className="flex min-w-0 items-center gap-1.5 whitespace-nowrap">
          <span className={cn("truncate", blurClass(hideSensitive))}>{walletName}</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">{formatTimestamp(timestamp)}</span>
        </div>
        <div className={cn("mt-1 truncate font-mono text-[11px] text-muted-foreground/80", blurClass(hideSensitive))}>
          id {compactRecordId(txId)}
        </div>
      </div>
      <div className="min-w-[8.5rem] text-right">
        <div
          className={cn(
            "font-mono text-sm font-semibold tabular-nums",
            direction === "out"
              ? "text-red-700 dark:text-red-300"
              : "text-emerald-700 dark:text-emerald-300",
            blurClass(hideSensitive),
          )}
        >
          {formatBtc(amount)}
        </div>
        <div className="mt-1 flex justify-end">
          <RailBadge rail={rail} asset={asset} />
        </div>
      </div>
    </div>
  );
}

function SwapFeeText({
  candidate,
  hideSensitive,
}: {
  candidate: { swap_fee: number; swap_fee_msat: number; out_amount_msat: number };
  hideSensitive: boolean;
}) {
  const percent = feePercent(candidate);
  const tone =
    percent <= 0.5
      ? "text-emerald-700 dark:text-emerald-300"
      : percent <= 1
        ? "text-amber-700 dark:text-amber-300"
        : "text-rose-700 dark:text-rose-300";
  return (
    <div className="text-right">
      <div className={cn("text-sm font-semibold tabular-nums", tone, blurClass(hideSensitive))}>
        {formatBtc(candidate.swap_fee)}
      </div>
      <div className={cn("mt-1 text-xs text-muted-foreground", blurClass(hideSensitive))}>
        {formatSats(candidate.swap_fee_msat)} · {percent.toFixed(2)}%
      </div>
    </div>
  );
}

interface SwapRowMenuProps {
  candidate: SwapCandidate;
  onOpen: () => void;
  onPair: () => void;
  onDismiss: () => void;
  pairDisabled: boolean;
  dismissDisabled: boolean;
}

function SwapRowMenu({
  candidate,
  onOpen,
  onPair,
  onDismiss,
  pairDisabled,
  dismissDisabled,
}: SwapRowMenuProps) {
  const { t } = useTranslation("review");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="size-8 text-muted-foreground hover:text-foreground"
          aria-label={t("swap.rowMenu.openActionsAria", { id: candidate.out_id })}
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={onOpen}>
          <Eye className="mr-2 size-4" aria-hidden="true" />
          {t("swap.rowMenu.openDetails")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={pairDisabled} onSelect={onPair}>
          <Check className="mr-2 size-4" aria-hidden="true" />
          {t("swap.rowMenu.pair")}
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive"
          disabled={dismissDisabled}
          onSelect={onDismiss}
        >
          <X className="mr-2 size-4" aria-hidden="true" />
          {t("swap.rowMenu.dismiss")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

interface SwapCandidateDetailSheetProps {
  candidate: SwapCandidate | null;
  override: { kind?: PairKind; policy?: PairPolicy };
  onOpenChange: (open: boolean) => void;
  onKindChange: (candidate: SwapCandidate, value: PairKind) => void;
  onPolicyChange: (candidate: SwapCandidate, value: PairPolicy) => void;
  onPair: (candidate: SwapCandidate) => void;
  onDismiss: (candidate: SwapCandidate) => void;
  pairDisabled: boolean;
  dismissDisabled: boolean;
  hideSensitive: boolean;
}

function SwapCandidateDetailSheet({
  candidate,
  override,
  onOpenChange,
  onKindChange,
  onPolicyChange,
  onPair,
  onDismiss,
  pairDisabled,
  dismissDisabled,
  hideSensitive,
}: SwapCandidateDetailSheetProps) {
  const { t } = useTranslation("review");
  const kind = candidate ? override.kind ?? candidate.default_kind : "manual";
  const policy = candidate ? override.policy ?? candidate.default_policy : "carrying-value";
  const presentationType = candidate ? candidatePairType(candidate) : "transfer";
  return (
    <Sheet open={Boolean(candidate)} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto p-0 sm:max-w-2xl">
        {candidate ? (
          <>
            <SheetHeader className="border-b p-4 sm:p-6">
              <SheetTitle>{t(candidateLabelKey(candidate))}</SheetTitle>
              <SheetDescription>
                {t(METHOD_LABEL_KEYS[candidate.method].matched)}
                {" "}
                <span className={blurClass(hideSensitive)}>
                  {t("swap.detail.delta", {
                    delta: formatSats(candidate.swap_fee_msat),
                    percent: feePercent(candidate).toFixed(2),
                  })}
                </span>
              </SheetDescription>
            </SheetHeader>
            <div className="space-y-4 p-4 sm:p-6">
              <div className="grid gap-4 md:grid-cols-2">
                <SwapLegDetails
                  title={t("swap.detail.outgoing")}
                  asset={candidate.out_asset}
                  amount={candidate.out_amount}
                  amountMsat={candidate.out_amount_msat}
                  wallet={candidate.out_wallet_label}
                  walletKind={candidate.out_wallet_kind}
                  timestamp={candidate.out_occurred_at}
                  txId={candidate.out_id}
                  hideSensitive={hideSensitive}
                />
                <SwapLegDetails
                  title={t("swap.detail.incoming")}
                  asset={candidate.in_asset}
                  amount={candidate.in_amount}
                  amountMsat={candidate.in_amount_msat}
                  wallet={candidate.in_wallet_label}
                  walletKind={candidate.in_wallet_kind}
                  timestamp={candidate.in_occurred_at}
                  txId={candidate.in_id}
                  hideSensitive={hideSensitive}
                />
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>{t("swap.detail.kind")}</Label>
                  <Select
                    value={kind}
                    onValueChange={(value) => onKindChange(candidate, value as PairKind)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PAIR_KIND_OPTIONS.map((option) => (
                        <SelectItem key={option} value={option}>
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label>{t("swap.detail.policy")}</Label>
                  <Select
                    value={policy}
                    onValueChange={(value) => onPolicyChange(candidate, value as PairPolicy)}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PAIR_POLICY_OPTIONS.map((option) => (
                        <SelectItem key={option} value={option}>
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-lg border bg-muted/20 p-3 text-sm">
                  <div className="font-medium">{t("swap.detail.matchRationale")}</div>
                  <p className="mt-1 text-muted-foreground">
                    {t(METHOD_LABEL_KEYS[candidate.method].rationale)}
                  </p>
                  {candidate.rule_match ? (
                    <p className="mt-2 text-xs text-muted-foreground">
                      {t("swap.detail.autoPairRoute", {
                        rule: candidate.rule_match.rule_name ?? candidate.rule_match.rule_id,
                      })}
                    </p>
                  ) : null}
                  {candidate.conflict_size > 1 ? (
                    <p className="mt-2 inline-flex items-start gap-1 text-xs text-amber-700 dark:text-amber-300">
                      <AlertTriangle className="mt-0.5 size-3 shrink-0" />
                      <span>
                        {t("swap.detail.conflictNote", { count: candidate.conflict_size - 1 })}
                      </span>
                    </p>
                  ) : null}
                </div>
                <div className="rounded-lg border bg-muted/20 p-3 text-sm">
                  <div className="font-medium">{t("swap.detail.accountingPreview")}</div>
                  <dl className="mt-2 space-y-1 text-xs">
                    <DetailRow label={t("swap.detail.pairKind")} value={kind} />
                    <DetailRow label={t("swap.detail.policy")} value={policy} />
                    <DetailRow
                      label={t(candidateFeeLabelKey(candidate))}
                      value={
                        <span className={blurClass(hideSensitive)}>
                          {t("swap.detail.feeLine", {
                            fee: formatSats(candidate.swap_fee_msat),
                            percent: feePercent(candidate).toFixed(2),
                          })}
                        </span>
                      }
                    />
                  </dl>
                  <p className="mt-2 text-xs text-muted-foreground">
                    {t("swap.detail.deltasNote")}
                  </p>
                  {presentationType === "layer-transition" ? (
                    <p className="mt-2 text-xs text-muted-foreground">
                      {t("swap.detail.layerTransitionOwnershipHint")}
                    </p>
                  ) : null}
                </div>
              </div>
            </div>
            <SheetFooter className="border-t p-4 sm:flex-row sm:items-center sm:justify-between sm:px-6">
              <Button
                variant="outline"
                onClick={() => onDismiss(candidate)}
                disabled={dismissDisabled}
              >
                {t("swap.rowMenu.dismiss")}
              </Button>
              <Button onClick={() => onPair(candidate)} disabled={pairDisabled}>
                {t("swap.rowMenu.pair")}
              </Button>
            </SheetFooter>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

function previewSummary(candidates: SwapCandidate[]): {
  key: "swap.preview.summaryEmpty" | "swap.preview.summary";
  params?: Record<string, unknown>;
} {
  if (candidates.length === 0) return { key: "swap.preview.summaryEmpty" };
  const totalFeeMsat = candidates.reduce((acc, c) => acc + c.swap_fee_msat, 0);
  const totalCarry = candidates.reduce((acc, c) => acc + c.out_amount, 0);
  return {
    key: "swap.preview.summary",
    params: {
      count: candidates.length,
      value: formatBtc(totalCarry),
      fees: formatSats(totalFeeMsat),
    },
  };
}

interface SaveViewDialogProps {
  open: boolean;
  name: string;
  onNameChange: (value: string) => void;
  onCancel: () => void;
  onSave: () => void | Promise<void>;
  isSaving: boolean;
}

function SaveViewDialog({
  open,
  name,
  onNameChange,
  onCancel,
  onSave,
  isSaving,
}: SaveViewDialogProps) {
  const { t } = useTranslation(["review", "common"]);
  return (
    <Dialog open={open} onOpenChange={(value) => (!value ? onCancel() : undefined)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("swap.saveDialog.title")}</DialogTitle>
          <DialogDescription>
            {t("swap.saveDialog.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="view-name">{t("swap.saveDialog.nameLabel")}</Label>
          <Input
            id="view-name"
            autoFocus
            placeholder={t("swap.saveDialog.namePlaceholder")}
            value={name}
            onChange={(event) => onNameChange(event.target.value)}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={() => void onSave()} disabled={isSaving || !name.trim()}>
            {isSaving ? <Loader2 className="size-4 animate-spin" /> : null}
            <span className="ml-1">{t("common:actions.save")}</span>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface CreateRulePayload {
  name: string | null;
  predicate: Record<string, unknown>;
  kind: PairKind;
  policy: PairPolicy;
  enabled: boolean;
}

interface CreateRuleDialogProps {
  open: boolean;
  onClose: () => void;
  onCreate: (payload: CreateRulePayload) => Promise<void>;
  isCreating: boolean;
}

function CreateRuleDialog({ open, onClose, onCreate, isCreating }: CreateRuleDialogProps) {
  const { t } = useTranslation(["review", "common"]);
  const [name, setName] = useState("");
  const [outAsset, setOutAsset] = useState("any");
  const [inAsset, setInAsset] = useState("any");
  const [outKind, setOutKind] = useState("any");
  const [inKind, setInKind] = useState("any");
  const [maxFeePct, setMaxFeePct] = useState("");
  const [minConfidence, setMinConfidence] = useState<"strong" | "exact">("strong");
  const [kind, setKind] = useState<PairKind>("submarine-swap");
  const [policy, setPolicy] = useState<PairPolicy>("carrying-value");

  const reset = () => {
    setName("");
    setOutAsset("any");
    setInAsset("any");
    setOutKind("any");
    setInKind("any");
    setMaxFeePct("");
    setMinConfidence("strong");
    setKind("submarine-swap");
    setPolicy("carrying-value");
  };

  const submit = async () => {
    const predicate: Record<string, unknown> = {};
    if (outAsset !== "any") predicate.out_asset = outAsset;
    if (inAsset !== "any") predicate.in_asset = inAsset;
    if (outKind !== "any") predicate.out_wallet_kind = outKind;
    if (inKind !== "any") predicate.in_wallet_kind = inKind;
    if (maxFeePct.trim()) {
      const parsed = Number.parseFloat(maxFeePct.trim());
      if (Number.isFinite(parsed)) predicate.max_fee_pct = parsed;
    }
    predicate.min_confidence = minConfidence;
    await onCreate({
      name: name.trim() || null,
      predicate,
      kind,
      policy,
      enabled: true,
    });
    reset();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value) {
          reset();
          onClose();
        }
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("swap.createRule.title")}</DialogTitle>
          <DialogDescription>
            {t("swap.createRule.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <div className="col-span-2 space-y-1">
            <Label htmlFor="rule-name">{t("swap.createRule.nameLabel")}</Label>
            <Input
              id="rule-name"
              placeholder={t("swap.createRule.namePlaceholder")}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <RulePredicateAssetField
            label={t("swap.createRule.outAsset")}
            value={outAsset}
            onChange={setOutAsset}
          />
          <RulePredicateAssetField
            label={t("swap.createRule.inAsset")}
            value={inAsset}
            onChange={setInAsset}
          />
          <RulePredicateKindField
            label={t("swap.createRule.outWalletKind")}
            value={outKind}
            onChange={setOutKind}
          />
          <RulePredicateKindField
            label={t("swap.createRule.inWalletKind")}
            value={inKind}
            onChange={setInKind}
          />
          <div className="space-y-1">
            <Label htmlFor="max-fee">{t("swap.createRule.maxFee")}</Label>
            <Input
              id="max-fee"
              placeholder={t("swap.createRule.maxFeePlaceholder")}
              value={maxFeePct}
              onChange={(e) => setMaxFeePct(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.minConfidence")}</Label>
            <Select
              value={minConfidence}
              onValueChange={(v) => setMinConfidence(v as "strong" | "exact")}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="strong">{t("swap.createRule.minStrong")}</SelectItem>
                <SelectItem value="exact">{t("swap.createRule.minExact")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.kind")}</Label>
            <Select value={kind} onValueChange={(v) => setKind(v as PairKind)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PAIR_KIND_OPTIONS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {option}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>{t("swap.createRule.policy")}</Label>
            <Select value={policy} onValueChange={(v) => setPolicy(v as PairPolicy)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {PAIR_POLICY_OPTIONS.map((option) => (
                  <SelectItem key={option} value={option}>
                    {option}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => {
              reset();
              onClose();
            }}
          >
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={() => void submit()} disabled={isCreating}>
            {isCreating ? <Loader2 className="size-4 animate-spin" /> : null}
            <span className="ml-1">{t("swap.createRule.createButton")}</span>
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface RuleFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
}

function RulePredicateAssetField({ label, value, onChange }: RuleFieldProps) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">{t("swap.createRule.any")}</SelectItem>
          <SelectItem value="BTC">BTC</SelectItem>
          <SelectItem value="LBTC">BTC on Liquid (LBTC)</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

interface KeymapHelpDialogProps {
  open: boolean;
  onClose: () => void;
  bindings: Keybinding[];
}

function KeymapHelpDialog({ open, onClose, bindings }: KeymapHelpDialogProps) {
  const { t } = useTranslation(["review", "common"]);
  const grouped = useMemo(() => {
    const groups: Record<string, Keybinding[]> = {};
    for (const binding of bindings) {
      const key = binding.category ?? "Other";
      (groups[key] ??= []).push(binding);
    }
    return groups;
  }, [bindings]);

  return (
    <Dialog open={open} onOpenChange={(value) => (!value ? onClose() : undefined)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("swap.keymap.title")}</DialogTitle>
          <DialogDescription>
            {t("swap.keymap.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3 text-sm">
          {Object.entries(grouped).map(([category, items]) => (
            <section key={category}>
              <h3 className="mb-1 text-xs font-semibold uppercase text-muted-foreground">
                {category}
              </h3>
              <ul className="space-y-1">
                {items.map((binding) => (
                  <li
                    key={`${category}-${binding.description}`}
                    className="flex items-center justify-between gap-3 rounded border border-border/40 bg-background/50 px-2 py-1"
                  >
                    <span>{binding.description}</span>
                    <kbd className="rounded bg-muted px-1.5 text-xs">
                      {formatKeybindingKeys(binding.keys)}
                    </kbd>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <DialogFooter>
          <Button onClick={onClose}>{t("common:actions.close")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function formatKeybindingKeys(keys: string | string[]): string {
  const list = Array.isArray(keys) ? keys : [keys];
  return list
    .map((key) => {
      if (key === " ") return "Space";
      if (key === "ArrowUp") return "↑";
      if (key === "ArrowDown") return "↓";
      return key.length === 1 ? key.toUpperCase() : key;
    })
    .join(" / ");
}

function RulePredicateKindField({ label, value, onChange }: RuleFieldProps) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">{t("swap.createRule.any")}</SelectItem>
          <SelectItem value="phoenix">phoenix</SelectItem>
          <SelectItem value="coreln">coreln</SelectItem>
          <SelectItem value="lnd">lnd</SelectItem>
          <SelectItem value="nwc">nwc</SelectItem>
          <SelectItem value="descriptor">descriptor</SelectItem>
          <SelectItem value="xpub">xpub</SelectItem>
          <SelectItem value="address">address</SelectItem>
          <SelectItem value="custom">custom</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}

interface SwapLegDetailsProps {
  title: string;
  asset: string;
  amount: number;
  amountMsat: number;
  wallet: string;
  walletKind: string;
  timestamp: string;
  txId: string;
  hideSensitive: boolean;
}

function SwapLegDetails({
  title,
  asset,
  amount,
  amountMsat,
  wallet,
  walletKind,
  timestamp,
  txId,
  hideSensitive,
}: SwapLegDetailsProps) {
  const { t } = useTranslation("review");
  const rail = railForLeg(asset, walletKind);
  const walletName = displayWalletName(wallet, walletKind);
  const walletLabel = wallet?.trim() ?? "";
  const showKind = walletLabel.length > 0 && walletLabel !== walletKind;
  return (
    <section className="rounded-lg border bg-background p-3">
      <div className="mb-3 flex items-center justify-between gap-2">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {title}
        </span>
        <RailBadge rail={rail} asset={asset} />
      </div>
      <dl className="space-y-2 text-sm">
        <DetailRow
          label={t("swap.detail.wallet")}
          value={
            <span className="inline-flex min-w-0 items-center gap-2">
              <RailIcon rail={rail} size="compact" />
              <span className={cn("truncate", blurClass(hideSensitive))}>{walletName}</span>
              {showKind ? (
                <span className="text-xs uppercase text-muted-foreground">· {walletKind}</span>
              ) : null}
            </span>
          }
        />
        <DetailRow
          label={t("swap.detail.amount")}
          value={
            <span className={cn("font-mono tabular-nums", blurClass(hideSensitive))}>
              {formatBtc(amount)} <span className="text-xs text-muted-foreground">({formatSats(amountMsat)})</span>
            </span>
          }
        />
        <DetailRow label={t("swap.detail.occurred")} value={formatTimestamp(timestamp)} />
        <DetailRow
          label={t("swap.detail.recordId")}
          value={<span className={cn("font-mono text-xs", blurClass(hideSensitive))}>{txId}</span>}
        />
      </dl>
    </section>
  );
}

function displayWalletName(wallet: string | null | undefined, walletKind: string) {
  const trimmed = wallet?.trim() ?? "";
  return trimmed.length > 0 ? trimmed : walletKind;
}

function DetailRow({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="grid grid-cols-[6rem_minmax(0,1fr)] gap-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-right text-sm text-foreground">{value}</dd>
    </div>
  );
}

interface RailIconProps {
  rail: SwapRail;
  size?: "compact" | "regular" | "large";
}

function RailIcon({ rail, size = "regular" }: RailIconProps) {
  const details = RAIL_DETAILS[rail];
  const frameSize = {
    compact: "size-6",
    regular: "size-7",
    large: "size-12",
  }[size];
  const iconSize = {
    compact: "size-5",
    regular: "size-6",
    large: "size-11",
  }[size];
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center justify-center bg-white shadow-sm ring-1 ring-black/10 dark:ring-white/15",
        size === "large" ? "rounded-md" : "rounded-full",
        frameSize,
      )}
      title={details.label}
    >
      <img
        src={details.icon}
        alt=""
        aria-hidden="true"
        className={cn(
          "object-contain drop-shadow-sm",
          iconSize,
          rail === "liquid" ? "scale-150" : null,
        )}
      />
    </span>
  );
}

interface RailBadgeProps {
  rail: SwapRail;
  asset: string;
}

function RailBadge({ rail, asset }: RailBadgeProps) {
  const details = RAIL_DETAILS[rail];
  const railLabel = details.shortLabel.toUpperCase();
  const assetLabel = displayAssetLabel(asset).toUpperCase();
  const labelParts = railLabel === assetLabel ? [assetLabel] : [railLabel, assetLabel];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase",
        details.className,
      )}
      title={`${details.label} ${displayAssetLabel(asset)}`}
    >
      {labelParts.map((part, index) => (
        <Fragment key={part}>
          {index > 0 ? <span className="text-current/50">·</span> : null}
          <span>{part}</span>
        </Fragment>
      ))}
    </span>
  );
}

function displayAssetLabel(asset: string) {
  const normalized = asset.toUpperCase();
  return normalized === "LBTC" || normalized === "L-BTC" ? "BTC" : normalized;
}

interface SwapStatusCellProps {
  candidate: SwapCandidate;
  conflicted: boolean;
  hiddenSiblings: number;
}

/**
 * Fused match signal: one traffic light answering "how safe is this to pair?".
 * Confidence and the match method encode the same thing, so they collapse into
 * a single cell — green exact / amber strong / red conflict — with the dot
 * carrying colour and a word carrying meaning (never colour alone). The match
 * method and conflict detail ride the tooltip; the full rationale is in the
 * detail sheet. The rule glyph shows only when an auto-pair rule matched.
 */
function SwapStatusCell({ candidate, conflicted, hiddenSiblings }: SwapStatusCellProps) {
  const { t } = useTranslation("review");
  const dotClass = conflicted
    ? "bg-rose-500"
    : candidate.confidence === "exact"
      ? "bg-emerald-500"
      : "bg-amber-500";
  const label = conflicted
    ? t("swap.table.statusConflict")
    : candidate.confidence === "exact"
      ? t("swap.metric.exact")
      : t("swap.metric.strong");
  const labelClass = conflicted
    ? "font-medium text-rose-700 dark:text-rose-300"
    : "text-foreground";
  const title = conflicted
    ? `${t("swap.table.conflictTitle", { count: candidate.conflict_size })}${
        hiddenSiblings > 0
          ? t("swap.table.conflictHiddenTitle", { count: hiddenSiblings })
          : ""
      }`
    : t(METHOD_LABEL_KEYS[candidate.method].matched);
  return (
    <div className="flex flex-wrap items-center gap-1.5" title={title}>
      <span className={cn("size-2 shrink-0 rounded-full", dotClass)} aria-hidden="true" />
      <span className={cn("text-sm", labelClass)}>{label}</span>
      {candidate.rule_match ? (
        <Badge variant="outline" className="px-1 py-0 text-[9px] leading-tight">
          {t("swap.table.ruleBadge")}
        </Badge>
      ) : null}
    </div>
  );
}
