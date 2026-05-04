import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import {
  Link,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import {
  BadgeCheck,
  BarChart3,
  Bell,
  BookOpen,
  Bug,
  ChevronRight,
  ChevronsUpDown,
  ClipboardList,
  Eye,
  EyeOff,
  Gauge,
  Heart,
  LockKeyhole,
  LogOut,
  MessageSquareText,
  Search,
  Server,
  Settings,
  CircleDollarSign,
  ShieldAlert,
  User,
  Users,
  Wallet,
} from "lucide-react";
import * as React from "react";

import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
} from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";
import {
  DAEMON_AUTH_REQUIRED_EVENT,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { clearImportProject, getTransport } from "@/daemon/transport";
import { cn } from "@/lib/utils";
import {
  clearSessionUnlockPassphrase,
  hasSessionUnlockPassphrase,
  setSessionUnlockPassphrase,
  verifySessionUnlockPassphrase,
} from "@/store/sessionLock";
import type { OverviewSnapshot } from "@/mocks/seed";
import { AssistantSessionProvider } from "@/components/ai/AssistantSessionProvider";
import type { AssistantReturnPath } from "@/components/ai/assistantSession";
import { ScreenAssistantMockup } from "./ScreenAssistantMockup";
import { PreAlphaBanner } from "./PreAlphaBanner";

type AppRoutePath =
  | "/overview"
  | "/transactions"
  | "/reports"
  | "/source-of-funds"
  | "/connections"
  | "/books"
  | "/journals"
  | "/tax-events"
  | "/quarantine"
  | "/settings"
  | "/assistant";

type NavItem = {
  label: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  href: AppRoutePath;
  children?: NavItem[];
};

type NavGroup = {
  title: string;
  items: NavItem[];
};

type RouteMeta = {
  title: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  searchLabel: string;
  searchPlaceholder: string;
};

type SearchResult = {
  id: string;
  title: string;
  detail: string;
  keywords: string[];
  to: AppRoutePath | "/connections/$connectionId";
  connectionId?: string;
};

type NotificationItem = {
  id: string;
  title: string;
  body: string;
  tone: "info" | "success" | "warning" | "error";
  to?: AppRoutePath;
  action?: "process-journals";
  actionLabel?: string;
};

type JournalProcessResult = {
  entries_created?: number;
  quarantined?: number;
  processed_transactions?: number;
};

const APP_VERSION = "0.22.0";
const APP_COMMIT = __APP_COMMIT__;
const APP_COMMIT_SHORT = APP_COMMIT ? APP_COMMIT.slice(0, 7) : "unknown";

const NAV_GROUPS: NavGroup[] = [
  {
    title: "Main",
    items: [
      { label: "Overview", icon: Gauge, href: "/overview" },
      { label: "Transactions", icon: ClipboardList, href: "/transactions" },
      { label: "Wallets", icon: Wallet, href: "/connections" },
      { label: "Reports", icon: BarChart3, href: "/reports" },
      { label: "Source of Funds", icon: BadgeCheck, href: "/source-of-funds" },
      { label: "Assistant", icon: MessageSquareText, href: "/assistant" },
    ],
  },
  {
    title: "Review",
    items: [
      { label: "Journals", icon: BookOpen, href: "/journals" },
      { label: "Tax Events", icon: CircleDollarSign, href: "/tax-events" },
      { label: "Quarantine", icon: ShieldAlert, href: "/quarantine" },
    ],
  },
];

const ROUTE_META: Array<[string, RouteMeta]> = [
  [
    "/connections/",
    {
      title: "Connection Detail",
      icon: Wallet,
      searchLabel: "Search connections",
      searchPlaceholder: "Search wallets, books...",
    },
  ],
  [
    "/connections",
    {
      title: "Wallets",
      icon: Wallet,
      searchLabel: "Search wallets",
      searchPlaceholder: "Search wallets, backends...",
    },
  ],
  [
    "/books",
    {
      title: "Books",
      icon: Users,
      searchLabel: "Search books",
      searchPlaceholder: "Search books, countries...",
    },
  ],
  [
    "/journals",
    {
      title: "Journals",
      icon: BookOpen,
      searchLabel: "Search journals",
      searchPlaceholder: "Search entry type, wallet, asset...",
    },
  ],
  [
    "/settings",
    {
      title: "Settings",
      icon: Settings,
      searchLabel: "Search settings",
      searchPlaceholder: "Search settings, backends...",
    },
  ],
  [
    "/reports",
    {
      title: "Reports",
      icon: BarChart3,
      searchLabel: "Search reports",
      searchPlaceholder: "Search reports, exports...",
    },
  ],
  [
    "/source-of-funds",
    {
      title: "Source of Funds",
      icon: BadgeCheck,
      searchLabel: "Search source of funds",
      searchPlaceholder: "Search sources, wallets...",
    },
  ],
  [
    "/tax-events",
    {
      title: "Tax Events",
      icon: CircleDollarSign,
      searchLabel: "Search tax events",
      searchPlaceholder: "Search event, basis, account...",
    },
  ],
  [
    "/quarantine",
    {
      title: "Quarantine",
      icon: ShieldAlert,
      searchLabel: "Search quarantine",
      searchPlaceholder: "Search issue, account, source...",
    },
  ],
  [
    "/transactions",
    {
      title: "Transactions",
      icon: ClipboardList,
      searchLabel: "Search transactions",
      searchPlaceholder: "Search counterparty, tag, account...",
    },
  ],
  [
    "/assistant",
    {
      title: "Assistant",
      icon: MessageSquareText,
      searchLabel: "Search assistant",
      searchPlaceholder: "Search conversation...",
    },
  ],
  [
    "/overview",
    {
      title: "Overview",
      icon: Gauge,
      searchLabel: "Search overview",
      searchPlaceholder: "Search transactions, reports...",
    },
  ],
];

const STATIC_SEARCH_RESULTS: SearchResult[] = [
  {
    id: "route:overview",
    title: "Overview",
    detail: "Portfolio, balance, activity",
    keywords: ["dashboard", "home", "balance", "portfolio"],
    to: "/overview",
  },
  {
    id: "route:transactions",
    title: "Transactions",
    detail: "Transaction rows and filters",
    keywords: ["tx", "counterparty", "account", "amount", "import"],
    to: "/transactions",
  },
  {
    id: "route:connections",
    title: "Wallets",
    detail: "Wallet sources and sync",
    keywords: ["wallets", "xpub", "backend", "sync"],
    to: "/connections",
  },
  {
    id: "route:books",
    title: "Books",
    detail: "Books and tax settings",
    keywords: ["book", "books", "tax", "country"],
    to: "/books",
  },
  {
    id: "route:source-of-funds",
    title: "Source of Funds",
    detail: "Wallet sources and local provenance summaries",
    keywords: ["source", "funds", "wallet", "balance", "provenance"],
    to: "/source-of-funds",
  },
  {
    id: "route:journals",
    title: "Journals",
    detail: "Processed tax journal",
    keywords: ["process", "entries", "fees", "basis"],
    to: "/journals",
  },
  {
    id: "route:reports",
    title: "Reports",
    detail: "Capital gains and exports",
    keywords: ["csv", "pdf", "xlsx", "tax", "austria", "e1kv"],
    to: "/reports",
  },
  {
    id: "route:quarantine",
    title: "Quarantine",
    detail: "Review ambiguous rows",
    keywords: ["review", "issues", "missing", "price"],
    to: "/quarantine",
  },
  {
    id: "route:settings",
    title: "Settings",
    detail: "Preferences, integrations, local data",
    keywords: ["preferences", "backends", "providers", "privacy", "lock"],
    to: "/settings",
  },
  {
    id: "route:assistant",
    title: "Assistant",
    detail: "Ask Kassiber",
    keywords: ["chat", "ai", "tools"],
    to: "/assistant",
  },
];

function searchMatches(result: SearchResult, query: string) {
  const terms = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  if (!terms.length) return false;
  const haystack = [
    result.title,
    result.detail,
    ...result.keywords,
  ]
    .join(" ")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}

function buildSearchResults(
  snapshot: OverviewSnapshot | undefined,
  query: string,
): SearchResult[] {
  if (!query.trim()) return [];

  const dynamicResults: SearchResult[] = [
    ...(snapshot?.connections.map((connection) => ({
      id: `connection:${connection.id}`,
      title: connection.label,
      detail: `${connection.kind.toUpperCase()} · ${connection.status}`,
      keywords: [
        "connection",
        "wallet",
        "sync",
        connection.kind,
        connection.status,
      ],
      to: "/connections/$connectionId" as const,
      connectionId: connection.id,
    })) ?? []),
    ...(snapshot?.txs.map((tx) => ({
      id: `tx:${tx.id}`,
      title: `${tx.id} · ${tx.counter}`,
      detail: `${tx.account} · ${tx.type} · ${tx.tag}`,
      keywords: [
        "transaction",
        "transactions",
        tx.id,
        tx.account,
        tx.counter,
        tx.type,
        tx.tag,
      ],
      to: "/transactions" as const,
    })) ?? []),
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "status:journals",
            title: "Journals need processing",
            detail: "Reports are stale until journals are processed",
            keywords: ["journal", "reports", "stale", "process"],
            to: "/journals" as const,
          },
        ]
      : []),
    ...((snapshot?.status?.quarantines ?? 0) > 0
      ? [
          {
            id: "status:quarantine",
            title: "Transactions quarantined",
            detail: `${snapshot?.status?.quarantines ?? 0} rows need review`,
            keywords: ["quarantine", "review", "missing", "price"],
            to: "/quarantine" as const,
          },
        ]
      : []),
  ];

  return [...STATIC_SEARCH_RESULTS, ...dynamicResults]
    .filter((result) => searchMatches(result, query))
    .slice(0, 8);
}

