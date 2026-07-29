import {
  useIsFetching,
  useIsMutating,
  useQueryClient,
} from "@tanstack/react-query";
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
  ChevronDown,
  ChevronRight,
  ChevronsUpDown,
  ClipboardList,
  Database,
  Download,
  Eye,
  EyeOff,
  Fingerprint,
  Folder,
  Gauge,
  Heart,
  History,
  LifeBuoy,
  LockKeyhole,
  LogOut,
  MessageSquareText,
  Moon,
  Plane,
  Plus,
  RefreshCw,
  RotateCcw,
  Route,
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
  X,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

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
  useSidebar,
} from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { bookIdentityKey, useUiStore } from "@/store/ui";
import type { AppNotification, Identity, ThemePreference } from "@/store/ui";
import { BOOK_REFRESH_PROGRESS_ID } from "@/lib/syncProgress";
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
  forgetTouchIdPassphrase,
  getTransport,
  isImportProjectActive,
  noteActiveImportProject,
  openExternalUrl,
  storeTouchIdPassphrase,
  touchIdPassphraseStatus,
  unlockTouchIdPassphrase,
} from "@/daemon/transport";
import type { TouchIdPassphraseStatus } from "@/daemon/transport";
import {
  canEnrollTouchIdPassphrase,
  lockScreenConfig,
  shouldAutoPromptTouchId,
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
import {
  backendRowToSettingsBackend,
  backendsForLayer,
  type BackendSettingsData,
} from "@/components/kb/settings/SettingsModel";
import { SettingsSidebarNav } from "@/components/kb/settings/SettingsNavigation";
import {
  DEFAULT_SETTINGS_SECTION_ID,
  settingsSectionForPathname,
  type SettingsSectionId,
} from "@/components/kb/settingsSections";
import { ShellSearch } from "@/components/kb/shell/ShellSearch";
import {
  LedgerStageBand,
  SidebarStageBackdrop,
} from "@/components/kb/shell/SidebarStageBackdrop";
import { AssistantSessionProvider } from "@/components/ai/AssistantSessionProvider";
import type { AssistantScreenContext } from "@/components/ai/assistantSession";
import { assistantScreenContextFor } from "@/components/ai/assistantScreenContext";
import { RouteErrorBoundary } from "@/components/AppErrorBoundary";
import {
  canCheckAppUpdates,
  runManualAppUpdateCheck,
} from "@/lib/appUpdate";
import { APP_COMMIT, APP_VERSION } from "@/lib/appVersion";
import { appWorkflowHotkeyAction } from "@/lib/appWorkflowHotkeys";
import {
  startDaemonLogBridge,
  stopDaemonLogBridge,
} from "@/lib/daemonLogBridge";
import {
  classifyDaemonFreshnessEvent,
  DAEMON_EVENT_CHANNEL,
} from "@/lib/daemonFreshnessEvent";
import { safeTauriUnlisten } from "@/lib/tauriUnlisten";
import {
  dataModeForActiveBackend,
  dataModeLabelKey,
} from "@/components/kb/dataMode";
import { isTypingTarget } from "@/lib/keymap";
import { FirstSyncCard } from "./FirstSyncCard";
import { AssistantDock } from "./AssistantDock";
import { PreAlphaBanner } from "./PreAlphaBanner";
import { nextAssistantDockCollapsed } from "./assistantDockLayout";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { BookSwitcherPopover } from "./BookSwitcherPopover";
import { NetworkStatusIndicator } from "./NetworkStatusIndicator";
import {
  routeProgressFromActiveMaintenance,
  routeProgressFromNotifications,
  type RouteProgressState,
} from "./progressIndicator";
import {
  dispatchMenuIntent,
  type AppRoutePath,
  type NativeMenuPayload,
} from "./menuIntent";
import { notificationTarget } from "./notificationRouting";
import { shouldHideNotificationProgressLabel } from "./notificationDisplay";
import { planHeaderRefresh } from "./headerRefresh";

// `labelKey` indexes the `nav` namespace (book.*); keep `href` as the stable id.
type NavItem = {
  labelKey: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  href: AppRoutePath;
  children?: NavItem[];
};

// `titleKey` indexes the `nav` namespace (section.*).
type NavGroup = {
  titleKey: string;
  items: NavItem[];
};

// Cheap unresolved-item counts for the active book, surfaced as side-nav hints
// (ui.review.badges). `swaps` is null until the transfer matcher has run once.
type ReviewBadgesSnapshot = {
  quarantine: number;
  journals_needs_processing: boolean;
  swaps: number | null;
};

type ProjectCatalogEntry = {
  id: string;
  name: string;
  path: string;
  data_root: string;
  database: string;
  encrypted: boolean;
  last_opened_at?: string | null;
  selected?: boolean;
};

type ProjectsListSnapshot = {
  selected_project_id: string | null;
  projects: ProjectCatalogEntry[];
};

type ProjectSelectSnapshot = {
  project: ProjectCatalogEntry;
  status?: {
    current_workspace?: string | null;
    current_profile?: string | null;
    database_encrypted?: boolean;
  };
};

type ProjectIdentity = Identity & {
  importedProject: NonNullable<Identity["importedProject"]>;
};

type NavBadgeTone = "blocker" | "review";

// A resolved hint for one nav item. `count: null` renders a presence-only dot
// (e.g. "journals need processing"); a number renders a count pill.
type NavBadge = {
  count: number | null;
  tone: NavBadgeTone;
  labelKey: string;
};

// Mirror the notification bell's existing severity language (see
// notificationAlertClassName) so the same concept reads the same everywhere:
// red = blocks a correct report (quarantine), amber = needs review/processing
// (swaps, journals). Soft-tint pills tuned per theme so the count clears WCAG AA
// on both the near-white (light) and near-black (dark) sidebar.
const NAV_BADGE_PILL_TONE: Record<NavBadgeTone, string> = {
  blocker: "bg-red-500/15 text-red-700 dark:bg-red-400/15 dark:text-red-300",
  review:
    "bg-amber-500/15 text-amber-800 dark:bg-amber-400/15 dark:text-amber-300",
};

// Dots reuse each tone's pill-text shade: solid fills need a darker tint in
// light mode to stay visible on the near-white sidebar (amber-500 was only
// ~2:1), and these shades are already AA-verified for the pills.
const NAV_BADGE_DOT_TONE: Record<NavBadgeTone, string> = {
  blocker: "bg-red-700 dark:bg-red-300",
  review: "bg-amber-800 dark:bg-amber-300",
};

// `titleKey` indexes `nav:book.*` for the breadcrumb title; the search keys
// index `chrome:routeMeta.*`. Keep the literal route prefixes as stable lookups.
type RouteMeta = {
  titleKey: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  searchKey: string;
};

type NotificationItem = Omit<AppNotification, "createdAt"> & {
  createdAt?: string;
  to?: AppRoutePath;
  action?: "process-journals";
  actionLabel?: string;
};

const APP_COMMIT_SHORT = APP_COMMIT ? APP_COMMIT.slice(0, 7) : "unknown";
const APP_IS_DEV_BUILD = APP_VERSION === "dev";
const NATIVE_MENU_EVENT = "kassiber:intent";
const ACTIVE_PROGRESS_CLEAR_GRACE_MS = 750;
/*
 * T3Code's ghost icon-button recipe, for the controls that float over the
 * content panel: no chrome at rest, a `--accent` fill on hover, a muted glyph
 * against foreground-coloured text, and `rounded-lg` rather than a pill.
 */
const shellIconButtonClassName =
  "size-8 rounded-lg border border-transparent text-foreground hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background [&>svg]:text-muted-foreground hover:[&>svg]:text-foreground";
/*
 * T3Code's nav-row recipe: rows sit quiet in a muted foreground with a faint
 * hover fill, and the current page lifts onto its own surface. Both tones come
 * from `--sidebar-row-*` so hover and active never collapse into one colour.
 */
/* Same recipe on the nav surface, where hover/glyph read from `sidebar-*`. */
const navIconButtonClassName =
  // Sized off the same variables as the nav rows: 8 tall next to an h-8 row when
  // expanded, and the rail's own icon metrics when collapsed, so the collapse
  // trigger reads as one of the sidebar's buttons rather than a stray control.
  "size-8 shrink-0 rounded-md border border-transparent text-sidebar-foreground hover:bg-sidebar-row-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-sidebar [&>svg]:text-sidebar-muted-foreground hover:[&>svg]:text-sidebar-foreground group-data-[collapsible=icon]:size-(--sidebar-icon-button) group-data-[collapsible=icon]:[&>svg]:size-(--sidebar-icon-glyph)";
const navRowClassName =
  "h-8 gap-2 rounded-md text-sm font-medium text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground data-[active=true]:bg-sidebar-row-active data-[active=true]:text-sidebar-foreground";
const navSubRowClassName =
  "text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground data-[active=true]:bg-sidebar-row-active data-[active=true]:text-sidebar-foreground";

/**
 * Click handler for a submenu row's trigger, for the collapsed rail.
 *
 * Collapsed, `SidebarMenuSub` is `hidden`, so a submenu row had nowhere to put
 * its children and the click read as dead. Expand the nav and open the submenu
 * instead. `preventDefault` is what keeps it open: Radix runs the trigger's own
 * handler first and skips its toggle when the event was default-prevented, so
 * the submenu is not immediately closed again by the same click.
 */
function useRailSubmenuTrigger(setSubmenuOpen: (open: boolean) => void) {
  const { state, isMobile, setOpen: setNavOpen } = useSidebar();
  const collapsedRail = state === "collapsed" && !isMobile;
  return (event: React.MouseEvent) => {
    if (!collapsedRail) return;
    event.preventDefault();
    setNavOpen(true);
    setSubmenuOpen(true);
  };
}

function appCanStartTouchIdPrompt() {
  if (typeof document === "undefined") {
    return { appVisible: true, windowFocused: true };
  }
  const appVisible = document.visibilityState === "visible";
  const windowFocused =
    typeof document.hasFocus !== "function" || document.hasFocus();
  return { appVisible, windowFocused };
}

function foregroundTouchIdAutoPrompt(autoPromptRequested: boolean) {
  const { appVisible, windowFocused } = appCanStartTouchIdPrompt();
  return shouldAutoPromptTouchId({
    autoPromptRequested,
    canUseTouchId: true,
    appVisible,
    windowFocused,
  });
}

function notificationProgressValue(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

const appMainClassName =
  "relative min-h-0 w-full flex-1 overflow-auto overscroll-contain bg-background text-foreground";

const NAV_GROUPS: NavGroup[] = [
  {
    titleKey: "section.main",
    items: [
      { labelKey: "book.overview", icon: Gauge, href: "/overview" },
      { labelKey: "book.transactions", icon: ClipboardList, href: "/transactions" },
      { labelKey: "book.wallets", icon: WalletCards, href: "/connections" },
      { labelKey: "book.reports", icon: BarChart3, href: "/reports" },
      { labelKey: "book.assistant", icon: MessageSquareText, href: "/assistant" },
    ],
  },
  {
    titleKey: "section.review",
    items: [
      { labelKey: "book.quarantine", icon: ShieldAlert, href: "/quarantine" },
      { labelKey: "book.reconcile", icon: Fingerprint, href: "/reconcile" },
      { labelKey: "book.sourceFunds", icon: BadgeCheck, href: "/source-of-funds" },
      { labelKey: "book.custodyGaps", icon: Route, href: "/custody-gaps" },
      { labelKey: "book.swaps", icon: ArrowLeftRight, href: "/swaps" },
      { labelKey: "book.ledger", icon: BookOpen, href: "/journals" },
    ],
  },
];

// `titleKey` is a fully-qualified i18n key (nav:* for breadcrumb titles that
// reuse the sidebar names, chrome:routeMeta.* for shell-only titles).
// `searchKey` is a chrome:routeMeta.* prefix resolved to `.label` / `.placeholder`.
const ROUTE_META: Array<[string, RouteMeta]> = [
  [
    "/activity",
    {
      titleKey: "nav:book.activity",
      icon: History,
      searchKey: "routeMeta.activity",
    },
  ],
  [
    "/connections/",
    {
      titleKey: "routeMeta.walletDetail.title",
      icon: Wallet,
      searchKey: "routeMeta.walletDetail",
    },
  ],
  [
    "/connections",
    {
      titleKey: "nav:book.wallets",
      icon: Wallet,
      searchKey: "routeMeta.wallets",
    },
  ],
  [
    "/books/",
    {
      titleKey: "routeMeta.booksOverview.title",
      icon: Users,
      searchKey: "routeMeta.booksOverview",
    },
  ],
  [
    "/books",
    {
      titleKey: "routeMeta.books.title",
      icon: Users,
      searchKey: "routeMeta.books",
    },
  ],
  [
    "/journals",
    {
      titleKey: "nav:book.ledger",
      icon: BookOpen,
      searchKey: "routeMeta.ledger",
    },
  ],
  [
    "/imports",
    {
      titleKey: "routeMeta.imports.title",
      icon: WalletCards,
      searchKey: "routeMeta.imports",
    },
  ],
  [
    "/egress",
    {
      titleKey: "nav:book.egress",
      icon: Plane,
      searchKey: "routeMeta.egress",
    },
  ],
  [
    "/logs",
    {
      titleKey: "nav:book.logs",
      icon: TerminalSquare,
      searchKey: "routeMeta.logs",
    },
  ],
  [
    "/settings",
    {
      titleKey: "nav:book.settings",
      icon: Settings,
      searchKey: "routeMeta.settings",
    },
  ],
  [
    "/reports",
    {
      titleKey: "nav:book.reports",
      icon: BarChart3,
      searchKey: "routeMeta.reports",
    },
  ],
  [
    "/privacy-mirror",
    {
      titleKey: "nav:book.privacyMirror",
      icon: Eye,
      searchKey: "routeMeta.privacyMirror",
    },
  ],
  [
    "/exit-tax",
    {
      titleKey: "nav:book.exitTax",
      icon: Plane,
      searchKey: "routeMeta.exitTax",
    },
  ],
  [
    "/source-of-funds",
    {
      titleKey: "routeMeta.sourceOfFunds.title",
      icon: BadgeCheck,
      searchKey: "routeMeta.sourceOfFunds",
    },
  ],
  [
    "/quarantine",
    {
      titleKey: "nav:book.quarantine",
      icon: ShieldAlert,
      searchKey: "routeMeta.quarantine",
    },
  ],
  [
    "/reconcile",
    {
      titleKey: "nav:book.reconcile",
      icon: Fingerprint,
      searchKey: "routeMeta.reconcile",
    },
  ],
  [
    "/custody-gaps",
    {
      titleKey: "nav:book.custodyGaps",
      icon: Route,
      searchKey: "routeMeta.custodyGaps",
    },
  ],
  [
    "/swaps",
    {
      titleKey: "nav:book.swaps",
      icon: ArrowLeftRight,
      searchKey: "routeMeta.swaps",
    },
  ],
  [
    "/transactions",
    {
      titleKey: "nav:book.transactions",
      icon: ClipboardList,
      searchKey: "routeMeta.transactions",
    },
  ],
  [
    "/assistant",
    {
      titleKey: "nav:book.assistant",
      icon: MessageSquareText,
      searchKey: "routeMeta.assistant",
    },
  ],
  [
    "/overview",
    {
      titleKey: "nav:book.overview",
      icon: Gauge,
      searchKey: "routeMeta.overview",
    },
  ],
];

function identityFromProject(
  project: ProjectCatalogEntry,
  status?: ProjectSelectSnapshot["status"],
): ProjectIdentity {
  const encrypted = Boolean(status?.database_encrypted ?? project.encrypted);
  return {
    name: status?.current_profile ?? project.name,
    workspace: status?.current_workspace ?? project.name,
    profile: status?.current_profile ?? project.name,
    country: "Generic",
    encrypted,
    databaseMode: encrypted ? "sqlcipher" : "plaintext",
    importedProject: {
      stateRoot: project.path,
      dataRoot: project.data_root,
      database: project.database,
    },
  };
}

export function AppShell() {
  const { t } = useTranslation(["chrome", "overview"]);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const routeSearch = useRouterState({ select: (s) => s.location.search });
  const identity = useUiStore((s) => s.identity);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setAppLockPolicy = useUiStore((s) => s.setAppLockPolicy);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const setDeferredConnectionSetup = useUiStore(
    (s) => s.setDeferredConnectionSetup,
  );
  const addNotification = useUiStore((s) => s.addNotification);
  const appNotifications = useUiStore((s) => s.notifications);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const assistantDockAutoHide = useUiStore((s) => s.assistantDockAutoHide);
  const assistantDockPosition = useUiStore((s) => s.assistantDockPosition);
  const assistantDockMinimized = useUiStore((s) => s.assistantDockMinimized);
  const assistantDockExpanded = useUiStore((s) => s.assistantDockExpanded);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const preAlphaBannerVisible = useUiStore((s) => s.preAlphaBannerVisible);
  const bumpDaemonSession = useUiStore((s) => s.bumpDaemonSession);
  const activeMaintenanceProgress = useUiStore(
    (s) => s.activeMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (s) => s.clearActiveMaintenanceProgress,
  );
  const { syncAll, isSyncing } = useWalletSyncAction();
  const dataMode = useUiStore((s) => s.dataMode);
  const freshnessRunsInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.freshness.run"),
  });
  const walletSyncsInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.wallets.sync"),
  });
  const journalRunsInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.journals.process"),
  });
  const marketRateRunsInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.rates.latest"),
  });
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const lockEncryptedWorkspaceOnLaunch = shouldLockEncryptedWorkspaceOnLaunch({
    encryptedWorkspace,
    hasSessionUnlock: hasSessionUnlockPassphrase(),
  });
  const [daemonAuthRequired, setDaemonAuthRequired] = React.useState(false);
  const [pendingProjectUnlock, setPendingProjectUnlock] =
    React.useState<ProjectCatalogEntry | null>(null);
  const importedProjectRoot = identity?.importedProject?.dataRoot ?? null;
  const touchIdDataRoot = pendingProjectUnlock?.data_root ?? importedProjectRoot;
  const touchIdPlatformSupported = canUseTouchIdPassphraseUnlock();
  const [importRootReady, setImportRootReady] = React.useState(
    () => !importedProjectRoot,
  );
  const [importRootError, setImportRootError] = React.useState<string | null>(
    null,
  );
  const [touchIdStatus, setTouchIdStatus] =
    React.useState<TouchIdPassphraseStatus | null>(null);
  const requiresDaemonUnlock = shouldUseDaemonUnlock({
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
  const [assistantDockSuppressed, setAssistantDockSuppressed] =
    React.useState(false);
  const [locked, setLocked] = React.useState(
    () => lockEncryptedWorkspaceOnLaunch,
  );
  const [touchIdAutoPromptPending, setTouchIdAutoPromptPending] =
    React.useState(() =>
      foregroundTouchIdAutoPrompt(lockEncryptedWorkspaceOnLaunch),
    );
  const [assistantScreenContext, setAssistantScreenContext] =
    React.useState<AssistantScreenContext>(() =>
      assistantScreenContextFor("/overview"),
    );
  const mainRef = React.useRef<HTMLElement>(null);
  const launchLockApplied = React.useRef(false);
  const workspaceValidationApplied = React.useRef(false);
  const importedProjectActive = importedProjectRoot
    ? isImportProjectActive(importedProjectRoot)
    : true;
  const importRootBlocked = !importRootReady || !importedProjectActive;
  const daemonEnabled = !locked && !importRootBlocked;
  const shellProgress =
    routeProgressFromActiveMaintenance(activeMaintenanceProgress) ??
    routeProgressFromNotifications(appNotifications);
  const shellBusy =
    routerBusy || daemonFetchCount > 0 || Boolean(shellProgress);
  const firstSyncDone = useUiStore((s) => s.firstSyncDone);
  const bookKey = React.useMemo(() => bookIdentityKey(identity), [identity]);
  const bookRefreshActive =
    activeMaintenanceProgress?.id === BOOK_REFRESH_PROGRESS_ID &&
    activeMaintenanceProgress.state === "running";
  const bookRefreshFailed =
    activeMaintenanceProgress?.id === BOOK_REFRESH_PROGRESS_ID &&
    activeMaintenanceProgress.state === "failed";
  // Show the full-screen sync card for ANY active book refresh (the first sync
  // OR a later incremental refresh), not only the very first one.
  const syncCardEligible =
    (bookRefreshActive || bookRefreshFailed) && bookKey !== null;
  // Whether this is the book's very first sync — only switches the card's copy
  // ("setting up / building your history" vs a plain refresh).
  const isFirstSync = bookKey !== null && !firstSyncDone[bookKey];
  // Minimized state lives in the store (keyed by book) so the book-refresh
  // notification in the header can re-open the card. (The store field is still
  // named `firstSyncCardDismissed` for historical reasons; it now tracks the
  // minimized state of the sync card for any refresh.)
  const syncCardMinimizedMap = useUiStore((s) => s.firstSyncCardDismissed);
  const minimizeSyncCardStore = useUiStore((s) => s.dismissFirstSyncCard);
  const restoreSyncCardStore = useUiStore((s) => s.reopenFirstSyncCard);
  const syncCardMinimized =
    bookKey !== null && Boolean(syncCardMinimizedMap[bookKey]);
  React.useEffect(() => {
    // A failure always returns to the foreground. Once a run is gone, also drop
    // any "continue in background" choice so the next refresh starts expanded.
    if (
      (bookRefreshFailed || !syncCardEligible) &&
      bookKey !== null &&
      syncCardMinimizedMap[bookKey]
    ) {
      restoreSyncCardStore(bookKey);
    }
  }, [
    bookRefreshFailed,
    syncCardEligible,
    bookKey,
    syncCardMinimizedMap,
    restoreSyncCardStore,
  ]);
  const showSyncCard = syncCardEligible && !syncCardMinimized;
  const minimizeSyncCard = React.useCallback(() => {
    if (bookRefreshFailed) {
      clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
    } else if (bookKey !== null) {
      minimizeSyncCardStore(bookKey);
    }
  }, [
    bookKey,
    bookRefreshFailed,
    clearActiveMaintenanceProgress,
    minimizeSyncCardStore,
  ]);
  // The card already shows the title in its header, so feed it the raw phase
  // label rather than the route-composed "Title: detail" string. Terminal
  // failures remain available until explicitly dismissed.
  const syncCardProgress: RouteProgressState | null =
    activeMaintenanceProgress?.state === "running" ||
    activeMaintenanceProgress?.state === "failed"
      ? {
          indeterminate: Boolean(
            activeMaintenanceProgress.progress.indeterminate,
          ),
          label:
            activeMaintenanceProgress.progress.label?.trim() ||
            activeMaintenanceProgress.body,
          value: activeMaintenanceProgress.progress.value,
        }
      : shellProgress;
  const isAssistantRoute = pathname === "/assistant";
  const routeMeta =
    ROUTE_META.find(([prefix]) => pathname.startsWith(prefix))?.[1] ?? {
      titleKey: "shell.fallbackTitle",
      icon: Gauge,
      searchKey: "shell.fallback",
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
        stale: false,
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
    setPendingProjectUnlock(null);
    setTouchIdAutoPromptPending(
      foregroundTouchIdAutoPrompt(autoPromptTouchId),
    );
    if (requiresDaemonUnlock) {
      clearSessionUnlockPassphrase();
      clearDaemonQueryCache();
      setLocked(true);
      void getTransport().invoke({ kind: "daemon.lock" });
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
            error: importRootError ?? t("lock.importRootOpening"),
          };
        }
        bumpDaemonSession();
        let envelope;
        let nextIdentity: ProjectIdentity | null = null;
        if (pendingProjectUnlock) {
          const projectEnvelope =
            await getTransport().invoke<ProjectSelectSnapshot>({
              kind: "ui.projects.select",
              args: {
                project_id: pendingProjectUnlock.id,
                require_existing_project: true,
                auth_response: { passphrase_secret: passphrase },
              },
            });
          envelope = projectEnvelope;
          if (
            projectEnvelope.kind === "ui.projects.select" &&
            projectEnvelope.data?.project
          ) {
            nextIdentity = identityFromProject(
              projectEnvelope.data.project,
              projectEnvelope.data.status,
            );
          }
        } else {
          envelope = await getTransport().invoke({
            kind: "daemon.unlock",
            args: {
              ...(identity?.importedProject
                ? { require_existing_project: true }
                : {}),
              auth_response: { passphrase_secret: passphrase },
            },
          });
        }
        const unlocked = pendingProjectUnlock
          ? envelope.kind === "ui.projects.select"
          : envelope.kind === "daemon.unlock";
        if (unlocked) {
          if (nextIdentity) {
            noteActiveImportProject({
              ...nextIdentity.importedProject,
              encrypted: nextIdentity.encrypted,
            });
            setIdentity(nextIdentity);
          }
          setPendingProjectUnlock(null);
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
            void storeTouchIdPassphrase(
              passphrase,
              touchIdDataRoot,
              touchIdStatus?.staleGeneration ?? null,
            )
              .then((status) => {
                setTouchIdStatus(status);
                if (!status.configured) {
                  setAppLockPolicy({ touchIdUnlock: false });
                  addNotification({
                    title: t("lock.touchIdNotSavedTitle"),
                    body: status.reason
                      ? t("lock.touchIdNotSavedReason", { reason: status.reason })
                      : t("lock.touchIdNotSavedKeychain"),
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
                  title: t("lock.touchIdNotSavedTitle"),
                  body:
                    error instanceof Error
                      ? error.message
                      : t("lock.touchIdNotAcceptedKeychain"),
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
              ? t("lock.passphraseRequiredError")
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
      pendingProjectUnlock,
      queryClient,
      refreshTouchIdStatus,
      requiresDaemonUnlock,
      setIdentity,
      setAppLockPolicy,
      t,
      touchIdDataRoot,
      touchIdPlatformSupported,
      touchIdStatus?.configured,
    ],
  );

  const unlockWithTouchId = React.useCallback(async () => {
    if (importRootBlocked) {
      return {
        ok: false,
        error: importRootError ?? t("lock.importRootOpening"),
      };
    }
    bumpDaemonSession();
    const envelope = await unlockTouchIdPassphrase<ProjectSelectSnapshot>(
      touchIdDataRoot,
      {
        requireExistingProject: Boolean(
          pendingProjectUnlock ?? identity?.importedProject,
        ),
        projectId: pendingProjectUnlock?.id ?? null,
      },
    );
    const unlocked = pendingProjectUnlock
      ? envelope.kind === "ui.projects.select"
      : envelope.kind === "daemon.unlock";
    if (unlocked) {
      if (
        pendingProjectUnlock &&
        envelope.kind === "ui.projects.select" &&
        envelope.data?.project
      ) {
        const nextIdentity = identityFromProject(
          envelope.data.project,
          envelope.data.status,
        );
        noteActiveImportProject({
          ...nextIdentity.importedProject,
          encrypted: nextIdentity.encrypted,
        });
        setIdentity(nextIdentity);
        setPendingProjectUnlock(null);
      }
      setDaemonAuthRequired(false);
      setTouchIdAutoPromptPending(false);
      setLocked(false);
      void queryClient.invalidateQueries({
        queryKey: ["daemon"],
      });
      return { ok: true, error: null };
    }
    if (envelope.kind === "auth_required") {
      setDaemonAuthRequired(true);
      clearSessionUnlockPassphrase();
      clearDaemonQueryCache();
      setLocked(true);
    } else if (envelope.error?.code === "touch_id_passphrase_not_found") {
      setAppLockPolicy({ touchIdUnlock: false });
      await refreshTouchIdStatus();
    } else if (envelope.error?.code === "local_auth_denied") {
      // The Keychain item passed biometric access but no longer opens this
      // database (for example after a CLI-side passphrase rotation). Remove it
      // instead of offering a known-stale credential on every lock screen.
      await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
      setAppLockPolicy({ touchIdUnlock: false });
      await refreshTouchIdStatus();
    }
    return {
      ok: false,
      error: formatDaemonEnvelopeError(envelope) ?? t("lock.touchIdNotFound"),
    };
  }, [
    bumpDaemonSession,
    clearDaemonQueryCache,
    identity?.importedProject,
    importRootBlocked,
    importRootError,
    pendingProjectUnlock,
    queryClient,
    refreshTouchIdStatus,
    setIdentity,
    setAppLockPolicy,
    t,
    touchIdDataRoot,
  ]);

  const switchProject = React.useCallback(
    async (project: ProjectCatalogEntry) => {
      try {
        const envelope = await getTransport().invoke<ProjectSelectSnapshot>({
          kind: "ui.projects.select",
          args: { project_id: project.id },
        });
        if (envelope.kind === "ui.projects.select" && envelope.data?.project) {
          const nextIdentity = identityFromProject(
            envelope.data.project,
            envelope.data.status,
          );
          noteActiveImportProject({
            ...nextIdentity.importedProject,
            encrypted: nextIdentity.encrypted,
          });
          setIdentity(nextIdentity);
          setDaemonAuthRequired(false);
          setTouchIdAutoPromptPending(false);
          setLocked(false);
          bumpDaemonSession();
          clearDaemonQueryCache();
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
          void navigate({ to: "/overview" });
          return;
        }
        if (envelope.kind === "auth_required") {
          setPendingProjectUnlock(project);
          setDaemonAuthRequired(true);
          setTouchIdAutoPromptPending(false);
          clearSessionUnlockPassphrase();
          clearDaemonQueryCache();
          bumpDaemonSession();
          setLocked(true);
          return;
        }
        addNotification({
          title: t("projects.switchFailedTitle"),
          body: formatDaemonEnvelopeError(envelope) ?? t("projects.switchFailedBody"),
          tone: "error",
        });
      } catch (error) {
        addNotification({
          title: t("projects.switchFailedTitle"),
          body: error instanceof Error ? error.message : t("projects.switchFailedBody"),
          tone: "error",
        });
      }
    },
    [
      addNotification,
      bumpDaemonSession,
      clearDaemonQueryCache,
      navigate,
      queryClient,
      setIdentity,
      t,
    ],
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
  const activeProgressHasMatchingMutation = React.useCallback(
    (id: string) => {
      switch (id) {
        case "book-refresh":
          return (
            isDaemonKindMutating("ui.freshness.run") ||
            isDaemonKindMutating("ui.wallets.sync")
          );
        case "journal-processing":
          return isDaemonKindMutating("ui.journals.process");
        case "market-rate-refresh":
          return isDaemonKindMutating("ui.rates.latest");
        default:
          return true;
      }
    },
    [isDaemonKindMutating],
  );

  React.useEffect(() => {
    if (activeMaintenanceProgress?.state !== "running") return;
    if (activeProgressHasMatchingMutation(activeMaintenanceProgress.id)) return;

    const updatedAt = Date.parse(activeMaintenanceProgress.updatedAt);
    const ageMs = Number.isFinite(updatedAt) ? Date.now() - updatedAt : 0;
    const delayMs = Math.max(0, ACTIVE_PROGRESS_CLEAR_GRACE_MS - ageMs);
    const timeout = window.setTimeout(() => {
      const current = useUiStore.getState().activeMaintenanceProgress;
      if (current?.state !== "running") return;
      if (current.id !== activeMaintenanceProgress.id) return;
      if (activeProgressHasMatchingMutation(current.id)) return;
      clearActiveMaintenanceProgress(current.id);
    }, delayMs);

    return () => window.clearTimeout(timeout);
  }, [
    activeMaintenanceProgress?.state,
    activeMaintenanceProgress?.id,
    activeMaintenanceProgress?.updatedAt,
    activeProgressHasMatchingMutation,
    clearActiveMaintenanceProgress,
    freshnessRunsInFlight,
    journalRunsInFlight,
    marketRateRunsInFlight,
    walletSyncsInFlight,
  ]);
  const { runJournalProcessing: runMenuJournalProcessing } =
    useJournalProcessingAction({
      beforeRun: ensureWorkspaceForMenuAction,
      notifyAlreadyRunning: true,
      notifyStart: true,
    });

  const runMenuWalletSync = React.useCallback(
    (options?: { forceFull?: boolean }) => {
      if (!ensureWorkspaceForMenuAction()) return;
      if (
        isSyncing ||
        isDaemonKindMutating("ui.freshness.run") ||
        isDaemonKindMutating("ui.wallets.sync")
      ) {
        addNotification({
          title: t("notifications.bookRefreshRunning.title"),
          body: t("notifications.bookRefreshRunning.body"),
          tone: "info",
        });
        return;
      }
      syncAll({ forceFull: Boolean(options?.forceFull) });
    },
    [
      addNotification,
      ensureWorkspaceForMenuAction,
      isDaemonKindMutating,
      isSyncing,
      syncAll,
      t,
    ],
  );

  // The header refresh button AND the Cmd/Ctrl+R / Cmd/Ctrl+Shift+R shortcuts
  // route here, so the advertised shortcut behaves exactly like the button:
  // surface the loader (un-minimize the sync card if it was sent to the
  // background) and start the book refresh. `syncAll` no-ops while one is
  // already running, so a mid-refresh trigger just re-opens the card.
  const runHeaderRefresh = React.useCallback(
    (options?: { forceFull?: boolean }) => {
      const plan = planHeaderRefresh({
        hasWorkspace: ensureWorkspaceForMenuAction(),
        bookKey,
      });
      if (plan.reopenSyncCardForBook !== null) {
        restoreSyncCardStore(plan.reopenSyncCardForBook);
      }
      if (plan.startRefresh) {
        syncAll({ forceFull: Boolean(options?.forceFull) });
      }
    },
    [bookKey, ensureWorkspaceForMenuAction, restoreSyncCardStore, syncAll],
  );

  const openAddWalletConnection = React.useCallback(
    (reason: string) => {
      if (!ensureWorkspaceForMenuAction()) return;
      setDeferredConnectionSetup({
        sourceId: "descriptor",
        reason,
      });
      void navigate({ to: "/connections" });
    },
    [ensureWorkspaceForMenuAction, navigate, setDeferredConnectionSetup],
  );

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (isTypingTarget(event.target)) return;
      const action = appWorkflowHotkeyAction(event);
      if (!action) return;
      event.preventDefault();
      if (action === "add-wallet") {
        openAddWalletConnection(t("search.actionReason.fromKeyboard"));
        return;
      }
      if (action === "process-journals") {
        runMenuJournalProcessing();
        return;
      }
      runHeaderRefresh({ forceFull: action === "rescan-book" });
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [openAddWalletConnection, runHeaderRefresh, runMenuJournalProcessing, t]);

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
    if (!daemonEnabled) return;
    if (identity?.importedProject) return;
    if (!identity) return;
    if (workspaceValidationApplied.current) return;
    workspaceValidationApplied.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const envelope = await getTransport().invoke<ProfilesSnapshot>({
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
      hasSessionUnlock: false,
    });
    setTouchIdAutoPromptPending(
      foregroundTouchIdAutoPrompt(nextLocked),
    );
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
            : t("importRoot.couldNotOpen"),
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
    t,
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
      setTouchIdAutoPromptPending(
        foregroundTouchIdAutoPrompt(true),
      );
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
    if (!daemonEnabled) return;
    startDaemonLogBridge({ isEnabled: () => daemonEnabled });
    return () => stopDaemonLogBridge();
  }, [daemonEnabled]);

  React.useEffect(() => {
    if (!isAssistantRoute) {
      void routeSearch;
      setAssistantScreenContext(
        assistantScreenContextFor(
          pathname,
          typeof window === "undefined" ? "" : window.location.search,
        ),
      );
    }
  }, [isAssistantRoute, pathname, routeSearch]);

  React.useEffect(() => {
    const handleAssistantDockSuppressed = (event: Event) => {
      const detail = (event as CustomEvent<{ suppressed?: boolean }>).detail;
      setAssistantDockSuppressed(Boolean(detail?.suppressed));
    };

    window.addEventListener(
      "kassiber:assistant-dock-suppressed",
      handleAssistantDockSuppressed,
    );
    return () => {
      window.removeEventListener(
        "kassiber:assistant-dock-suppressed",
        handleAssistantDockSuppressed,
      );
    };
  }, []);

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
          // AppShell only handles workspace-scoped actions (lock, add wallet,
          // sync, process-journals). Global actions (navigate, open-settings,
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
              runAppUpdateCheck: () => {
                console.error("AppShell should not receive global menu actions");
              },
              runAddWalletConnection: () =>
                openAddWalletConnection(t("search.actionReason.fromNativeMenu")),
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
          safeTauriUnlisten(nextUnlisten);
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((error) => {
        console.warn("Could not attach Kassiber native menu listener", error);
      });

    return () => {
      disposed = true;
      safeTauriUnlisten(unlisten);
    };
  }, [
    lockApp,
    navigate,
    openAddWalletConnection,
    runMenuJournalProcessing,
    runMenuWalletSync,
    aiFeaturesEnabled,
    addNotification,
    setHideSensitive,
    t,
  ]);

  React.useEffect(() => {
    if (!daemonEnabled || !("__TAURI_INTERNALS__" in window)) return;

    let disposed = false;
    let unlisten: (() => void) | null = null;

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<unknown>(DAEMON_EVENT_CHANNEL, (event) => {
          if (disposed) return;
          const signal = classifyDaemonFreshnessEvent(event.payload);
          if (!signal) return;

          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
          if (signal === "refresh") {
            const store = useUiStore.getState();
            const previousFailure = store.notifications.find(
              (item) => item.dedupeKey === "background-freshness",
            );
            if (previousFailure) store.clearNotification(previousFailure.id);
            return;
          }

          addNotification({
            title:
              signal === "worker-error"
                ? t("overview:bookRefresh.failedTitle")
                : t("overview:bookRefresh.needsAttentionTitle"),
            body: t("overview:bookRefresh.failedBody"),
            tone: signal === "worker-error" ? "error" : "warning",
            dedupeKey: "background-freshness",
            target: "/logs",
          });
        }),
      )
      .then((nextUnlisten) => {
        if (disposed) {
          safeTauriUnlisten(nextUnlisten);
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((error) => {
        console.warn("Could not attach Kassiber daemon event listener", error);
      });

    return () => {
      disposed = true;
      safeTauriUnlisten(unlisten);
    };
  }, [addNotification, daemonEnabled, queryClient, t]);

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
      setAssistantCollapsed((collapsed) =>
        nextAssistantDockCollapsed({
          collapsed,
          scrollTop: main.scrollTop,
          scrollHeight: main.scrollHeight,
          clientHeight: main.clientHeight,
        }),
      );
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
      {/* The top strip is always there — red while the pre-alpha warning is on,
          plain window background once it is dismissed — so the macOS traffic
          lights always have their own row and nothing below has to dodge them.
          The sidebar is `fixed`, so it reads the strip's height off this
          variable to sit under it. */}
      <div
        className="flex h-svh flex-col overflow-hidden bg-sidebar"
        style={{ "--kb-banner-height": "28px" } as React.CSSProperties}
      >
        <PreAlphaBanner className="shrink-0" muted={!preAlphaBannerVisible} />
        {/*
          The shell is a two-column frame: the side nav owns all navigation
          (brand, book switcher, search, pages, settings), and the content panel
          carries only its own page plus a floating strip of shell controls.
          There is no full-width top bar — the controls float over the panel.
        */}
        <SidebarProvider className="min-h-0 flex-1 bg-sidebar">
          <a
            href="#app-main"
            className="sr-only focus:not-sr-only focus:absolute focus:top-4 focus:left-4 focus:z-50 focus:rounded-md focus:bg-background focus:px-3 focus:py-2 focus:text-sm focus:text-foreground focus:ring-2 focus:ring-ring"
          >
            {t("shell.skipToContent")}
          </a>
          <div className="flex min-h-0 flex-1">
            <AppSidebar
              pathname={pathname}
              meta={routeMeta}
              onLock={lockApp}
              onProjectSelect={switchProject}
              daemonEnabled={daemonEnabled}
              aiFeaturesEnabled={aiFeaturesEnabled}
              developerToolsEnabled={developerToolsEnabled}
            />
            {/* `pl-3` is two gutters: the nav card is shifted right by one, so
                this is what leaves a matching gap on the seam between them. */}
            <div className="min-h-0 w-full overflow-hidden lg:pt-1.5 lg:pr-1.5 lg:pb-1.5 lg:pl-3">
              <div className="relative flex h-full w-full flex-col items-center justify-start overflow-hidden bg-background lg:rounded-xl">
                <ShellFloatingControls
                  meta={routeMeta}
                  onLock={lockApp}
                  onRefresh={runHeaderRefresh}
                  isRefreshing={isSyncing}
                  daemonEnabled={daemonEnabled}
                />
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
                    <AssistantSessionProvider screenContext={assistantScreenContext}>
                      <main
                        id="app-main"
                        ref={mainRef}
                        tabIndex={-1}
                        className={cn(
                          appMainClassName,
                          // A live conversation expands the dock (even under
                          // auto-hide), so reserve real space for it. Otherwise
                          // the parked pill / minimized chip only needs a
                          // sliver; the legacy scroll-collapse path applies when
                          // auto-hide is off and there is no thread.
                          isAssistantRoute || assistantDockSuppressed
                            ? "pb-0"
                            : // Expanded thread needs the full reserve; a minimized
                              // "Working + follow-up" surface still sets expanded
                              // so it gets a mid-size pad instead of the pill sliver.
                              assistantDockExpanded && !assistantDockMinimized
                              ? "pb-[240px]"
                              : assistantDockExpanded && assistantDockMinimized
                                ? "pb-36"
                                : assistantDockMinimized
                                  ? "pb-6"
                                  : assistantDockAutoHide
                                    ? "pb-6"
                                    : assistantCollapsed
                                      ? "pb-16"
                                      : "pb-[240px]",
                        )}
                      >
                        <RouteErrorBoundary>
                          <Outlet />
                        </RouteErrorBoundary>
                      </main>
                      {isAssistantRoute || assistantDockSuppressed ? null : (
                        <AssistantDock
                          collapsed={assistantCollapsed}
                          autoHide={assistantDockAutoHide}
                          position={assistantDockPosition}
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
                      <RouteErrorBoundary>
                        <Outlet />
                      </RouteErrorBoundary>
                    </main>
                  )
                )}
                {!locked && !importRootBlocked ? (
                  <>
                    <RouteTopProgressLine
                      // While the full-screen sync card is up it already shows
                      // this progress (plus the blur scrim), so suppress the
                      // hairline here — it returns once "Continue in background"
                      // minimizes the card.
                      active={shellBusy && !showSyncCard}
                      progress={shellProgress}
                      announce={!showSyncCard}
                    />
                    {showSyncCard ? (
                      <FirstSyncCard
                        progress={syncCardProgress}
                        title={activeMaintenanceProgress?.title}
                        isFirstSync={isFirstSync}
                        failed={bookRefreshFailed}
                        failedPhase={activeMaintenanceProgress?.phase}
                        onDismiss={minimizeSyncCard}
                      />
                    ) : null}
                  </>
                ) : null}
              </div>
            </div>
          </div>
        </SidebarProvider>
      </div>
    </TooltipProvider>
  );
}

/**
 * Hairline refresh indicator pinned to the top edge of the content area.
 *
 * It overlays content (`absolute`, outside the scroll container) rather than
 * sitting in normal flow, so starting/stopping a refresh never reflows the
 * route below it. The previous in-flow bar grew from 0→36px and shoved
 * everything down on every sync; this stays a constant 3px line and just fades
 * in. The richer "what's syncing" detail now lives in the notifications panel
 * and, for a brand-new book, the FirstSyncCard.
 */
function RouteTopProgressLine({
  active,
  progress,
  announce = true,
}: {
  active: boolean;
  progress?: RouteProgressState | null;
  /** When false, skip the off-screen live region (the FirstSyncCard announces instead). */
  announce?: boolean;
}) {
  const value = notificationProgressValue(progress?.value);
  const isDeterminate =
    Boolean(progress) &&
    !progress?.indeterminate &&
    typeof progress?.value === "number";
  const label = progress?.label;

  return (
    <div
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 z-30 h-[3px] transition-opacity duration-200",
        active ? "opacity-100" : "opacity-0",
      )}
    >
      <div className="h-full w-full overflow-hidden" aria-hidden="true">
        <div
          className={cn(
            "h-full bg-primary/80",
            isDeterminate
              ? "transition-[width] duration-200 ease-out"
              : "w-1/3 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
          )}
          style={isDeterminate ? { width: `${value}%` } : undefined}
        />
      </div>
      {/* The visible label moved out of the layout, so keep an off-screen live
          region for assistive tech to announce refresh progress. */}
      {announce && active && label ? (
        <span role="status" aria-live="polite" className="sr-only">
          {label}
        </span>
      ) : null}
    </div>
  );
}

