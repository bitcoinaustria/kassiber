/**
 * Transfers: has every movement between the owner's wallets been accounted
 * for? The screen answers that first, then lists what needs a decision
 * (held moves, then suggestions), moves the owner unlinked recently, and
 * every linked move with how it was linked and the way to unlink it.
 *
 * Linking by hand ("Link manually") and a transfer's details open in a panel
 * docked beside the lists when the page is wide enough, one at a time, and
 * as dialogs otherwise; both show the review core's dry run where it has
 * one. What an action did is said where it happened, in a strip at the
 * transfer's row with the way back, or in a toast when that row is not in
 * view. Rules and
 * saved views sit in a sheet off the menu. The developer-gated custody tools
 * (missing wallets, components, scan coverage) are the one secondary
 * section. Navigation lives in the URL so other screens can open "Link
 * manually" on a transaction and the assistant describes the visible
 * context.
 */
import * as React from "react";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  Bitcoin,
  Check,
  CheckCircle2,
  CircleHelp,
  Info,
  Link2,
  Loader2,
  MoreHorizontal,
  RefreshCw,
  Settings2,
  SlidersHorizontal,
  Sparkles,
  Split,
  Unlink,
  UserCheck,
  Wrench,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { StatTile } from "@/components/kb/StatTile";
import {
  SurfaceCardHeader,
  surfaceCountClassName,
  surfaceFilterChipClassName,
} from "@/components/kb/SurfaceCardHeader";
import type { QuarantineItem, QuarantineSnapshot } from "@/components/kb/quarantine/types";
import { ReadinessPill } from "@/components/kb/ReadinessPill";
import { ReviewTransfers, type SuggestionFilter } from "@/components/review/ReviewTransfers";
import { differentAssetBadgeClassName, isBitcoinAsset, type TransferSuggestions } from "@/components/review/transferModel";
import { useJournalRefreshAfterDecisions } from "@/components/review/useJournalRefreshAfterDecisions";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { blurClass, formatDisplayMoney } from "@/components/transactions/model";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useDaemon, useDaemonInfinite, useDaemonMutation } from "@/daemon/client";
import type { SwapRail } from "@/lib/bitcoinRail";
import { useCurrency } from "@/lib/currency";
import { formatCount } from "@/lib/localeFormat";
import type { ReviewBadges } from "@/lib/reviewCount";
import {
  pageDescriptionClassName,
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
  pageHeaderIconButtonClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { CustodyCoverageSection, CustodyGapsContent } from "../CustodyGaps";
import { lineageEvidenceLabel } from "../lineageEvidence";
import { collectCustodyLineagePages, type CustodyLineageSnapshot } from "../custodyGapsModel";
import {
  LINK_NEW,
  transfersCustodyView,
  type CustodySurfaceTab,
  type TransfersCustodySearch,
} from "../transfers-custody/routeSearch";
import { CustodyComponentResolver } from "./CustodyComponentResolver";
import { AssetFigure } from "./figures";
import { LinkManuallyDialog, type LinkedResult, type LinkStart } from "./LinkManuallyDialog";
import {
  buildLinkedMoves,
  countLinkedMoves,
  filterLinkedMoves,
  heldUndonePairings,
  moveYear,
  moveYears,
  automaticLinkKind,
  canUnlinkMove,
  fixedLinkKind,
  moveIsTaxable,
  moveType,
  recentlyUnlinked,
  setAsideSuggestions,
  splitWalletName,
  transfersVerdict,
  withoutHeld,
  type LinkedMove,
  type LinkedMoveFilter,
  type LinkedMoveSource,
  type RecentlyUnlinked,
  type TransferDismissal,
  type TransferDismissalsEnvelope,
  type TransferPairSummary,
  type TransferPairsEnvelope,
  type TransfersVerdict,
} from "./linkedMovesModel";
import { datePart, hasOpenPart, timePart, type LegSide, type OpenPart } from "./linkModel";
import { RecentlyUnlinkedCard } from "./RecentlyUnlinkedCard";
import { RowMenu, TransactionsMenuItems } from "./RowMenu";
import { SetAsideCard } from "./SetAsideCard";
import { useOpenParts } from "./useOpenParts";
import { useSettledSuggestions } from "./useSettledSuggestions";
import { RulesAndViewsSheet, type TransfersSavedView } from "./RulesSheet";
import { formatMsatAsBtc, formatMsatAsSats, formatMsatIn, sumMsat } from "./satsFormat";
import { UnlinkDialog, type MoveDialogIntent, type UnlinkResult } from "./UnlinkDialog";

const LINKED_PAGE = 25;

export function TransfersPage() {
  const navigate = useNavigate();
  const rawSearch = useSearch({ strict: false });
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const search = transfersCustodyView(rawSearch, developerToolsEnabled);
  React.useEffect(() => {
    if (!developerToolsEnabled && rawSearch.section === "advanced") {
      void navigate({ to: "/transfers", search: {}, replace: true });
    }
  }, [developerToolsEnabled, rawSearch.section, navigate]);
  const select = (next: TransfersCustodySearch, replace = false) => {
    void navigate({ to: "/transfers", search: next, replace });
  };

  return (
    <div className={cn(screenShellClassName, "@container/page")}>
      {search.section === "advanced" ? (
        <SecondarySection
          titleKey="transfersPage.advanced.title"
          descriptionKey="transfersPage.advanced.description"
          onBack={() => select({})}
          developer
        >
          <AdvancedCustodyTools
            tab={search.tab}
            gap={search.gap ?? null}
            onSelect={(tab, gap) => select({ section: "advanced", tab, ...(gap ? { gap } : {}) })}
          />
        </SecondarySection>
      ) : (
        <TransfersOverview
          link={search.link ?? null}
          savedViewId={search.savedView ?? null}
          onOpen={select}
          onConsumeLink={() => select(search.savedView ? { savedView: search.savedView } : {}, true)}
        />
      )}
    </div>
  );
}

function SecondarySection({
  titleKey,
  descriptionKey,
  onBack,
  developer = false,
  children,
}: {
  titleKey: "transfersPage.advanced.title";
  descriptionKey: "transfersPage.advanced.description";
  onBack: () => void;
  developer?: boolean;
  children: React.ReactNode;
}) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <Button type="button" variant="ghost" size="sm" className="-ml-2 h-7 gap-1 px-2 text-xs text-muted-foreground" onClick={onBack}>
          <ArrowLeft className="size-3.5" aria-hidden="true" />
          {t("transfersPage.back")}
        </Button>
        <h1 className="flex items-center gap-2 text-lg leading-tight font-semibold">
          {t(titleKey)}
          {developer ? <DeveloperBadge /> : null}
        </h1>
        <p className={pageDescriptionClassName}>{t(descriptionKey)}</p>
      </div>
      {children}
    </div>
  );
}

function DeveloperBadge() {
  const { t } = useTranslation("review");
  return (
    <Badge variant="outline" className="border-sky-500/40 text-2xs font-normal text-sky-700 dark:text-sky-300">
      {t("transfersPage.checked.developer")}
    </Badge>
  );
}

function AdvancedCustodyTools({
  tab,
  gap,
  onSelect,
}: {
  tab: CustodySurfaceTab;
  gap: string | null;
  onSelect: (tab: CustodySurfaceTab, gap?: string | null) => void;
}) {
  const { t } = useTranslation("review");
  return (
    <div className="space-y-4">
      <Tabs value={tab} onValueChange={(next) => onSelect(next as CustodySurfaceTab)}>
        <TabsList>
          <TabsTrigger value="gaps">{t("transfersPage.advanced.tabs.gaps")}</TabsTrigger>
          <TabsTrigger value="components">{t("transfersPage.advanced.tabs.components")}</TabsTrigger>
          <TabsTrigger value="coverage">{t("transfersPage.advanced.tabs.coverage")}</TabsTrigger>
        </TabsList>
      </Tabs>
      {tab === "components" ? (
        <CustodyComponentResolver />
      ) : tab === "coverage" ? (
        <CustodyCoverageSection />
      ) : (
        <CustodyGapsContent focusGapId={gap} onFocusGap={(next) => onSelect("gaps", next)} />
      )}
    </div>
  );
}

/** The transfer a note is about: on the Linked list it is said at that row. */
interface Spot {
  outId: string;
  inId: string;
}

/**
 * A note after an action, with a way back. On the Linked list it shows as a
 * strip at its transfer's row (or where that row was) until it is closed;
 * anywhere else it is a toast that goes away on its own. Never both.
 */
type Toast = { spot?: Spot } & (
  | { kind: "unlinked"; dismissalId: string | null; move: Pick<LinkedMove, "outTransactionId" | "inTransactionId"> }
  /** `portion`: only part of the send was swapped, so linking it again is checked first. */
  | { kind: "unpaired"; result: Extract<UnlinkResult, { kind: "pair" }>; portion: boolean }
  | { kind: "linkedBack" }
  /** A link made by hand, with the pair to remove if the owner takes it back. */
  | { kind: "linked"; pairId: string | null; sendId: string }
  | { kind: "linkUndone" }
);

const spotOf = (move: Pick<LinkedMove, "outTransactionId" | "inTransactionId">): Spot => ({
  outId: move.outTransactionId,
  inId: move.inTransactionId,
});

/**
 * Whether a pair swapped only part of its send. Every pair reports the
 * amount it covers (`out_amount`), so that alone says nothing: it is a
 * portion when the pair is between two assets and leaves some of the send.
 * A core that does not say what is left is taken at its `out_amount`.
 */
function swappedPortion(listed: TransferPairSummary | undefined, removed: TransferPairSummary | null): boolean {
  const left = listed?.out.unlinked_msat;
  if (!listed || left === null || left === undefined) return Boolean(removed?.out_amount);
  try {
    return listed.out.asset !== listed.in.asset && BigInt(left) > 0n;
  } catch {
    return true;
  }
}

/**
 * What a pair leaves of its send for a further link, as msat text; null when
 * nothing is left, the pair takes no other, or what is left is small enough
 * to be the difference between the two sides (the rule is `openParts`').
 */
function restOfPair(pairs: readonly TransferPairSummary[] | undefined, parts: Record<LegSide, Map<string, OpenPart>> | null, pairId: string | null): string | null {
  const pair = pairId ? pairs?.find((row) => row.id === pairId) : undefined;
  const open = pair ? parts?.send.get(pair.out_transaction_id) : undefined;
  return open && hasOpenPart({ linked: true, open }) ? open.unlinkedMsat.toString() : null;
}

/**
 * Whether a hold is the rest of a send the owner linked in part: the core's
 * residual hold on a pair whose send still has a part worth a link. A hold
 * for another reason that only names the pair (an old fee allocation to
 * repair) is not one.
 */
