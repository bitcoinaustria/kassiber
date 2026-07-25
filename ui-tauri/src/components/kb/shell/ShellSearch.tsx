/**
 * Global search: a quiet row at the top of the side nav that opens a command
 * palette.
 *
 * Both halves are ported from T3Code. The nav row is the entry point only (icon
 * + label + ⌘K hint) because the nav is ~16rem wide; the palette itself is a
 * top-anchored frosted modal — `max-w-xl`, results grouped under small caption
 * labels, first row auto-highlighted, arrows to move, Enter to run, Esc to
 * close, and a footer spelling those keys out.
 *
 * The engine underneath is unchanged from the old top-nav search:
 * `buildAppSearchResults` over the overview snapshot, plus an on-demand
 * `ui.transactions.resolve` lookup once the query looks like a txid.
 */
import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Dialog as DialogPrimitive } from "radix-ui";
import {
  ArrowDown,
  ArrowLeftRight,
  ArrowUp,
  BarChart3,
  BookOpen,
  ClipboardList,
  Database,
  FileSearch,
  Gauge,
  LockKeyhole,
  MessageSquareText,
  Search,
  Settings,
  ShieldAlert,
  TerminalSquare,
  Wallet,
} from "lucide-react";

import { Kbd, KbdGroup } from "@/components/ui/kbd";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { useDaemon } from "@/daemon/client";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import type { OverviewSnapshot } from "@/mocks/seed";
import {
  buildAppSearchResults,
  isLikelyTransactionLookupQuery,
  isSearchResultActivatable,
  searchResultForActivation,
  type RankedSearchResult,
  type ResolvedTransactionLookup,
  type SearchActionId,
  type SearchIconKey,
} from "../search";
import { settingsSectionRoute } from "../settingsSections";

const SEARCH_ICON_BY_KEY: Record<
  SearchIconKey | string,
  React.ComponentType<React.SVGProps<SVGSVGElement>>
> = {
  activity: Gauge,
  assistant: MessageSquareText,
  book: BookOpen,
  database: Database,
  file_search: FileSearch,
  ledger: ClipboardList,
  lock: LockKeyhole,
  logs: TerminalSquare,
  report: BarChart3,
  search: Search,
  settings: Settings,
  shield: ShieldAlert,
  sync: ArrowLeftRight,
  transaction: ArrowLeftRight,
  wallet: Wallet,
};

/*
 * Group heading per result category, as `chrome:search.group.*` keys.
 *
 * Plural domain names ("Actions", "Transactions") the way T3Code names its
 * palette groups — a heading labels a section, so the singular category name
 * ("Action") read as a badge stuck above a list.
 */
const SEARCH_GROUP_LABEL_KEYS: Record<
  RankedSearchResult["category"],
  string
> = {
  action: "actions",
  page: "pages",
  report: "reports",
  review_item: "review",
  setting: "settings",
  transaction: "transactions",
  wallet: "wallets",
};

function searchResultIcon(result: RankedSearchResult) {
  const key = result.iconKey ?? result.category;
  return SEARCH_ICON_BY_KEY[key] ?? Search;
}

function exhaustiveSearchAction(actionId: never): never {
  throw new Error(`Unhandled search action: ${actionId}`);
}

function nextSearchIndex(current: number, delta: number, total: number) {
  if (total <= 0) return 0;
  return (current + delta + total) % total;
}

/**
 * Bucket ranked results by category without disturbing the ranking: groups
 * appear in the order their best result does, and rows keep their global index
 * so arrow-key navigation still walks the ranked list top to bottom.
 */
function groupRankedResults(results: readonly RankedSearchResult[]) {
  const groups: Array<{
    category: RankedSearchResult["category"];
    rows: Array<{ result: RankedSearchResult; index: number }>;
  }> = [];
  results.forEach((result, index) => {
    const group = groups.find((entry) => entry.category === result.category);
    if (group) {
      group.rows.push({ result, index });
      return;
    }
    groups.push({ category: result.category, rows: [{ result, index }] });
  });
  return groups;
}