/**
 * The app's single navigation surface.
 *
 * Two modes share one frame: the book navigation, and — on any `/settings/*`
 * route — the settings navigation, which takes over the whole nav rather than
 * squeezing a second rail into the page. The frame itself is a frosted panel
 * (`.kb-glass-panel`) lifted off the app chrome.
 */
function AppSidebar({
  pathname,
  meta,
  onLock,
  onProjectSelect,
  daemonEnabled,
  aiFeaturesEnabled,
  developerToolsEnabled,
}: {
  pathname: string;
  meta: RouteMeta;
  onLock: () => void;
  onProjectSelect: (project: ProjectCatalogEntry) => void;
  daemonEnabled: boolean;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
}) {
  const { t } = useTranslation("nav");
  const inSettings = pathname === "/settings" || pathname.startsWith("/settings/");
  const settingsSectionId =
    settingsSectionForPathname(pathname) ?? DEFAULT_SETTINGS_SECTION_ID;
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
  const { data: reviewBadgesData } = useDaemon<ReviewBadgesSnapshot>(
    "ui.review.badges",
    undefined,
    { enabled: daemonEnabled },
  );
  const badges = reviewBadgesData?.data;
  const navBadges = React.useMemo<Record<string, NavBadge>>(() => {
    const map: Record<string, NavBadge> = {};
    if (!badges) return map;
    // Signal, not reassurance: only surface a hint when there is something to
    // act on — never a "0" or an all-clear marker.
    if (badges.quarantine > 0) {
      // Quarantine blocks correct reports — same red as the bell's quarantine alert.
      map["/quarantine"] = {
        count: badges.quarantine,
        tone: "blocker",
        labelKey: "badge.quarantine",
      };
    }
    if (typeof badges.swaps === "number" && badges.swaps > 0) {
      map["/swaps"] = {
        count: badges.swaps,
        tone: "review",
        labelKey: "badge.swaps",
      };
    }
    if (badges.journals_needs_processing) {
      map["/journals"] = {
        count: null,
        tone: "review",
        labelKey: "badge.journals",
      };
    }
    return map;
  }, [badges]);

  return (
    <Sidebar
      variant="sidebar"
      collapsible="icon"
      /* A card under the top strip, on the same terms as the content panel: the
         same 1.5 gutter on every free side, the same corner radius, and a
         hairline all the way round. The frosted nav and the panel land within a
         few percent of each other in lightness by design (T3Code's quiet
         hierarchy), so that hairline is what keeps the two surfaces apart.
         The card keeps its full width and only shifts right by one gutter — the
         seam gap comes from the content panel's own `pl-3` instead. Narrowing it
         here would eat the collapsed rail, which is only 3rem wide to begin
         with. */
      className="kb-glass-panel top-[calc(var(--kb-banner-height,0px)+(--spacing(1.5)))] left-1.5 h-[calc(100svh-var(--kb-banner-height,0px)-(--spacing(3)))] overflow-hidden rounded-xl border border-sidebar-border/70"
    >
      {/* Header stays mounted across both nav modes, so the wordmark and the ⌘K
          palette are reachable from settings too. `relative` + the children's
          `z-10` let the stage art sit behind them; the art renders nothing at
          all on a stable release. */}
      <SidebarHeader className="relative gap-1 px-2 pt-2 pb-1">
        <SidebarStageBackdrop />
        <div className="relative z-10 flex flex-col gap-1">
          {/*
            Only the brand row is relit, and only in dark mode (the CSS is
            `.dark`-scoped): the art fades out before the search row, so
            relighting that row too would put white text on the near-white faded
            tail. The class is unconditional because every channel now draws art.
          */}
          <div className="kb-stage-header-content">
            <SidebarBrand />
          </div>
          <ShellSearch searchKey={meta.searchKey} daemonEnabled={daemonEnabled} />
        </div>
      </SidebarHeader>
      {inSettings ? (
        <SettingsNavSection
          activeId={settingsSectionId}
          daemonEnabled={daemonEnabled}
        />
      ) : (
        <>
          <SidebarContent className="gap-0">
            {navGroups.map((group) => (
              <SidebarGroup key={group.titleKey} className="gap-1.5 px-2 py-1.5">
                {/* T3Code's "Projects" label recipe: plain sentence-case text at
                    `text-xs font-medium` in the muted nav tone. This replaced a
                    9px uppercase-mono caption that read as noise at nav scale. */}
                <SidebarGroupLabel className="mb-1 h-auto px-2 text-xs font-medium text-sidebar-muted-foreground/80">
                  {t(group.titleKey as never) /* dynamic key */}
                </SidebarGroupLabel>
                <SidebarGroupContent>
                  <SidebarMenu>
                    {group.items.map((item) => (
                      <NavMenuItem
                        key={item.labelKey}
                        item={item}
                        pathname={pathname}
                        badge={navBadges[item.href]}
                      />
                    ))}
                  </SidebarMenu>
                </SidebarGroupContent>
              </SidebarGroup>
            ))}
          </SidebarContent>
          <SidebarFooter className="gap-1 p-2">
            <SidebarActions
              pathname={pathname}
              developerToolsEnabled={developerToolsEnabled}
            />
            <SidebarUpdatePill />
            <NavUser
              onLock={onLock}
              onProjectSelect={onProjectSelect}
              daemonEnabled={daemonEnabled}
            />
            <AppVersion />
          </SidebarFooter>
        </>
      )}
      <SidebarRail className="after:hidden" />
    </Sidebar>
  );
}