function nextSearchIndex(current: number, delta: number, total: number) {
  if (total <= 0) return 0;
  return (current + delta + total) % total;
}

function notificationRouteFor(title: string): AppRoutePath | undefined {
  const normalized = title.toLowerCase();
  if (normalized.includes("journal")) return "/journals";
  if (normalized.includes("quarantine")) return "/quarantine";
  if (normalized.includes("sync") || normalized.includes("wallet")) {
    return "/connections";
  }
  if (normalized.includes("report") || normalized.includes("export")) {
    return "/reports";
  }
  if (
    normalized.includes("book") ||
    normalized.includes("books")
  ) {
    return "/books";
  }
  if (normalized.includes("transaction")) return "/transactions";
  return undefined;
}

function assistantReturnPathFor(pathname: string): AssistantReturnPath {
  if (pathname.startsWith("/connections")) return "/connections";
  if (pathname === "/transactions") return "/transactions";
  if (pathname === "/reports") return "/reports";
  if (pathname === "/source-of-funds") return "/source-of-funds";
  if (pathname === "/books" || pathname === "/profiles") return "/books";
  if (pathname === "/journals") return "/journals";
  if (pathname === "/tax-events") return "/tax-events";
  if (pathname === "/quarantine") return "/quarantine";
  if (pathname === "/settings") return "/settings";
  return "/overview";
}