function restHoldPair(item: QuarantineItem, pairs: readonly TransferPairSummary[] | undefined, parts: Record<LegSide, Map<string, OpenPart>> | null): { pair: TransferPairSummary; rest: string } | null {
  const blocker = item.evidence?.blocker_code;
  if (blocker && blocker !== "reviewed_residual_suspense") return null;
  if (item.evidence?.review_codes?.includes("legacy_pair_fee_allocation_review_required")) return null;
  const pairId = item.actions?.find((action) => action.kind === "review_pair")?.pair_id ?? null;
  const pair = pairId ? pairs?.find((row) => row.id === pairId) : undefined;
  const rest = pair ? restOfPair(pairs, parts, pair.id) : null;
  return pair && rest ? { pair, rest } : null;
}

const TOAST_MS = 10_000;

/** The most undone links and set-aside suggestions one read returns. */
const DISMISSALS_LIMIT = 500;

/** The most rows one read of the holds returns. */
const HELD_PAGE = 100;

/** The next page of holds, or nothing once every row is read. */
function nextHeldOffset(page: { data?: QuarantineSnapshot }): number | undefined {
  const summary = page.data?.summary;
  const items = page.data?.items.length ?? 0;
  if (!summary || !items) return undefined;
  const next = (summary.offset ?? 0) + items;
  return next < (summary.scope_count ?? summary.count ?? 0) ? next : undefined;
}

interface SavedViewsEnvelope {
  views: TransfersSavedView[];
}

/** The parts of a saved view's filter the suggestions understand. */
function viewFilter(view: TransfersSavedView): SuggestionFilter {
  const filter: SuggestionFilter = {};
  for (const key of ["confidence", "method", "asset_pair", "route_pair", "candidate_type"] as const) {
    const value = view.filter[key];
    if (typeof value === "string" && value && value !== "all") filter[key] = value;
  }
  if (view.surface === "swap_candidates" && !filter.candidate_type) filter.candidate_type = "swap";
  if (view.surface === "transfer_candidates" && !filter.candidate_type) filter.candidate_type = "transfer";
  return filter;
}