/**
 * Settings navigation, with the per-layer backend counts the old in-page rail
 * showed. The backends query is the same key the settings panels use, so
 * react-query serves it from cache instead of issuing a second request.
 */
function SettingsNavSection({
  activeId,
  daemonEnabled,
}: {
  activeId: SettingsSectionId;
  daemonEnabled: boolean;
}) {
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    { enabled: daemonEnabled, staleTime: 15_000 },
  );
  const counts = React.useMemo<Partial<Record<SettingsSectionId, number>>>(() => {
    const backends = (backendSettingsQuery.data?.data?.backends ?? []).map(
      backendRowToSettingsBackend,
    );
    return {
      "network-bitcoin": backendsForLayer(backends, "bitcoin").length,
      "network-lightning": backendsForLayer(backends, "lightning").length,
      "network-liquid": backendsForLayer(backends, "liquid").length,
      "network-market": backends.filter((backend) => backend.net === "FX").length,
    };
  }, [backendSettingsQuery.data]);

  return <SettingsSidebarNav activeId={activeId} counts={counts} />;
}

/**
 * Nav header, arranged as T3Code arranges it.
 *
 * T3Code puts the nav-collapse control and the wordmark side by side on the very
 * top-left row — its toggle is `fixed` at the window corner and the brand is
 * pushed right by exactly the control's width to sit beside it. The book
 * switcher is NOT on this row: T3Code keeps its equivalent (the project-scope
 * picker) on its own row further down, which is what leaves this row roomy
 * enough for the history controls to join the toggle.
 *
 * Collapsed to the icon rail, only the toggle survives — the wordmark and the
 * history buttons would not fit, and the toggle is what gets the nav back.
 */
