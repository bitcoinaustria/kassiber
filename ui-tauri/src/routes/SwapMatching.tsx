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

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  ArrowRight,
  Check,
  Eye,
  Loader2,
  MoreHorizontal,
  Plus,
  Settings as SettingsIcon,
  Sparkles,
  Star,
  Trash2,
  Undo2,
  X,
} from "lucide-react";

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
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useKeymap, type Keybinding } from "@/lib/keymap";
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
type SwapRail = "onchain" | "lightning" | "liquid";

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

interface SavedView {
  id: string;
  surface: string;
  name: string;
  filter: {
    confidence?: string;
    method?: string;
    asset_pair?: string;
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

const SAVED_VIEW_SURFACE = "swap_candidates";

const UNDO_WINDOW_MS = 20_000;

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
    shortLabel: "LBTC",
    icon: liquidIcon,
    className:
      "border-cyan-200 bg-cyan-50 text-cyan-800 dark:border-cyan-400/30 dark:bg-cyan-950/40 dark:text-cyan-100",
  },
};

function railForLeg(asset: string, walletKind: string): SwapRail {
  const assetKey = asset.toUpperCase();
  const kindKey = walletKind.toLowerCase();
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

export function SwapMatching() {
  const [confidence, setConfidence] = useState<string>("all");
  const [method, setMethod] = useState<string>("all");
  const [assetPair, setAssetPair] = useState<string>("");
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

  const savedViewsQuery = useDaemon<SavedViewsEnvelope>("ui.saved_views.list", {
    surface: SAVED_VIEW_SURFACE,
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
  const filterInputRef = useRef<HTMLInputElement | null>(null);

  const savedViews = savedViewsQuery.data?.data?.views ?? [];
  const rules = rulesQuery.data?.data?.rules ?? [];
  const enabledRuleCount = rules.filter((rule) => rule.enabled).length;

  const filterIsDirty = confidence !== "all" || method !== "all" || assetPair.trim() !== "";

  const applySavedView = (view: SavedView) => {
    setConfidence(typeof view.filter.confidence === "string" ? view.filter.confidence : "all");
    setMethod(typeof view.filter.method === "string" ? view.filter.method : "all");
    setAssetPair(typeof view.filter.asset_pair === "string" ? view.filter.asset_pair : "");
  };

  const commitSaveView = async () => {
    const name = saveViewName.trim();
    if (!name) return;
    const filterPayload: Record<string, unknown> = {};
    if (confidence !== "all") filterPayload.confidence = confidence;
    if (method !== "all") filterPayload.method = method;
    if (assetPair.trim()) filterPayload.asset_pair = assetPair.trim().toUpperCase();
    try {
      await savedViewCreate.mutateAsync({
        surface: SAVED_VIEW_SURFACE,
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

  const ruleSolo = useMemo(
    () =>
      candidates.filter(
        (c) =>
          c.rule_match &&
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

  const openRulesPreview = () => {
    setPreviewState({ mode: "rules", candidates: ruleSolo });
  };

  const openSelectedPreview = () => {
    setPreviewState({ mode: "selected", candidates: selectedCandidates });
  };

  const commitBulk = async () => {
    if (!previewState) return;
    if (previewState.mode === "exact" || previewState.mode === "rules") {
      const envelope =
        previewState.mode === "exact"
          ? await bulkPairMutation.mutateAsync({ ...args, confidence: "exact" })
          : await ruleApply.mutateAsync(args);
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
          kind: override.kind ?? bulkKind ?? candidate.default_kind,
          policy: override.policy ?? bulkPolicy ?? candidate.default_policy,
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
        description: "Show keyboard shortcuts",
        category: "Help",
        handler: () => setHelpOpen(true),
      },
      {
        keys: "Escape",
        description: "Clear selection / close overlays",
        category: "Selection",
        handler: () => {
          if (helpOpen) setHelpOpen(false);
          else if (detailCandidate) setDetailCandidate(null);
          else if (previewState) setPreviewState(null);
          else if (selected.size > 0) setSelected(new Set());
        },
      },
      {
        keys: ["j", "ArrowDown"],
        description: "Move cursor down",
        category: "Navigation",
        handler: () => {
          if (candidates.length === 0) return;
          setCursorIndex((idx) => Math.min(candidates.length - 1, idx + 1));
        },
      },
      {
        keys: ["k", "ArrowUp"],
        description: "Move cursor up",
        category: "Navigation",
        handler: () => {
          if (candidates.length === 0) return;
          setCursorIndex((idx) => Math.max(0, idx - 1));
        },
      },
      {
        keys: " ",
        description: "Toggle selection on current candidate",
        category: "Selection",
        handler: () => {
          if (!cursorCandidate) return;
          if ((clusterSizes[cursorCandidate.conflict_set_id] ?? 0) > 1) return;
          toggleSelected(candidateKey(cursorCandidate));
        },
      },
      {
        keys: "a",
        description: "Select all non-conflicted",
        category: "Selection",
        handler: () => handleSelectAll(),
      },
      {
        keys: "p",
        description: "Pair current candidate",
        category: "Actions",
        handler: () => {
          if (cursorCandidate) void handlePair(cursorCandidate);
        },
      },
      {
        keys: "d",
        description: "Dismiss current candidate",
        category: "Actions",
        handler: () => {
          if (cursorCandidate) void handleDismiss(cursorCandidate);
        },
      },
      {
        keys: "e",
        description: "Open 'Apply all exact' preview",
        category: "Actions",
        handler: () => {
          if (exactSolo.length > 0) openExactPreview();
        },
      },
      {
        keys: "u",
        description: "Undo last bulk action",
        category: "Actions",
        handler: () => {
          if (undoState) void performUndo();
        },
      },
      {
        keys: "f",
        description: "Focus asset-pair filter",
        category: "Navigation",
        handler: () => {
          filterInputRef.current?.focus();
        },
      },
      {
        keys: "r",
        description: "Refresh candidates",
        category: "Navigation",
        handler: () => void refetch(),
      },
    ];
  }, [
    candidates,
    clusterSizes,
    cursorCandidate,
    exactSolo,
    detailCandidate,
    helpOpen,
    previewState,
    selected,
    undoState,
  ]);

  useKeymap(bindings);

  return (
    <div className={screenShellClassName}>
      <Collapsible open={rulesExpanded} onOpenChange={setRulesExpanded}>
        <div className={cn(screenPanelClassName, "overflow-hidden rounded-xl border bg-card")}>
          <header className="flex flex-col gap-3 px-3 py-4 sm:flex-row sm:items-start sm:justify-between sm:px-6">
            <div className="min-w-0 space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <h1 className="text-2xl font-semibold tracking-tight">
                  Swap candidates
                </h1>
                <Badge variant="secondary" className="rounded-md">
                  {counts.total}
                </Badge>
                <MetricPill
                  label="Exact"
                  value={counts.exact}
                  active={confidence === "exact"}
                  tone="good"
                  onClick={() => setConfidence(confidence === "exact" ? "all" : "exact")}
                />
                <MetricPill
                  label="Strong"
                  value={counts.strong}
                  active={confidence === "strong"}
                  tone="warning"
                  onClick={() => setConfidence(confidence === "strong" ? "all" : "strong")}
                />
                <MetricPill label="Conflicts" value={counts.conflicts} tone="alert" />
              </div>
              <p className="max-w-3xl text-sm text-muted-foreground">
                Review likely Lightning, Liquid, and on-chain legs before they
                become carrying-value pairs.
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
                        aria-label={`Delete view ${view.name}`}
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
                  <span>Apply exact {exactSolo.length}</span>
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
                <span className="ml-1">Refresh</span>
              </Button>
              <CollapsibleTrigger asChild>
                <Button variant="outline" size="sm" className="h-9">
                  <SettingsIcon className="size-3.5" />
                  <span>Rules {enabledRuleCount}/{rules.length}</span>
                </Button>
              </CollapsibleTrigger>
            </div>
          </header>

          <div className="grid gap-2 border-t px-3 py-3 text-sm xl:grid-cols-[minmax(0,1fr)_auto] xl:items-center sm:px-6">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <label className="flex shrink-0 items-center gap-2">
                <Checkbox
                  checked={selected.size > 0}
                  onCheckedChange={handleSelectAll}
                />
                <span className="text-xs text-muted-foreground">
                  Select
                </span>
              </label>
              <span className="shrink-0 text-xs text-muted-foreground">
                {candidates.length} visible
              </span>
              <Select value={confidence} onValueChange={setConfidence}>
                <SelectTrigger className="h-8 w-40 shrink-0">
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
                <SelectTrigger className="h-8 w-36 shrink-0">
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
                ref={filterInputRef}
                aria-label="Asset pair filter"
                className="h-8 w-24 shrink-0 rounded border border-input bg-background px-2 text-sm"
                placeholder="OUT-IN"
                value={assetPair}
                onChange={(e) => setAssetPair(e.target.value)}
              />
              {filterIsDirty ? (
                <>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 shrink-0 px-2"
                    onClick={() => setSaveViewOpen(true)}
                  >
                    <Star className="size-3.5" />
                    <span>Save</span>
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 shrink-0 px-2"
                    onClick={() => {
                      setConfidence("all");
                      setMethod("all");
                      setAssetPair("");
                    }}
                  >
                    Clear
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
                  <span>Apply rule {ruleSolo.length}</span>
                </Button>
              ) : null}
            </div>
          </div>

          <CollapsibleContent>
            <div className="space-y-2 border-t bg-muted/10 p-3 text-xs sm:px-6">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">Auto-pair rules</span>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 text-xs"
                  onClick={() => setCreateRuleOpen(true)}
                >
                  <Plus className="size-3" />
                  <span>New rule</span>
                </Button>
              </div>
              {rules.length === 0 ? (
                <p className="text-muted-foreground">
                  No auto-pair rules yet. Rules apply when a candidate matches
                  the predicate and isn't part of a conflict cluster.
                </p>
              ) : (
                rules.map((rule) => (
                  <div
                    key={rule.id}
                    className="flex flex-wrap items-center gap-2 rounded border border-border/60 bg-background px-2 py-1"
                  >
                    <span className="font-medium">{rule.name ?? "(unnamed)"}</span>
                    <code className="rounded bg-muted px-1 text-[10px]">
                      {Object.entries(rule.predicate)
                        .filter(([, v]) => v !== null && v !== "")
                        .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
                        .join(" · ") || "any candidate"}
                    </code>
                    <Badge variant="outline" className="text-[10px]">
                      {rule.kind} · {rule.policy}
                    </Badge>
                    <div className="ml-auto flex items-center gap-2">
                      <Switch
                        checked={rule.enabled}
                        onCheckedChange={() => void toggleRule(rule)}
                        aria-label="Toggle rule"
                      />
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-6 px-1"
                        onClick={() => void deleteRule(rule)}
                        aria-label="Delete rule"
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
              <Loader2 className="size-4 animate-spin" /> Loading candidates…
            </div>
          ) : isError ? (
            <div className="border-t px-6 py-6">
              <div className="rounded border border-destructive/40 bg-destructive/10 p-3 text-sm">
                Failed to load candidates: {String(error)}
              </div>
            </div>
          ) : candidates.length === 0 ? (
            <div className="border-t px-6 py-8">
              <div className="rounded border border-dashed border-muted-foreground/40 p-6 text-center text-sm text-muted-foreground">
                No unpaired swap candidates. Once a Lightning ↔ Liquid swap (or
                BTC ↔ LBTC peg) shows up in your wallets, it will appear here.
              </div>
            </div>
          ) : (
            <Table className="min-w-[980px] border-t">
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead className="w-[42px]"></TableHead>
                  <TableHead className="w-[118px] text-xs font-medium text-muted-foreground">
                    Confidence
                  </TableHead>
                  <TableHead className="w-[132px] text-xs font-medium text-muted-foreground">
                    Method
                  </TableHead>
                  <TableHead className="min-w-[250px] text-xs font-medium text-muted-foreground">
                    Outgoing
                  </TableHead>
                  <TableHead className="w-[44px] text-center"></TableHead>
                  <TableHead className="min-w-[250px] text-xs font-medium text-muted-foreground">
                    Incoming
                  </TableHead>
                  <TableHead className="min-w-[120px] text-right text-xs font-medium text-muted-foreground">
                    Swap fee
                  </TableHead>
                  <TableHead className="w-[44px]"></TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {candidates.map((candidate) => {
                  const key = candidateKey(candidate);
                  const conflicted = (clusterSizes[candidate.conflict_set_id] ?? 0) > 1;
                  return (
                    <TableRow
                      key={key}
                      className={cn(
                        "cursor-pointer align-top hover:bg-muted/35",
                        conflicted ? "bg-amber-50/35 dark:bg-amber-950/20" : null,
                        cursorKey === key ? "bg-muted/60" : null,
                      )}
                      onClick={() => setDetailCandidate(candidate)}
                    >
                      <TableCell>
                        <Checkbox
                          aria-label="Select candidate"
                          disabled={conflicted}
                          checked={selected.has(key)}
                          onClick={(event) => event.stopPropagation()}
                          onCheckedChange={() => toggleSelected(key)}
                        />
                      </TableCell>
                      <TableCell className="whitespace-normal">
                        <ConfidenceBadge candidate={candidate} />
                        {conflicted ? (
                          <p className="mt-1 inline-flex items-center gap-1 text-xs text-amber-700 dark:text-amber-300">
                            <AlertTriangle className="size-3" />
                            {clusterSizes[candidate.conflict_set_id]} share a leg
                          </p>
                        ) : null}
                      </TableCell>
                      <TableCell className="whitespace-normal">
                        <div className="text-xs text-muted-foreground">
                          {candidate.method === "payment_hash"
                            ? "payment hash"
                            : "time + amount"}
                        </div>
                        {candidate.rule_match ? (
                          <Badge variant="outline" className="mt-1 text-[10px]">
                            Rule
                          </Badge>
                        ) : null}
                      </TableCell>
                      <TableCell className="whitespace-normal">
                        <SwapLegInline
                          direction="out"
                          asset={candidate.out_asset}
                          amount={candidate.out_amount}
                          wallet={candidate.out_wallet_label}
                          walletKind={candidate.out_wallet_kind}
                          timestamp={candidate.out_occurred_at}
                          txId={candidate.out_id}
                        />
                      </TableCell>
                      <TableCell className="text-center text-muted-foreground">
                        <ArrowRight className="mx-auto mt-1 size-4" aria-hidden="true" />
                      </TableCell>
                      <TableCell className="whitespace-normal">
                        <SwapLegInline
                          direction="in"
                          asset={candidate.in_asset}
                          amount={candidate.in_amount}
                          wallet={candidate.in_wallet_label}
                          walletKind={candidate.in_wallet_kind}
                          timestamp={candidate.in_occurred_at}
                          txId={candidate.in_id}
                        />
                      </TableCell>
                      <TableCell className="whitespace-normal text-right">
                        <SwapFeeText candidate={candidate} />
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
          )}

          {selected.size > 0 ? (
            <div className="flex flex-wrap items-center gap-2 border-t bg-muted/25 px-3 py-3 text-sm sm:px-6">
              <span className="mr-1 text-xs font-medium text-foreground">
                {selected.size} selected
              </span>
              <label className="flex items-center gap-1 text-xs text-muted-foreground">
                Kind
                <Select
                  value={bulkKind ?? "default"}
                  onValueChange={(v) => setBulkKind(v === "default" ? null : v as PairKind)}
                >
                  <SelectTrigger className="ml-1 h-8 w-40">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">Candidate default</SelectItem>
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
                  value={bulkPolicy ?? "default"}
                  onValueChange={(v) => setBulkPolicy(v === "default" ? null : v as PairPolicy)}
                >
                  <SelectTrigger className="ml-1 h-8 w-40">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="default">Candidate default</SelectItem>
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
                Pair selected
              </Button>
            </div>
          ) : null}

          <div className="flex flex-col items-center justify-between gap-2 border-t px-3 py-3 text-xs text-muted-foreground sm:flex-row sm:px-6">
            <span>
              Showing {candidates.length === 0 ? 0 : 1}-{candidates.length} of {counts.total}
            </span>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2"
              onClick={() => setHelpOpen(true)}
              aria-label="Show keyboard shortcuts"
              title="Show keyboard shortcuts"
            >
              ?
            </Button>
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
      />

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
                : previewState?.mode === "rules"
                  ? "Apply matching rules"
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

function MetricPill({
  label,
  value,
  tone = "neutral",
  active = false,
  onClick,
}: {
  label: string;
  value: number;
  tone?: "neutral" | "good" | "warning" | "alert";
  active?: boolean;
  onClick?: () => void;
}) {
  const toneClass = {
    neutral: "text-muted-foreground",
    good: "text-emerald-700 dark:text-emerald-300",
    warning: "text-amber-700 dark:text-amber-300",
    alert: "text-rose-700 dark:text-rose-300",
  }[tone];
  const className = cn(
    "inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-xs",
    active ? "border-primary bg-primary/10 text-primary" : "bg-background text-muted-foreground",
  );
  const content = (
    <>
      <span>{label}</span>
      <span className={cn("font-semibold tabular-nums", active ? "text-primary" : toneClass)}>
        {value.toLocaleString("en-US")}
      </span>
    </>
  );
  if (!onClick) {
    return <span className={className}>{content}</span>;
  }
  return (
    <button type="button" className={cn(className, "hover:bg-muted/60")} onClick={onClick}>
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
}

function SwapLegInline({
  direction,
  asset,
  amount,
  wallet,
  walletKind,
  timestamp,
  txId,
}: SwapLegInlineProps) {
  const rail = railForLeg(asset, walletKind);
  const walletName = displayWalletName(wallet, walletKind);
  return (
    <div className="grid min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_auto] items-start gap-2">
      <RailIcon rail={rail} size="compact" />
      <div className="min-w-0 text-xs text-muted-foreground">
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
          <span className="truncate">{walletName}</span>
          <span aria-hidden="true">·</span>
          <span>{formatTimestamp(timestamp)}</span>
        </div>
        <div className="mt-1 truncate font-mono text-[11px] text-muted-foreground/80">
          tx {txId.slice(0, 8)}…{txId.slice(-4)}
        </div>
      </div>
      <div className="min-w-[8.5rem] text-right">
        <div
          className={cn(
            "font-mono text-sm font-semibold tabular-nums",
            direction === "out"
              ? "text-red-700 dark:text-red-300"
              : "text-emerald-700 dark:text-emerald-300",
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

function SwapFeeText({ candidate }: { candidate: SwapCandidate }) {
  const percent = feePercent(candidate);
  const tone =
    percent <= 0.5
      ? "text-emerald-700 dark:text-emerald-300"
      : percent <= 1
        ? "text-amber-700 dark:text-amber-300"
        : "text-rose-700 dark:text-rose-300";
  return (
    <div className="text-right">
      <div className={cn("text-sm font-semibold tabular-nums", tone)}>
        {formatBtc(candidate.swap_fee)}
      </div>
      <div className="mt-1 text-xs text-muted-foreground">
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
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="size-8 text-muted-foreground hover:text-foreground"
          aria-label={`Open actions for ${candidate.out_id}`}
          onClick={(event) => event.stopPropagation()}
        >
          <MoreHorizontal className="size-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={onOpen}>
          <Eye className="mr-2 size-4" aria-hidden="true" />
          Open details
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={pairDisabled} onSelect={onPair}>
          <Check className="mr-2 size-4" aria-hidden="true" />
          Pair
        </DropdownMenuItem>
        <DropdownMenuItem
          className="text-destructive"
          disabled={dismissDisabled}
          onSelect={onDismiss}
        >
          <X className="mr-2 size-4" aria-hidden="true" />
          Dismiss
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
}: SwapCandidateDetailSheetProps) {
  const kind = candidate ? override.kind ?? candidate.default_kind : "manual";
  const policy = candidate ? override.policy ?? candidate.default_policy : "carrying-value";
  return (
    <Sheet open={Boolean(candidate)} onOpenChange={onOpenChange}>
      <SheetContent className="w-full overflow-y-auto p-0 sm:max-w-2xl">
        {candidate ? (
          <>
            <SheetHeader className="border-b p-4 sm:p-6">
              <SheetTitle>Swap candidate</SheetTitle>
              <SheetDescription>
                {candidate.method === "payment_hash"
                  ? "Matched by payment hash."
                  : "Matched by time and amount."}
                {" "}
                Delta {formatSats(candidate.swap_fee_msat)} ({feePercent(candidate).toFixed(2)}%).
              </SheetDescription>
            </SheetHeader>
            <div className="space-y-4 p-4 sm:p-6">
              <div className="grid gap-4 md:grid-cols-2">
                <SwapLegDetails
                  title="Outgoing"
                  asset={candidate.out_asset}
                  amount={candidate.out_amount}
                  amountMsat={candidate.out_amount_msat}
                  wallet={candidate.out_wallet_label}
                  walletKind={candidate.out_wallet_kind}
                  timestamp={candidate.out_occurred_at}
                  txId={candidate.out_id}
                />
                <SwapLegDetails
                  title="Incoming"
                  asset={candidate.in_asset}
                  amount={candidate.in_amount}
                  amountMsat={candidate.in_amount_msat}
                  wallet={candidate.in_wallet_label}
                  walletKind={candidate.in_wallet_kind}
                  timestamp={candidate.in_occurred_at}
                  txId={candidate.in_id}
                />
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>Kind</Label>
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
                  <Label>Policy</Label>
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
                  <div className="font-medium">Match rationale</div>
                  <p className="mt-1 text-muted-foreground">
                    {candidate.method === "payment_hash"
                      ? "Both legs share the same Lightning payment hash."
                      : "The legs are close in time and amount after accounting for the swap fee."}
                  </p>
                  {candidate.rule_match ? (
                    <p className="mt-2 text-xs text-muted-foreground">
                      Auto-pair route: {candidate.rule_match.rule_name ?? candidate.rule_match.rule_id}
                    </p>
                  ) : null}
                </div>
                <div className="rounded-lg border bg-muted/20 p-3 text-sm">
                  <div className="font-medium">Accounting preview</div>
                  <dl className="mt-2 space-y-1 text-xs">
                    <DetailRow label="Pair kind" value={kind} />
                    <DetailRow label="Policy" value={policy} />
                    <DetailRow
                      label="Swap fee"
                      value={`${formatSats(candidate.swap_fee_msat)} · ${feePercent(candidate).toFixed(2)}%`}
                    />
                  </dl>
                  <p className="mt-2 text-xs text-muted-foreground">
                    Journal and tax deltas are computed on the next journal reprocess.
                  </p>
                </div>
              </div>
            </div>
            <SheetFooter className="border-t p-4 sm:flex-row sm:items-center sm:justify-between sm:px-6">
              <Button
                variant="outline"
                onClick={() => onDismiss(candidate)}
                disabled={dismissDisabled}
              >
                Dismiss
              </Button>
              <Button onClick={() => onPair(candidate)} disabled={pairDisabled}>
                Pair
              </Button>
            </SheetFooter>
          </>
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

function previewSummaryText(candidates: SwapCandidate[]): string {
  if (candidates.length === 0) return "No candidates to pair.";
  const totalFeeMsat = candidates.reduce((acc, c) => acc + c.swap_fee_msat, 0);
  const totalCarry = candidates.reduce((acc, c) => acc + c.out_amount, 0);
  return `${candidates.length} pair${candidates.length === 1 ? "" : "s"} · carrying value ${formatBtc(totalCarry)} · total swap fees ${formatSats(totalFeeMsat)}.`;
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
  return (
    <Dialog open={open} onOpenChange={(value) => (!value ? onCancel() : undefined)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Save current filter as a view</DialogTitle>
          <DialogDescription>
            The active confidence, method, and asset-pair filters are saved
            verbatim. Pick a short name; the chip appears at the top of the
            queue for one-click recall.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="view-name">View name</Label>
          <Input
            id="view-name"
            autoFocus
            placeholder="e.g. Boltz pegouts"
            value={name}
            onChange={(event) => onNameChange(event.target.value)}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={() => void onSave()} disabled={isSaving || !name.trim()}>
            {isSaving ? <Loader2 className="size-4 animate-spin" /> : null}
            <span className="ml-1">Save</span>
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
          <DialogTitle>Create auto-pair rule</DialogTitle>
          <DialogDescription>
            Candidates matching every non-default field will auto-pair
            with the chosen kind / policy. Conflict clusters never auto-pair.
          </DialogDescription>
        </DialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <div className="col-span-2 space-y-1">
            <Label htmlFor="rule-name">Name (optional)</Label>
            <Input
              id="rule-name"
              placeholder="e.g. Phoenix → Liquid"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>
          <RulePredicateAssetField
            label="Out asset"
            value={outAsset}
            onChange={setOutAsset}
          />
          <RulePredicateAssetField
            label="In asset"
            value={inAsset}
            onChange={setInAsset}
          />
          <RulePredicateKindField
            label="Out wallet kind"
            value={outKind}
            onChange={setOutKind}
          />
          <RulePredicateKindField
            label="In wallet kind"
            value={inKind}
            onChange={setInKind}
          />
          <div className="space-y-1">
            <Label htmlFor="max-fee">Max fee % of principal</Label>
            <Input
              id="max-fee"
              placeholder="e.g. 0.01"
              value={maxFeePct}
              onChange={(e) => setMaxFeePct(e.target.value)}
            />
          </div>
          <div className="space-y-1">
            <Label>Min confidence</Label>
            <Select
              value={minConfidence}
              onValueChange={(v) => setMinConfidence(v as "strong" | "exact")}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="strong">Strong (or exact)</SelectItem>
                <SelectItem value="exact">Exact only</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1">
            <Label>Kind</Label>
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
            <Label>Policy</Label>
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
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={isCreating}>
            {isCreating ? <Loader2 className="size-4 animate-spin" /> : null}
            <span className="ml-1">Create rule</span>
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
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">Any</SelectItem>
          <SelectItem value="BTC">BTC</SelectItem>
          <SelectItem value="LBTC">LBTC</SelectItem>
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
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            Shortcuts work whenever the focus isn't in a text field.
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
          <Button onClick={onClose}>Close</Button>
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
  return (
    <div className="space-y-1">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="any">Any</SelectItem>
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
}: SwapLegDetailsProps) {
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
          label="Wallet"
          value={
            <span className="inline-flex min-w-0 items-center gap-2">
              <RailIcon rail={rail} size="compact" />
              <span className="truncate">{walletName}</span>
              {showKind ? (
                <span className="text-xs uppercase text-muted-foreground">· {walletKind}</span>
              ) : null}
            </span>
          }
        />
        <DetailRow
          label="Amount"
          value={
            <span className="font-mono tabular-nums">
              {formatBtc(amount)} <span className="text-xs text-muted-foreground">({formatSats(amountMsat)})</span>
            </span>
          }
        />
        <DetailRow label="Occurred" value={formatTimestamp(timestamp)} />
        <DetailRow
          label="Transaction"
          value={<span className="font-mono text-xs">{txId}</span>}
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
  const assetLabel = asset.toUpperCase();
  const labelParts = railLabel === assetLabel ? [assetLabel] : [railLabel, assetLabel];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase",
        details.className,
      )}
      title={`${details.label} ${asset}`}
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