export function AppShell() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const identity = useUiStore((s) => s.identity);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const [daemonAuthRequired, setDaemonAuthRequired] = React.useState(false);
  const requiresDaemonUnlock = encryptedWorkspace || daemonAuthRequired;
  const routerBusy = useRouterState({
    select: (s) => s.isLoading || s.isTransitioning || s.status === "pending",
  });
  const daemonFetchCount = useIsFetching({ queryKey: ["daemon"] });
  const [assistantCollapsed, setAssistantCollapsed] = React.useState(false);
  const [locked, setLocked] = React.useState(
    () => encryptedWorkspace && !hasSessionUnlockPassphrase(),
  );
  const [assistantReturnPath, setAssistantReturnPath] =
    React.useState<AssistantReturnPath>("/overview");
  const mainRef = React.useRef<HTMLElement>(null);
  const launchLockApplied = React.useRef(false);
  const shellBusy = routerBusy || daemonFetchCount > 0;
  const isAssistantRoute = pathname === "/assistant";
  const routeMeta =
    ROUTE_META.find(([prefix]) => pathname.startsWith(prefix))?.[1] ?? {
      title: "Kassiber",
      icon: Gauge,
      searchLabel: "Search Kassiber",
      searchPlaceholder: "Search transactions, reports...",
    };
  const clearDaemonQueryCache = React.useCallback(() => {
    void queryClient.cancelQueries({ queryKey: ["daemon"] });
    queryClient.removeQueries({ queryKey: ["daemon"] });
  }, [queryClient]);
  const clearImportedProjectRoot = React.useCallback(async () => {
    if (identity?.importedProject) {
      await clearImportProject();
    }
  }, [identity?.importedProject]);

  const lockApp = React.useCallback(() => {
    setHideSensitive(true);
    if (requiresDaemonUnlock) {
      clearSessionUnlockPassphrase();
      clearDaemonQueryCache();
      setLocked(true);
      void getTransport("real").invoke({ kind: "daemon.lock" });
      return;
    }
    if (!hasSessionUnlockPassphrase()) {
      clearSessionUnlockPassphrase();
      void clearImportedProjectRoot()
        .catch(() => {})
        .finally(() => {
          setIdentity(null);
          void navigate({ to: "/", replace: true });
        });
      return;
    }
    setLocked(true);
  }, [
    clearDaemonQueryCache,
    clearImportedProjectRoot,
    navigate,
    requiresDaemonUnlock,
    setHideSensitive,
    setIdentity,
  ]);

  const unlockApp = React.useCallback(
    async (
      passphrase: string,
    ): Promise<{ ok: boolean; error?: string | null }> => {
      if (requiresDaemonUnlock) {
        const envelope = await getTransport("real").invoke({
          kind: "daemon.unlock",
          args: {
            ...(identity?.importedProject
              ? { require_existing_project: true }
              : {}),
            auth_response: { passphrase_secret: passphrase },
          },
        });
        const unlocked = envelope.kind === "daemon.unlock";
        if (unlocked) {
          await setSessionUnlockPassphrase(passphrase);
          setDaemonAuthRequired(false);
          await queryClient.invalidateQueries({
            queryKey: ["daemon"],
          });
          setLocked(false);
        }
        return {
          ok: unlocked,
          error:
            envelope.error?.message ??
            (envelope.kind === "auth_required"
              ? "Database passphrase is required."
              : null),
        };
      }

      const unlocked = await verifySessionUnlockPassphrase(passphrase);
      if (unlocked) {
        setLocked(false);
      }
      return { ok: unlocked, error: null };
    },
    [identity?.importedProject, queryClient, requiresDaemonUnlock],
  );

  const resetLocalUiSession = React.useCallback(() => {
    clearSessionUnlockPassphrase();
    clearDaemonQueryCache();
    setDaemonAuthRequired(false);
    setHideSensitive(false);
    void clearImportedProjectRoot()
      .catch(() => {})
      .finally(() => {
        setIdentity(null);
        void navigate({ to: "/", replace: true });
      });
  }, [
    clearDaemonQueryCache,
    clearImportedProjectRoot,
    navigate,
    setHideSensitive,
    setIdentity,
  ]);

  React.useEffect(() => {
    if (identity) return;
    launchLockApplied.current = false;
    void navigate({ to: "/", replace: true });
  }, [identity, navigate]);

  React.useEffect(() => {
    const onAuthRequired = () => {
      setDaemonAuthRequired(true);
      clearSessionUnlockPassphrase();
      clearDaemonQueryCache();
      setHideSensitive(true);
      setLocked(true);
    };

    window.addEventListener(DAEMON_AUTH_REQUIRED_EVENT, onAuthRequired);
    return () => {
      window.removeEventListener(DAEMON_AUTH_REQUIRED_EVENT, onAuthRequired);
    };
  }, [clearDaemonQueryCache, setHideSensitive]);

  React.useEffect(() => {
    if (!encryptedWorkspace) return;
    if (hasSessionUnlockPassphrase()) return;
    if (launchLockApplied.current) return;
    launchLockApplied.current = true;
    lockApp();
  }, [encryptedWorkspace, lockApp]);

  React.useEffect(() => {
    if (!encryptedWorkspace || !appLockPolicy.autoLockWhenIdle || locked) {
      return;
    }

    let timeout: number | undefined;
    const reset = () => {
      window.clearTimeout(timeout);
      timeout = window.setTimeout(
        lockApp,
        Math.max(1, appLockPolicy.idleMinutes) * 60_000,
      );
    };
    const events = ["pointerdown", "keydown", "wheel", "touchstart"];
    events.forEach((eventName) =>
      window.addEventListener(eventName, reset, { passive: true }),
    );
    reset();

    return () => {
      window.clearTimeout(timeout);
      events.forEach((eventName) =>
        window.removeEventListener(eventName, reset),
      );
    };
  }, [
    appLockPolicy.autoLockWhenIdle,
    appLockPolicy.idleMinutes,
    encryptedWorkspace,
    lockApp,
    locked,
  ]);

  React.useEffect(() => {
    if (!encryptedWorkspace || !appLockPolicy.lockOnWindowClose) return;

    const lockOnHidden = () => {
      if (document.visibilityState === "hidden") {
        lockApp();
      }
    };
    window.addEventListener("pagehide", lockApp);
    document.addEventListener("visibilitychange", lockOnHidden);
    return () => {
      window.removeEventListener("pagehide", lockApp);
      document.removeEventListener("visibilitychange", lockOnHidden);
    };
  }, [appLockPolicy.lockOnWindowClose, encryptedWorkspace, lockApp]);

  React.useEffect(() => {
    if (!isAssistantRoute) {
      setAssistantReturnPath(assistantReturnPathFor(pathname));
    }
  }, [isAssistantRoute, pathname]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "l") return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.altKey || event.shiftKey) return;
      event.preventDefault();
      lockApp();
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [lockApp]);

  React.useEffect(() => {
    window.addEventListener("kassiber:lock-app", lockApp);
    return () => window.removeEventListener("kassiber:lock-app", lockApp);
  }, [lockApp]);

  React.useLayoutEffect(() => {
    if (locked) return;
    const main = mainRef.current;
    if (!main) return;
    main.scrollTo({ top: 0, left: 0 });
    setAssistantCollapsed(false);
  }, [locked, pathname]);

  React.useEffect(() => {
    const main = mainRef.current;
    if (!main) {
      return;
    }

    const syncAssistantState = () => {
      setAssistantCollapsed(main.scrollTop > 32);
    };

    syncAssistantState();
    main.addEventListener("scroll", syncAssistantState, { passive: true });

    return () => {
      main.removeEventListener("scroll", syncAssistantState);
    };
  }, [locked, pathname]);

  React.useEffect(() => {
    const openSettings = (event: Event) => {
      const detail = (event as CustomEvent<{ section?: "backends" | "ai" }>)
        .detail;
      void navigate({
        to: "/settings",
        hash: detail?.section ?? undefined,
      });
    };

    window.addEventListener("kassiber:open-settings", openSettings);

    return () => {
      window.removeEventListener("kassiber:open-settings", openSettings);
    };
  }, [navigate]);

  if (!identity) return null;

  return (
    <TooltipProvider>
      <div className="flex h-svh flex-col overflow-hidden bg-sidebar">
        <PreAlphaBanner className="shrink-0" />
        <SidebarProvider className="min-h-0 flex-1 bg-sidebar">
          <a
            href="#app-main"
            className="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:text-foreground focus:ring-2 focus:ring-ring"
          >
            Skip to main content
          </a>
          <AppSidebar
            pathname={pathname}
            onLock={lockApp}
            daemonEnabled={!locked}
          />
          <div className="min-h-0 w-full overflow-hidden lg:p-2">
            <div className="relative flex h-full w-full flex-col items-center justify-start overflow-hidden bg-background lg:rounded-xl lg:border">
              <AppDashboardHeader
                meta={routeMeta}
                onLock={lockApp}
                daemonEnabled={!locked}
              />
              {locked ? (
                <main
                  id="app-main"
                  ref={mainRef}
                  tabIndex={-1}
                  className="relative min-h-0 w-full flex-1 overflow-auto bg-background"
                >
                  <LockScreen
                    reason={
                      daemonAuthRequired
                        ? "The daemon needs the database passphrase before it can return live books data."
                        : undefined
                    }
                    onUnlock={unlockApp}
                    onReset={resetLocalUiSession}
                  />
                </main>
              ) : (
                <AssistantSessionProvider returnPath={assistantReturnPath}>
                  <main
                    id="app-main"
                    ref={mainRef}
                    tabIndex={-1}
                    className={`relative min-h-0 w-full flex-1 overflow-auto bg-background ${
                      isAssistantRoute
                        ? "pb-0"
                        : assistantCollapsed
                          ? "pb-[150px]"
                          : "pb-[240px]"
                    }`}
                  >
                    <RouteTransitionIndicator active={shellBusy} />
                    <Outlet />
                  </main>
                  {isAssistantRoute ? null : (
                    <ScreenAssistantMockup
                      collapsed={assistantCollapsed}
                      className="absolute inset-x-0 bottom-0 z-20"
                    />
                  )}
                </AssistantSessionProvider>
              )}
            </div>
          </div>
        </SidebarProvider>
      </div>
    </TooltipProvider>
  );
}