function SidebarBrand() {
  const { t } = useTranslation("chrome");
  const { state, isMobile } = useSidebar();
  const collapsed = state === "collapsed" && !isMobile;
  return (
    <div
      className={cn(
        "flex h-8 min-w-0 items-center gap-0.5",
        collapsed && "justify-center",
      )}
    >
      <SidebarTrigger className={navIconButtonClassName} />
      {collapsed ? null : (
        <>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={navIconButtonClassName}
            aria-label={t("shell.back")}
            title={t("shell.back")}
            onClick={() => window.history.back()}
          >
            <ArrowLeft className="size-4" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={navIconButtonClassName}
            aria-label={t("shell.forward")}
            title={t("shell.forward")}
            onClick={() => window.history.forward()}
          >
            <ArrowRight className="size-4" aria-hidden="true" />
          </Button>
          <Link
            to="/overview"
            aria-label={t("shell.overviewLink")}
            className="ml-1 flex h-7 min-w-0 shrink items-center truncate rounded-md text-sm font-medium tracking-tight text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            Kassiber
          </Link>
        </>
      )}
    </div>
  );
}

/**
 * Leading crumb: the active book, and the control that switches it.
 *
 * T3Code's breadcrumb leads with the project — `[favicon] Project / Thread` —
 * on the reasoning that knowing which project you are in is priority zero and
 * the title alone does not answer it. The same holds here for the book, so this
 * takes the same slot and the same styling: a 3.5-size glyph, a `max-w-40`
 * truncating name in muted `text-sm font-medium`, and a `/` at 40% opacity
 * before the current page.
 *
 * T3Code's crumb is static text; ours is a button, because the book is
 * switchable and this is where a user looks to switch it.
 */