export function ShellSearch({
  /**
   * `chrome:routeMeta.*` prefix for the active route, resolved to `.label` /
   * `.placeholder`. Keeps the palette's placeholder route-aware ("Search
   * transactions, txids…") even though the entry point is route-independent.
   */
  searchKey,
  daemonEnabled,
}: {
  searchKey: string;
  daemonEnabled: boolean;
}) {
  const { t } = useTranslation(["chrome", "nav", "search", "settings"]);
  const navigate = useNavigate();
  const { isMobile, setOpenMobile } = useSidebar();
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const setDeferredConnectionSetup = useUiStore(
    (s) => s.setDeferredConnectionSetup,
  );
  const { runJournalProcessing } = useJournalProcessingAction();
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState(0);
  const listId = React.useId();

  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const snapshot = data?.data;
  const shouldResolveTransaction = isLikelyTransactionLookupQuery(query);
  const resolvedTransaction = useDaemon<ResolvedTransactionLookup>(
    "ui.transactions.resolve",
    { query: query.trim() },
    { enabled: daemonEnabled && shouldResolveTransaction },
  );
  const results = React.useMemo(
    () =>
      buildAppSearchResults({
        snapshot,
        query,
        aiFeaturesEnabled,
        developerToolsEnabled,
        resolvedTransaction: resolvedTransaction.data?.data ?? null,
        isResolvingTransaction:
          shouldResolveTransaction &&
          (resolvedTransaction.isFetching || resolvedTransaction.isLoading),
        // Dynamic, prefixed keys fall outside the typed-key union; resolve via
        // a thin structural adapter over the namespace-branded translator.
        t: (key: string, options?: Record<string, unknown>) =>
          t(key as never, options as never) as unknown,
      }),
    [
      snapshot,
      query,
      aiFeaturesEnabled,
      developerToolsEnabled,
      resolvedTransaction.data?.data,
      resolvedTransaction.isFetching,
      resolvedTransaction.isLoading,
      shouldResolveTransaction,
      t,
    ],
  );
  const groups = React.useMemo(() => groupRankedResults(results), [results]);
  const resultId = (result: RankedSearchResult) =>
    `search-result-${result.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const activeResult = results[activeIndex];
  const activeDescendantId = activeResult ? resultId(activeResult) : undefined;

  const closeSearch = React.useCallback(() => {
    setOpen(false);
    setQuery("");
  }, []);

  const activateSearchAction = React.useCallback(
    (actionId: SearchActionId) => {
      switch (actionId) {
        case "process-journals":
          runJournalProcessing();
          return;
        case "add-wallet":
        case "connect-btcpay":
        case "import-btcpay":
          setDeferredConnectionSetup({
            sourceId:
              actionId === "connect-btcpay"
                ? "btcpay"
                : actionId === "import-btcpay"
                  ? "btcpay-csv"
                  : "descriptor",
            reason: t("search.actionReason.fromSearch"),
          });
          void navigate({ to: "/connections" });
          return;
        default:
          exhaustiveSearchAction(actionId);
      }
    },
    [navigate, runJournalProcessing, setDeferredConnectionSetup, t],
  );

  const activateResult = React.useCallback(
    (result: RankedSearchResult | undefined) => {
      if (!result) return;
      const actionId = result.action?.id;
      if (actionId) {
        closeSearch();
        activateSearchAction(actionId);
        return;
      }

      const route = result.route;
      if (!route) return;
      closeSearch();
      if (isMobile) setOpenMobile(false);
      if (
        route.to === "/connections/$connectionId" &&
        typeof route.params?.connectionId === "string"
      ) {
        void navigate({
          to: "/connections/$connectionId",
          params: { connectionId: route.params.connectionId },
        });
        return;
      }
      if (route.to === "/connections/$connectionId") return;
      if (route.to === "/transactions" && typeof route.search?.tx === "string") {
        void navigate({
          to: "/transactions",
          search: { tx: route.search.tx },
        });
        return;
      }
      // Settings results still carry the historical `#slug` hash; each category
      // is now its own route, so resolve the slug to that route instead of
      // navigating to /settings and letting the redirect bounce.
      if (route.to === "/settings") {
        void navigate({ to: settingsSectionRoute(route.hash ?? null) });
        return;
      }
      void navigate({ to: route.to });
    },
    [activateSearchAction, closeSearch, isMobile, navigate, setOpenMobile],
  );

  // Auto-highlight the top result, the way T3Code's palette does, so Enter runs
  // the best match without pressing Down first.
  React.useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  React.useEffect(() => {
    if (activeIndex < results.length) return;
    setActiveIndex(Math.max(0, results.length - 1));
  }, [activeIndex, results.length]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "k") return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.altKey || event.shiftKey) return;
      event.preventDefault();
      setOpen(true);
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const label = t(`${searchKey}.label` as never) as string;

  return (
    <>
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            type="button"
            aria-label={label}
            aria-haspopup="dialog"
            tooltip={label}
            data-testid="shell-search-trigger"
            onClick={() => setOpen(true)}
            className="h-8 gap-2 rounded-md text-sm font-medium text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
          >
            <Search className="size-4 shrink-0 opacity-80" aria-hidden="true" />
            <span className="flex-1 truncate text-left">
              {t("shell.searchLabel")}
            </span>
            <Kbd className="hidden bg-sidebar-control-surface text-sidebar-muted-foreground group-data-[collapsible=icon]:hidden md:inline-flex">
              {"⌘"}K
            </Kbd>
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>

      <DialogPrimitive.Root
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) setQuery("");
        }}
      >
        <DialogPrimitive.Portal>
          <DialogPrimitive.Overlay className="kb-glass-backdrop fixed inset-0 z-50 duration-200 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0" />
          {/*
            T3Code's viewport: a full-screen flex column that centres
            horizontally and pushes the palette down with padding, rather than
            absolute centring. The palette therefore grows downward from a fixed
            top edge — the input stays put as results come and go — and it can
            never overflow past the viewport bottom.
          */}
          <DialogPrimitive.Content
            aria-label={label}
            className="kb-glass-dialog fixed top-[max(1rem,4vh)] left-1/2 z-50 flex max-h-[calc(100dvh-max(2rem,8vh))] w-[calc(100%-2rem)] max-w-xl -translate-x-1/2 flex-col overflow-hidden rounded-2xl border p-0 text-foreground duration-200 ease-in-out outline-none data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-98 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-98 sm:top-[10vh]"
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setActiveIndex((current) =>
                  nextSearchIndex(current, 1, results.length),
                );
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setActiveIndex((current) =>
                  nextSearchIndex(current, -1, results.length),
                );
              } else if (event.key === "Enter") {
                event.preventDefault();
                activateResult(
                  searchResultForActivation(results, activeIndex) ?? undefined,
                );
              }
            }}
          >
            <DialogPrimitive.Title className="sr-only">
              {label}
            </DialogPrimitive.Title>
            {/* T3Code's input row: a `px-2.5 py-1.5` wrapper around a large,
                chrome-free control — the popup border is the field's edge, so a
                second border inside it would read as a box in a box. */}
            <div className="flex h-13 shrink-0 items-center gap-2.5 px-4">
              <Search
                className="size-4 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
              <input
                autoFocus
                type="text"
                name="shell-search"
                inputMode="search"
                autoComplete="off"
                aria-label={label}
                aria-controls={listId}
                aria-activedescendant={activeDescendantId}
                placeholder={
                  t(`${searchKey}.placeholder` as never) /* dynamic key */
                }
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="min-w-0 flex-1 border-none bg-transparent text-sm text-foreground shadow-none outline-none placeholder:text-muted-foreground/80"
              />
            </div>
            <div className="h-px shrink-0 bg-border" />
            <div
              id={listId}
              role="listbox"
              aria-label={label}
              className="min-h-0 flex-1 scroll-py-2 overflow-y-auto p-2"
            >
              {!query.trim() ? (
                <div className="py-10 text-center text-sm text-muted-foreground">
                  {t("shell.searchPrompt")}
                </div>
              ) : groups.length === 0 ? (
                <div className="py-10 text-center text-sm text-muted-foreground">
                  {t("shell.searchNoMatches")}
                </div>
              ) : (
                groups.map((group) => (
                  <div key={group.category} className="not-first:mt-2">
                    <p className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                      {t(
                        `search.group.${SEARCH_GROUP_LABEL_KEYS[group.category]}` as never, // dynamic key
                      )}
                    </p>
                    {group.rows.map(({ result, index }) => {
                      const active = index === activeIndex;
                      const ResultIcon = searchResultIcon(result);
                      const activatable = isSearchResultActivatable(result);
                      return (
                        <button
                          key={result.id}
                          id={resultId(result)}
                          type="button"
                          role="option"
                          aria-selected={active}
                          aria-disabled={!activatable}
                          onMouseDown={(event) => {
                            event.preventDefault();
                            activateResult(result);
                          }}
                          onMouseEnter={() => setActiveIndex(index)}
                          className={cn(
                            "flex min-h-8 w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-sm sm:min-h-7",
                            active ? "bg-accent text-accent-foreground" : "",
                          )}
                        >
                          <ResultIcon
                            className="size-4 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                          {result.subtitle ? (
                            <span className="flex min-w-0 flex-1 flex-col">
                              <span className="truncate text-sm text-foreground">
                                {result.title}
                              </span>
                              <span className="truncate text-xs text-muted-foreground/85">
                                {result.subtitle}
                              </span>
                            </span>
                          ) : (
                            <span className="min-w-0 flex-1 truncate text-sm text-foreground">
                              {result.title}
                            </span>
                          )}
                        </button>
                      );
                    })}
                  </div>
                ))
              )}
            </div>
            <div className="flex shrink-0 items-center gap-3 border-t bg-foreground/[0.025] px-5 py-3 text-sm font-medium text-muted-foreground max-sm:flex-col max-sm:items-start">
              <div className="flex items-center gap-3">
                <KbdGroup className="items-center gap-1.5">
                  <Kbd>
                    <ArrowUp />
                  </Kbd>
                  <Kbd>
                    <ArrowDown />
                  </Kbd>
                  <span>{t("shell.searchHints.navigate")}</span>
                </KbdGroup>
                <KbdGroup className="items-center gap-1.5">
                  <Kbd>Enter</Kbd>
                  <span>{t("shell.searchHints.open")}</span>
                </KbdGroup>
                <KbdGroup className="items-center gap-1.5">
                  <Kbd>Esc</Kbd>
                  <span>{t("shell.searchHints.close")}</span>
                </KbdGroup>
              </div>
            </div>
          </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    </>
  );
}