function RouteTransitionIndicator({ active }: { active: boolean }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none sticky top-0 z-10 h-px w-full overflow-hidden transition-opacity duration-150",
        active ? "opacity-100" : "opacity-0",
      )}
    >
      <div className="h-full w-1/2 bg-primary/70 motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full" />
    </div>
  );
}

function AppSidebar({
  pathname,
  onLock,
  daemonEnabled,
}: {
  pathname: string;
  onLock: () => void;
  daemonEnabled: boolean;
}) {
  return (
    <Sidebar
      variant="inset"
      collapsible="icon"
      className="top-9 h-[calc(100svh-2.25rem)]"
    >
      <SidebarHeader>
        <div className="flex items-center gap-2 group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:items-center">
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                size="lg"
                tooltip="Kassiber"
                asChild
                className="group-data-[collapsible=icon]:size-9! group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0!"
              >
                <Link
                  to="/overview"
                  className="min-w-0 group-data-[collapsible=icon]:justify-center"
                >
                  <div className="flex aspect-square size-8 shrink-0 items-center justify-center rounded-sm bg-primary group-data-[collapsible=icon]:size-9">
                    <Wallet
                      className="size-5 text-primary-foreground"
                      aria-hidden="true"
                    />
                  </div>
                  <div className="flex min-w-0 flex-col gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
                    <span className="truncate font-medium">Kassiber</span>
                    <span className="truncate text-xs text-muted-foreground">
                      Private Bitcoin Books
                    </span>
                  </div>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
          <SidebarTrigger className="ml-auto group-data-[collapsible=icon]:ml-0" />
        </div>
      </SidebarHeader>
      <SidebarContent>
        {NAV_GROUPS.map((group) => (
          <SidebarGroup key={group.title}>
            <SidebarGroupLabel>{group.title}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.items.map((item) => (
                  <NavMenuItem key={item.label} item={item} pathname={pathname} />
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
      <SidebarFooter>
        <SidebarActions pathname={pathname} />
        <NavUser onLock={onLock} daemonEnabled={daemonEnabled} />
        <AppVersion />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}

function SidebarActions({ pathname }: { pathname: string }) {
  const dataMode = useUiStore((state) => state.dataMode);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const isRealData = dataMode === "real";

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <div className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
          <Server className="size-4 shrink-0" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate group-data-[collapsible=icon]:hidden">
            {isRealData ? "Real data" : "Mock data"}
          </span>
          <Switch
            checked={isRealData}
            aria-label="Toggle real data mode"
            onCheckedChange={(checked) =>
              setDataMode(checked ? "real" : "mock")
            }
            className="group-data-[collapsible=icon]:hidden"
          />
        </div>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton asChild tooltip="Donate sats">
          <a href="#donate">
            <Heart className="size-4" aria-hidden="true" />
            <span>Donate sats</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton asChild tooltip="Bug report">
          <a
            href="https://github.com/bitcoinaustria/kassiber/issues"
            target="_blank"
            rel="noreferrer"
          >
            <Bug className="size-4" aria-hidden="true" />
            <span>Bug report</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton
          asChild
          isActive={pathname === "/settings"}
          tooltip="Settings"
        >
          <Link to="/settings">
            <Settings className="size-4" aria-hidden="true" />
            <span>Settings</span>
          </Link>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function NavMenuItem({
  item,
  pathname,
}: {
  item: NavItem;
  pathname: string;
}) {
  const Icon = item.icon;
  const active = pathname === item.href || pathname.startsWith(`${item.href}/`);

  if (!item.children?.length) {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton asChild isActive={active} tooltip={item.label}>
          <Link to={item.href}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{item.label}</span>
          </Link>
        </SidebarMenuButton>
      </SidebarMenuItem>
    );
  }

  return (
    <Collapsible asChild defaultOpen={active} className="group/collapsible">
      <SidebarMenuItem>
        <CollapsibleTrigger asChild>
          <SidebarMenuButton isActive={active} tooltip={item.label}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{item.label}</span>
            <ChevronRight className="ml-auto size-4 transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
          </SidebarMenuButton>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <SidebarMenuSub>
            {item.children.map((child) => {
              const childActive =
                pathname === child.href || pathname.startsWith(`${child.href}/`);
              return (
                <SidebarMenuSubItem key={child.label}>
                  <SidebarMenuSubButton asChild isActive={childActive}>
                    <Link to={child.href}>
                      {child.label}
                    </Link>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              );
            })}
          </SidebarMenuSub>
        </CollapsibleContent>
      </SidebarMenuItem>
    </Collapsible>
  );
}

function NavUser({
  onLock,
  daemonEnabled,
}: {
  onLock: () => void;
  daemonEnabled: boolean;
}) {
  const identity = useUiStore((s) => s.identity);
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const status = data?.data?.status;
  const name = status?.workspace ?? identity?.workspace ?? "My Books";
  const detail = status?.profile ?? identity?.profile ?? identity?.name ?? "Private";

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <SidebarMenuButton
              size="lg"
              className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground group-data-[collapsible=icon]:size-9! group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-0!"
            >
              <Avatar className="size-8 shrink-0 rounded-lg group-data-[collapsible=icon]:size-9">
                <AvatarFallback className="rounded-lg text-sm font-medium group-data-[collapsible=icon]:text-sm">
                  {name
                    .split(" ")
                    .map((part) => part[0])
                    .join("")
                    .slice(0, 2)}
                </AvatarFallback>
              </Avatar>
              <div className="grid flex-1 text-left text-sm leading-tight group-data-[collapsible=icon]:hidden">
                <span className="truncate font-medium">{name}</span>
                <span className="truncate text-xs text-muted-foreground">
                  {detail}
                </span>
              </div>
              <ChevronsUpDown
                className="ml-auto size-4 group-data-[collapsible=icon]:hidden"
                aria-hidden="true"
              />
            </SidebarMenuButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--radix-dropdown-menu-trigger-width) min-w-56 rounded-lg"
            side="bottom"
            align="end"
            sideOffset={4}
          >
            <DropdownMenuLabel className="p-0 font-normal">
              <div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
                <Avatar className="size-8 rounded-lg">
                  <AvatarFallback className="rounded-lg">
                    {name
                      .split(" ")
                      .map((part) => part[0])
                      .join("")
                      .slice(0, 2)}
                  </AvatarFallback>
                </Avatar>
                <div className="grid flex-1 text-left text-sm leading-tight">
                  <span className="truncate font-medium">{name}</span>
                  <span className="truncate text-xs text-muted-foreground">
                    {detail}
                  </span>
                </div>
              </div>
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem asChild>
              <Link to="/books">
                <User className="mr-2 size-4" aria-hidden="true" />
                Books
              </Link>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onLock()}>
              <LogOut className="mr-2 size-4" aria-hidden="true" />
              Lock Kassiber
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

function AppVersion() {
  return (
    <a
      href="https://github.com/bitcoinaustria/kassiber"
      target="_blank"
      rel="noreferrer"
      title={`Kassiber v${APP_VERSION} (${APP_COMMIT})`}
      className="inline-flex items-center justify-center gap-1 px-2 pb-1 text-center text-[11px] leading-none text-muted-foreground underline-offset-4 hover:text-foreground hover:underline group-data-[collapsible=icon]:hidden"
    >
      <span>Kassiber v{APP_VERSION}</span>
      <span aria-hidden="true">·</span>
      <span className="font-mono text-[11px] leading-none">
        {APP_COMMIT_SHORT}
      </span>
    </a>
  );
}

function AppDashboardHeader({
  meta,
  onLock,
  daemonEnabled,
}: {
  meta: RouteMeta;
  onLock: () => void;
  daemonEnabled: boolean;
}) {
  const navigate = useNavigate();
  const Icon = meta.icon;
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const dataMode = useUiStore((s) => s.dataMode);
  const appNotifications = useUiStore((s) => s.notifications);
  const addNotification = useUiStore((s) => s.addNotification);
  const clearNotifications = useUiStore((s) => s.clearNotifications);
  const processJournals =
    useDaemonMutation<JournalProcessResult>("ui.journals.process");
  const [searchQuery, setSearchQuery] = React.useState("");
  const [searchOpen, setSearchOpen] = React.useState(false);
  const [activeSearchIndex, setActiveSearchIndex] = React.useState(0);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const searchRootRef = React.useRef<HTMLDivElement>(null);
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const snapshot = data?.data;
  const searchResults = React.useMemo(
    () => buildSearchResults(snapshot, searchQuery),
    [snapshot, searchQuery],
  );
  const searchListId = React.useId();
  const searchActiveId = searchResults[activeSearchIndex]?.id
    ? `search-result-${searchResults[activeSearchIndex].id.replace(/[^a-zA-Z0-9_-]/g, "-")}`
    : undefined;
  const activateSearchResult = React.useCallback(
    (result: SearchResult | undefined) => {
      if (!result) return;
      setSearchOpen(false);
      setSearchQuery("");
      if (result.to === "/connections/$connectionId" && result.connectionId) {
        void navigate({
          to: "/connections/$connectionId",
          params: { connectionId: result.connectionId },
        });
        return;
      }
      void navigate({ to: result.to });
    },
    [navigate],
  );

  React.useEffect(() => {
    setActiveSearchIndex(0);
  }, [searchQuery]);

  React.useEffect(() => {
    if (activeSearchIndex < searchResults.length) return;
    setActiveSearchIndex(Math.max(0, searchResults.length - 1));
  }, [activeSearchIndex, searchResults.length]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "k") return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.altKey || event.shiftKey) return;
      event.preventDefault();
      setSearchOpen(true);
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const runJournalProcessing = React.useCallback(() => {
    if (processJournals.isPending) return;
    processJournals.mutate(undefined, {
      onSuccess: (envelope) => {
        const payload = envelope.data;
        const parts = [
          payload?.processed_transactions !== undefined
            ? `${payload.processed_transactions} transactions`
            : null,
          payload?.entries_created !== undefined
            ? `${payload.entries_created} entries`
            : null,
          payload?.quarantined ? `${payload.quarantined} quarantined` : null,
        ].filter(Boolean);
        addNotification({
          title: "Journals processed",
          body: parts.join(", ") || "Journal state refreshed.",
          tone: payload?.quarantined ? "warning" : "success",
        });
      },
      onError: (error) => {
        addNotification({
          title: "Journal processing failed",
          body:
            error instanceof Error
              ? error.message
              : "Could not process journals.",
          tone: "error",
        });
      },
    });
  }, [addNotification, processJournals]);

  const systemNotificationItems: NotificationItem[] = [
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "journals-stale",
            title: "Journals need processing",
            body: "Reports are not trusted until journals are processed.",
            tone: "warning" as const,
            to: "/journals" as const,
            action: "process-journals" as const,
            actionLabel: "Process now",
          },
        ]
      : []),
    ...((snapshot?.status?.quarantines ?? 0) > 0
      ? [
          {
            id: "quarantines",
            title: "Transactions quarantined",
            body: `${snapshot?.status?.quarantines ?? 0} transactions need review.`,
            tone: "warning" as const,
            to: "/quarantine" as const,
          },
        ]
      : []),
    {
      id: "data-mode",
      title: dataMode === "mock" ? "Mock data active" : "Real data active",
      body:
        dataMode === "mock"
          ? "The UI is showing fixture data."
          : "The UI is reading from the local daemon.",
      tone: "info" as const,
      to: "/settings" as const,
    },
  ];
  const notificationItems: NotificationItem[] = [
    ...appNotifications.map((item) => ({
      ...item,
      to: notificationRouteFor(item.title),
    })),
    ...systemNotificationItems,
  ];
  const notificationCount = notificationItems.filter(
    (item) =>
      item.tone !== "info" ||
      item.title.toLowerCase().includes("sync"),
  ).length;

  return (
    <header className="flex w-full items-center gap-3 border-b bg-background px-4 py-4 sm:px-6">
      <Icon className="size-5" aria-hidden="true" />
      <h1 className="text-base font-medium">{meta.title}</h1>

      <div className="ml-auto flex items-center gap-2">
        <div
          ref={searchRootRef}
          className="relative hidden w-80 lg:block lg:w-96 xl:w-[28rem]"
          onBlur={(event) => {
            if (
              event.relatedTarget instanceof Node &&
              searchRootRef.current?.contains(event.relatedTarget)
            ) {
              return;
            }
            setSearchOpen(false);
          }}
        >
          <Search
            className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            ref={searchInputRef}
            type="search"
            name="header-search"
            inputMode="search"
            autoComplete="off"
            aria-label={meta.searchLabel}
            aria-expanded={searchOpen}
            aria-controls={searchListId}
            aria-activedescendant={searchActiveId}
            placeholder={meta.searchPlaceholder}
            value={searchQuery}
            onChange={(event) => {
              setSearchQuery(event.target.value);
              setSearchOpen(true);
            }}
            onFocus={() => setSearchOpen(true)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setSearchOpen(true);
                setActiveSearchIndex((current) =>
                  nextSearchIndex(current, 1, searchResults.length),
                );
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setSearchOpen(true);
                setActiveSearchIndex((current) =>
                  nextSearchIndex(current, -1, searchResults.length),
                );
              } else if (event.key === "Enter") {
                event.preventDefault();
                activateSearchResult(searchResults[activeSearchIndex]);
              } else if (event.key === "Escape") {
                setSearchOpen(false);
                searchInputRef.current?.blur();
              }
            }}
            className="h-10 w-full pr-14 pl-9 text-sm"
          />
          <kbd className="pointer-events-none absolute top-1/2 right-2 hidden -translate-y-1/2 rounded-md border bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground md:inline-flex">
            {"\u2318"}
            {"\u00a0"}K
          </kbd>
          {searchOpen && searchQuery.trim() && (
            <div
              id={searchListId}
              role="listbox"
              className="absolute top-11 right-0 left-0 z-30 overflow-hidden rounded-md border bg-popover p-1 text-popover-foreground shadow-lg"
            >
              {searchResults.length > 0 ? (
                searchResults.map((result, index) => {
                  const active = index === activeSearchIndex;
                  const itemId = `search-result-${result.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
                  return (
                    <button
                      key={result.id}
                      id={itemId}
                      type="button"
                      role="option"
                      aria-selected={active}
                      onMouseDown={(event) => {
                        event.preventDefault();
                        activateSearchResult(result);
                      }}
                      onMouseEnter={() => setActiveSearchIndex(index)}
                      className={cn(
                        "flex w-full flex-col gap-0.5 rounded-sm px-3 py-2 text-left text-sm",
                        active ? "bg-accent text-accent-foreground" : "",
                      )}
                    >
                      <span className="font-medium">{result.title}</span>
                      <span className="text-xs text-muted-foreground">
                        {result.detail}
                      </span>
                    </button>
                  );
                })
              ) : (
                <div className="px-3 py-2 text-sm text-muted-foreground">
                  No matches
                </div>
              )}
            </div>
          )}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className="relative size-9"
              aria-label="Notifications"
            >
              <Bell className="size-4" aria-hidden="true" />
              {notificationCount > 0 && (
                <span className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full bg-destructive text-[9px] font-medium text-destructive-foreground">
                  {notificationCount}
                </span>
              )}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-80">
            <div className="flex items-center justify-between gap-2 px-2 py-1.5">
              <DropdownMenuLabel className="p-0">
                Notifications
              </DropdownMenuLabel>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-7 px-2 text-xs"
                disabled={appNotifications.length === 0}
                onClick={(event) => {
                  event.preventDefault();
                  clearNotifications();
                }}
              >
                Clear all
              </Button>
            </div>
            <DropdownMenuSeparator />
            {notificationItems.map((item) => (
              <div key={item.id} className="px-1 py-1">
                <DropdownMenuItem
                  className="flex cursor-pointer items-start justify-between gap-3 whitespace-normal rounded-md"
                  onSelect={(event) => {
                    if (!item.to) return;
                    event.preventDefault();
                    void navigate({ to: item.to });
                  }}
                >
                  <span className="min-w-0">
                    <span className="block font-medium">{item.title}</span>
                    <span className="block text-xs text-muted-foreground">
                      {item.body}
                    </span>
                  </span>
                  {item.to ? (
                    <ChevronRight
                      className="mt-1 size-4 shrink-0 text-muted-foreground"
                      aria-hidden="true"
                    />
                  ) : null}
                </DropdownMenuItem>
                {item.action === "process-journals" ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="mt-1 h-7 w-full justify-center text-xs"
                    disabled={processJournals.isPending}
                    onClick={(event) => {
                      event.preventDefault();
                      runJournalProcessing();
                    }}
                  >
                    {processJournals.isPending
                      ? "Processing..."
                      : item.actionLabel}
                  </Button>
                ) : null}
              </div>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="outline"
          size="icon"
          className={
            hideSensitive
              ? "size-9 bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground"
              : "size-9"
          }
          aria-label={
            hideSensitive ? "Show sensitive data" : "Hide sensitive data"
          }
          aria-pressed={hideSensitive}
          title={hideSensitive ? "Show sensitive data" : "Hide sensitive data"}
          onClick={() => setHideSensitive(!hideSensitive)}
        >
          {hideSensitive ? (
            <EyeOff className="size-4" aria-hidden="true" />
          ) : (
            <Eye className="size-4" aria-hidden="true" />
          )}
        </Button>
        <Button
          variant="outline"
          size="icon"
          className="size-9"
          aria-label="Lock Kassiber"
          title="Lock Kassiber (Cmd/Ctrl+L)"
          onClick={onLock}
        >
          <LockKeyhole className="size-4" aria-hidden="true" />
        </Button>
      </div>
    </header>
  );
}

function LockScreen({
  reason,
  onUnlock,
  onReset,
}: {
  reason?: string;
  onUnlock: (
    passphrase: string,
  ) => Promise<{ ok: boolean; error?: string | null }>;
  onReset: () => void;
}) {
  const [passphrase, setPassphrase] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement | null>(null);

  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      const result = await onUnlock(passphrase);
      if (!result.ok) {
        setError(result.error ?? "Passphrase did not unlock this session.");
        setPassphrase("");
        inputRef.current?.focus();
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/95 px-4 backdrop-blur-sm">
      <form
        className="w-full max-w-sm rounded-lg border bg-card p-5 shadow-xl"
        onSubmit={(event) => {
          void submit(event);
        }}
      >
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <LockKeyhole className="size-5" aria-hidden="true" />
          </div>
          <div>
            <h2 className="text-base font-semibold">
              Database passphrase required
            </h2>
            <p className="m-0 text-xs text-muted-foreground">
              {reason ?? "Enter the database passphrase to unlock."}
            </p>
          </div>
        </div>
        <div className="mt-5 space-y-2">
          <label
            htmlFor="lock-passphrase"
            className="text-sm font-medium text-foreground"
          >
            Passphrase
          </label>
          <Input
            id="lock-passphrase"
            ref={inputRef}
            type="password"
            autoComplete="current-password"
            value={passphrase}
            onChange={(event) => setPassphrase(event.target.value)}
            disabled={submitting}
          />
          {error && <p className="m-0 text-xs text-destructive">{error}</p>}
        </div>
        <Button className="mt-5 w-full" type="submit" disabled={submitting}>
          {submitting ? "Unlocking..." : "Unlock"}
        </Button>
        <Button
          className="mt-2 w-full"
          type="button"
          variant="ghost"
          disabled={submitting}
          onClick={onReset}
        >
          Back to setup
        </Button>
      </form>
    </div>
  );
}