function BreadcrumbBook({ daemonEnabled }: { daemonEnabled: boolean }) {
  const { t } = useTranslation("chrome");
  const identity = useUiStore((s) => s.identity);
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const [bookSwitcherOpen, setBookSwitcherOpen] = React.useState(false);
  const bookLabel =
    data?.data?.status?.profile ??
    data?.data?.status?.workspace ??
    identity?.profile ??
    t("shell.booksFallback");
  const label = t("shell.switchBooksLabel", { book: bookLabel });

  return (
    <span className="inline-flex shrink-0 items-center gap-2">
      <button
        type="button"
        aria-label={label}
        title={label}
        aria-haspopup="dialog"
        aria-expanded={bookSwitcherOpen}
        onClick={() => setBookSwitcherOpen(true)}
        className="group inline-flex min-w-0 items-center gap-1.5 rounded-md px-1 py-0.5 -mx-1 hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <Folder
          className="size-3.5 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <span className="max-w-40 truncate text-sm font-medium text-muted-foreground transition-colors group-hover:text-foreground">
          {bookLabel}
        </span>
        <ChevronsUpDown
          className="size-3 shrink-0 text-muted-foreground/50 transition-colors group-hover:text-muted-foreground"
          aria-hidden="true"
        />
      </button>
      <span aria-hidden="true" className="text-muted-foreground/40">
        /
      </span>
      <BookSwitcherPopover
        open={bookSwitcherOpen}
        onClose={() => setBookSwitcherOpen(false)}
      />
    </span>
  );
}

