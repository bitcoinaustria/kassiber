import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import {
  Link,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import {
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  BadgeCheck,
  BarChart3,
  Bell,
  BookOpen,
  Bug,
  ChevronRight,
  ChevronsUpDown,
  ClipboardList,
  Database,
  Eye,
  EyeOff,
  FileSearch,
  Fingerprint,
  Gauge,
  Heart,
  History,
  LifeBuoy,
  LockKeyhole,
  LogOut,
  MessageSquareText,
  Moon,
  Search,
  Server,
  Settings,
  ShieldAlert,
  Sun,
  SunMoon,
  TerminalSquare,
  User,
  Users,
  Wallet,
  WalletCards,
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
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
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
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";
import type { AppNotification, ThemePreference } from "@/store/ui";
import {
  DAEMON_AUTH_REQUIRED_EVENT,
  daemonMutationKey,
  formatDaemonEnvelopeError,
  shouldHandleDaemonAuthRequiredEvent,
  useDaemon,
} from "@/daemon/client";
import {
  activateImportProject,
  canUseTouchIdPassphraseUnlock,
  clearImportProject,
  getTransport,
  isImportProjectActive,
  storeTouchIdPassphrase,
  touchIdPassphraseStatus,
  unlockTouchIdPassphrase,
} from "@/daemon/transport";
import type { TouchIdPassphraseStatus } from "@/daemon/transport";
import {
  lockScreenConfig,
  shouldLockEncryptedWorkspaceOnLaunch,
  shouldStoreTouchIdPassphrase,
  shouldUseDaemonUnlock,
} from "@/lib/appLock";
import { cn } from "@/lib/utils";
import {
  clearSessionUnlockPassphrase,
  hasSessionUnlockPassphrase,
  setSessionUnlockPassphrase,
  verifySessionUnlockPassphrase,
} from "@/store/sessionLock";
import type { OverviewSnapshot } from "@/mocks/seed";
import type { ProfilesSnapshot } from "@/mocks/profiles";
import { AssistantSessionProvider } from "@/components/ai/AssistantSessionProvider";
import type { AssistantReturnPath } from "@/components/ai/assistantSession";
import kLedgerMarkUrl from "@/assets/k-ledger-mark-transparent.svg";
import { APP_COMMIT, APP_VERSION } from "@/lib/appVersion";
import { ScreenAssistantMockup } from "./ScreenAssistantMockup";
import { PreAlphaBanner } from "./PreAlphaBanner";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { BookSwitcherPopover } from "./BookSwitcherPopover";
import {
  buildAppSearchResults,
  isLikelyTransactionLookupQuery,
  isSearchResultActivatable,
  searchResultForActivation,
  type RankedSearchResult,
  type ResolvedTransactionLookup,
  type SearchActionId,
  type SearchIconKey,
} from "./search";

import {
  dispatchMenuIntent,
  type AppRoutePath,
  type NativeMenuPayload,
} from "./menuIntent";

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

type NotificationItem = Omit<AppNotification, "createdAt"> & {
  createdAt?: string;
  to?: AppRoutePath;
  action?: "process-journals";
  actionLabel?: string;
};

const APP_COMMIT_SHORT = APP_COMMIT ? APP_COMMIT.slice(0, 7) : "unknown";
const NATIVE_MENU_EVENT = "kassiber:intent";
const topNavIconButtonClassName =
  "size-8 text-sidebar-foreground/75 hover:bg-sidebar-accent hover:text-sidebar-foreground";

function notificationProgressValue(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}
const appMainClassName =
  "relative min-h-0 w-full flex-1 overflow-auto overscroll-contain bg-background text-zinc-950 dark:text-zinc-50";

const NAV_GROUPS: NavGroup[] = [
  {
    title: "Main",
    items: [
      { label: "Overview", icon: Gauge, href: "/overview" },
      { label: "Transactions", icon: ClipboardList, href: "/transactions" },
      { label: "Activity", icon: History, href: "/activity" },
      { label: "Wallets", icon: WalletCards, href: "/connections" },
      { label: "Reports", icon: BarChart3, href: "/reports" },
      { label: "Assistant", icon: MessageSquareText, href: "/assistant" },
    ],
  },
  {
    title: "Review",
    items: [
      { label: "Quarantine", icon: ShieldAlert, href: "/quarantine" },
      { label: "Source Funds", icon: BadgeCheck, href: "/source-of-funds" },
      { label: "Swaps & Transfers", icon: ArrowLeftRight, href: "/swaps" },
      { label: "Ledger", icon: BookOpen, href: "/journals" },
    ],
  },
];

const ROUTE_META: Array<[string, RouteMeta]> = [
  [
    "/activity",
    {
      title: "Activity",
      icon: History,
      searchLabel: "Search activity",
      searchPlaceholder: "Search transactions, wallets...",
    },
  ],
  [
    "/connections/",
    {
      title: "Wallet Detail",
      icon: Wallet,
      searchLabel: "Search wallets",
      searchPlaceholder: "Search wallets, books...",
    },
  ],
  [
    "/connections",
    {
      title: "Wallets",
      icon: Wallet,
      searchLabel: "Search wallets",
      searchPlaceholder: "Search wallets, imports, backends...",
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
      title: "Ledger",
      icon: BookOpen,
      searchLabel: "Search ledger",
      searchPlaceholder: "Search entries, wallets, assets...",
    },
  ],
  [
    "/logs",
    {
      title: "Logs",
      icon: TerminalSquare,
      searchLabel: "Search logs",
      searchPlaceholder: "Search daemon errors, logs...",
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
    "/quarantine",
    {
      title: "Quarantine",
      icon: ShieldAlert,
      searchLabel: "Search quarantine",
      searchPlaceholder: "Search issue, account, source...",
    },
  ],
  [
    "/transfers",
    {
      title: "Swaps & Transfers",
      icon: ArrowLeftRight,
      searchLabel: "Search swaps and transfers",
      searchPlaceholder: "Search wallet, asset pair, txid...",
    },
  ],
  [
    "/swaps",
    {
      title: "Swaps & Transfers",
      icon: ArrowLeftRight,
      searchLabel: "Search swaps and transfers",
      searchPlaceholder: "Search wallet, asset pair, txid...",
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

function nextSearchIndex(current: number, delta: number, total: number) {
  if (total <= 0) return 0;
  return (current + delta + total) % total;
}

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

const SEARCH_CATEGORY_LABELS: Record<RankedSearchResult["category"], string> = {
  action: "Action",
  page: "Page",
  report: "Report",
  review_item: "Review",
  setting: "Setting",
  transaction: "Transaction",
  wallet: "Wallet",
};

function searchResultIcon(result: RankedSearchResult) {
  const key = result.iconKey ?? result.category;
  return SEARCH_ICON_BY_KEY[key] ?? Search;
}

function exhaustiveSearchAction(actionId: never): never {
  throw new Error(`Unhandled search action: ${actionId}`);
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
  if (
    normalized.includes("error") ||
    normalized.includes("failed") ||
    normalized.includes("daemon")
  ) {
    return "/logs";
  }
  return undefined;
}

function assistantReturnPathFor(pathname: string): AssistantReturnPath {
  if (pathname.startsWith("/connections")) return "/connections";
  if (pathname === "/transactions") return "/transactions";
  if (pathname === "/reports") return "/reports";
  if (pathname === "/source-of-funds") return "/source-of-funds";
  if (pathname === "/books" || pathname === "/profiles") return "/books";
  if (pathname === "/journals") return "/journals";
  if (pathname === "/quarantine") return "/quarantine";
  if (pathname === "/logs" || pathname === "/diagnostics") return "/logs";
  if (pathname === "/settings") return "/settings";
  return "/overview";
}

export function AppShell() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const identity = useUiStore((s) => s.identity);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setAppLockPolicy = useUiStore((s) => s.setAppLockPolicy);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const addNotification = useUiStore((s) => s.addNotification);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const bumpDaemonSession = useUiStore((s) => s.bumpDaemonSession);
  const { syncAll, isSyncing } = useWalletSyncAction();
  const dataMode = useUiStore((s) => s.dataMode);
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const lockEncryptedWorkspaceOnLaunch = shouldLockEncryptedWorkspaceOnLaunch({
    encryptedWorkspace,
    requirePassphraseOnLaunch: appLockPolicy.requirePassphraseOnLaunch,
    hasSessionUnlock: hasSessionUnlockPassphrase(),
  });
  const importedProjectRoot = identity?.importedProject?.dataRoot ?? null;
  const touchIdDataRoot = importedProjectRoot;
  const touchIdPlatformSupported = canUseTouchIdPassphraseUnlock();
  const [importRootReady, setImportRootReady] = React.useState(
    () => !importedProjectRoot,
  );
  const [importRootError, setImportRootError] = React.useState<string | null>(
    null,
  );
  const [daemonAuthRequired, setDaemonAuthRequired] = React.useState(false);
  const [touchIdStatus, setTouchIdStatus] =
    React.useState<TouchIdPassphraseStatus | null>(null);
  const requiresDaemonUnlock = shouldUseDaemonUnlock({
    dataMode,
    hasIdentity: Boolean(identity),
    daemonAuthRequired,
  });
  const lockedScreen = lockScreenConfig({
    daemonAuthRequired,
    encryptedWorkspace,
  });
  const routerBusy = useRouterState({
    select: (s) => s.isLoading || s.isTransitioning || s.status === "pending",
  });
  const daemonFetchCount = useIsFetching({ queryKey: ["daemon"] });
  const [assistantCollapsed, setAssistantCollapsed] = React.useState(false);
  const [locked, setLocked] = React.useState(
    () => lockEncryptedWorkspaceOnLaunch,
  );
  const [touchIdAutoPromptPending, setTouchIdAutoPromptPending] =
    React.useState(() => lockEncryptedWorkspaceOnLaunch);
  const [assistantReturnPath, setAssistantReturnPath] =
    React.useState<AssistantReturnPath>("/overview");
  const mainRef = React.useRef<HTMLElement>(null);
  const launchLockApplied = React.useRef(false);
  const workspaceValidationApplied = React.useRef(false);
  const importedProjectActive = importedProjectRoot
    ? isImportProjectActive(importedProjectRoot)
    : true;
  const importRootBlocked = !importRootReady || !importedProjectActive;
  const daemonEnabled = !locked && !importRootBlocked;
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

  const refreshTouchIdStatus = React.useCallback(async () => {
    if (!encryptedWorkspace || !touchIdPlatformSupported) {
      setTouchIdStatus(null);
      return null;
    }
    try {
      const status = await touchIdPassphraseStatus(touchIdDataRoot);
      setTouchIdStatus(status);
      return status;
    } catch (error) {
      const status: TouchIdPassphraseStatus = {
        platform: "macos",
        available: false,
        configured: false,
        reason: error instanceof Error ? error.message : String(error),
      };
      setTouchIdStatus(status);
      return status;
    }
  }, [
    encryptedWorkspace,
    touchIdDataRoot,
    touchIdPlatformSupported,
  ]);

  const applyLock = React.useCallback((autoPromptTouchId: boolean) => {
    setTouchIdAutoPromptPending(autoPromptTouchId);
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
    setIdentity,
  ]);
  const lockApp = React.useCallback(() => applyLock(false), [applyLock]);
  const lockAppWithTouchIdAutoPrompt = React.useCallback(
    () => applyLock(true),
    [applyLock],
  );

  const unlockApp = React.useCallback(
    async (
      passphrase: string,
      options?: { rememberWithTouchId?: boolean },
    ): Promise<{ ok: boolean; error?: string | null }> => {
      if (requiresDaemonUnlock) {
        if (importRootBlocked) {
          return {
            ok: false,
            error:
              importRootError ??
              "Kassiber is still opening the selected local books folder.",
          };
        }
        bumpDaemonSession();
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
          setTouchIdAutoPromptPending(false);
          setLocked(false);
          const shouldRememberWithTouchId = shouldStoreTouchIdPassphrase({
            platformSupported: touchIdPlatformSupported,
            rememberWithTouchId: options?.rememberWithTouchId,
            touchIdStatusConfigured: touchIdStatus?.configured === true,
          });
          if (shouldRememberWithTouchId) {
            void storeTouchIdPassphrase(passphrase, touchIdDataRoot)
              .then((status) => {
                setTouchIdStatus(status);
                if (!status.configured) {
                  setAppLockPolicy({ touchIdUnlock: false });
                  addNotification({
                    title: "Touch ID unlock was not saved",
                    body: status.reason
                      ? `Touch ID unlock is not set up: ${status.reason}`
                      : "macOS Keychain did not report the saved passphrase.",
                    tone: "warning",
                  });
                  return;
                }
                if (options?.rememberWithTouchId === true) {
                  setAppLockPolicy({ touchIdUnlock: true });
                }
              })
              .catch((error: unknown) => {
                addNotification({
                  title: "Touch ID unlock was not saved",
                  body:
                    error instanceof Error
                      ? error.message
                      : "macOS Keychain did not accept the saved passphrase.",
                  tone: "warning",
                });
                void refreshTouchIdStatus();
              });
          }
          void queryClient.invalidateQueries({
            queryKey: ["daemon"],
          });
        } else if (envelope.kind === "auth_required") {
          setDaemonAuthRequired(true);
          clearSessionUnlockPassphrase();
          clearDaemonQueryCache();
          setLocked(true);
        }
        return {
          ok: unlocked,
          error:
            formatDaemonEnvelopeError(envelope) ??
            (envelope.kind === "auth_required"
              ? "Database passphrase is required."
              : null),
        };
      }

      const unlocked = await verifySessionUnlockPassphrase(passphrase);
      if (unlocked) {
        setTouchIdAutoPromptPending(false);
        setLocked(false);
      }
      return { ok: unlocked, error: null };
    },
    [
      addNotification,
      bumpDaemonSession,
      clearDaemonQueryCache,
      identity?.importedProject,
      importRootBlocked,
      importRootError,
      queryClient,
      refreshTouchIdStatus,
      requiresDaemonUnlock,
      setAppLockPolicy,
      touchIdDataRoot,
      touchIdPlatformSupported,
      touchIdStatus?.configured,
    ],
  );

  const unlockWithTouchId = React.useCallback(async () => {
    const unlocked = await unlockTouchIdPassphrase(touchIdDataRoot);
    if (!unlocked?.passphraseSecret) {
      await refreshTouchIdStatus();
      return {
        ok: false,
        error:
          "No Touch ID passphrase was found for these books. Unlock once with the passphrase to save it again.",
      };
    }
    return unlockApp(unlocked.passphraseSecret, {
      rememberWithTouchId: false,
    });
  }, [
    refreshTouchIdStatus,
    touchIdDataRoot,
    unlockApp,
  ]);

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

  const ensureWorkspaceForMenuAction = React.useCallback(() => {
    if (identity) return true;
    void navigate({ to: "/", replace: true });
    return false;
  }, [identity, navigate]);

  const isDaemonKindMutating = React.useCallback(
    (kind: string) =>
      queryClient.isMutating({ mutationKey: daemonMutationKey(dataMode, kind) }) >
      0,
    [dataMode, queryClient],
  );
  const { runJournalProcessing: runMenuJournalProcessing } =
    useJournalProcessingAction({
      beforeRun: ensureWorkspaceForMenuAction,
      notifyAlreadyRunning: true,
      notifyStart: true,
    });

  const runMenuWalletSync = React.useCallback(() => {
    if (!ensureWorkspaceForMenuAction()) return;
    if (
      isSyncing ||
      isDaemonKindMutating("ui.freshness.run") ||
      isDaemonKindMutating("ui.wallets.sync")
    ) {
      addNotification({
        title: "Book refresh already running",
        body: "Kassiber is already refreshing sources, rates, or journals.",
        tone: "info",
      });
      return;
    }
    syncAll();
  }, [
    addNotification,
    ensureWorkspaceForMenuAction,
    isDaemonKindMutating,
    isSyncing,
    syncAll,
  ]);

  React.useEffect(() => {
    if (identity) return;
    launchLockApplied.current = false;
    workspaceValidationApplied.current = false;
    void navigate({ to: "/", replace: true });
  }, [identity, navigate]);

  // A persisted ``identity`` survives across reinstalls of the same Tauri
  // bundle id because WKWebView localStorage is per-OS-user, not per-app-install.
  // After the daemon is reachable, confirm it actually has at least one
  // workspace; if not, drop the stale identity and bounce back to onboarding
  // instead of stranding the user on /overview with no data.
  React.useEffect(() => {
    if (dataMode !== "real") return;
    if (!daemonEnabled) return;
    if (identity?.importedProject) return;
    if (!identity) return;
    if (workspaceValidationApplied.current) return;
    workspaceValidationApplied.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const envelope = await getTransport("real").invoke<ProfilesSnapshot>({
          kind: "ui.profiles.snapshot",
        });
        if (cancelled) return;
        if (envelope.kind === "auth_required" || envelope.error) return;
        const workspaces = envelope.data?.workspaces ?? [];
        if (workspaces.length === 0) {
          resetLocalUiSession();
        }
      } catch {
        // A transport hiccup is not authoritative evidence of an empty
        // daemon; leave the persisted identity in place and let the user
        // retry or hit "Reset Welcome state" manually.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    dataMode,
    daemonEnabled,
    identity,
    resetLocalUiSession,
  ]);

  React.useEffect(() => {
    if (!importedProjectRoot) {
      setImportRootReady(true);
      setImportRootError(null);
      return;
    }

    if (isImportProjectActive(importedProjectRoot)) {
      setImportRootReady(true);
      setImportRootError(null);
      return;
    }

    let disposed = false;
    setImportRootReady(false);
    setImportRootError(null);
    clearDaemonQueryCache();
    clearSessionUnlockPassphrase();
    const nextLocked = shouldLockEncryptedWorkspaceOnLaunch({
      encryptedWorkspace,
      requirePassphraseOnLaunch: appLockPolicy.requirePassphraseOnLaunch,
      hasSessionUnlock: false,
    });
    setTouchIdAutoPromptPending(nextLocked);
    setLocked(nextLocked);
    void activateImportProject(importedProjectRoot)
      .then(() => {
        if (disposed) return;
        setImportRootReady(true);
        setImportRootError(null);
        setDaemonAuthRequired(false);
      })
      .catch((error: unknown) => {
        if (disposed) return;
        setImportRootReady(false);
        setImportRootError(
          error instanceof Error
            ? error.message
            : "Could not open the selected local books folder.",
        );
        setLocked(true);
      });

    return () => {
      disposed = true;
    };
  }, [
    clearDaemonQueryCache,
    encryptedWorkspace,
    importedProjectRoot,
    appLockPolicy.requirePassphraseOnLaunch,
  ]);

  React.useEffect(() => {
    const onAuthRequired = (event: Event) => {
      if (
        !shouldHandleDaemonAuthRequiredEvent(
          (event as CustomEvent).detail,
          useUiStore.getState().daemonSession,
        )
      ) {
        return;
      }
      setDaemonAuthRequired(true);
      clearSessionUnlockPassphrase();
      clearDaemonQueryCache();
      setTouchIdAutoPromptPending(true);
      setLocked(true);
    };

    window.addEventListener(DAEMON_AUTH_REQUIRED_EVENT, onAuthRequired);
    return () => {
      window.removeEventListener(DAEMON_AUTH_REQUIRED_EVENT, onAuthRequired);
    };
  }, [clearDaemonQueryCache]);

  React.useEffect(() => {
    if (!lockEncryptedWorkspaceOnLaunch) return;
    if (hasSessionUnlockPassphrase()) return;
    if (launchLockApplied.current) return;
    launchLockApplied.current = true;
    lockAppWithTouchIdAutoPrompt();
  }, [lockEncryptedWorkspaceOnLaunch, lockAppWithTouchIdAutoPrompt]);

  React.useEffect(() => {
    if (!locked) return;
    void refreshTouchIdStatus();
  }, [locked, refreshTouchIdStatus]);

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

    window.addEventListener("pagehide", lockApp);
    return () => {
      window.removeEventListener("pagehide", lockApp);
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

  React.useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let unlisten: (() => void) | null = null;

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<NativeMenuPayload>(NATIVE_MENU_EVENT, (event) => {
          const store = useUiStore.getState();
          // AppShell only handles workspace-scoped actions (lock, sync,
          // process-journals). Global actions (navigate, open-settings,
          // toggle-sensitive) flow through RootIntentListener at the
          // route-tree root so they work pre-workspace too. The "workspace"
          // scope filter prevents this listener from double-handling.
          dispatchMenuIntent(
            event.payload,
            {
              hasWorkspace: store.identity !== null,
              aiFeaturesEnabled: store.aiFeaturesEnabled,
              hideSensitive: store.hideSensitive,
              navigate: ({ to, hash }) => {
                void navigate({ to, hash: hash ?? undefined });
              },
              lockApp,
              setHideSensitive,
              decreaseAppScale: store.decreaseAppScale,
              increaseAppScale: store.increaseAppScale,
              resetAppScale: store.resetAppScale,
              runWalletSync: runMenuWalletSync,
              runJournalProcessing: runMenuJournalProcessing,
              addNotification,
              emitSettingsSection: (section) => {
                window.dispatchEvent(
                  new CustomEvent("kassiber:settings-section", {
                    detail: { section },
                  }),
                );
              },
            },
            "workspace",
          );
        }),
      )
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((error) => {
        console.warn("Could not attach Kassiber native menu listener", error);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [
    lockApp,
    navigate,
    runMenuJournalProcessing,
    runMenuWalletSync,
    aiFeaturesEnabled,
    addNotification,
    setHideSensitive,
  ]);

  React.useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let disposed = false;
    const hasWorkspace = Boolean(identity);
    void import("@tauri-apps/api/core")
      .then(({ invoke }) => {
        if (disposed) return;
        return invoke("set_menu_state", {
          aiFeaturesEnabled,
          hasWorkspace,
          locked,
        });
      })
      .catch((error) => {
        console.warn("Could not sync Kassiber native menu state", error);
      });
    return () => {
      disposed = true;
    };
  }, [aiFeaturesEnabled, identity, locked]);

  React.useEffect(() => {
    if (aiFeaturesEnabled || !isAssistantRoute) return;
    void navigate({ to: "/overview", replace: true });
  }, [aiFeaturesEnabled, isAssistantRoute, navigate]);

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
      const scrollableHeight = Math.max(1, main.scrollHeight - main.clientHeight);
      const scrolledProgress = main.scrollTop / scrollableHeight;
      setAssistantCollapsed(main.scrollTop > 96 && scrolledProgress > 0.04);
    };

    syncAssistantState();
    main.addEventListener("scroll", syncAssistantState, { passive: true });

    return () => {
      main.removeEventListener("scroll", syncAssistantState);
    };
  }, [locked, pathname]);

  if (!identity) return null;

  return (
    <TooltipProvider>
      <div className="flex h-svh flex-col overflow-hidden bg-sidebar">
        <PreAlphaBanner className="shrink-0" />
        <SidebarProvider className="min-h-0 flex-1 flex-col bg-sidebar">
          <a
            href="#app-main"
            className="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:text-foreground focus:ring-2 focus:ring-ring"
          >
            Skip to main content
          </a>
          <AppDashboardHeader
            meta={routeMeta}
            onLock={lockApp}
            daemonEnabled={daemonEnabled}
          />
          <div className="flex min-h-0 flex-1">
            <AppSidebar
              pathname={pathname}
              onLock={lockApp}
              daemonEnabled={daemonEnabled}
              aiFeaturesEnabled={aiFeaturesEnabled}
              developerToolsEnabled={developerToolsEnabled}
            />
            <div className="min-h-0 w-full overflow-hidden lg:pt-1.5 lg:pr-1.5 lg:pb-1.5">
              <div className="relative flex h-full w-full flex-col items-center justify-start overflow-hidden bg-background lg:rounded-tl-xl lg:rounded-tr-xl">
                {importRootBlocked ? (
                  <main
                    id="app-main"
                    ref={mainRef}
                    tabIndex={-1}
                    className={appMainClassName}
                  >
                    <ImportRootRestoreScreen
                      error={importRootError}
                      onReset={resetLocalUiSession}
                    />
                  </main>
                ) : locked ? (
                  <main
                    id="app-main"
                    ref={mainRef}
                    tabIndex={-1}
                    className={appMainClassName}
                  >
                    <LockScreen
                      reason={lockedScreen.reason}
                      passphraseRequired={lockedScreen.passphraseRequired}
                      onUnlock={unlockApp}
                      onTouchIdUnlock={unlockWithTouchId}
                      touchIdEnabled={appLockPolicy.touchIdUnlock}
                      touchIdPlatformSupported={touchIdPlatformSupported}
                      touchIdStatus={touchIdStatus}
                      autoTouchIdPrompt={
                        appLockPolicy.touchIdUnlock &&
                        touchIdAutoPromptPending
                      }
                      onReset={resetLocalUiSession}
                    />
                  </main>
                ) : (
                  aiFeaturesEnabled ? (
                    <AssistantSessionProvider returnPath={assistantReturnPath}>
                      <main
                        id="app-main"
                        ref={mainRef}
                        tabIndex={-1}
                        className={cn(
                          appMainClassName,
                          isAssistantRoute
                            ? "pb-0"
                            : "pb-[240px]",
                        )}
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
                  ) : (
                    <main
                      id="app-main"
                      ref={mainRef}
                      tabIndex={-1}
                      className={appMainClassName}
                    >
                      <RouteTransitionIndicator active={shellBusy} />
                      <Outlet />
                    </main>
                  )
                )}
              </div>
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
      <div className="h-full w-1/2 bg-primary/70 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto" />
    </div>
  );
}