export function TransfersOverview({
  link = null,
  savedViewId = null,
  onOpen,
  onConsumeLink,
}: {
  /** Open "Link manually" on this transaction (`new`: nothing picked). */
  link?: string | null;
  savedViewId?: string | null;
  onOpen: (next: TransfersCustodySearch) => void;
  /** Takes the one-shot `link` out of the URL once the dialog opened. */
  onConsumeLink?: () => void;
}) {
  const { t } = useTranslation("review");
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const { requestRefresh, isProcessingJournals, refreshPending } = useJournalRefreshAfterDecisions();
  const updating = isProcessingJournals || refreshPending;

  const badgesQuery = useDaemon<ReviewBadges>("ui.review.badges");
  const suggestQuery = useDaemon<TransferSuggestions>("ui.transfers.suggest", {});
  const pairsQuery = useDaemon<TransferPairsEnvelope>("ui.transfers.list", { include_unlinked: true });
  // As many as one read returns; the core has no further pages to ask for.
  const dismissalsQuery = useDaemon<TransferDismissalsEnvelope>("ui.transfers.dismissals.list", { limit: DISMISSALS_LIMIT });
  // Every page of what the journals hold: a held transfer may be on any of them.
  const heldQuery = useDaemonInfinite<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: HELD_PAGE, offset: 0, scope: "attention" },
    nextHeldOffset,
  );
  const { hasNextPage: moreHeld, isFetchingNextPage: fetchingHeld, isError: heldFailed, fetchNextPage: fetchMoreHeld } = heldQuery;
  React.useEffect(() => {
    if (moreHeld && !fetchingHeld && !heldFailed) void fetchMoreHeld();
  }, [moreHeld, fetchingHeld, heldFailed, fetchMoreHeld]);
  const lineageQuery = useDaemonInfinite<CustodyLineageSnapshot>(
    "ui.custody.lineage.snapshot",
    // `priced` adds each move's fiat value and its send's network fee.
    { limit: 100, priced: true },
    (lastPage) => lastPage.data?.next_cursor ?? undefined,
  );
  const transferViews = useDaemon<SavedViewsEnvelope>("ui.saved_views.list", { surface: "transfer_candidates" }, { enabled: Boolean(savedViewId) });
  const swapViews = useDaemon<SavedViewsEnvelope>("ui.saved_views.list", { surface: "swap_candidates" }, { enabled: Boolean(savedViewId) });

  const undismiss = useDaemonMutation("ui.transfers.dismissals.delete");
  const repair = useDaemonMutation("ui.transfers.pair");
  const unpair = useDaemonMutation("ui.transfers.unpair");

  const lineage = React.useMemo(
    () =>
      collectCustodyLineagePages(
        (lineageQuery.data?.pages ?? [])
          .map((page) => page.data)
          .filter((page): page is CustodyLineageSnapshot => Boolean(page)),
      ),
    [lineageQuery.data?.pages],
  );
  const pairs = pairsQuery.data?.data?.pairs;
  const unlinkedPairs = pairsQuery.data?.data?.unlinked;
  const dismissals = dismissalsQuery.data?.data?.dismissals;
  const heldItems = React.useMemo(
    () => (heldQuery.data?.pages ?? []).flatMap((page) => page.data?.items ?? []),
    [heldQuery.data?.pages],
  );
  const heldIds = React.useMemo(() => new Set(heldItems.map((item) => item.transaction_id)), [heldItems]);
  const held = React.useMemo(() => heldUndonePairings(dismissals ?? [], heldIds), [dismissals, heldIds]);
  // Holds a missing-wallet review explains: read-only until developer tools are on.
  const parts = useOpenParts();
  // So is the unlinked rest of a send the owner linked in part, which anyone can link.
  const gapHolds = React.useMemo(
    () =>
      heldItems.filter((item) =>
        (item.actions ?? []).some(
          (action) => action.kind === "review_custody_gap" || (action.kind === "review_pair" && restHoldPair(item, pairsQuery.data?.data?.pairs, parts) !== null),
        ),
      ),
    [heldItems, pairsQuery.data, parts],
  );
  const moves = React.useMemo(
    () =>
      withoutHeld(
        buildLinkedMoves({ lineage: lineage?.items ?? [], pairs: pairs ?? [], dismissals: dismissals ?? [] }),
        held,
      ),
    [lineage, pairs, dismissals, held],
  );
  const unlinked = React.useMemo(
    () => recentlyUnlinked({ unlinked: unlinkedPairs ?? [], dismissals: dismissals ?? [], held, linked: moves }),
    [unlinkedPairs, dismissals, held, moves],
  );
  const counts = countLinkedMoves(moves);
  const setAside = React.useMemo(() => setAsideSuggestions(dismissals ?? []), [dismissals]);
  // A read of stale journals suggests moves that are already linked, so while
  // the journals recalculate the page keeps the suggestions it had.
  const candidates = useSettledSuggestions(suggestQuery.data?.data?.candidates, updating, suggestQuery.refetch);
  const linkedTotal = Math.max(counts.automatic + counts.reviewed, lineage?.summary.total_count ?? 0);
  const waiting = held.length + gapHolds.length;
  // A failed read of the holds is not "nothing held": the page says it could not tell.
  // More undone links than one read holds: which of them are held is not known.
  const dismissalsIncomplete = Boolean(dismissalsQuery.data?.data?.has_more);
  const heldUnavailable = heldQuery.isError || dismissalsQuery.isError || dismissalsIncomplete;
  const readsUnavailable = heldUnavailable || badgesQuery.isError;
  const retryReads = () => {
    for (const query of [badgesQuery, heldQuery, dismissalsQuery]) if (query.isError) void query.refetch();
  };
  const verdict = transfersVerdict({
    // Until every page of holds is read, what waits is not known.
    loading:
      badgesQuery.isLoading || suggestQuery.isLoading || lineageQuery.isLoading || dismissalsQuery.isLoading || heldQuery.isLoading || (Boolean(moreHeld) && !heldFailed),
    unavailable: readsUnavailable,
    journalsStale: Boolean(badgesQuery.data?.data?.journals_needs_processing) && !updating,
    held: waiting,
    suggested: candidates.length,
    linked: linkedTotal,
  });
  const savedView = React.useMemo(() => {
    if (!savedViewId) return null;
    const views = [...(transferViews.data?.data?.views ?? []), ...(swapViews.data?.data?.views ?? [])];
    return views.find((view) => view.id === savedViewId) ?? null;
  }, [savedViewId, swapViews.data, transferViews.data]);

  const [filter, setFilter] = React.useState<LinkedMoveFilter>("all");
  const [year, setYear] = React.useState<number | null>(null);
  const [rulesOpen, setRulesOpen] = React.useState(false);
  const [toast, setToast] = React.useState<Toast | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [pendingUnlink, setPendingUnlink] = React.useState<LinkedMove | null>(null);
  // A row opens the move's details; its menu goes straight to unlinking.
  const [moveIntent, setMoveIntent] = React.useState<MoveDialogIntent>("details");
  const openMove = (move: LinkedMove, intent: MoveDialogIntent) => {
    setMoveIntent(intent);
    setPendingUnlink(move);
  };
  const [linkStart, setLinkStartState] = React.useState<LinkStart | null>(null);
  // One panel at a time: starting a link closes the transfer that was open.
  const setLinkStart = (next: LinkStart | null) => {
    if (next) setPendingUnlink(null);
    setLinkStartState(next);
  };
  const sortOutRef = React.useRef<HTMLHeadingElement>(null);
  const unlinkedRef = React.useRef<HTMLHeadingElement>(null);
  const suggestionsRef = React.useRef<HTMLDivElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);

  // Wide enough, "Link manually" and a transfer's details open in a panel
  // beside the lists that keeps them in view; narrower, as a dialog over
  // them. While a link is being made in the panel, a transfer opens as a
  // dialog, so the link is not lost.
  const contentRef = React.useRef<HTMLDivElement>(null);
  const contentWidth = useElementWidth(listRef);
  const wide = contentWidth >= PANEL_MIN_WIDTH;
  const linkPanel = wide ? linkStart : null;
  const panelMove = pendingUnlink && wide && !linkPanel ? pendingUnlink : null;
  // The panel is as tall as the window leaves room under its top edge, so
  // its buttons stay in view wherever the page is scrolled to.
  const [panelElement, setPanelElement] = React.useState<HTMLDivElement | null>(null);
  useRoomBelow(panelElement);

  const [tab, setTab] = React.useState<StatusTab | null>(null);
  // A suggestion's Undo is on offer: its list stays, though nothing is left in it.
  const [suggestionUndo, setSuggestionUndo] = React.useState(false);
  const needsCount = waiting + candidates.length;
  const tabCounts: Record<StatusTab, number> = {
    needs: needsCount,
    linked: linkedTotal,
    unlinked: unlinked.length,
    setAside: setAside.length,
  };
  // The owner's choice holds until that list empties; otherwise what needs
  // them comes first.
  const activeTab: StatusTab =
    tab && (tab === "linked" || tab === "needs" || tabCounts[tab] > 0) ? tab : needsCount > 0 ? "needs" : "linked";
  const chooseTab = (next: StatusTab) => {
    setTab(next);
    setPendingUnlink(null);
  };
  // A saved view filters the suggestions, so opening one (from the sheet or
  // a link) shows their list, even when nothing in it matches.
  React.useEffect(() => {
    if (!savedViewId) return;
    setTab("needs");
    setPendingUnlink(null);
  }, [savedViewId]);

  // On the Linked list a note about a transfer is a strip at its row.
  const strip = toast?.spot && activeTab === "linked" ? toast : null;
  React.useEffect(() => {
    // A strip stays until it is closed; a toast goes away on its own, once
    // the journals are done and its way back can be taken.
    if (!toast || strip || updating) return;
    const timer = window.setTimeout(() => setToast(null), TOAST_MS);
    return () => window.clearTimeout(timer);
  }, [toast, strip, updating]);

  // Another screen asked for "Link manually" on a transaction.
  React.useEffect(() => {
    if (!link) return;
    setPendingUnlink(null);
    setLinkStartState(link === LINK_NEW ? {} : { transactionId: link });
    onConsumeLink?.();
  }, [link, onConsumeLink]);

  const linkBack = async (
    dismissalId: string | null,
    move?: Pick<LinkedMove, "outTransactionId" | "inTransactionId">,
  ) => {
    setError(null);
    try {
      await undismiss.mutateAsync(
        dismissalId ? { dismissal_id: dismissalId } : { tx_out: move?.outTransactionId, tx_in: move?.inTransactionId },
      );
      setToast({ kind: "linkedBack", ...(move ? { spot: spotOf(move) } : {}) });
      requestRefresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("transfersPage.toast.linkBackFailed"));
    }
  };

  /** "Link again" with the terms the pair had, after the usual preview. */
  // A suggestion set aside comes back to the list; nothing is linked.
  const restoreSuggestion = async (dismissal: TransferDismissal) => {
    setError(null);
    try {
      await undismiss.mutateAsync({ dismissal_id: dismissal.id });
      void suggestQuery.refetch();
      void dismissalsQuery.refetch();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("transfersPage.toast.linkBackFailed"));
    }
  };

  const linkAgain = (row: RecentlyUnlinked) => {
    if (row.dismissal) {
      void linkBack(row.dismissal.id, row);
      return;
    }
    setLinkStart({
      sendId: row.outTransactionId,
      receiptId: row.inTransactionId,
      relink: true,
      preset: row.pair
        ? { kind: row.pair.kind, policy: row.pair.policy, outAmountMsat: row.pair.out_amount ?? null }
        : undefined,
    });
  };

  const onUnlinked = (result: UnlinkResult, differently: boolean) => {
    setPendingUnlink(null);
    // The list in view stays, though unlinking may add to what needs the owner.
    setTab(activeTab);
    if (result.kind === "automatic") {
      setToast({ kind: "unlinked", dismissalId: result.dismissalId, move: result.move, spot: spotOf(result.move) });
    } else {
      const listed = pairs?.find((pair) => pair.id === result.move.pairId);
      setToast({ kind: "unpaired", result, portion: swappedPortion(listed, result.pair), spot: spotOf(result.move) });
      // Removing a pair leaves the journals to recalculate.
      requestRefresh();
    }
    if (differently) setLinkStart({ transactionId: result.move.outTransactionId });
  };

  const showList = (next: LinkedMoveFilter) => {
    setFilter(next);
    chooseTab("linked");
    listRef.current?.scrollIntoView({ block: "start", behavior: "smooth" });
  };
  const openLink = (transactionId?: string) => setLinkStart(transactionId ? { transactionId } : {});

  const undoLink = async (pairId: string, spot?: Spot) => {
    setError(null);
    try {
      await unpair.mutateAsync({ pair_id: pairId });
      setToast({ kind: "linkUndone", ...(spot ? { spot } : {}) });
      requestRefresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("transfersPage.link.failed"));
    }
  };
  const undoUnpair = (next: Extract<Toast, { kind: "unpaired" }>) => {
    const { move, pair } = next.result;
    // Undo right after unlinking restores the pair with the terms it had,
    // returning the book to the state it was just in. A swapped portion
    // goes through Link again's preview instead.
    if (pair && !next.portion) {
      void (async () => {
        setError(null);
        try {
          await repair.mutateAsync({
            tx_out: move.outTransactionId,
            tx_in: move.inTransactionId,
            kind: pair.kind,
            policy: pair.policy,
            pair_source: pair.pair_source ?? "manual",
            ...(pair.notes ? { notes: pair.notes } : {}),
          });
          setToast({ kind: "linkedBack", spot: spotOf(move) });
          requestRefresh();
        } catch (cause) {
          setError(cause instanceof Error ? cause.message : t("transfersPage.toast.linkBackFailed"));
        }
      })();
      return;
    }
    setToast(null);
    setLinkStart({
      sendId: move.outTransactionId,
      receiptId: move.inTransactionId,
      relink: true,
      preset: {
        kind: pair?.kind ?? move.pairKind,
        policy: pair?.policy ?? move.pairPolicy,
        outAmountMsat: pair?.out_amount ?? null,
      },
    });
  };
  const noteProps = {
    // The core takes no link or unlink while it recalculates the journals.
    pending: undismiss.isPending || repair.isPending || unpair.isPending || isProcessingJournals,
    // What the new link leaves of its send, once the pair list names it.
    rest: toast?.kind === "linked" ? restOfPair(pairs, parts, toast.pairId) : null,
    onUndoLink: (next: Extract<Toast, { kind: "linked" }>) => (next.pairId ? void undoLink(next.pairId, next.spot) : undefined),
    // The note stays until its way back worked, so a refusal loses nothing.
    onLinkBack: (next: Extract<Toast, { kind: "unlinked" }>) => void linkBack(next.dismissalId, next.move),
    onLinkAgain: undoUnpair,
    onLinkDifferently: (transactionId: string) => {
      setToast(null);
      openLink(transactionId);
    },
    // The note stays with its Undo: the next link replaces it once it is made.
    onLinkRest: (transactionId: string) => openLink(transactionId),
    onClose: () => setToast(null),
  };
  const onLinked = ({ pairId, sendId, receiptId }: LinkedResult) => {
    // The link is made: its screen closes and the note offers the way back.
    // The list in view stays, though the link may add to what needs the owner.
    setTab(activeTab);
    setLinkStart(null);
    setToast({ kind: "linked", pairId, sendId, spot: { outId: sendId, inId: receiptId } });
    requestRefresh();
  };
  const suggestionFilter = savedView ? viewFilter(savedView) : undefined;
  const noSuggestions = !suggestQuery.isLoading && !suggestQuery.isError && candidates.length === 0;

  return (
    <div className="space-y-(--kb-page-gap)">
      <TransfersHeader
        verdict={verdict}
        tab={activeTab}
        counts={tabCounts}
        onTabChange={chooseTab}
        updating={updating}
        developerToolsEnabled={developerToolsEnabled}
        onRecalculate={requestRefresh}
        onRetry={retryReads}
        onOpen={onOpen}
        onOpenRules={() => setRulesOpen(true)}
        onLinkManually={() => openLink()}
      />

      <TransfersStats
        loading={verdict.state === "loading"}
        linked={linkedTotal}
        counts={counts}
        moves={moves}
        onShowList={showList}
      />

      <p role="alert" className="text-sm text-destructive empty:hidden">{error ?? null}</p>

      {/* One card per status, chosen by tab: what needs the owner opens first
          when there is any, otherwise the linked transfers. */}
      <div ref={listRef} className="scroll-mt-4 space-y-3">
        <div
          ref={contentRef}
          className={cn(
            (panelMove || linkPanel) && "grid items-start gap-(--kb-page-gap)",
            panelMove && "grid-cols-[minmax(0,1fr)_22rem]",
            // Linking needs room for its list of candidates.
            linkPanel && "grid-cols-[minmax(0,1fr)_27rem]",
          )}
        >
          {/* The lists size themselves by this column, so they fit beside a panel. */}
          <div className="@container/list min-w-0 space-y-(--kb-page-gap)">
            {activeTab === "needs" ? (
              <>
                {heldUnavailable ? <HeldUnavailable incomplete={dismissalsIncomplete && !heldQuery.isError && !dismissalsQuery.isError} onRetry={retryReads} /> : null}
                {waiting ? (
                  <SortOutCard
                    headingRef={sortOutRef}
                    held={held}
                    gapHolds={gapHolds}
                    pairs={pairs ?? []}
                    parts={parts}
                    moves={moves}
                    onLinkRest={openLink}
                    onShowMove={(move) => openMove(move, "details")}
                    pending={undismiss.isPending || updating}
                    developerToolsEnabled={developerToolsEnabled}
                    onLinkBack={(dismissal) => void linkBack(dismissal.id)}
                    onLinkDifferently={(dismissal) => openLink(dismissal.out_transaction_id)}
                    onDescribe={() => onOpen({ section: "advanced", tab: "components" })}
                    onReviewGap={(gapId) => onOpen({ section: "advanced", tab: "gaps", ...(gapId ? { gap: gapId } : {}) })}
                  />
                ) : null}
                <div ref={suggestionsRef} className="scroll-mt-4 empty:hidden">
                  {noSuggestions && !savedView && !suggestionUndo ? (
                    waiting || heldUnavailable ? null : <SuggestionsEmpty updating={updating} onCheckAgain={requestRefresh} />
                  ) : (
                    <ReviewTransfers
                      onDecided={() => {
                        // The list in view stays, though the decision may empty it.
                        setTab("needs");
                        requestRefresh();
                        // Recalculating the journals does not read these again: a link
                        // brings its pair (and Unlink), a suggestion set aside its row.
                        void pairsQuery.refetch();
                        void dismissalsQuery.refetch();
                      }}
                      onUndoChange={setSuggestionUndo}
                      refreshing={updating}
                      framed
                      filter={suggestionFilter}
                      filterLabel={savedView?.name}
                      onClearFilter={() => onOpen({})}
                      onLinkManually={openLink}
                    />
                  )}
                </div>
              </>
            ) : activeTab === "unlinked" ? (
              <RecentlyUnlinkedCard
                rows={unlinked}
                pending={undismiss.isPending}
                headingRef={unlinkedRef}
                onLinkAgain={linkAgain}
                onLinkDifferently={(row) => openLink(row.outTransactionId)}
              />
            ) : activeTab === "setAside" ? (
              <SetAsideCard rows={setAside} pending={undismiss.isPending} defaultOpen onRestore={(row) => void restoreSuggestion(row)} />
            ) : (
              <LinkedMovesCard
                moves={moves}
                updating={updating}
                counts={counts}
                filter={filter}
                year={year}
                onFilterChange={setFilter}
                onYearChange={setYear}
                loading={lineageQuery.isLoading || pairsQuery.isLoading}
                failed={lineageQuery.isError && pairsQuery.isError}
                total={linkedTotal}
                hasMore={Boolean(lineageQuery.hasNextPage)}
                loadingMore={lineageQuery.isFetchingNextPage}
                onLoadMore={() => void lineageQuery.fetchNextPage()}
                selectedKey={panelMove?.key ?? null}
                parts={parts}
                strip={strip?.spot ? { spot: strip.spot, node: <TransfersNote toast={strip} compact {...noteProps} /> } : null}
                onOpen={(move) => openMove(move, "details")}
                onUnlink={(move) => openMove(move, "unlink")}
                onLinkManually={() => openLink()}
              />
            )}
          </div>
          {panelMove ? (
            <div ref={setPanelElement} className="sticky top-4">
              <UnlinkDialog
                variant="panel"
                move={panelMove}
                intent={moveIntent}
                onRequestUnlink={() => setMoveIntent("unlink")}
                onLinkRest={openLink}
                onClose={() => setPendingUnlink(null)}
                onUnlinked={onUnlinked}
              />
            </div>
          ) : linkPanel ? (
            // The link's one tree renders into this place while the page is wide enough.
            <div ref={setPanelElement} className="sticky top-4" data-testid="link-panel-slot" />
          ) : null}
        </div>
      </div>

      <UnlinkDialog
        move={panelMove ? null : pendingUnlink}
        intent={moveIntent}
        onRequestUnlink={() => setMoveIntent("unlink")}
        // A link in progress in the panel is not replaced from a transfer opened over it.
        onLinkRest={linkStart ? undefined : openLink}
        onClose={() => setPendingUnlink(null)}
        onUnlinked={onUnlinked}
      />
      {/* One tree for the link, in the panel or as a dialog, so its progress survives the page changing width. */}
      <LinkManuallyDialog
        start={linkStart}
        variant={wide ? "panel" : "dialog"}
        panelSlot={linkPanel ? panelElement : null}
        updating={updating}
        onClose={() => setLinkStart(null)}
        onLinked={onLinked}
      />
      <RulesAndViewsSheet
        open={rulesOpen}
        onOpenChange={setRulesOpen}
        onOpenView={(view) => {
          setRulesOpen(false);
          // Also when the view is the one already open, so its URL does not change.
          chooseTab("needs");
          onOpen({ savedView: view.id });
          suggestionsRef.current?.scrollIntoView({ block: "start", behavior: "smooth" });
        }}
      />
      {/* A note the Linked list shows at its row is not repeated here. */}
      <div role="status" aria-live="polite" className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex justify-center px-4">
        {/* A strip is read out from here too: this region is always there, the strip's row is not. */}
        {strip ? <span className="sr-only">{noteText(t, strip, noteProps.rest, true)}</span> : null}
        {toast && !strip ? (
          <div className="pointer-events-auto max-w-2xl rounded-lg border bg-popover px-4 py-2.5 text-popover-foreground shadow-lg" data-testid="transfers-toast">
            <TransfersNote toast={toast} {...noteProps} />
          </div>
        ) : null}
      </div>
    </div>
  );
}