function SidebarActions({
  pathname,
  developerToolsEnabled,
}: {
  pathname: string;
  developerToolsEnabled: boolean;
}) {
  const { t } = useTranslation(["chrome", "nav"]);
  const dataMode = useUiStore((state) => state.dataMode);
  const setDataMode = useUiStore((state) => state.setDataMode);
  const explorerPublicFallbacks = useUiStore(
    (state) => state.explorerSettings.publicFallbacks,
  );
  const setExplorerSettings = useUiStore((state) => state.setExplorerSettings);
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    {
      staleTime: 15_000,
      refetchOnWindowFocus: false,
    },
  );
  const defaultBackendName =
    backendSettingsQuery.data?.data?.summary.default_backend ?? null;
  const defaultBackend =
    backendSettingsQuery.data?.data?.backends.find(
      (backend) => backend.name === defaultBackendName || backend.is_default,
    ) ?? null;
  const activeRegtestBackend = ["regtest", "elementsregtest"].includes(
    String(defaultBackend?.network ?? "").toLowerCase(),
  );
  // Until the backends query resolves, activeRegtestBackend is a placeholder
  // false; coercing on it would bounce a persisted regtest mode through "real"
  // and re-key every daemon query on launch.
  const backendSettingsLoaded = backendSettingsQuery.isSuccess;
  const normalizedDataMode = backendSettingsLoaded
    ? dataModeForActiveBackend(dataMode, activeRegtestBackend)
    : dataMode;

  React.useEffect(() => {
    if (!backendSettingsLoaded) return;
    if (normalizedDataMode !== dataMode) {
      setDataMode(normalizedDataMode);
    }
  }, [backendSettingsLoaded, dataMode, normalizedDataMode, setDataMode]);

  // Keep public-explorer fallbacks disabled whenever a regtest/elementsregtest
  // book is active, without waiting for the Settings screen to mount. Otherwise
  // the store default (publicFallbacks: true) leaves a freshly-launched or
  // onboarding-opened regtest book handing regtest txids to a public explorer
  // until Settings is visited. Mirrors deriveExplorerSettings' publicFallbacks
  // rule so the two writers never disagree; base URLs stay owned by Settings.
  React.useEffect(() => {
    if (!backendSettingsLoaded) return;
    const allowPublicFallbacks = !activeRegtestBackend;
    if (explorerPublicFallbacks !== allowPublicFallbacks) {
      setExplorerSettings({ publicFallbacks: allowPublicFallbacks });
    }
  }, [
    backendSettingsLoaded,
    activeRegtestBackend,
    explorerPublicFallbacks,
    setExplorerSettings,
  ]);

  const dataModeLabel = t(
    `shell.dataMode.${dataModeLabelKey(normalizedDataMode)}`,
  );
  const supportActive = pathname === "/diagnostics";
  // Controlled, so a click on the collapsed rail can force them open while it
  // expands the nav (see `useRailSubmenuTrigger`).
  const [supportOpen, setSupportOpen] = React.useState(supportActive);
  const [extrasOpen, setExtrasOpen] = React.useState(false);
  const onSupportClick = useRailSubmenuTrigger(setSupportOpen);
  const onExtrasClick = useRailSubmenuTrigger(setExtrasOpen);

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          asChild
          isActive={pathname === "/activity"}
          tooltip={t("nav:book.activity")}
          className={navRowClassName}
        >
          <Link to="/activity">
            <History className="size-4" aria-hidden="true" />
            <span>{t("nav:book.activity")}</span>
          </Link>
        </SidebarMenuButton>
      </SidebarMenuItem>
      {developerToolsEnabled ? (
        <SidebarMenuItem>
          <SidebarMenuButton
            asChild
            isActive={pathname === "/logs"}
            tooltip={t("nav:book.logs")}
            className={navRowClassName}
          >
            <Link to="/logs">
              <TerminalSquare className="size-4" aria-hidden="true" />
              <span>{t("nav:book.logs")}</span>
            </Link>
          </SidebarMenuButton>
        </SidebarMenuItem>
      ) : null}
      {developerToolsEnabled ? (
        <SidebarMenuItem>
          <div className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0">
            <Server className="size-4 shrink-0" aria-hidden="true" />
            <span className="min-w-0 flex-1 truncate group-data-[collapsible=icon]:hidden">
              {dataModeLabel}
            </span>
            <span className="size-2 rounded-full bg-emerald-500 group-data-[collapsible=icon]:hidden" />
          </div>
        </SidebarMenuItem>
      ) : null}
      <SidebarMenuItem>
        <Collapsible
          asChild
          open={supportOpen}
          onOpenChange={setSupportOpen}
          className="group/collapsible"
        >
          <div>
            <CollapsibleTrigger asChild>
              <SidebarMenuButton
                isActive={supportActive}
                tooltip={t("shell.support.title")}
                className={navRowClassName}
                onClick={onSupportClick}
              >
                <LifeBuoy className="size-4" aria-hidden="true" />
                <span>{t("shell.support.title")}</span>
                <ChevronRight className="ml-auto size-3.5! shrink-0 text-muted-foreground/70 transition-transform duration-150 group-data-[state=open]/collapsible:rotate-90 group-data-[collapsible=icon]:hidden" aria-hidden="true" />
              </SidebarMenuButton>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <SidebarMenuSub>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton asChild className={navSubRowClassName}>
                    <a
                      href="https://github.com/bitcoinaustria/kassiber/issues"
                      target="_blank"
                      rel="noreferrer"
                    >
                      <Bug className="size-3.5" aria-hidden="true" />
                      <span>{t("shell.support.bugReport")}</span>
                    </a>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton asChild className={navSubRowClassName}>
                    <a
                      href="https://github.com/bitcoinaustria/kassiber/discussions"
                      target="_blank"
                      rel="noreferrer"
                    >
                      <Heart className="size-3.5" aria-hidden="true" />
                      <span>{t("shell.support.discussions")}</span>
                    </a>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              </SidebarMenuSub>
            </CollapsibleContent>
          </div>
        </Collapsible>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <Collapsible
          asChild
          open={extrasOpen}
          onOpenChange={setExtrasOpen}
          className="group/collapsible"
        >
          <div>
            <CollapsibleTrigger asChild>
              <SidebarMenuButton
                tooltip={t("shell.extras.title")}
                className={navRowClassName}
                onClick={onExtrasClick}
              >
                <Plus className="size-4" aria-hidden="true" />
                <span>{t("shell.extras.title")}</span>
                <ChevronRight className="ml-auto size-3.5! shrink-0 text-muted-foreground/70 transition-transform duration-150 group-data-[state=open]/collapsible:rotate-90 group-data-[collapsible=icon]:hidden" aria-hidden="true" />
              </SidebarMenuButton>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <SidebarMenuSub>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton
                    asChild
                    className={navSubRowClassName}
                    isActive={pathname === "/exit-tax"}
                  >
                    <Link to="/exit-tax">
                      <LogOut className="size-3.5" aria-hidden="true" />
                      <span>{t("shell.extras.exitCalculator")}</span>
                    </Link>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton
                    asChild
                    className={navSubRowClassName}
                    isActive={pathname === "/privacy-mirror"}
                  >
                    <Link to="/privacy-mirror">
                      <Eye className="size-3.5" aria-hidden="true" />
                      <span>{t("shell.extras.privacyMirror")}</span>
                    </Link>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
                <SidebarMenuSubItem>
                  <SidebarMenuSubButton
                    asChild
                    className={navSubRowClassName}
                    isActive={pathname === "/egress"}
                  >
                    <Link to="/egress">
                      <Plane className="size-3.5" aria-hidden="true" />
                      <span>{t("shell.extras.egress")}</span>
                    </Link>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              </SidebarMenuSub>
            </CollapsibleContent>
          </div>
        </Collapsible>
      </SidebarMenuItem>
      {/* Hairline above Settings, as T3Code has it: everything above is a place
          in the book, Settings is not, and the line marks that boundary. The
          negative inset lets it span the nav's full width rather than stopping
          at the row's padding. */}
      <SidebarMenuItem className="mt-1 -mx-2 border-t border-sidebar-border/60 px-2 pt-1">
        <SidebarMenuButton
          asChild
          isActive={pathname.startsWith("/settings")}
          tooltip={t("nav:book.settings")}
          className={navRowClassName}
        >
          <Link to="/settings">
            <Settings className="size-4" aria-hidden="true" />
            <span>{t("nav:book.settings")}</span>
          </Link>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

// Renders a hint next to a nav item. Only mounted when there is something to
// act on, so the count/dot itself is the signal — there is no empty/all-clear
// state. Shows a count pill when the sidebar is expanded and a corner dot once
// it collapses to icons (where the pill is hidden).
function NavItemBadge({ badge }: { badge: NavBadge }) {
  const { t } = useTranslation("nav");
  const label = t(badge.labelKey as never, { count: badge.count ?? 0 });
  const display =
    badge.count === null ? null : badge.count > 99 ? "99+" : String(badge.count);
  return (
    <>
      {display === null ? (
        <span
          aria-label={label}
          className={cn(
            "pointer-events-none absolute top-1/2 right-2.5 size-2 -translate-y-1/2 rounded-full select-none group-data-[collapsible=icon]:hidden",
            NAV_BADGE_DOT_TONE[badge.tone],
          )}
        />
      ) : (
        <span
          aria-label={label}
          className={cn(
            "pointer-events-none absolute top-1.5 right-1 flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium tabular-nums select-none group-data-[collapsible=icon]:hidden",
            NAV_BADGE_PILL_TONE[badge.tone],
          )}
        >
          {display}
        </span>
      )}
      <span
        aria-label={label}
        className={cn(
          "pointer-events-none absolute top-1.5 right-1.5 hidden size-2 rounded-full ring-2 ring-sidebar group-data-[collapsible=icon]:block",
          NAV_BADGE_DOT_TONE[badge.tone],
        )}
      />
    </>
  );
}

