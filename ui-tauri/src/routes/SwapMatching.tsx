/**
 * Swap-matching review queue.
 *
 * Drives the ``ui.transfers.suggest`` daemon kind to surface candidate
 * pairings the matcher believes form one swap (Lightning ↔ Liquid,
 * Liquid ↔ on-chain BTC, etc.). Each row exposes inline kind / policy
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

import { useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowRight,
  Loader2,
  Sparkles,
  Undo2,
  X,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenPanelClassName, screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";

const PAIR_KIND_OPTIONS = ["manual", "peg-in", "peg-out", "submarine-swap"] as const;
const PAIR_POLICY_OPTIONS = ["carrying-value", "taxable"] as const;
const CONFIDENCE_OPTIONS = [
  { value: "all", label: "Any confidence" },
  { value: "exact", label: "Exact (payment_hash)" },
  { value: "strong", label: "Strong (heuristic)" },
] as const;
const METHOD_OPTIONS = [
  { value: "all", label: "Any method" },
  { value: "payment_hash", label: "Payment hash" },
  { value: "heuristic", label: "Time + amount" },
] as const;

type PairKind = (typeof PAIR_KIND_OPTIONS)[number];
type PairPolicy = (typeof PAIR_POLICY_OPTIONS)[number];

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
  method: "payment_hash" | "heuristic";
  swap_fee_msat: number;
  swap_fee: number;
  swap_fee_kind: string;
  default_kind: PairKind;
  default_policy: PairPolicy;
  conflict_set_id: string;
}

interface SuggestEnvelope {
  candidates: SwapCandidate[];
  counts: {
    total: number;
    exact: number;
    strong: number;
    conflicts: number;
  };
}

const btcFmt = new Intl.NumberFormat("en-US", {
  maximumFractionDigits: 8,
  minimumFractionDigits: 8,
});

function formatBtc(value: number) {
  return `₿${btcFmt.format(value)}`;
}

function formatSats(msat: number) {
  return `${Math.round(msat / 1000).toLocaleString()} sats`;
}

function formatTimestamp(value: string) {
  if (!value) return "—";
  try {
    return new Intl.DateTimeFormat("en-US", {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(value));
  } catch {
    return value;
  }
}

function feePercent(candidate: SwapCandidate) {
  if (!candidate.out_amount_msat) return 0;
  return (Math.abs(candidate.swap_fee_msat) / candidate.out_amount_msat) * 100;
}

interface BulkPairResult {
  applied: Array<{ id: string; swap_fee_msat?: number | null }>;
  summary: {
    count: number;
    skipped_conflicts: number;
    total_swap_fee_msat: number;
  };
}

const UNDO_WINDOW_MS = 20_000;

export function SwapMatching() {
  const [confidence, setConfidence] = useState<string>("all");
  const [method, setMethod] = useState<string>("all");
  const [assetPair, setAssetPair] = useState<string>("");
  const [overrides, setOverrides] = useState<
    Record<string, { kind?: PairKind; policy?: PairPolicy }>
  >({});
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [bulkKind, setBulkKind] = useState<PairKind>("submarine-swap");
  const [bulkPolicy, setBulkPolicy] = useState<PairPolicy>("carrying-value");
  const [previewState, setPreviewState] = useState<
    | { mode: "exact"; candidates: SwapCandidate[] }
    | { mode: "selected"; candidates: SwapCandidate[] }
    | null
  >(null);
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
    const next: Record<string, unknown> = {};
    if (confidence !== "all") next.confidence = confidence;
    if (method !== "all") next.method = method;
    if (assetPair.trim()) next.asset_pair = assetPair.trim().toUpperCase();
    return next;
  }, [confidence, method, assetPair]);

  const { data, isLoading, isError, error, refetch, isFetching } =
    useDaemon<SuggestEnvelope>("ui.transfers.suggest", args);

  const pairMutation = useDaemonMutation<unknown>("ui.transfers.pair");
  const dismissMutation = useDaemonMutation<unknown>("ui.transfers.dismiss");
  const bulkPairMutation = useDaemonMutation<BulkPairResult>("ui.transfers.bulk_pair");
  const unpairMutation = useDaemonMutation<unknown>("ui.transfers.unpair");

  const candidates = data?.data?.candidates ?? [];
  const counts = data?.data?.counts ?? { total: 0, exact: 0, strong: 0, conflicts: 0 };

  const clusterSizes = useMemo(() => {
    const sizes: Record<string, number> = {};
    for (const candidate of candidates) {
      sizes[candidate.conflict_set_id] = (sizes[candidate.conflict_set_id] ?? 0) + 1;
    }
    return sizes;
  }, [candidates]);

  const candidateKey = (c: SwapCandidate) => `${c.out_id}->${c.in_id}`;

  const candidatesByKey = useMemo(() => {
    const map: Record<string, SwapCandidate> = {};
    for (const candidate of candidates) {
      map[candidateKey(candidate)] = candidate;
    }
    return map;
  }, [candidates]);

  const exactSolo = useMemo(
    () =>
      candidates.filter(
        (c) =>
          c.confidence === "exact" &&
          (clusterSizes[c.conflict_set_id] ?? 0) <= 1,
      ),
    [candidates, clusterSizes],
  );

  const selectedCandidates = useMemo(
    () =>
      Array.from(selected)
        .map((key) => candidatesByKey[key])
        .filter((c): c is SwapCandidate => Boolean(c)),
    [selected, candidatesByKey],
  );

  const toggleSelected = (key: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  const handleSelectAll = () => {
    const eligible = candidates.filter(
      (c) => (clusterSizes[c.conflict_set_id] ?? 0) <= 1,
    );
    if (selected.size === eligible.length && eligible.length > 0) {
      setSelected(new Set());
      return;
    }
    setSelected(new Set(eligible.map(candidateKey)));
  };

  const handlePair = async (candidate: SwapCandidate) => {
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
  };

  const handleDismiss = async (candidate: SwapCandidate) => {
    await dismissMutation.mutateAsync({
      tx_out: candidate.out_id,
      tx_in: candidate.in_id,
      reason: "user dismissed from review queue",
    });
    void refetch();
  };

  const openExactPreview = () => {
    setPreviewState({ mode: "exact", candidates: exactSolo });
  };

  const openSelectedPreview = () => {
    setPreviewState({ mode: "selected", candidates: selectedCandidates });
  };

  const commitBulk = async () => {
    if (!previewState) return;
    if (previewState.mode === "exact") {
      const envelope = await bulkPairMutation.mutateAsync({ confidence: "exact" });
      const result = envelope.data;
      if (result) {
        setUndoState({
          pairIds: result.applied.map((p) => p.id),
          summary: result.summary,
          deadline: Date.now() + UNDO_WINDOW_MS,
        });
      }
    } else {
      const applied: string[] = [];
      let totalFee = 0;
      for (const candidate of previewState.candidates) {
        const key = candidateKey(candidate);
        const override = overrides[key] ?? {};
        const envelope = await pairMutation.mutateAsync({
          tx_out: candidate.out_id,
          tx_in: candidate.in_id,
          kind: override.kind ?? bulkKind,
          policy: override.policy ?? bulkPolicy,
          pair_source: "bulk_selected",
          confidence_at_pair: candidate.confidence,
        });
        const created = envelope.data as { id?: string; swap_fee_msat?: number } | undefined;
        if (created?.id) applied.push(created.id);
        if (typeof created?.swap_fee_msat === "number") totalFee += created.swap_fee_msat;
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
    void refetch();
  };

  const cancelUndo = () => {
    setUndoState(null);
  };

  const performUndo = async () => {
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
  };

  return (
    <div className={screenShellClassName}>
      <header className="flex flex-col gap-2 px-4 py-3 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <h1 className="text-xl font-semibold">Swap candidates</h1>
          <p className="text-sm text-muted-foreground">
            Cross-wallet, cross-network legs the matcher believes form one
            swap. Pair to apply the carrying-value math; dismiss to suppress
            for 90 days.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void refetch()}
          disabled={isFetching}
        >
          {isFetching ? <Loader2 className="size-4 animate-spin" /> : null}
          <span className="ml-1">Refresh</span>
        </Button>
      </header>

      <div className="flex flex-wrap gap-2 px-4">
        <CountPill label="Candidates" value={counts.total} tone="neutral" />
        <CountPill label="Exact" value={counts.exact} tone="good" />
        <CountPill label="Strong" value={counts.strong} tone="warning" />
        <CountPill label="Conflicts" value={counts.conflicts} tone="alert" />
      </div>

      <div className="flex flex-wrap items-center gap-2 px-4 pb-2 pt-1 text-sm">
        <span className="text-muted-foreground">Filter:</span>
        <Select value={confidence} onValueChange={setConfidence}>
          <SelectTrigger className="h-8 w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CONFIDENCE_OPTIONS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={method} onValueChange={setMethod}>
          <SelectTrigger className="h-8 w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {METHOD_OPTIONS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <input
          aria-label="Asset pair filter"
          className="h-8 w-32 rounded border border-input bg-transparent px-2 text-sm"
          placeholder="OUT-IN"
          value={assetPair}
          onChange={(e) => setAssetPair(e.target.value)}
        />
        {(confidence !== "all" || method !== "all" || assetPair) && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setConfidence("all");
              setMethod("all");
              setAssetPair("");
            }}
          >
            Clear filters
          </Button>
        )}
      </div>

      <div className={cn(screenPanelClassName, "flex flex-col gap-3 p-4")}>
        {!isLoading && !isError && candidates.length > 0 ? (
          <div className="flex flex-wrap items-center gap-2 rounded-md border bg-background/50 px-3 py-2 text-sm">
            <label className="flex items-center gap-2">
              <Checkbox
                checked={selected.size > 0}
                onCheckedChange={handleSelectAll}
              />
              <span className="text-xs text-muted-foreground">
                {selected.size > 0
                  ? `${selected.size} selected`
                  : "Select all non-conflicted"}
              </span>
            </label>
            {selected.size > 0 ? (
              <>
                <label className="flex items-center gap-1 text-xs text-muted-foreground">
                  Kind
                  <Select value={bulkKind} onValueChange={(v) => setBulkKind(v as PairKind)}>
                    <SelectTrigger className="ml-1 h-8 w-44">
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
                </label>
                <label className="flex items-center gap-1 text-xs text-muted-foreground">
                  Policy
                  <Select
                    value={bulkPolicy}
                    onValueChange={(v) => setBulkPolicy(v as PairPolicy)}
                  >
                    <SelectTrigger className="ml-1 h-8 w-44">
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
                </label>
                <Button
                  size="sm"
                  onClick={openSelectedPreview}
                  disabled={pairMutation.isPending}
                >
                  Pair {selected.size} selected
                </Button>
              </>
            ) : null}
            <div className="ml-auto flex items-center gap-2">
              {exactSolo.length > 0 ? (
                <Button
                  size="sm"
                  variant="default"
                  onClick={openExactPreview}
                  disabled={bulkPairMutation.isPending}
                >
                  <Sparkles className="size-4" />
                  <span>Apply all {exactSolo.length} exact</span>
                </Button>
              ) : null}
            </div>
          </div>
        ) : null}
        {isLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Loading candidates…
          </div>
        ) : isError ? (
          <div className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm">
            Failed to load candidates: {String(error)}
          </div>
        ) : candidates.length === 0 ? (
          <div className="rounded border border-dashed border-muted-foreground/40 p-6 text-center text-sm text-muted-foreground">
            No unpaired swap candidates. Once a Lightning ↔ Liquid swap (or
            BTC ↔ LBTC peg) shows up in your wallets, it will appear here.
          </div>
        ) : (
          candidates.map((candidate) => {
            const key = candidateKey(candidate);
            const override = overrides[key] ?? {};
            const conflicted = (clusterSizes[candidate.conflict_set_id] ?? 0) > 1;
            return (
              <article
                key={key}
                className={cn(
                  "rounded-lg border bg-card text-card-foreground shadow-sm",
                  conflicted ? "border-amber-400/60" : "border-border",
                )}
              >
                <header className="flex flex-wrap items-center gap-2 border-b border-border/60 px-4 py-2">
                  <Checkbox
                    aria-label="Select candidate"
                    disabled={conflicted}
                    checked={selected.has(key)}
                    onCheckedChange={() => toggleSelected(key)}
                  />
                  <ConfidenceBadge candidate={candidate} />
                  <span className="text-xs text-muted-foreground">
                    {candidate.method === "payment_hash"
                      ? "matched on payment_hash"
                      : "matched on time + amount"}
                  </span>
                  <span className="ml-auto text-sm">
                    <span className="font-semibold">Swap fee </span>
                    {formatBtc(candidate.swap_fee)}
                    <span className="ml-1 text-xs text-muted-foreground">
                      · {formatSats(candidate.swap_fee_msat)} · {feePercent(candidate).toFixed(2)}%
                    </span>
                  </span>
                </header>

                {conflicted ? (
                  <div className="flex items-start gap-2 border-b border-amber-400/40 bg-amber-50/40 px-4 py-2 text-xs text-amber-900 dark:bg-amber-950/30 dark:text-amber-100">
                    <AlertTriangle className="mt-0.5 size-3.5" />
                    <span>
                      Conflict cluster — {clusterSizes[candidate.conflict_set_id]} candidates share a leg.
                      Pick the right pair manually; bulk-pair skips this cluster.
                    </span>
                  </div>
                ) : null}

                <div className="grid gap-2 px-4 py-3 sm:grid-cols-[1fr_auto_1fr] sm:items-center">
                  <LegCard
                    title="Outgoing"
                    asset={candidate.out_asset}
                    amount={candidate.out_amount}
                    wallet={candidate.out_wallet_label}
                    walletKind={candidate.out_wallet_kind}
                    timestamp={candidate.out_occurred_at}
                    txId={candidate.out_id}
                  />
                  <ArrowRight className="size-5 self-center text-muted-foreground" />
                  <LegCard
                    title="Incoming"
                    asset={candidate.in_asset}
                    amount={candidate.in_amount}
                    wallet={candidate.in_wallet_label}
                    walletKind={candidate.in_wallet_kind}
                    timestamp={candidate.in_occurred_at}
                    txId={candidate.in_id}
                  />
                </div>

                <footer className="flex flex-wrap items-center gap-2 border-t border-border/60 px-4 py-2 text-sm">
                  <label className="flex items-center gap-1 text-xs text-muted-foreground">
                    Kind
                    <Select
                      value={override.kind ?? candidate.default_kind}
                      onValueChange={(value) =>
                        setOverrides((prev) => ({
                          ...prev,
                          [key]: { ...prev[key], kind: value as PairKind },
                        }))
                      }
                    >
                      <SelectTrigger className="ml-1 h-8 w-44">
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
                  </label>
                  <label className="flex items-center gap-1 text-xs text-muted-foreground">
                    Policy
                    <Select
                      value={override.policy ?? candidate.default_policy}
                      onValueChange={(value) =>
                        setOverrides((prev) => ({
                          ...prev,
                          [key]: { ...prev[key], policy: value as PairPolicy },
                        }))
                      }
                    >
                      <SelectTrigger className="ml-1 h-8 w-44">
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
                  </label>
                  <div className="ml-auto flex items-center gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void handleDismiss(candidate)}
                      disabled={dismissMutation.isPending || pairMutation.isPending}
                    >
                      Dismiss
                    </Button>
                    <Button
                      size="sm"
                      onClick={() => void handlePair(candidate)}
                      disabled={pairMutation.isPending}
                    >
                      Pair
                    </Button>
                  </div>
                </footer>
              </article>
            );
          })
        )}
      </div>

      <Dialog
        open={previewState !== null}
        onOpenChange={(open) => {
          if (!open) setPreviewState(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {previewState?.mode === "exact"
                ? "Apply all exact matches"
                : "Pair selected candidates"}
            </DialogTitle>
            <DialogDescription>
              {previewState
                ? previewSummaryText(previewState.candidates)
                : null}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-64 overflow-auto rounded border border-border/60 p-2 text-sm">
            {previewState?.candidates.map((candidate) => (
              <div
                key={`${candidate.out_id}->${candidate.in_id}`}
                className="flex items-center justify-between gap-2 border-b border-border/40 py-1 last:border-b-0"
              >
                <span className="truncate text-xs">
                  {candidate.out_asset} {formatBtc(candidate.out_amount)} →{" "}
                  {candidate.in_asset} {formatBtc(candidate.in_amount)}
                </span>
                <span className="text-xs text-muted-foreground">
                  fee {formatSats(candidate.swap_fee_msat)}
                </span>
              </div>
            ))}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPreviewState(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => void commitBulk()}
              disabled={bulkPairMutation.isPending || pairMutation.isPending}
            >
              {bulkPairMutation.isPending || pairMutation.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              <span className="ml-1">Confirm</span>
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {undoState ? (
        <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex justify-center">
          <div className="pointer-events-auto flex items-center gap-3 rounded-full bg-zinc-900 px-4 py-2 text-sm text-zinc-50 shadow-lg dark:bg-zinc-100 dark:text-zinc-900">
            <span>
              Paired <strong>{undoState.summary.count}</strong> candidate
              {undoState.summary.count === 1 ? "" : "s"}
              {undoState.summary.total_swap_fee_msat
                ? ` · swap fees ${formatSats(undoState.summary.total_swap_fee_msat)}`
                : ""}
            </span>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-inherit hover:bg-zinc-700 dark:hover:bg-zinc-300"
              onClick={() => void performUndo()}
              disabled={unpairMutation.isPending}
            >
              <Undo2 className="size-3.5" />
              <span className="ml-1">Undo</span>
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-1 text-inherit hover:bg-zinc-700 dark:hover:bg-zinc-300"
              onClick={cancelUndo}
              aria-label="Dismiss undo toast"
            >
              <X className="size-3.5" />
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function previewSummaryText(candidates: SwapCandidate[]): string {
  if (candidates.length === 0) return "No candidates to pair.";
  const totalFeeMsat = candidates.reduce((acc, c) => acc + c.swap_fee_msat, 0);
  const totalCarry = candidates.reduce((acc, c) => acc + c.out_amount, 0);
  return `${candidates.length} pair${candidates.length === 1 ? "" : "s"} · carrying value ${formatBtc(totalCarry)} · total swap fees ${formatSats(totalFeeMsat)}.`;
}

interface LegCardProps {
  title: string;
  asset: string;
  amount: number;
  wallet: string;
  walletKind: string;
  timestamp: string;
  txId: string;
}

function LegCard({ title, asset, amount, wallet, walletKind, timestamp, txId }: LegCardProps) {
  return (
    <div className="rounded-md border bg-background p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs uppercase tracking-wide text-muted-foreground">
          {title}
        </span>
        <Badge variant="outline" className="text-[10px] uppercase">
          {asset}
        </Badge>
      </div>
      <div className="mt-1 font-mono text-base">{formatBtc(amount)}</div>
      <div className="mt-1 truncate text-xs text-muted-foreground">
        {wallet} <span className="text-[10px] uppercase">· {walletKind}</span>
      </div>
      <div className="text-xs text-muted-foreground">{formatTimestamp(timestamp)}</div>
      <div className="mt-1 font-mono text-[11px] text-muted-foreground/80">
        tx {txId.slice(0, 8)}…{txId.slice(-4)}
      </div>
    </div>
  );
}

interface CountPillProps {
  label: string;
  value: number;
  tone: "neutral" | "good" | "warning" | "alert";
}

function CountPill({ label, value, tone }: CountPillProps) {
  const toneClass = {
    neutral: "bg-muted text-muted-foreground",
    good: "bg-emerald-100 text-emerald-900 dark:bg-emerald-950/40 dark:text-emerald-100",
    warning: "bg-amber-100 text-amber-900 dark:bg-amber-950/40 dark:text-amber-100",
    alert: "bg-rose-100 text-rose-900 dark:bg-rose-950/40 dark:text-rose-100",
  }[tone];
  return (
    <span className={cn("inline-flex items-center gap-1 rounded-full px-3 py-1 text-xs", toneClass)}>
      <strong className="font-semibold">{value}</strong>
      {label}
    </span>
  );
}

interface ConfidenceBadgeProps {
  candidate: SwapCandidate;
}

function ConfidenceBadge({ candidate }: ConfidenceBadgeProps) {
  if (candidate.confidence === "exact") {
    return (
      <Badge className="bg-emerald-100 text-emerald-900 hover:bg-emerald-100 dark:bg-emerald-950/50 dark:text-emerald-100">
        Exact
      </Badge>
    );
  }
  return (
    <Badge className="bg-amber-100 text-amber-900 hover:bg-amber-100 dark:bg-amber-950/50 dark:text-amber-100">
      Strong
    </Badge>
  );
}