function AppSidebar({
  pathname,
  onLock,
  daemonEnabled,
  aiFeaturesEnabled,
  developerToolsEnabled,
}: {
  pathname: string;
  onLock: () => void;
  daemonEnabled: boolean;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
}) {
  const navGroups = React.useMemo(
    () =>
      NAV_GROUPS.map((group) => ({
        ...group,
        items: group.items.filter(
          (item) => aiFeaturesEnabled || item.href !== "/assistant",
        ),
      })).filter((group) => group.items.length > 0),
    [aiFeaturesEnabled],
  );

  return (
    <Sidebar
      variant="sidebar"
      collapsible="icon"
      className="top-[4.5rem] h-[calc(100svh-4.5rem)] !border-r-0 group-data-[side=left]:!border-r-0"
    >
      <SidebarContent>
        {navGroups.map((group) => (
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
        <SidebarActions
          pathname={pathname}
          developerToolsEnabled={developerToolsEnabled}
        />
        <NavUser onLock={onLock} daemonEnabled={daemonEnabled} />
        <AppVersion />
      </SidebarFooter>
      <SidebarRail className="after:hidden" />
    </Sidebar>
  );
}

function SidebarActions({
  pathname,
  developerToolsEnabled,
}: {
  pathname: string;
  developerToolsEnabled: boolean;
}) {
  const dataMode = useUiStore((state) => state.dataMode);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const isRealData = dataMode === "real";
  const supportActive = pathname === "/diagnostics";

  return (
    <SidebarMenu>
      {developerToolsEnabled ? (
        <SidebarMenuItem>
          <SidebarMenuButton
            asChild
            isActive={pathname === "/logs"}
            tooltip="Logs"
          >
            <Link to="/logs">
              <TerminalSquare className="size-4" aria-hidden="true" />
              <span>Logs</span>
            </Link>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ) : null}
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
        <Collapsible asChild defaultOpen={supportActive} className="group/collapsible">
          <div>
            <CollapsibleTrigger asChild>
              <SidebarMenuButton isActive={supportActive} tooltip="Support">
                <LifeBuoy className="size-4" aria-hidden="true" />
                <span>Support</span>
                <ChevronRight className="ml-auto size-4 transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90 group-data-[collapsible=icon]:hidden" />
              </SidebarMenuButton>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <SidebarMenuSub>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton asChild>
                    <a
                      href="https://github.com/bitcoinaustria/kassiber/issues"
                      target="_blank"
                      rel="noreferrer"
                    >
                      <Bug className="size-3.5" aria-hidden="true" />
                      <span>Bug report</span>
                    </a>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton asChild>
                    <a href="#donate">
                      <Heart className="size-3.5" aria-hidden="true" />
                      <span>Donate sats</span>
                    </a>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              </SidebarMenuSub>
            </CollapsibleContent>
          </div>
        </Collapsible>
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
  const childActive = item.children?.some(
    (child) => pathname === child.href || pathname.startsWith(`${child.href}/`),
  );
  const active =
    pathname === item.href ||
    pathname.startsWith(`${item.href}/`) ||
    Boolean(childActive);
  const [open, setOpen] = React.useState(active);

  React.useEffect(() => {
    if (active) setOpen(true);
  }, [active]);

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
    <Collapsible
      asChild
      open={open}
      onOpenChange={setOpen}
      className="group/collapsible"
    >
      <SidebarMenuItem>
        <CollapsibleTrigger asChild>
          <SidebarMenuButton isActive={active} tooltip={item.label}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{item.label}</span>
            <ChevronRight className="ml-auto size-4 transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90 group-data-[collapsible=icon]:hidden" />
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
  const { state: sidebarState } = useSidebar();
  const navigate = useNavigate();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const dataMode = useUiStore((s) => s.dataMode);
  const appNotifications = useUiStore((s) => s.notifications);
  const clearNotifications = useUiStore((s) => s.clearNotifications);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const setDeferredConnectionSetup = useUiStore(
    (s) => s.setDeferredConnectionSetup,
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const [searchQuery, setSearchQuery] = React.useState("");
  const [searchOpen, setSearchOpen] = React.useState(false);
  const [bookSwitcherOpen, setBookSwitcherOpen] = React.useState(false);
  const [activeSearchIndex, setActiveSearchIndex] = React.useState(0);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const searchRootRef = React.useRef<HTMLDivElement>(null);
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const snapshot = data?.data;
  const shouldResolveTransaction = isLikelyTransactionLookupQuery(searchQuery);
  const resolvedTransaction = useDaemon<ResolvedTransactionLookup>(
    "ui.transactions.resolve",
    { query: searchQuery.trim() },
    { enabled: daemonEnabled && shouldResolveTransaction },
  );
  const searchResults = React.useMemo(
    () =>
      buildAppSearchResults({
        snapshot,
        query: searchQuery,
        aiFeaturesEnabled,
        developerToolsEnabled,
        resolvedTransaction: resolvedTransaction.data?.data ?? null,
        isResolvingTransaction:
          shouldResolveTransaction &&
          (resolvedTransaction.isFetching || resolvedTransaction.isLoading),
      }),
    [
      snapshot,
      searchQuery,
      aiFeaturesEnabled,
      developerToolsEnabled,
      resolvedTransaction.data?.data,
      resolvedTransaction.isFetching,
      resolvedTransaction.isLoading,
      shouldResolveTransaction,
    ],
  );
  const searchListId = React.useId();
  const searchActiveId = searchResults[activeSearchIndex]?.id
    ? `search-result-${searchResults[activeSearchIndex].id.replace(/[^a-zA-Z0-9_-]/g, "-")}`
    : undefined;
  const activateSearchAction = React.useCallback(
    (actionId: SearchActionId) => {
      switch (actionId) {
        case "process-journals":
          runJournalProcessing();
          return;
        case "add-wallet":
        case "import-btcpay":
          setDeferredConnectionSetup({
            sourceId: actionId === "import-btcpay" ? "btcpay" : "descriptor",
            reason: "Opened from global search",
          });
          void navigate({ to: "/connections" });
          return;
        default:
          exhaustiveSearchAction(actionId);
      }
    },
    [navigate, runJournalProcessing, setDeferredConnectionSetup],
  );
  const activateSearchResult = React.useCallback(
    (result: RankedSearchResult | undefined) => {
      if (!result) return;
      const actionId = result.action?.id;
      if (actionId) {
        setSearchOpen(false);
        setSearchQuery("");
        activateSearchAction(actionId);
        return;
      }

      const route = result.route;
      if (!route) return;
      setSearchOpen(false);
      setSearchQuery("");
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
      if (route.to === "/settings" && route.hash) {
        void navigate({ to: "/settings", hash: route.hash });
        window.dispatchEvent(
          new CustomEvent("kassiber:settings-section", {
            detail: { section: route.hash },
          }),
        );
        return;
      }
      void navigate({ to: route.to });
    },
    [activateSearchAction, navigate],
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

  const systemNotificationItems: NotificationItem[] = [
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "journals-stale",
            title: "Ledger needs processing",
            body: "Reports are not trusted until journal processing runs.",
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
      to:
        item.tone === "error"
          ? (developerToolsEnabled ? ("/logs" as const) : ("/settings" as const))
          : notificationRouteFor(item.title),
    })),
    ...systemNotificationItems,
  ];
  const notificationCount = notificationItems.filter(
    (item) =>
      item.tone !== "info" ||
      item.title.toLowerCase().includes("sync"),
  ).length;
  const bookLabel =
    snapshot?.status?.profile ?? snapshot?.status?.workspace ?? "Local books";
  const reviewCount = snapshot?.status?.quarantines ?? 0;
  const sidebarCollapsed = sidebarState === "collapsed";
  const needsJournals = Boolean(snapshot?.status?.needsJournals);
  const notificationAlertClassName =
    reviewCount > 0
      ? "border border-red-500/35 bg-red-500/10 text-red-700 hover:bg-red-500/15 hover:text-red-700 dark:text-red-300 dark:hover:text-red-300"
      : needsJournals
        ? "border border-amber-500/35 bg-amber-500/10 text-amber-700 hover:bg-amber-500/15 hover:text-amber-700 dark:text-amber-300 dark:hover:text-amber-300"
        : "";
  const notificationLabel =
    notificationCount > 0
      ? `Notifications (${notificationCount} active)`
      : "Notifications";

  return (
    <header
      className="grid h-12 w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-2 bg-sidebar px-2 text-sidebar-foreground md:grid-cols-[minmax(0,1fr)_minmax(10rem,28rem)_minmax(0,1fr)] 2xl:grid-cols-[minmax(0,1fr)_minmax(16rem,38rem)_minmax(0,1fr)]"
    >
      <div className="flex min-w-0 items-center gap-1.5 sm:gap-2">
        <Link
          to="/overview"
          aria-label="Kassiber overview"
          className={cn(
            "flex h-8 shrink-0 items-center rounded-md text-sidebar-foreground hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
            sidebarCollapsed ? "w-8 justify-center" : "gap-2 pr-1.5",
          )}
        >
          <span className="kledger-app-icon size-8 shrink-0">
            <img
              src={kLedgerMarkUrl}
              alt=""
              aria-hidden="true"
              className="kledger-app-icon__mark"
            />
          </span>
          <span
            className={cn(
              "hidden text-sm font-semibold leading-none sm:inline",
              sidebarCollapsed && "sm:hidden",
            )}
          >
            Kassiber
          </span>
        </Link>
        <SidebarTrigger
          className={cn(
            "size-8 shrink-0 rounded-md border border-sidebar-border/70 bg-sidebar-accent/35",
            topNavIconButtonClassName,
          )}
        />
        <div className="hidden items-center gap-0.5 sm:flex">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={topNavIconButtonClassName}
            aria-label="Back"
            onClick={() => window.history.back()}
          >
            <ArrowLeft className="size-4" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={topNavIconButtonClassName}
            aria-label="Forward"
            onClick={() => window.history.forward()}
          >
            <ArrowRight className="size-4" aria-hidden="true" />
          </Button>
        </div>
        <div className="min-w-0 pl-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="group flex min-w-0 items-center gap-1 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-sidebar-accent/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              aria-label={`Switch books. Current books: ${bookLabel}`}
              aria-haspopup="dialog"
              aria-expanded={bookSwitcherOpen}
              onClick={() => setBookSwitcherOpen(true)}
            >
              <span className="truncate text-sm font-semibold text-sidebar-foreground">
                {bookLabel}
              </span>
              <ChevronsUpDown
                className="hidden size-3.5 shrink-0 text-sidebar-foreground/55 transition-colors group-hover:text-sidebar-foreground/80 sm:block"
                aria-hidden="true"
              />
            </Button>
            <span className="hidden text-sidebar-foreground/35 xl:inline">
              /
            </span>
            <span className="hidden truncate text-sm text-sidebar-foreground/65 xl:inline">
              {meta.title}
            </span>
          </div>
        </div>
      </div>
      <BookSwitcherPopover
        open={bookSwitcherOpen}
        onClose={() => setBookSwitcherOpen(false)}
      />

      <div
        ref={searchRootRef}
        className="relative hidden w-full min-w-0 md:block"
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
            className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-sidebar-foreground/55"
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
            placeholder="Search pages, actions, transactions..."
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
                activateSearchResult(
                  searchResultForActivation(searchResults, activeSearchIndex) ??
                    undefined,
                );
              } else if (event.key === "Escape") {
                setSearchOpen(false);
                searchInputRef.current?.blur();
              }
            }}
            className="h-8 w-full border-sidebar-border/75 bg-background/10 pr-14 pl-9 text-sm text-sidebar-foreground shadow-none placeholder:text-sidebar-foreground/50 focus-visible:bg-background focus-visible:text-foreground focus-visible:placeholder:text-muted-foreground"
          />
          <kbd className="pointer-events-none absolute top-1/2 right-2 hidden h-5 -translate-y-1/2 items-center gap-1 rounded-md border border-sidebar-border bg-sidebar-accent px-1.5 font-mono text-[11px] font-semibold leading-none text-sidebar-foreground shadow-sm md:inline-flex">
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
                  const ResultIcon = searchResultIcon(result);
                  const activatable = isSearchResultActivatable(result);
                  return (
                    <button
                      key={result.id}
                      id={itemId}
                      type="button"
                      role="option"
                      aria-selected={active}
                      aria-disabled={!activatable}
                      onMouseDown={(event) => {
                        event.preventDefault();
                        activateSearchResult(result);
                      }}
                      onMouseEnter={() => setActiveSearchIndex(index)}
                      className={cn(
                        "grid w-full grid-cols-[auto_minmax(0,1fr)] gap-2 rounded-sm px-3 py-2 text-left text-sm",
                        active ? "bg-accent text-accent-foreground" : "",
                      )}
                    >
                      <span className="mt-0.5 flex size-7 items-center justify-center rounded-md border bg-background text-muted-foreground">
                        <ResultIcon className="size-3.5" aria-hidden="true" />
                      </span>
                      <span className="min-w-0">
                        <span className="flex min-w-0 items-center gap-2">
                          <span className="truncate font-medium">
                            {result.title}
                          </span>
                          <span className="shrink-0 rounded border px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                            {SEARCH_CATEGORY_LABELS[result.category]}
                          </span>
                        </span>
                        {result.subtitle ? (
                          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                            {result.subtitle}
                          </span>
                        ) : null}
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
      <div className="flex min-w-0 items-center justify-end gap-2">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "relative",
                topNavIconButtonClassName,
                notificationAlertClassName,
              )}
              aria-label={notificationLabel}
              title={notificationLabel}
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
                {item.progress ? (
                  <div className="px-2 pb-1">
                    <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                      <div
                        className={cn(
                          "h-full rounded-full bg-primary transition-[width] duration-300",
                          item.progress.indeterminate &&
                            "w-1/2 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
                        )}
                        style={
                          item.progress.indeterminate
                            ? undefined
                            : {
                                width: `${notificationProgressValue(item.progress.value)}%`,
                              }
                        }
                      />
                    </div>
                    {item.progress.label ? (
                      <div className="mt-1 text-[11px] text-muted-foreground">
                        {item.progress.label}
                      </div>
                    ) : null}
                  </div>
                ) : null}
                {item.action === "process-journals" ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="mt-1 h-7 w-full justify-center text-xs"
                    disabled={isProcessingJournals}
                    onClick={(event) => {
                      event.preventDefault();
                      runJournalProcessing();
                    }}
                  >
                    {isProcessingJournals
                      ? "Processing..."
                      : item.actionLabel}
                  </Button>
                ) : null}
              </div>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <CurrencyToggle />
        <ThemeMenu />
        <Button
          variant="ghost"
          size="icon"
          className={
            hideSensitive
              ? "size-8 bg-sidebar-accent text-sidebar-foreground hover:bg-sidebar-accent/85 hover:text-sidebar-foreground"
              : topNavIconButtonClassName
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
          variant="ghost"
          size="icon"
          className={topNavIconButtonClassName}
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

function ThemeMenu() {
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const Icon = theme === "dark" ? Moon : theme === "light" ? Sun : SunMoon;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={topNavIconButtonClassName}
          aria-label="Theme"
          title="Theme"
        >
          <Icon className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuLabel>Theme</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuRadioGroup
          value={theme}
          onValueChange={(value) => setTheme(value as ThemePreference)}
        >
          <DropdownMenuRadioItem value="system">
            <SunMoon className="size-4" aria-hidden="true" />
            System
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="light">
            <Sun className="size-4" aria-hidden="true" />
            Light
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon className="size-4" aria-hidden="true" />
            Dark
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CurrencyToggle() {
  const currency = useUiStore((state) => state.currency);
  const setCurrency = useUiStore((state) => state.setCurrency);
  const symbol = currency === "btc" ? "₿" : "€";
  const currentLabel = currency === "btc" ? "bitcoin" : "euro";
  const nextCurrency = currency === "btc" ? "eur" : "btc";
  const nextLabel = nextCurrency === "btc" ? "bitcoin" : "euro";

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className={topNavIconButtonClassName}
      aria-label={`Display currency is ${currentLabel}. Switch to ${nextLabel}.`}
      aria-pressed={currency === "btc"}
      title={`Display currency: ${currentLabel}. Switch to ${nextLabel}.`}
      onClick={() => setCurrency(nextCurrency)}
    >
      <span aria-hidden="true" className="text-sm font-semibold leading-none">
        {symbol}
      </span>
    </Button>
  );
}

function LockScreen({
  reason,
  passphraseRequired = true,
  onUnlock,
  onTouchIdUnlock,
  touchIdEnabled,
  touchIdPlatformSupported,
  touchIdStatus,
  autoTouchIdPrompt,
  onReset,
}: {
  reason?: string;
  passphraseRequired?: boolean;
  onUnlock: (
    passphrase: string,
    options?: { rememberWithTouchId?: boolean },
  ) => Promise<{ ok: boolean; error?: string | null }>;
  onTouchIdUnlock: () => Promise<{ ok: boolean; error?: string | null }>;
  touchIdEnabled: boolean;
  touchIdPlatformSupported: boolean;
  touchIdStatus: TouchIdPassphraseStatus | null;
  autoTouchIdPrompt: boolean;
  onReset: () => void;
}) {
  const [passphrase, setPassphrase] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);
  const [touchIdSubmitting, setTouchIdSubmitting] = React.useState(false);
  const autoTouchIdPrompted = React.useRef(false);
  const canEnrollTouchId =
    touchIdPlatformSupported &&
    passphraseRequired &&
    !touchIdEnabled &&
    touchIdStatus?.available !== false;
  const [enrollTouchId, setEnrollTouchId] = React.useState(
    () => touchIdEnabled && canEnrollTouchId,
  );
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const canUseTouchId =
    touchIdEnabled &&
    touchIdPlatformSupported &&
    passphraseRequired &&
    touchIdStatus?.available === true &&
    touchIdStatus.configured;
  React.useEffect(() => {
    if (passphraseRequired) inputRef.current?.focus();
  }, [passphraseRequired]);

  React.useEffect(() => {
    if (!canEnrollTouchId) {
      setEnrollTouchId(false);
    }
  }, [canEnrollTouchId]);

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      const result = await onUnlock(passphrase, {
        rememberWithTouchId: canEnrollTouchId ? enrollTouchId : undefined,
      });
      if (!result.ok) {
        setError(result.error ?? "Passphrase did not unlock this session.");
        setPassphrase("");
        if (passphraseRequired) inputRef.current?.focus();
      }
    } finally {
      setSubmitting(false);
    }
  };

  const submitTouchId = React.useCallback(async () => {
    if (touchIdSubmitting || submitting) return;
    let keepPending = false;
    setError(null);
    setTouchIdSubmitting(true);
    try {
      const result = await onTouchIdUnlock();
      if (!result.ok) {
        setError(result.error ?? "Touch ID did not unlock this session.");
      } else {
        keepPending = true;
      }
    } finally {
      if (!keepPending) {
        setTouchIdSubmitting(false);
      }
    }
  }, [onTouchIdUnlock, submitting, touchIdSubmitting]);

  React.useEffect(() => {
    if (!autoTouchIdPrompt || !canUseTouchId) return;
    if (autoTouchIdPrompted.current) return;
    autoTouchIdPrompted.current = true;
    void submitTouchId();
  }, [autoTouchIdPrompt, canUseTouchId, submitTouchId]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background px-4 text-foreground">
      <form
        className="w-full max-w-md rounded-lg border border-border bg-card p-5 text-card-foreground shadow-xl ring-1 ring-border/60"
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
              {passphraseRequired
                ? "Database passphrase required"
                : "Books locked"}
            </h2>
            <p className="m-0 text-xs text-muted-foreground">
              {reason ?? "Enter the database passphrase to unlock."}
            </p>
          </div>
        </div>
        {passphraseRequired && touchIdSubmitting ? (
          <div className="mt-5 flex items-center gap-3 rounded-md border bg-background p-3 text-sm text-muted-foreground">
            <Fingerprint className="size-4 text-foreground" aria-hidden="true" />
            <span>Unlocking with Touch ID...</span>
          </div>
        ) : passphraseRequired ? (
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
            {touchIdEnabled &&
            touchIdPlatformSupported &&
            touchIdStatus?.available === false ? (
              <p className="m-0 text-xs text-muted-foreground">
                {touchIdStatus.reason
                  ? `Touch ID unlock is unavailable: ${touchIdStatus.reason}`
                  : "Touch ID unlock is unavailable for this desktop session."}
              </p>
            ) : null}
            {touchIdEnabled &&
            touchIdPlatformSupported &&
            touchIdStatus?.available === true &&
            !touchIdStatus.configured ? (
              <p className="m-0 text-xs text-muted-foreground">
                {touchIdStatus.reason
                  ? `Touch ID unlock is not set up: ${touchIdStatus.reason}`
                  : "No Touch ID passphrase is saved for these books. Unlock once with the passphrase to save it again."}
              </p>
            ) : null}
            {canEnrollTouchId ? (
              <label
                htmlFor="lock-touch-id-enroll"
                className="flex items-center justify-between gap-3 rounded-md border bg-background p-3"
              >
                <span className="min-w-0">
                  <span className="block text-sm font-medium text-foreground">
                    Use Touch ID next time
                  </span>
                  <span className="block text-xs leading-5 text-muted-foreground">
                    Save this database passphrase in macOS Keychain behind
                    local user presence.
                  </span>
                </span>
                <Switch
                  id="lock-touch-id-enroll"
                  checked={enrollTouchId}
                  disabled={submitting}
                  onCheckedChange={setEnrollTouchId}
                />
              </label>
            ) : null}
          </div>
        ) : (
          error && (
            <p className="mt-5 rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {error}
            </p>
          )
        )}
        {canUseTouchId ? (
          <Button
            className="mt-5 w-full"
            type="button"
            variant="outline"
            disabled={submitting || touchIdSubmitting}
            onClick={() => {
              void submitTouchId();
            }}
          >
            <Fingerprint className="size-4" aria-hidden="true" />
            {touchIdSubmitting
              ? "Waiting for Touch ID..."
              : "Unlock with Touch ID"}
          </Button>
        ) : null}
        <Button
          className="mt-5 w-full"
          type="submit"
          disabled={submitting || touchIdSubmitting}
        >
          {submitting
            ? "Unlocking..."
            : passphraseRequired
              ? "Unlock"
              : "Open books"}
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

function ImportRootRestoreScreen({
  error,
  onReset,
}: {
  error: string | null;
  onReset: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background px-4 text-foreground">
      <div className="w-full max-w-sm rounded-lg border border-border bg-card p-5 text-card-foreground shadow-xl ring-1 ring-border/60">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Database className="size-5" aria-hidden="true" />
          </div>
          <div>
            <h2 className="text-base font-semibold">Opening local books</h2>
            <p className="m-0 text-xs text-muted-foreground">
              Restoring the selected Kassiber data root.
            </p>
          </div>
        </div>
        {error ? (
          <>
            <p className="mt-4 text-xs text-destructive">{error}</p>
            <Button className="mt-5 w-full" type="button" onClick={onReset}>
              Back to setup
            </Button>
          </>
        ) : (
          <div className="mt-5 h-1.5 overflow-hidden rounded-full bg-muted">
            <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
          </div>
        )}
      </div>
    </div>
  );
}