function NavMenuItem({
  item,
  pathname,
  badge,
}: {
  item: NavItem;
  pathname: string;
  badge?: NavBadge;
}) {
  const { t } = useTranslation("nav");
  const Icon = item.icon;
  const childActive = item.children?.some(
    (child) => pathname === child.href || pathname.startsWith(`${child.href}/`),
  );
  const active =
    pathname === item.href ||
    pathname.startsWith(`${item.href}/`) ||
    Boolean(childActive);
  const [open, setOpen] = React.useState(active);
  const onTriggerClick = useRailSubmenuTrigger(setOpen);

  React.useEffect(() => {
    if (active) setOpen(true);
  }, [active]);

  if (!item.children?.length) {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton
          asChild
          isActive={active}
          tooltip={t(item.labelKey as never) /* dynamic key */}
          className={navRowClassName}
        >
          <Link to={item.href}>
            <Icon className="size-4" aria-hidden="true" />
            <span>{t(item.labelKey as never) /* dynamic key */}</span>
          </Link>
        </SidebarMenuButton>
        {badge ? <NavItemBadge badge={badge} /> : null}
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
          <SidebarMenuButton
            isActive={active}
            tooltip={t(item.labelKey as never) /* dynamic key */}
            className={navRowClassName}
            onClick={onTriggerClick}
          >
            <Icon className="size-4" aria-hidden="true" />
            <span>{t(item.labelKey as never) /* dynamic key */}</span>
            <ChevronRight className="ml-auto size-3.5! shrink-0 text-muted-foreground/70 transition-transform duration-150 group-data-[state=open]/collapsible:rotate-90 group-data-[collapsible=icon]:hidden" aria-hidden="true" />
          </SidebarMenuButton>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <SidebarMenuSub>
            {item.children.map((child) => {
              const childActive =
                pathname === child.href || pathname.startsWith(`${child.href}/`);
              return (
                <SidebarMenuSubItem key={child.labelKey}>
                  <SidebarMenuSubButton asChild className={navSubRowClassName} isActive={childActive}>
                    <Link to={child.href}>
                      {t(child.labelKey as never) /* dynamic key */}
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
  onProjectSelect,
  daemonEnabled,
}: {
  onLock: () => void;
  onProjectSelect: (project: ProjectCatalogEntry) => void;
  daemonEnabled: boolean;
}) {
  const { t } = useTranslation("chrome");
  const identity = useUiStore((s) => s.identity);
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const projectsQuery = useDaemon<ProjectsListSnapshot>(
    "ui.projects.list",
    undefined,
    { enabled: daemonEnabled },
  );
  const status = data?.data?.status;
  const projects = projectsQuery.data?.data?.projects ?? [];
  const name =
    status?.workspace ?? identity?.workspace ?? t("shell.user.fallbackWorkspace");
  const detail =
    status?.profile ??
    identity?.profile ??
    identity?.name ??
    t("shell.user.fallbackProfile");

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
            {projects.length > 0 ? (
              <>
                <DropdownMenuLabel className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
                  {t("projects.menuLabel")}
                </DropdownMenuLabel>
                {projects.map((project) => {
                  const selected =
                    project.selected ||
                    identity?.importedProject?.dataRoot === project.data_root;
                  return (
                    <DropdownMenuItem
                      key={project.id}
                      disabled={selected}
                      onSelect={(event) => {
                        event.preventDefault();
                        onProjectSelect(project);
                      }}
                    >
                      <Database className="mr-2 size-4" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate">{project.name}</span>
                      {selected ? (
                        <BadgeCheck className="ml-2 size-4 text-primary" aria-hidden="true" />
                      ) : null}
                    </DropdownMenuItem>
                  );
                })}
                <DropdownMenuSeparator />
              </>
            ) : null}
            <DropdownMenuItem asChild>
              <Link to="/books">
                <User className="mr-2 size-4" aria-hidden="true" />
                {t("shell.user.books")}
              </Link>
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => onLock()}>
              <LogOut className="mr-2 size-4" aria-hidden="true" />
              {t("shell.lockKassiber")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

/**
 * Update notice in the nav footer, ported from T3Code's `SidebarUpdatePill`:
 * a primary-tinted `rounded-lg` row with a download glyph, a label, and a ✕ that
 * dismisses it until the next launch.
 *
 * T3Code's pill also drives an in-app downloader ("Downloading (42%)" →
 * "Restart to update"). Kassiber has no updater of its own — `checkForAppUpdate`
 * only compares versions — so the pill's single action opens the release page,
 * and the download/install states are deliberately absent rather than faked.
 */
function SidebarUpdatePill() {
  const { t } = useTranslation("chrome");
  const automaticUpdateChecks = useUiStore(
    (state) => state.automaticUpdateChecks,
  );
  const appUpdate = useUiStore((state) => state.appUpdate);
  const [dismissed, setDismissed] = React.useState(false);
  const releaseUrl =
    automaticUpdateChecks && appUpdate?.updateAvailable
      ? appUpdate.releaseUrl
      : null;
  const latestVersion = appUpdate?.latestVersion;

  if (dismissed || !releaseUrl || !latestVersion) return null;

  const tooltip = t("shell.version.updateTitle", {
    current: appUpdate.currentVersion,
    latest: latestVersion,
  });

  return (
    <div className="group/update relative flex h-7 w-full items-center rounded-lg bg-primary/15 text-xs font-medium text-primary group-data-[collapsible=icon]:hidden">
      <div className="pointer-events-none absolute inset-0 rounded-lg transition-colors group-has-[button.update-main:hover]/update:bg-primary/22" />
      <button
        type="button"
        aria-label={tooltip}
        title={tooltip}
        className="update-main relative flex h-full min-w-0 flex-1 items-center gap-2 rounded-l-lg px-2"
        onClick={() => void openExternalUrl(releaseUrl).catch(() => undefined)}
      >
        <Download className="size-3.5 shrink-0" aria-hidden="true" />
        <span className="truncate">
          {t("shell.version.updateLabel", { version: latestVersion })}
        </span>
      </button>
      <button
        type="button"
        aria-label={t("shell.version.updateDismiss")}
        title={t("shell.version.updateDismiss")}
        className="relative mr-1 inline-flex size-5 shrink-0 items-center justify-center rounded-md text-primary/60 transition-colors hover:text-primary"
        onClick={() => setDismissed(true)}
      >
        <X className="size-3.5" aria-hidden="true" />
      </button>
    </div>
  );
}

function AppVersion() {
  const { t } = useTranslation("chrome");
  // With consent granted, the version line is the obvious place to ask "am I
  // current?" — same native check the "Check for Updates…" menu item runs.
  // Without consent it stays a plain link so the click never reaches GitHub.
  const canCheck =
    useUiStore((state) => state.automaticUpdateChecks) && canCheckAppUpdates();
  const buildTitle = APP_IS_DEV_BUILD
    ? t("shell.version.devTitle", { commit: APP_COMMIT })
    : t("shell.version.releaseTitle", {
        version: APP_VERSION,
        commit: APP_COMMIT,
      });
  return (
    <a
      href="https://github.com/bitcoinaustria/kassiber"
      target="_blank"
      rel="noreferrer"
      onClick={
        canCheck
          ? (event) => {
              event.preventDefault();
              void runManualAppUpdateCheck();
            }
          : undefined
      }
      title={
        canCheck
          ? t("shell.version.checkTitle", { build: buildTitle })
          : buildTitle
      }
      className="inline-flex w-full items-center justify-center gap-1 px-2 pb-1 text-center text-xs leading-none text-muted-foreground underline-offset-4 hover:text-foreground hover:underline group-data-[collapsible=icon]:hidden"
    >
      <span>
        {APP_IS_DEV_BUILD
          ? t("shell.version.devLabel")
          : t("shell.version.releaseLabel", { version: APP_VERSION })}
      </span>
      <span aria-hidden="true">·</span>
      <span className="font-mono text-xs leading-none">
        {APP_COMMIT_SHORT}
      </span>
    </a>
  );
}

/**
 * Floating shell controls.
 *
 * The old full-width top bar is gone: its navigation half moved into the side
 * nav (brand, book switcher, search), and what is left — the book-refresh split
 * button, notifications, and the view toggles — floats over the content panel as
 * frosted-glass pills. The strip still occupies its own row rather than
 * overlaying, so nothing on the page ever hides behind a button.
 */
function ShellFloatingControls({
  meta,
  onLock,
  onRefresh,
  isRefreshing,
  daemonEnabled,
}: {
  meta: RouteMeta;
  onLock: () => void;
  onRefresh: (options?: { forceFull?: boolean }) => void;
  isRefreshing: boolean;
  daemonEnabled: boolean;
}) {
  const { t } = useTranslation(["chrome", "nav"]);
  const navigate = useNavigate();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const appNotifications = useUiStore((s) => s.notifications);
  const clearNotifications = useUiStore((s) => s.clearNotifications);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const identity = useUiStore((s) => s.identity);
  const reopenFirstSyncCard = useUiStore((s) => s.reopenFirstSyncCard);
  const headerBookKey = bookIdentityKey(identity);
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const { data } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
    undefined,
    { enabled: daemonEnabled },
  );
  const snapshot = data?.data;

  const systemNotificationItems: NotificationItem[] = [
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "journals-stale",
            title: t("notifications.ledgerStale.title"),
            body: t("notifications.ledgerStale.body"),
            tone: "warning" as const,
            to: "/journals" as const,
            action: "process-journals" as const,
            actionLabel: t("notifications.ledgerStale.action"),
          },
        ]
      : []),
    ...((snapshot?.status?.quarantines ?? 0) > 0
      ? [
          {
            id: "quarantines",
            title: t("notifications.quarantined.title"),
            body: t("notifications.quarantined.body", {
              count: snapshot?.status?.quarantines ?? 0,
            }),
            tone: "warning" as const,
            to: "/quarantine" as const,
          },
        ]
      : []),
  ];
  const notificationItems: NotificationItem[] = [
    ...appNotifications.map((item) => ({
      ...item,
      to: notificationTarget(
        item.title,
        item.tone,
        developerToolsEnabled,
        item.target,
      ),
    })),
    ...systemNotificationItems,
  ];
  const notificationCount = notificationItems.filter(
    (item) => item.tone !== "info" || item.title.toLowerCase().includes("sync"),
  ).length;
  const reviewCount = snapshot?.status?.quarantines ?? 0;
  const needsJournals = Boolean(snapshot?.status?.needsJournals);
  const notificationAlertClassName =
    reviewCount > 0
      ? "border border-red-500/35 bg-red-500/10 text-red-700 hover:bg-red-500/15 hover:text-red-700 dark:text-red-300 dark:hover:text-red-300"
      : needsJournals
        ? "border border-amber-500/35 bg-amber-500/10 text-amber-700 hover:bg-amber-500/15 hover:text-amber-700 dark:text-amber-300 dark:hover:text-amber-300"
        : "";
  const notificationLabel =
    notificationCount > 0
      ? t("notifications.labelActive", { count: notificationCount })
      : t("notifications.label");

  return (
    /*
      There is no top bar: this row paints nothing at all. It reserves the
      controls' height inside the content panel — so page content never scrolls
      under a button — and the buttons themselves sit bare on the panel, as
      T3Code's workspace controls do.
    */
    <div
      className="relative z-20 flex h-[var(--kb-topbar-height)] w-full shrink-0 items-center justify-between gap-2 px-3 sm:gap-3 sm:px-4 md:px-5"
    >
      {/*
        T3Code's breadcrumb, shape for shape: the owning scope leads in muted
        text, a 40%-opacity `/` separates, and the current item sits in the
        foreground weight. It is not an ancestor chain — T3Code has no
        Breadcrumb component and no deeper trail than these two levels.
      */}
      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden sm:gap-3">
        <BreadcrumbBook daemonEnabled={daemonEnabled} />
        <span
          className="min-w-0 flex-1 truncate text-sm font-medium text-foreground"
          title={t(meta.titleKey as never) /* dynamic key */}
        >
          {t(meta.titleKey as never) /* dynamic key */}
        </span>
      </div>
      <div className="flex shrink-0 items-center gap-0.5 pl-2">
        {/* Split control: primary click runs an incremental book refresh; the
            caret opens the other "bring the book current" actions. The book
            refresh already chains source sync + auto-pair + journals, so this
            is the single home for sync / refresh / reprocess. */}
        <div className="flex items-center">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={cn(shellIconButtonClassName, "rounded-r-none")}
            aria-label={t("shell.refresh")}
            title={t("shell.refreshTitle")}
            onClick={() => onRefresh()}
          >
            <RefreshCw
              className={cn(
                "size-4",
                isRefreshing && "animate-spin motion-reduce:animate-none",
              )}
              aria-hidden="true"
            />
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className={cn(
                  shellIconButtonClassName,
                  // `border-l-*` (border-left-COLOR), not `border border-*`:
                  // the shared recipe already carries `border border-transparent`
                  // for layout, and a plain `border-border/60` sits in the same
                  // tailwind-merge group as `border-transparent`, so it replaces
                  // it and paints all four sides — which is what made this caret
                  // read as an outlined box next to its borderless neighbours.
                  // Colouring only the left edge keeps the pair reading as one
                  // split control without adding chrome the others lack.
                  "w-5 rounded-l-none border-l-border/60",
                )}
                aria-label={t("shell.refreshMenu.options")}
                title={t("shell.refreshMenu.options")}
              >
                <ChevronDown className="size-3" aria-hidden="true" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-60">
              <DropdownMenuItem onSelect={() => onRefresh()}>
                <RefreshCw className="size-4" aria-hidden="true" />
                {t("shell.refresh")}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={isProcessingJournals}
                onSelect={() => runJournalProcessing()}
              >
                <ClipboardList className="size-4" aria-hidden="true" />
                {t("shell.refreshMenu.reprocessJournals")}
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onRefresh({ forceFull: true })}>
                <RotateCcw className="size-4" aria-hidden="true" />
                {t("shell.refreshMenu.fullRescan")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "relative",
                shellIconButtonClassName,
                notificationAlertClassName,
              )}
              aria-label={notificationLabel}
              title={notificationLabel}
            >
              <Bell className="size-4" aria-hidden="true" />
              {notificationCount > 0 && (
                <span className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full bg-destructive text-3xs font-medium text-destructive-foreground">
                  {notificationCount}
                </span>
              )}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-80">
            <div className="flex items-center justify-between gap-2 px-2 py-1.5">
              <DropdownMenuLabel className="p-0">
                {t("notifications.label")}
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
                {t("notifications.clearAll")}
              </Button>
            </div>
            <DropdownMenuSeparator />
            {notificationItems.map((item) => {
              const progressLabel =
                item.progress?.label &&
                !shouldHideNotificationProgressLabel(
                  item.body,
                  item.progress.label,
                )
                  ? item.progress.label
                  : null;
              return (
                <div key={item.id} className="px-1 py-1">
                  <DropdownMenuItem
                    className="flex cursor-pointer items-start justify-between gap-3 rounded-md whitespace-normal"
                    onSelect={(event) => {
                      // An in-progress book refresh minimized via "Continue in
                      // background" re-opens the full-screen sync card (rather than
                      // navigating); letting the menu close on select reveals it.
                      // A live `progress` means a refresh is active, so this covers
                      // first sync AND later incremental refreshes.
                      if (
                        item.dedupeKey === "book-refresh" &&
                        item.progress &&
                        headerBookKey !== null
                      ) {
                        reopenFirstSyncCard(headerBookKey);
                        return;
                      }
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
                      {progressLabel ? (
                        <div className="mt-1 text-xs text-muted-foreground">
                          {progressLabel}
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
                        ? t("notifications.processing")
                        : item.actionLabel}
                    </Button>
                  ) : null}
                </div>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
        <CurrencyToggle />
        <ThemeMenu />
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            shellIconButtonClassName,
            hideSensitive && "bg-accent [&>svg]:text-foreground",
          )}
          aria-label={hideSensitive ? t("sensitive.show") : t("sensitive.hide")}
          aria-pressed={hideSensitive}
          title={hideSensitive ? t("sensitive.show") : t("sensitive.hide")}
          onClick={() => setHideSensitive(!hideSensitive)}
        >
          {hideSensitive ? (
            <EyeOff className="size-4" aria-hidden="true" />
          ) : (
            <Eye className="size-4" aria-hidden="true" />
          )}
        </Button>
        <NetworkStatusIndicator daemonEnabled={daemonEnabled} />
        <Button
          variant="ghost"
          size="icon"
          className={shellIconButtonClassName}
          aria-label={t("shell.lockKassiber")}
          title={t("shell.lockKassiberTitle")}
          onClick={onLock}
        >
          <LockKeyhole className="size-4" aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}

function ThemeMenu() {
  const { t } = useTranslation("chrome");
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const Icon = theme === "dark" ? Moon : theme === "light" ? Sun : SunMoon;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={shellIconButtonClassName}
          aria-label={t("theme.label")}
          title={t("theme.label")}
        >
          <Icon className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuLabel>{t("theme.label")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuRadioGroup
          value={theme}
          onValueChange={(value) => setTheme(value as ThemePreference)}
        >
          <DropdownMenuRadioItem value="system">
            <SunMoon className="size-4" aria-hidden="true" />
            {t("theme.system")}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="light">
            <Sun className="size-4" aria-hidden="true" />
            {t("theme.light")}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value="dark">
            <Moon className="size-4" aria-hidden="true" />
            {t("theme.dark")}
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function CurrencyToggle() {
  const { t } = useTranslation("chrome");
  const currency = useUiStore((state) => state.currency);
  const setCurrency = useUiStore((state) => state.setCurrency);
  const symbol = currency === "btc" ? "₿" : "€";
  const currentLabel =
    currency === "btc"
      ? t("currencyToggle.bitcoin")
      : t("currencyToggle.euro");
  const nextCurrency = currency === "btc" ? "eur" : "btc";
  const nextLabel =
    nextCurrency === "btc"
      ? t("currencyToggle.bitcoin")
      : t("currencyToggle.euro");

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className={shellIconButtonClassName}
      aria-label={t("currencyToggle.label", {
        current: currentLabel,
        next: nextLabel,
      })}
      aria-pressed={currency === "btc"}
      title={t("currencyToggle.title", {
        current: currentLabel,
        next: nextLabel,
      })}
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
  const { t } = useTranslation("chrome");
  const [passphrase, setPassphrase] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);
  const [touchIdSubmitting, setTouchIdSubmitting] = React.useState(false);
  const autoTouchIdPrompted = React.useRef(false);
  const canEnrollTouchId = canEnrollTouchIdPassphrase({
    platformSupported: touchIdPlatformSupported,
    passphraseRequired,
    touchIdEnabled,
    touchIdAvailable: touchIdStatus?.available !== false,
    touchIdStale: touchIdStatus?.stale === true,
  });
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
        setError(result.error ?? t("lock.passphraseFailed"));
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
        setError(result.error ?? t("lock.touchIdFailed"));
      } else {
        keepPending = true;
      }
    } finally {
      if (!keepPending) {
        setTouchIdSubmitting(false);
      }
    }
  }, [onTouchIdUnlock, submitting, t, touchIdSubmitting]);

  React.useEffect(() => {
    if (!autoTouchIdPrompt || !canUseTouchId) return;
    if (autoTouchIdPrompted.current) return;
    const { appVisible, windowFocused } = appCanStartTouchIdPrompt();
    autoTouchIdPrompted.current = true;
    if (
      !shouldAutoPromptTouchId({
        autoPromptRequested: autoTouchIdPrompt,
        canUseTouchId,
        appVisible,
        windowFocused,
      })
    ) {
      return;
    }
    void submitTouchId();
  }, [autoTouchIdPrompt, canUseTouchId, submitTouchId]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center overflow-hidden bg-background px-4 text-foreground">
      {/*
       * The nav's ledger page, run across the lock screen: the same stock the
       * chrome is made of, so a locked window still looks like the app rather
       * than a bare dialog on a flat field. It fades into `--background`
       * because that is what this overlay paints.
       *
       * Full height with `fitPages`, so the card can sit centred — where a lock
       * prompt belongs — and still have art behind it for its frost to blur.
       * `fitPages` is what makes that safe: it derives the page count from the
       * measured height, holding the ruling at one scale instead of letting it
       * coarsen with window height.
       */}
      <LedgerStageBand className="h-full" fade="var(--background)" fitPages />
      <form
        className="kb-glass-dialog relative z-10 w-full max-w-md rounded-lg border p-5 text-card-foreground"
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
                ? t("lock.passphraseRequiredTitle")
                : t("lock.booksLocked")}
            </h2>
            <p className="m-0 text-xs text-muted-foreground">
              {reason ?? t("lock.defaultReason")}
            </p>
          </div>
        </div>
        {/*
         * The passphrase field stays mounted while Touch ID is pending — it used
         * to be swapped out for a "Unlocking with Touch ID…" row, which said the
         * same thing the Touch ID button below already says while it waits, and
         * collapsed the card mid-interaction. Disabling it instead keeps one
         * progress signal, holds the card's height steady, and leaves `inputRef`
         * alive so a failed Touch ID attempt can still focus it.
         */}
        {passphraseRequired ? (
          <div className="mt-5 space-y-2">
            <label
              htmlFor="lock-passphrase"
              className="text-sm font-medium text-foreground"
            >
              {t("lock.passphrase")}
            </label>
            <Input
              id="lock-passphrase"
              ref={inputRef}
              type="password"
              autoComplete="current-password"
              value={passphrase}
              onChange={(event) => setPassphrase(event.target.value)}
              disabled={submitting || touchIdSubmitting}
            />
            {error && <p className="m-0 text-xs text-destructive">{error}</p>}
            {touchIdEnabled &&
            touchIdPlatformSupported &&
            touchIdStatus?.available === false ? (
              <p className="m-0 text-xs text-muted-foreground">
                {touchIdStatus.reason
                  ? t("lock.touchIdUnavailableReason", {
                      reason: touchIdStatus.reason,
                    })
                  : t("lock.touchIdUnavailable")}
              </p>
            ) : null}
            {touchIdEnabled &&
            touchIdPlatformSupported &&
            touchIdStatus?.available === true &&
            !touchIdStatus.configured ? (
              <p className="m-0 text-xs text-muted-foreground">
                {touchIdStatus.reason
                  ? t("lock.touchIdNotSetUpReason", {
                      reason: touchIdStatus.reason,
                    })
                  : t("lock.touchIdNotSetUp")}
              </p>
            ) : null}
            {canEnrollTouchId ? (
              <label
                htmlFor="lock-touch-id-enroll"
                className="flex items-center justify-between gap-3 rounded-md border bg-background p-3"
              >
                <span className="min-w-0">
                  <span className="block text-sm font-medium text-foreground">
                    {t("lock.useTouchIdNextTime")}
                  </span>
                  <span className="block text-xs leading-5 text-muted-foreground">
                    {t("lock.useTouchIdNextTimeBody")}
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
              ? t("lock.waitingForTouchId")
              : t("lock.unlockWithTouchId")}
          </Button>
        ) : null}
        <Button
          className="mt-5 w-full"
          type="submit"
          disabled={submitting || touchIdSubmitting}
        >
          {submitting
            ? t("lock.unlocking")
            : passphraseRequired
              ? t("lock.unlock")
              : t("lock.openBooks")}
        </Button>
        <Button
          className="mt-2 w-full"
          type="button"
          variant="ghost"
          disabled={submitting}
          onClick={onReset}
        >
          {t("lock.backToSetup")}
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
  const { t } = useTranslation("chrome");
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background px-4 text-foreground">
      <div className="w-full max-w-sm rounded-lg border border-border bg-card p-5 text-card-foreground shadow-xl ring-1 ring-border/60">
        <div className="flex items-center gap-3">
          <div className="flex size-10 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <Database className="size-5" aria-hidden="true" />
          </div>
          <div>
            <h2 className="text-base font-semibold">{t("importRoot.title")}</h2>
            <p className="m-0 text-xs text-muted-foreground">
              {t("importRoot.body")}
            </p>
          </div>
        </div>
        {error ? (
          <>
            <p className="mt-4 text-xs text-destructive">{error}</p>
            <Button className="mt-5 w-full" type="button" onClick={onReset}>
              {t("importRoot.backToSetup")}
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