/**
 * The suggestions card when there is nothing to confirm: what Kassiber
 * checks on its own, and the way to link a move it did not find.
 */
/** The page's lists, one per status. */
type StatusTab = "needs" | "linked" | "unlinked" | "setAside";

/** Content this wide or wider keeps a transfer's panel beside the table. */
const PANEL_MIN_WIDTH = 1100;

/**
 * Says how much of the window is left under an element's top edge, as
 * `--kb-panel-room` on it, kept current while the page scrolls or resizes.
 */
function useRoomBelow(element: HTMLElement | null) {
  React.useLayoutEffect(() => {
    if (!element || typeof window === "undefined") return;
    const fit = () => {
      const top = Math.max(element.getBoundingClientRect().top, 0);
      element.style.setProperty("--kb-panel-room", `${Math.max(320, window.innerHeight - top - 16)}px`);
    };
    fit();
    // Whichever element scrolls the page: scroll events do not bubble, so capture them.
    window.addEventListener("scroll", fit, true);
    window.addEventListener("resize", fit);
    return () => {
      window.removeEventListener("scroll", fit, true);
      window.removeEventListener("resize", fit);
    };
  }, [element]);
}

/** An element's width, kept current as it resizes. */
function useElementWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = React.useState(0);
  React.useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    setWidth(element.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

/**
 * The status tabs: needs you, linked, unlinked by you, set aside. A list
 * with nothing in it keeps no tab, except the first two.
 */
function StatusTabs({
  tab,
  counts,
  onTabChange,
}: {
  tab: StatusTab;
  counts: Record<StatusTab, number>;
  onTabChange: (tab: StatusTab) => void;
}) {
  const { t } = useTranslation("review");
  const tabs = (["needs", "linked", "unlinked", "setAside"] as const).filter(
    (key) => key === "needs" || key === "linked" || counts[key] > 0,
  );
  // Drawn like the period selector on Transactions: one track, the chosen
  // segment raised.
  return (
    <div
      className="flex h-8 max-w-full min-w-0 items-center gap-1 overflow-x-auto rounded-lg bg-muted p-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      role="group"
      aria-label={t("transfersPage.tabs.label")}
      data-testid="transfers-tabs"
    >
      {tabs.map((key) => {
        const active = tab === key;
        const attention = key === "needs" && counts.needs > 0;
        return (
          <button
            key={key}
            type="button"
            aria-pressed={active}
            onClick={() => onTabChange(key)}
            className={cn(
              "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium whitespace-nowrap transition-all @4xl/page:px-3 @4xl/page:text-sm",
              active ? "bg-card text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {t(`transfersPage.tabs.${key}`)}
            <span
              className={cn(
                "rounded-full px-1.5 text-2xs tabular-nums",
                attention ? "bg-amber-500/20 text-amber-800 dark:text-amber-200" : active ? "bg-muted" : "",
              )}
            >
              {formatCount(counts[key])}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** The holds could not be read: what waits on the owner is unknown, not nothing. */
function HeldUnavailable({ incomplete, onRetry }: { /** Read, but only in part: reading again does not help. */ incomplete: boolean; onRetry: () => void }) {
  const { t } = useTranslation("review");
  return (
    <section className="kb-surface overflow-hidden" role="alert" data-testid="transfers-held-unavailable">
      <div className="flex min-w-0 items-center gap-2 px-3 py-2.5 text-sm sm:px-6">
        <AlertTriangle className="size-4 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
        <span className="min-w-0">{t(incomplete ? "transfersPage.sortOut.incomplete" : "transfersPage.sortOut.unavailable")}</span>
        {incomplete ? null : (
          <Button type="button" size="sm" variant="ghost" className="ml-auto h-7 shrink-0 gap-1.5 text-xs" onClick={onRetry}>
            <RefreshCw className="size-3.5" aria-hidden="true" />
            {t("transfersPage.verdict.retry")}
          </Button>
        )}
      </div>
    </section>
  );
}

/**
 * Nothing to confirm, in one line: Link manually sits in the page header, and
 * what Kassiber checks waits behind the info icon.
 */
function SuggestionsEmpty({ updating, onCheckAgain }: { updating: boolean; onCheckAgain: () => void }) {
  const { t } = useTranslation("review");
  return (
    <section className="kb-surface overflow-hidden" aria-label={t("inbox.transfers.title")} data-testid="transfers-suggestions-empty">
      <div className="flex min-w-0 items-center gap-2 px-3 py-2.5 text-sm sm:px-6">
        <CheckCircle2 className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
        <span className="shrink-0 font-medium">{t("transfersPage.empty.suggestionsTitle")}</span>
        <span className="min-w-0 truncate text-muted-foreground" title={t("transfersPage.empty.suggestionsMissed")}>
          {t("transfersPage.empty.suggestionsMissed")}
        </span>
        <RowHint label={t("transfersPage.empty.suggestionsChecks")} className="text-muted-foreground">
          <Info className="size-3.5" aria-hidden="true" />
        </RowHint>
        {/* Matching runs with the journals: this runs both again. */}
        <Button type="button" size="sm" variant="ghost" className="ml-auto h-7 shrink-0 gap-1.5 text-xs" disabled={updating} onClick={onCheckAgain}>
          <RefreshCw className={cn("size-3.5", updating && "animate-spin")} aria-hidden="true" />
          {t("transfersPage.empty.checkAgain")}
        </Button>
      </div>
    </section>
  );
}

/**
 * The page's first row, as on Overview: the verdict as a status pill with
 * one line of detail, and the page's actions on the right.
 */
/**
 * The page's first row, as on Transactions: the status selector on the left
 * (what needs the owner, linked, unlinked, set aside, each with its count),
 * the actions on the right. Stale journals add the button that recalculates
 * them; the verdict itself is read out for screen readers.
 */
function TransfersHeader({
  verdict,
  tab,
  counts,
  onTabChange,
  updating,
  developerToolsEnabled,
  onRecalculate,
  onRetry,
  onOpen,
  onOpenRules,
  onLinkManually,
}: {
  verdict: TransfersVerdict;
  tab: StatusTab;
  counts: Record<StatusTab, number>;
  onTabChange: (tab: StatusTab) => void;
  updating: boolean;
  developerToolsEnabled: boolean;
  onRecalculate: () => void;
  /** Reads again what could not be read. */
  onRetry: () => void;
  onOpen: (next: TransfersCustodySearch) => void;
  onOpenRules: () => void;
  onLinkManually: () => void;
}) {
  const { t } = useTranslation("review");
  const summary =
    verdict.state === "loading"
      ? t("transfersPage.verdict.loading")
      : verdict.state === "stale"
        ? t("transfersPage.verdict.staleBody")
        : verdict.state === "unavailable"
          ? t("transfersPage.verdict.unavailableBody")
          : verdict.state === "sortOut"
            ? t("transfersPage.verdict.sortOutTitle", { count: verdict.total })
            : verdict.state === "settled"
              ? t("transfersPage.verdict.settledTitle")
              : t("transfersPage.verdict.emptyTitle");
  return (
    <div className={pageHeaderClassName} data-testid="transfers-verdict" data-state={verdict.state}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {verdict.state === "loading" ? <Skeleton className="h-8 w-72 rounded-lg" /> : <StatusTabs tab={tab} counts={counts} onTabChange={onTabChange} />}
        <span className="sr-only" role="status">
          {summary}
        </span>
        {verdict.state === "stale" ? (
          <ReadinessPill
            tone="warning"
            icon={RefreshCw}
            title={t("transfersPage.verdict.recalculate")}
            description={t("transfersPage.verdict.staleBody")}
            onClick={onRecalculate}
            busy={updating}
            busyTitle={t("transfersPage.verdict.updating")}
          />
        ) : verdict.state === "unavailable" ? (
          <ReadinessPill
            tone="warning"
            icon={AlertTriangle}
            title={t("transfersPage.verdict.unavailableTitle")}
            description={t("transfersPage.verdict.unavailableBody")}
            onClick={onRetry}
          />
        ) : updating ? (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground" role="status">
            <Loader2 className="size-3 animate-spin" aria-hidden="true" />
            {t("transfersPage.verdict.updating")}
          </span>
        ) : (
          <span className="min-w-0 text-xs text-muted-foreground">
            <Caveat />
          </span>
        )}
      </div>
      {/* The actions stay on one line, so the menu never wraps under its button. */}
      <div className={cn(pageHeaderActionsClassName, "shrink-0 flex-nowrap")}>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className={pageHeaderActionClassName}
          onClick={onLinkManually}
        >
          <Link2 className="size-4" aria-hidden="true" />
          {t("transfersPage.linkManually")}
        </Button>
        <TransfersMenu
          developerToolsEnabled={developerToolsEnabled}
          updating={updating}
          onRecheck={onRecalculate}
          onOpen={onOpen}
          onOpenRules={onOpenRules}
        />
      </div>
    </div>
  );
}

function TransfersMenu({
  developerToolsEnabled,
  updating,
  onRecheck,
  onOpen,
  onOpenRules,
}: {
  developerToolsEnabled: boolean;
  updating: boolean;
  /** Recalculates the journals, which looks for transfers again. */
  onRecheck: () => void;
  onOpen: (next: TransfersCustodySearch) => void;
  onOpenRules: () => void;
}) {
  const { t } = useTranslation(["review", "nav"]);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="icon"
          className={pageHeaderIconButtonClassName}
          aria-label={t("review:transfersPage.menu.label")}
        >
          <MoreHorizontal className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuItem disabled={updating} onSelect={onRecheck}>
          <RefreshCw className="size-4" aria-hidden="true" />
          <span className="flex flex-col">
            <span>{t("review:transfersPage.menu.recheck")}</span>
            <span className="text-2xs text-muted-foreground">{t("review:transfersPage.menu.recheckHint")}</span>
          </span>
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={onOpenRules}>
          <Settings2 className="size-4" aria-hidden="true" />
          <span className="flex flex-col">
            <span>{t("review:transfersPage.menu.rules")}</span>
            <span className="text-2xs text-muted-foreground">{t("review:transfersPage.menu.rulesHint")}</span>
          </span>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={!developerToolsEnabled}
          onSelect={() => onOpen({ section: "advanced" })}
          title={developerToolsEnabled ? undefined : t("nav:devLocked")}
        >
          <Wrench className="size-4" aria-hidden="true" />
          <span className="flex flex-col">
            <span className="flex items-center gap-2">
              {t("review:transfersPage.menu.advanced")}
              <DeveloperBadge />
            </span>
            {developerToolsEnabled ? (
              <span className="text-2xs text-muted-foreground">{t("review:transfersPage.menu.advancedHint")}</span>
            ) : (
              <span className="text-2xs text-muted-foreground">{t("review:transfersPage.menu.advancedLocked")}</span>
            )}
          </span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The one caveat the screen makes: it can only check wallets it knows. */
function Caveat() {
  const { t } = useTranslation("review");
  return (
    <span className="inline-flex items-center gap-1">
      {t("transfersPage.verdict.caveat")}
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="inline-grid size-4 place-items-center rounded-full text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("transfersPage.verdict.caveatWhyLabel")}
          >
            <CircleHelp className="size-3.5" aria-hidden="true" />
          </button>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs">{t("transfersPage.verdict.caveatWhy")}</TooltipContent>
      </Tooltip>
    </span>
  );
}

/** Bitcoin on both sides: the moves whose amounts add up to "moved". */
function movedTotals(moves: readonly LinkedMove[]) {
  const linked = moves.filter((move) => move.source !== "undone" && !move.differentAsset && isBitcoinAsset(move.asset));
  const priced = linked.filter((move) => move.amountFiat !== null);
  return {
    amount: sumMsat(linked.map((move) => move.amountMsat)),
    fiat: priced.reduce((sum, move) => sum + (move.amountFiat ?? 0), 0),
    unpriced: linked.length - priced.length,
    count: linked.length,
  };
}

/** The stat strip, drawn like the strips on Transactions and Wallets. */
function TransfersStats({
  loading,
  linked,
  counts,
  moves,
  onShowList,
}: {
  loading: boolean;
  linked: number;
  counts: ReturnType<typeof countLinkedMoves>;
  moves: readonly LinkedMove[];
  onShowList: (filter: LinkedMoveFilter) => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const totals = movedTotals(moves);
  const tiles: React.ReactNode[] = [
    <StatTile
      key="linked"
      loading={loading}
      icon={ArrowLeftRight}
      tone="text-muted-foreground"
      label={t("transfersPage.stats.linked")}
      value={formatCount(linked)}
      meta={t("transfersPage.stats.linkedMeta", { automatic: counts.automatic, reviewed: counts.reviewed })}
      onClick={() => onShowList("all")}
      ariaLabel={t("transfersPage.stats.linkedAria")}
    />,
    <StatTile
      key="moved"
      loading={loading}
      icon={Bitcoin}
      tone="text-orange-500"
      label={t("transfersPage.stats.moved")}
      value={
        <span className={blurClass(hideSensitive)}>
          {currency === "btc" ? formatMsatAsBtc(totals.amount) : formatDisplayMoney(totals.fiat, 0, currency)}
        </span>
      }
      meta={
        currency !== "btc" && totals.unpriced
          ? t("transfersPage.stats.movedMetaUnpriced", { count: totals.count, unpriced: totals.unpriced })
          : t("transfersPage.stats.movedMeta", { count: totals.count })
      }
    />,
  ];
  // What waits on the owner and what they unlinked are counted on the tabs below.
  return (
    <section
      className={cn(
        "kb-surface relative grid overflow-hidden",
        tiles.length === 2 ? "grid-cols-2" : tiles.length === 3 ? "grid-cols-1 @2xl/page:grid-cols-3" : "grid-cols-2 @4xl/page:grid-cols-4",
      )}
      aria-label={t("transfersPage.stats.label")}
    >
      {tiles.map((tile, index) => (
        <div
          key={index}
          className={cn(
            "min-w-0 [&>*]:h-full",
            index > 0 && "border-l",
            tiles.length === 3 && index > 0 && "max-@2xl/page:border-t max-@2xl/page:border-l-0",
            tiles.length === 4 && index === 2 && "max-@4xl/page:border-l-0",
            tiles.length === 4 && index < 2 && "max-@4xl/page:border-b",
          )}
        >
          {tile}
        </div>
      ))}
    </section>
  );
}

function RouteSide({ label, rail, strong = false }: { label: string; rail: SwapRail; strong?: boolean }) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [head, tail] = splitWalletName(label);
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <RailIcon rail={rail} size="compact" />
      {/* Cut in the middle: wallets often differ only in their last words. */}
      <span
        className={cn("flex min-w-0 overflow-hidden text-sm", strong && "font-medium text-foreground", blurClass(hideSensitive))}
        title={hideSensitive ? undefined : label}
      >
        {/* The start keeps a few letters; the end gives way only after it. */}
        <span className={cn("truncate", tail && "min-w-[2.5em]")}>{head}</span>
        {tail ? <span className="min-w-0 shrink-[0.01] truncate whitespace-pre">{tail}</span> : null}
      </span>
    </span>
  );
}

function Route({ from, to, fromRail, toRail }: { from: string; to: string; fromRail: SwapRail; toRail: SwapRail }) {
  const { t } = useTranslation("review");
  return (
    <span className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2">
      <RouteSide label={from} rail={fromRail} strong />
      <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("inbox.transfers.to")} />
      <RouteSide label={to} rail={toRail} strong />
    </span>
  );
}


/**
 * What waits on the owner, asked once: a move the owner unlinked that the
 * journals now hold, as the question it leaves open, and any hold a
 * missing-wallet review explains.
 */
function SortOutCard({
  headingRef,
  held,
  gapHolds,
  pairs,
  parts,
  moves,
  onLinkRest,
  onShowMove,
  pending,
  developerToolsEnabled,
  onLinkBack,
  onLinkDifferently,
  onDescribe,
  onReviewGap,
}: {
  headingRef: React.RefObject<HTMLHeadingElement | null>;
  held: TransferDismissal[];
  gapHolds: QuarantineItem[];
  /** The owner's links, to tell a hold that is the unlinked rest of one. */
  pairs: readonly TransferPairSummary[];
  parts: Record<LegSide, Map<string, OpenPart>> | null;
  moves: readonly LinkedMove[];
  onLinkRest: (transactionId: string) => void;
  onShowMove: (move: LinkedMove) => void;
  pending: boolean;
  developerToolsEnabled: boolean;
  onLinkBack: (dismissal: TransferDismissal) => void;
  onLinkDifferently: (dismissal: TransferDismissal) => void;
  onDescribe: () => void;
  onReviewGap: (gapId: string | null) => void;
}) {
  const { t } = useTranslation("review");
  const count = held.length + gapHolds.length;
  // A hold that is the part of a linked send no receipt covers yet: its
  // answer is the link for the rest, which anyone can make.
  const restOf = (item: QuarantineItem) => {
    const found = restHoldPair(item, pairs, parts);
    return found ? { ...found, move: moves.find((move) => move.pairId === found.pair.id) ?? null } : null;
  };
  const gapOf = (item: QuarantineItem) => item.actions?.find((action) => action.kind === "review_custody_gap") ?? null;
  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="transfers-sort-out-title" data-testid="transfers-sort-out">
      <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <h2
            id="transfers-sort-out-title"
            ref={headingRef}
            tabIndex={-1}
            className="rounded-sm text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:text-base"
          >
            {t("transfersPage.sortOut.title")}
          </h2>
          <span className={surfaceCountClassName}>{count}</span>
        </div>
        <p className="text-xs text-muted-foreground sm:text-sm">{t("transfersPage.sortOut.reports")}</p>
      </div>
      <ul className="divide-y border-t">
        {held.map((dismissal) => (
          <HeldQuestion
            key={dismissal.id}
            dismissal={dismissal}
            pending={pending}
            developerToolsEnabled={developerToolsEnabled}
            onLinkBack={() => onLinkBack(dismissal)}
            onLinkDifferently={() => onLinkDifferently(dismissal)}
            onDescribe={onDescribe}
          />
        ))}
        {gapHolds.map((item) => {
          const rest = restOf(item);
          if (!rest) return null;
          return (
            <RestHold
              key={item.transaction_id}
              pair={rest.pair}
              rest={rest.rest}
              onLinkRest={() => onLinkRest(rest.pair.out_transaction_id)}
              onShow={rest.move ? () => onShowMove(rest.move as LinkedMove) : undefined}
              developerToolsEnabled={developerToolsEnabled}
              // The rest may have gone somewhere else: that review stays where it was.
              onReview={gapOf(item) ? () => onReviewGap(gapOf(item)?.gap_id ?? null) : undefined}
            />
          );
        })}
        {gapHolds.filter((item) => !restOf(item)).map((item) => (
          <GapHold
            key={item.transaction_id}
            item={item}
            developerToolsEnabled={developerToolsEnabled}
            onReview={() =>
              onReviewGap(item.actions?.find((action) => action.kind === "review_custody_gap")?.gap_id ?? null)
            }
          />
        ))}
      </ul>
    </section>
  );
}

function HeldQuestion({
  dismissal,
  pending,
  developerToolsEnabled,
  onLinkBack,
  onLinkDifferently,
  onDescribe,
}: {
  dismissal: TransferDismissal;
  pending: boolean;
  developerToolsEnabled: boolean;
  onLinkBack: () => void;
  onLinkDifferently: () => void;
  onDescribe: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const amount = formatMsatIn(dismissal.out.amount_msat, dismissal.out.asset);
  return (
    <li className="px-3 py-4 sm:px-6" data-testid="transfers-held-move">
      <div className="flex items-start gap-3">
        <span
          className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-amber-600/20 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
          aria-hidden="true"
        >
          <CircleHelp className="size-4" />
        </span>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="space-y-1">
            <p className="text-sm font-medium text-foreground">
              <span className={blurClass(hideSensitive)}>
                {t("transfersPage.held.question", { amount, from: dismissal.out.wallet })}
              </span>
            </p>
            <p className="text-sm text-muted-foreground">
              <span className={blurClass(hideSensitive)}>
                {t("transfersPage.held.why", { to: dismissal.in.wallet })}
              </span>{" "}
              <span className="text-foreground">{t("transfersPage.held.ask")}</span>
            </p>
            <p className="flex flex-wrap items-center gap-x-1.5 text-2xs text-muted-foreground sm:text-xs">
              <span>{t("transfersPage.held.sent", { date: datePart(dismissal.out.occurred_at) })}</span>
              <span aria-hidden="true">·</span>
              <span>{t("transfersPage.held.unlinkedOn", { date: datePart(dismissal.created_at) })}</span>
            </p>
          </div>
          <div className="flex flex-wrap gap-2" role="group" aria-label={t("transfersPage.held.answers")}>
            <Button type="button" size="sm" className="h-8 gap-2" disabled={pending} onClick={onLinkBack}>
              <Link2 className="size-4" aria-hidden="true" />
              <span className={blurClass(hideSensitive)}>
                {t("transfersPage.held.linkBack", { to: dismissal.in.wallet })}
              </span>
            </Button>
            <Button type="button" size="sm" variant="outline" className="h-8 gap-2" disabled={pending} onClick={onLinkDifferently}>
              <ArrowLeftRight className="size-4" aria-hidden="true" />
              {t("transfersPage.held.linkDifferently")}
            </Button>
            {developerToolsEnabled ? (
              <Button type="button" size="sm" variant="ghost" className="h-8 gap-2" onClick={onDescribe}>
                <Wrench className="size-4" aria-hidden="true" />
                {t("transfersPage.held.describe")}
                <DeveloperBadge />
              </Button>
            ) : null}
          </div>
        </div>
        <RowMenu
          route={t("inbox.transfers.route", { from: dismissal.out.wallet, to: dismissal.in.wallet })}
          outId={dismissal.out_transaction_id}
          inId={dismissal.in_transaction_id}
        />
      </div>
    </li>
  );
}

/**
 * The part of a linked send that no receipt covers yet, as the question it
 * leaves open. The core allows one answer anyone can give: link the receipt
 * the rest arrived in (a further link of the same send). Saying the rest was
 * a fee or went to someone else stays with the developer-gated tools.
 */
function RestHold({
  pair,
  rest,
  onLinkRest,
  onShow,
  developerToolsEnabled,
  onReview,
}: {
  pair: TransferPairSummary;
  rest: string;
  onLinkRest: () => void;
  onShow?: () => void;
  developerToolsEnabled: boolean;
  /** Opens the missing-wallet review of this hold, when it has one. */
  onReview?: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const route = t("inbox.transfers.route", { from: pair.out.wallet, to: pair.in.wallet });
  return (
    <li className="px-3 py-4 sm:px-6" data-testid="transfers-rest-hold">
      <div className="flex items-start gap-3">
        <span
          className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-amber-600/20 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
          aria-hidden="true"
        >
          <Split className="size-4" />
        </span>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="space-y-1">
            <p className="text-sm font-medium text-foreground">
              <span className={blurClass(hideSensitive)}>
                {t("transfersPage.restHold.title", { rest: formatMsatAsBtc(rest), from: pair.out.wallet })}
              </span>
            </p>
            <p className="text-sm text-muted-foreground">
              {t("transfersPage.restHold.body")} <span className="text-foreground">{t("transfersPage.restHold.ask")}</span>
            </p>
            <p className="text-2xs text-muted-foreground sm:text-xs">{t("transfersPage.held.sent", { date: datePart(pair.out.occurred_at) })}</p>
          </div>
          <div className="flex flex-wrap gap-2" role="group" aria-label={t("transfersPage.held.answers")}>
            <Button type="button" size="sm" className="h-8 gap-2" onClick={onLinkRest}>
              <Link2 className="size-4" aria-hidden="true" />
              {t("transfersPage.restHold.linkRest")}
            </Button>
            {onShow ? (
              <Button type="button" size="sm" variant="outline" className="h-8 gap-2" onClick={onShow}>
                <Info className="size-4" aria-hidden="true" />
                {t("transfersPage.restHold.show")}
              </Button>
            ) : null}
            {developerToolsEnabled && onReview ? (
              <Button type="button" size="sm" variant="ghost" className="h-8 gap-2" onClick={onReview}>
                <Wrench className="size-4" aria-hidden="true" />
                {t("transfersPage.gapHold.review")}
                <DeveloperBadge />
              </Button>
            ) : null}
          </div>
          {/* Another receipt is one answer, not the only one. */}
          {developerToolsEnabled ? null : <p className="text-xs text-muted-foreground">{t("transfersPage.restHold.locked")}</p>}
        </div>
        <RowMenu route={route} outId={pair.out_transaction_id} inId={pair.in_transaction_id} />
      </div>
    </li>
  );
}

/**
 * A hold a missing-wallet review explains. Everyone reads why; resolving it
 * stays with the developer-gated advanced tools.
 */
function GapHold({
  item,
  developerToolsEnabled,
  onReview,
}: {
  item: QuarantineItem;
  developerToolsEnabled: boolean;
  onReview: () => void;
}) {
  const { t } = useTranslation(["review", "nav"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  return (
    <li className="px-3 py-4 sm:px-6" data-testid="transfers-gap-hold">
      <div className="flex items-start gap-3">
        <span
          className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-amber-600/20 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
          aria-hidden="true"
        >
          <CircleHelp className="size-4" />
        </span>
        <div className="min-w-0 flex-1 space-y-3">
          <div className="space-y-1">
            <p className="text-sm font-medium text-foreground">
              <span className={blurClass(hideSensitive)}>
                {t("review:transfersPage.gapHold.title", {
                  amount: formatMsatAsBtc(item.amount_msat),
                  wallet: item.wallet,
                })}
              </span>
            </p>
            <p className="text-sm text-muted-foreground">{t("review:transfersPage.gapHold.body")}</p>
            <p className="text-2xs text-muted-foreground sm:text-xs">
              {t("review:transfersPage.held.sent", { date: datePart(item.occurred_at) })}
            </p>
          </div>
          {developerToolsEnabled ? (
            <Button type="button" size="sm" variant="outline" className="h-8 gap-2" onClick={onReview}>
              <Wrench className="size-4" aria-hidden="true" />
              {t("review:transfersPage.gapHold.review")}
              <DeveloperBadge />
            </Button>
          ) : (
            <p className="text-xs text-muted-foreground">{t("review:transfersPage.gapHold.locked")}</p>
          )}
        </div>
      </div>
    </li>
  );
}

/** Chips for how a move was linked, in the Transactions status-chip style. */
const SOURCE_CHIP: Record<LinkedMoveSource, string> = {
  automatic:
    "bg-blue-50 text-blue-700 ring-1 ring-inset ring-blue-700/10 dark:bg-blue-900/30 dark:text-blue-400 dark:ring-blue-400/20",
  reviewed:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  undone:
    "bg-zinc-50 text-zinc-700 ring-1 ring-inset ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

const SOURCE_ICON: Record<LinkedMoveSource, typeof Link2> = {
  automatic: Sparkles,
  reviewed: UserCheck,
  undone: Unlink,
};

/** Reasons that only repeat the "Linked by you" chip. */
const PLAIN_PAIR_EVIDENCE = new Set(["manual", "reviewed_transfer_pair", "bulk_exact", "bulk_selected", "rule_auto"]);

const FILTERS: LinkedMoveFilter[] = ["all", "automatic", "reviewed", "differentAsset"];

function LinkedMovesCard({
  moves,
  counts,
  filter,
  year,
  onFilterChange,
  onYearChange,
  loading,
  failed,
  total,
  hasMore,
  loadingMore,
  onLoadMore,
  onOpen,
  onUnlink,
  updating,
  selectedKey = null,
  parts = null,
  strip = null,
  onLinkManually,
}: {
  moves: LinkedMove[];
  /** The transfer open in the panel, marked in the list. */
  selectedKey?: string | null;
  /** What each linked transaction has left open, to mark a send linked in part. */
  parts?: Record<LegSide, Map<string, OpenPart>> | null;
  /** A note about one transfer, shown at its row or where that row was. */
  strip?: { spot: Spot; node: React.ReactNode } | null;
  counts: ReturnType<typeof countLinkedMoves>;
  filter: LinkedMoveFilter;
  year: number | null;
  onFilterChange: (filter: LinkedMoveFilter) => void;
  onYearChange: (year: number | null) => void;
  loading: boolean;
  failed: boolean;
  total: number;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  /** Opens the unlink dialog, which also explains a link that cannot be undone. */
  onOpen: (move: LinkedMove) => void;
  onUnlink: (move: LinkedMove) => void;
  updating: boolean;
  onLinkManually: () => void;
}) {
  const { t } = useTranslation("review");
  const [shown, setShown] = React.useState(LINKED_PAGE);
  React.useEffect(() => setShown(LINKED_PAGE), [filter, year]);
  const years = moveYears(moves.filter((move) => move.source !== "undone"));
  const visible = filterLinkedMoves(moves, filter).filter((move) => year === null || moveYear(move) === year);
  const page = visible.slice(0, shown);
  const activeFilters = (filter !== "all" ? 1 : 0) + (year !== null ? 1 : 0);
  // The strip sits under its transfer's row; once that row is gone (or
  // before it is listed) it keeps the place the row had, or the top.
  const stripRow = strip ? page.findIndex((move) => move.outTransactionId === strip.spot.outId && move.inTransactionId === strip.spot.inId) : -1;
  const [lastPlace, setLastPlace] = React.useState<number | null>(null);
  React.useEffect(() => {
    if (!strip) setLastPlace(null);
    else if (stripRow >= 0) setLastPlace(stripRow);
  }, [strip, stripRow]);
  const stripAt = !strip ? -1 : stripRow >= 0 ? stripRow + 1 : Math.min(lastPlace ?? 0, page.length);
  const stripLine = strip ? (
    <tr key="strip" className="border-b bg-muted/40" data-testid="transfers-strip">
      <td colSpan={7} className="py-1.5 whitespace-normal">
        {strip.node}
      </td>
    </tr>
  ) : null;
  const filterCount = (key: LinkedMoveFilter) =>
    key === "all" ? counts.automatic + counts.reviewed : counts[key];
  const filtered = activeFilters > 0;
  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="transfers-linked-title">
      <SurfaceCardHeader
        title={t("transfersPage.linked.title")}
        titleId="transfers-linked-title"
        count={filtered ? visible.length : null}
        actions={
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn("h-8 gap-1.5 sm:h-9 sm:gap-2", activeFilters > 0 && "border-primary")}
                aria-label={t("transfersPage.linked.filter.label")}
              >
                <SlidersHorizontal className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">{t("transfersPage.linked.filter.label")}</span>
                {activeFilters > 0 ? (
                  <span className="grid min-w-4 place-items-center rounded-full bg-primary px-1 text-2xs font-semibold leading-4 text-primary-foreground">
                    {activeFilters}
                  </span>
                ) : null}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[240px]">
              <DropdownMenuLabel>{t("transfersPage.linked.filter.how")}</DropdownMenuLabel>
              <DropdownMenuRadioGroup value={filter} onValueChange={(value) => onFilterChange(value as LinkedMoveFilter)}>
                {FILTERS.map((key) => (
                  <DropdownMenuRadioItem key={key} value={key} disabled={key !== "all" && filterCount(key) === 0}>
                    <span className="flex-1">{t(`transfersPage.linked.filter.${key}`)}</span>
                    <span className="text-xs text-muted-foreground tabular-nums">{filterCount(key)}</span>
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
              {years.length > 1 ? (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuLabel>{t("transfersPage.linked.filter.year")}</DropdownMenuLabel>
                  <DropdownMenuRadioGroup
                    value={year === null ? "all" : String(year)}
                    onValueChange={(value) => onYearChange(value === "all" ? null : Number(value))}
                  >
                    <DropdownMenuRadioItem value="all">{t("transfersPage.linked.filter.allYears")}</DropdownMenuRadioItem>
                    {years.map((option) => (
                      <DropdownMenuRadioItem key={option} value={String(option)}>
                        {option}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                </>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        }
      />
      {filtered ? (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
          <span className="text-2xs text-muted-foreground sm:text-xs">{t("transfersPage.linked.filter.active")}</span>
          {filter !== "all" ? (
            <button
              type="button"
              className={surfaceFilterChipClassName}
              onClick={() => onFilterChange("all")}
              aria-label={t("transfersPage.linked.filter.clear", { name: t(`transfersPage.linked.filter.${filter}`) })}
            >
              {t(`transfersPage.linked.filter.${filter}`)}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          ) : null}
          {year !== null ? (
            <button
              type="button"
              className={surfaceFilterChipClassName}
              onClick={() => onYearChange(null)}
              aria-label={t("transfersPage.linked.filter.clear", { name: String(year) })}
            >
              {year}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          ) : null}
        </div>
      ) : null}
      {/* Edge to edge, as the Transactions table: the header band and row
          lines run to the card's border, the first and last cells inset. */}
      <div className="overflow-x-auto bg-card [&_td:first-child]:pl-3 [&_td:last-child]:pr-3 sm:[&_td:first-child]:pl-6 sm:[&_td:last-child]:pr-6 [&_th:first-child]:pl-3 [&_th:last-child]:pr-3 sm:[&_th:first-child]:pl-6 sm:[&_th:last-child]:pr-6">
        {/* Narrow, the fee and the linked label give way first, so the type and the menu stay in view. */}
        <table data-slot="table" className="w-full min-w-[640px] table-fixed caption-bottom text-sm @6xl/list:min-w-[900px]" data-testid="transfers-linked-list">
          <colgroup>
            <col className="w-[104px] @6xl/list:w-[132px]" />
            <col />
            <col className="w-[120px]" />
            <col className="hidden w-[92px] @6xl/list:table-column" />
            <col className="w-[132px] @6xl/list:w-[150px]" />
            <col className="w-[140px] @6xl/list:w-[156px]" />
            <col className="w-[44px] sm:w-[52px]" />
          </colgroup>
          <TableHeader className="bg-card">
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">{t("transfersPage.linked.column.date")}</TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">{t("transfersPage.linked.column.route")}</TableHead>
              <TableHead className="text-right text-xs font-medium text-muted-foreground sm:text-sm">{t("transfersPage.linked.column.amount")}</TableHead>
              <TableHead className="hidden text-right text-xs font-medium text-muted-foreground sm:text-sm @6xl/list:table-cell">{t("transfersPage.linked.column.fee")}</TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">{t("transfersPage.linked.column.type")}</TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">{t("transfersPage.linked.column.how")}</TableHead>
              <TableHead>
                <span className="sr-only">{t("transfersPage.linked.column.actions")}</span>
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {loading ? (
              <TableRow>
                <TableCell colSpan={7} className="h-24 text-center text-sm text-muted-foreground" role="status">
                  <span className="inline-flex items-center gap-2">
                    <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                    {t("transfersPage.linked.loading")}
                  </span>
                </TableCell>
              </TableRow>
            ) : failed ? (
              <TableRow>
                <TableCell colSpan={7} className="h-24 text-center text-sm text-muted-foreground" role="alert">
                  {t("transfersPage.linked.unavailable")}
                </TableCell>
              </TableRow>
            ) : page.length === 0 && stripLine ? (
              stripLine
            ) : page.length === 0 ? (
              <TableRow>
                <TableCell colSpan={7} className="h-24 text-center text-sm whitespace-normal text-muted-foreground">
                  {filtered ? (
                    t("transfersPage.linked.emptyFilter")
                  ) : (
                    <span className="mx-auto flex max-w-xl flex-col items-center gap-2 py-2" data-testid="transfers-linked-empty">
                      <span className="font-medium text-foreground">{t("transfersPage.linked.empty")}</span>
                      <span>{t("transfersPage.empty.linkedBody")}</span>
                      <Button type="button" size="sm" variant="outline" className="h-8 gap-1.5" onClick={onLinkManually}>
                        <Link2 className="size-3.5" aria-hidden="true" />
                        {t("transfersPage.linkManually")}
                      </Button>
                    </span>
                  )}
                </TableCell>
              </TableRow>
            ) : (
              <>
                {page.map((move, index) => (
                  <React.Fragment key={move.key}>
                    {index === stripAt ? stripLine : null}
                    <LinkedMoveRow
                      move={move}
                      updating={updating}
                      selected={move.key === selectedKey}
                      rest={restOfMove(move, parts)}
                      onOpen={onOpen}
                      onUnlink={onUnlink}
                    />
                  </React.Fragment>
                ))}
                {stripAt >= page.length ? stripLine : null}
              </>
            )}
          </TableBody>
        </table>
        {/* Filters apply to the transfers loaded so far, so older ones stay a click away. */}
        {visible.length > shown || hasMore ? (
          <div className="flex items-center justify-center gap-3 pt-3 text-xs text-muted-foreground">
            <span>
              {filtered && visible.length <= shown
                ? t("transfersPage.linked.olderMayMatch")
                : t("transfersPage.linked.loaded", { loaded: page.length, total: Math.max(filtered ? visible.length : total, visible.length) })}
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={loadingMore}
              onClick={() => {
                if (visible.length > shown) setShown((count) => count + LINKED_PAGE);
                else onLoadMore();
              }}
            >
              {loadingMore ? t("transfersPage.linked.loadingMore") : t("transfersPage.linked.loadMore")}
            </Button>
          </div>
        ) : null}
      </div>
    </section>
  );
}

/** A row detail behind an icon: the text shows on hover or keyboard focus. */
function RowHint({ label, className, children }: { label: string; className?: string; children: React.ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          // The row opens its move; the hint only explains.
          onClick={(event) => event.stopPropagation()}
          className={cn(
            "inline-flex size-5 shrink-0 items-center justify-center rounded-sm hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
            className,
          )}
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-xs">
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

/** What a move the owner linked leaves of its send for a further link; null when nothing. */
function restOfMove(move: LinkedMove, parts: Record<LegSide, Map<string, OpenPart>> | null): bigint | null {
  const open = move.pairId ? parts?.send.get(move.outTransactionId) : undefined;
  return open && hasOpenPart({ linked: true, open }) ? open.unlinkedMsat : null;
}

function LinkedMoveRow({
  move,
  updating,
  selected = false,
  rest = null,
  onOpen,
  onUnlink,
}: {
  move: LinkedMove;
  /** Open in the panel beside the table. */
  selected?: boolean;
  /** What of its send no receipt covers yet. */
  rest?: bigint | null;
  /** The journals are recalculating. */
  updating: boolean;
  onOpen: (move: LinkedMove) => void;
  onUnlink: (move: LinkedMove) => void;
}) {
  const { t } = useTranslation(["review", "custodyGaps"]);
  const { t: tGaps } = useTranslation("custodyGaps");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const route = t("review:inbox.transfers.route", { from: move.fromLabel, to: move.toLabel });
  // Every row says why behind its info icon; a rule adds itself to the reason.
  const canUnlink = canUnlinkMove(move);
  // A link with nothing to unlink says so here, since the row has no menu.
  const evidence =
    move.source === "undone"
      ? t("review:transfersPage.linked.undoneReason")
      : !canUnlink
        ? t(`review:transfersPage.linked.fixed.${fixedLinkKind(move)}`)
        : move.lineage && !PLAIN_PAIR_EVIDENCE.has(move.lineage.evidence_reason)
        ? lineageEvidenceLabel(move.lineage, tGaps)
        : move.source === "reviewed" && !move.byRule
          ? t("review:transfersPage.linked.linkedByYouReason")
          : null;
  const ruleReason = move.byRule && move.source !== "undone" ? t("review:transfersPage.linked.byRuleReason") : null;
  // An automatic link shows only its icon, so its hint says it was automatic first.
  const automatic = move.source === "automatic" ? t("review:transfersPage.linked.automaticReason") : null;
  // Linked in part: the label says so and the hint says how much is left.
  const restReason = rest !== null && !hideSensitive ? t("review:transfersPage.linked.rest", { left: formatMsatIn(rest.toString(), move.asset) }) : null;
  const reason = [restReason?.replace(/\.$/, ""), automatic, evidence, ruleReason].filter(Boolean).join(". ") || null;
  const SourceIcon = SOURCE_ICON[move.source];
  const type = moveType(move);
  const taxable = moveIsTaxable(move);
  const time = timePart(move.occurredAt);
  const hasFee = move.feeMsat !== null && move.feeMsat !== undefined && String(move.feeMsat) !== "0";
  // Bitcoin stays exact; the euro side is the sending transaction's price.
  // An unpriced value says so in the owner's language.
  const fiat = (value: number | null) =>
    value === null ? t("review:transfersPage.linked.unpriced") : formatDisplayMoney(value, 0, currency);
  const fee = currency === "btc" ? formatMsatAsSats(move.feeMsat ?? 0) : fiat(move.feeFiat);
  const railTitle =
    move.fromRail === move.toRail
      ? t(`review:transfersPage.rail.${move.fromRail}`)
      : `${t(`review:transfersPage.rail.${move.fromRail}`)} → ${t(`review:transfersPage.rail.${move.toRail}`)}`;
  return (
    <tr
      className={cn(
        "cursor-pointer border-b transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:outline-none",
        selected && "bg-muted/60 hover:bg-muted/60",
      )}
      aria-current={selected ? "true" : undefined}
      data-source={move.source}
      tabIndex={0}
      aria-label={t("review:transfersPage.details.open", { route })}
      onClick={() => onOpen(move)}
      onKeyDown={(event) => {
        if (event.target !== event.currentTarget || (event.key !== "Enter" && event.key !== " ")) return;
        event.preventDefault();
        onOpen(move);
      }}
    >
      <TableCell className="py-1.5 tabular-nums">
        <span className="text-sm">{datePart(move.occurredAt)}</span>
        {time ? <span className="ml-1.5 hidden text-xs text-muted-foreground @6xl/list:inline">{time}</span> : null}
      </TableCell>
      <TableCell className="overflow-hidden py-1.5" title={railTitle}>
        <Route from={move.fromLabel} to={move.toLabel} fromRail={move.fromRail} toRail={move.toRail} />
      </TableCell>
      <TableCell className="overflow-hidden py-1.5 text-right">
        <CurrencyToggleText
          className={cn(
            "text-sm font-medium whitespace-nowrap tabular-nums",
            move.source === "undone" && "font-normal text-muted-foreground",
            blurClass(hideSensitive),
          )}
        >
          {currency === "btc" ? <AssetFigure msat={move.amountMsat} asset={move.asset} /> : fiat(move.amountFiat)}
        </CurrencyToggleText>
      </TableCell>
      <TableCell className="hidden overflow-hidden py-1.5 text-right @6xl/list:table-cell">
        {/* The sending transaction's network fee, or the fee a swap records. */}
        {hasFee ? (
          <span
            className={cn("text-xs whitespace-nowrap text-muted-foreground tabular-nums", blurClass(hideSensitive))}
            title={hideSensitive ? undefined : t("review:transfersPage.linked.fee", { amount: fee })}
          >
            {fee}
          </span>
        ) : (
          <span className="text-xs text-muted-foreground/60" aria-hidden="true">—</span>
        )}
      </TableCell>
      <TableCell className="overflow-hidden py-1.5">
        {/* What the move was and, the fact that decides taxes, whether linking it booked a sale. */}
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate text-2xs sm:text-xs">{t(`review:transfersPage.type.${type}`)}</span>
          {move.differentAsset ? (
            <Badge variant="outline" className={cn(differentAssetBadgeClassName, "min-w-0 truncate")}>
              {t("review:swap.table.differentAsset")}
            </Badge>
          ) : taxable ? (
            <span className="inline-flex shrink-0 items-center rounded-md bg-amber-50 px-1.5 py-0.5 text-2xs font-medium text-amber-800 ring-1 ring-amber-600/20 ring-inset sm:text-xs dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20">
              {t("review:transfersPage.linked.taxable")}
            </span>
          ) : null}
        </div>
      </TableCell>
      <TableCell className="overflow-hidden py-1.5">
        <div className="flex min-w-0 items-center gap-1.5">
          {/* Automatic is the usual case and stays quiet, naming what found it; what the owner did stands out. */}
          {move.source === "automatic" ? (
            <SourceIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          ) : (
            <span className={cn("inline-flex size-5 shrink-0 items-center justify-center rounded-md", SOURCE_CHIP[move.source])} aria-hidden="true">
              <SourceIcon className="size-3" />
            </span>
          )}
          <span
            className={cn(
              "min-w-0 truncate text-2xs sm:text-xs",
              move.source === "automatic" && "text-muted-foreground",
              rest !== null && "text-amber-700 dark:text-amber-300",
            )}
            title={restReason ?? undefined}
            data-testid={rest !== null ? "transfers-part-linked" : undefined}
          >
            {rest !== null
              ? t("review:transfersPage.linked.chip.part")
              : move.source === "automatic"
              ? t(`review:transfersPage.linked.how.${automaticLinkKind(move)}`)
              : move.byRule && move.source === "reviewed"
                ? t("review:transfersPage.linked.chip.rule")
                : t(`review:transfersPage.linked.chip.${move.source}`)}
          </span>
          {/* The hints keep the cell's right edge, so they line up down the column. */}
          <span className="ml-auto flex shrink-0 items-center">
            {move.basisBlocked ? (
              <RowHint
                label={`${t("review:transfersPage.linked.basisBlocked")}. ${t("review:transfersPage.linked.basisBlockedHint")}`}
                className="text-amber-700 dark:text-amber-300"
              >
                <AlertTriangle className="size-3.5" aria-hidden="true" />
              </RowHint>
            ) : null}
            {reason ? (
              <RowHint label={reason} className="text-muted-foreground">
                <Info className="size-3.5" aria-hidden="true" />
              </RowHint>
            ) : null}
          </span>
        </div>
      </TableCell>
      <TableCell className="py-1 text-right" onClick={(event) => event.stopPropagation()}>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button type="button" variant="ghost" size="icon" className="size-7" aria-label={t("review:transfersPage.linked.actions.label", { route })}>
              <MoreHorizontal className="size-4" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => onOpen(move)}>
              <Info className="size-4" aria-hidden="true" />
              {t("review:transfersPage.details.show")}
            </DropdownMenuItem>
            {/* Unlinking reads the journals, so it waits while they recalculate. */}
            {canUnlink ? (
              <DropdownMenuItem disabled={updating} onSelect={() => onUnlink(move)}>
                <Unlink className="size-4" aria-hidden="true" />
                {t("review:transfersPage.linked.actions.unlink")}
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuSeparator />
            <TransactionsMenuItems outId={move.outTransactionId} inId={move.inTransactionId} />
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </tr>
  );
}

/**
 * The note right after an unlink, a link back or a link, with the way back.
 * The same words and actions as a toast under the page and, `compact`, as a
 * strip at the transfer's row (rotki's resolution strip). It is never shown
 * again after a reload: an unlinked move stays under "Unlinked by you" (or
 * "Needs you" when it is held) with the way back.
 */
/** A note's words. In its row it is short: the row says which transfer it is about. */
function noteText(t: ReturnType<typeof useTranslation<"review">>["t"], toast: Toast, rest: string | null, compact: boolean): string {
  return toast.kind === "linked" && rest
    ? t("transfersPage.strip.linkedRest", { left: formatMsatAsBtc(rest) })
    : t(compact ? `transfersPage.strip.${toast.kind}` : `transfersPage.toast.${toast.kind}`);
}

function TransfersNote({
  toast,
  compact = false,
  pending,
  rest,
  onUndoLink,
  onLinkBack,
  onLinkAgain,
  onLinkDifferently,
  onLinkRest,
  onClose,
}: {
  toast: Toast;
  /** In a row of the list rather than over the page. */
  compact?: boolean;
  pending: boolean;
  /** What a new link leaves of its send, as msat; null when nothing. */
  rest: string | null;
  onUndoLink: (toast: Extract<Toast, { kind: "linked" }>) => void;
  onLinkBack: (toast: Extract<Toast, { kind: "unlinked" }>) => void;
  onLinkAgain: (toast: Extract<Toast, { kind: "unpaired" }>) => void;
  onLinkDifferently: (transactionId: string) => void;
  onLinkRest: (transactionId: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const unlinked = toast.kind === "unlinked" || toast.kind === "unpaired";
  const text = noteText(t, toast, rest, compact);
  const size = compact ? "h-6 px-2 text-xs" : "h-7";
  return (
    <div className={cn("flex flex-wrap items-center gap-x-3 gap-y-1.5 text-sm", compact && "gap-x-2 text-xs sm:text-sm")}>
      {unlinked || toast.kind === "linkUndone" ? (
        <Unlink className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
      ) : (
        <Check className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
      )}
      <span className={cn("min-w-0 flex-1", rest && blurClass(hideSensitive))}>{text}</span>
      {toast.kind === "linked" && rest ? (
        <Button type="button" size="sm" className={size} onClick={() => onLinkRest(toast.sendId)}>
          {t("transfersPage.strip.linkRest")}
        </Button>
      ) : null}
      {toast.kind === "linked" && toast.pairId ? (
        <Button type="button" size="sm" variant="outline" className={size} disabled={pending} onClick={() => onUndoLink(toast)}>
          {t("transfersPage.toast.undo")}
        </Button>
      ) : null}
      {toast.kind === "unlinked" ? (
        <Button type="button" size="sm" variant="outline" className={size} disabled={pending} onClick={() => onLinkBack(toast)}>
          {t("transfersPage.toast.undo")}
        </Button>
      ) : null}
      {toast.kind === "unpaired" ? (
        <Button type="button" size="sm" variant="outline" className={size} disabled={pending} onClick={() => onLinkAgain(toast)}>
          {toast.result.pair && !toast.portion ? t("transfersPage.toast.undo") : t("transfersPage.toast.linkAgain")}
        </Button>
      ) : null}
      {unlinked ? (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className={size}
          onClick={() => onLinkDifferently(toast.kind === "unlinked" ? toast.move.outTransactionId : toast.result.move.outTransactionId)}
        >
          {t("transfersPage.toast.linkDifferently")}
        </Button>
      ) : null}
      <Button type="button" size="icon" variant="ghost" className={compact ? "size-6" : "size-7"} aria-label={t("transfersPage.toast.close")} onClick={onClose}>
        <X className="size-3.5" aria-hidden="true" />
      </Button>
    </div>
  );
}
