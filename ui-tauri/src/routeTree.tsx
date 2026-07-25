/**
 * Hand-built route tree + router.
 *
 * Once more screens are translated we'll switch to TanStack Router's
 * file-based routing (`@tanstack/router-plugin/vite`) which generates
 * `routeTree.gen.ts` automatically.
 *
 * Layout: `/` shows Welcome (no chrome). Authenticated routes mount
 * under the AppShell layout and require a persisted identity; otherwise
 * the layout redirects to `/`.
 */
import {
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
  Outlet,
  redirect,
} from "@tanstack/react-router";
import { RootIntentListener } from "./components/kb/RootIntentListener";
import { settingsSectionRoute } from "./components/kb/settingsSections";
import { activateImportProject, canImportProjects } from "./daemon/transport";
import { useUiStore } from "./store/ui";

function RoutePending() {
  return (
    <div className="grid min-h-dvh place-items-center bg-background text-sm text-muted-foreground">
      Loading...
    </div>
  );
}

const Welcome = lazyRouteComponent(() => import("./routes/Welcome"), "Welcome");
const AppShell = lazyRouteComponent(
  () => import("./components/kb/AppShell"),
  "AppShell",
);
const Overview = lazyRouteComponent(
  () => import("./routes/Overview"),
  "Overview",
);
const Transactions = lazyRouteComponent(
  () => import("./routes/Transactions"),
  "Transactions",
);
const Activity = lazyRouteComponent(
  () => import("./routes/Activity"),
  "Activity",
);
const Reports = lazyRouteComponent(() => import("./routes/Reports"), "Reports");
const PrivacyMirror = lazyRouteComponent(
  () => import("./routes/PrivacyMirror"),
  "PrivacyMirror",
);
const ExitTax = lazyRouteComponent(() => import("./routes/ExitTax"), "ExitTax");
const SourceFunds = lazyRouteComponent(
  () => import("./routes/source-funds"),
  "SourceFunds",
);
const Journals = lazyRouteComponent(
  () => import("./routes/Journals"),
  "Journals",
);
const SwapMatching = lazyRouteComponent(
  () => import("./routes/SwapMatching"),
  "SwapMatching",
);
const CustodyGaps = lazyRouteComponent(
  () => import("./routes/CustodyGaps"),
  "CustodyGaps",
);
const Quarantine = lazyRouteComponent(
  () => import("./routes/Quarantine"),
  "Quarantine",
);
const Reconcile = lazyRouteComponent(
  () => import("./routes/Reconcile"),
  "Reconcile",
);
const Egress = lazyRouteComponent(() => import("./routes/Egress"), "Egress");
const Logs = lazyRouteComponent(() => import("./routes/Logs"), "Logs");
const Books = lazyRouteComponent(() => import("./routes/Books"), "Books");
const BirdsEye = lazyRouteComponent(
  () => import("./routes/BirdsEye"),
  "BirdsEye",
);
const Connections = lazyRouteComponent(
  () => import("./routes/Connections"),
  "Connections",
);
const ConnectionDetail = lazyRouteComponent(
  () => import("./routes/ConnectionDetail"),
  "ConnectionDetail",
);
const Imports = lazyRouteComponent(() => import("./routes/Imports"), "Imports");
const Settings = lazyRouteComponent(
  () => import("./routes/Settings"),
  "Settings",
);
const Assistant = lazyRouteComponent(
  () => import("./routes/Assistant"),
  "Assistant",
);

const rootRoute = createRootRoute({
  component: () => (
    <>
      <RootIntentListener />
      <Outlet />
    </>
  ),
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    if (useUiStore.getState().identity) {
      throw redirect({ to: "/overview" });
    }
  },
  component: Welcome,
});

const appLayoutRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "_app",
  beforeLoad: async () => {
    const { identity, setIdentity } = useUiStore.getState();
    if (!identity) {
      throw redirect({ to: "/" });
    }
    if (!identity.importedProject) {
      return;
    }
    if (!canImportProjects()) {
      setIdentity(null);
      throw redirect({ to: "/" });
    }
    try {
      await activateImportProject(identity.importedProject.dataRoot);
    } catch {
      setIdentity(null);
      throw redirect({ to: "/" });
    }
  },
  component: AppShell,
});

const overviewRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/overview",
  component: Overview,
});

const transactionsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/transactions",
  component: Transactions,
});

const activityRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/activity",
  component: Activity,
});

const reportsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/reports",
  component: Reports,
});

const privacyMirrorRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/privacy-mirror",
  component: PrivacyMirror,
});

const exitTaxRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/exit-tax",
  component: ExitTax,
});

const sourceFundsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/source-of-funds",
  component: SourceFunds,
});

const journalsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/journals",
  component: Journals,
});

const taxEventsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/tax-events",
  beforeLoad: () => {
    throw redirect({ to: "/journals" });
  },
});

const swapMatchingRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/swaps",
  component: SwapMatching,
});

const custodyGapsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/custody-gaps",
  component: CustodyGaps,
});

const transferMatchingRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/transfers",
  beforeLoad: () => {
    throw redirect({ to: "/swaps" });
  },
});

const quarantineRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/quarantine",
  component: Quarantine,
});

const reconcileRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/reconcile",
  component: Reconcile,
});

const egressRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/egress",
  component: Egress,
});

const logsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/logs",
  beforeLoad: () => {
    if (!useUiStore.getState().developerToolsEnabled) {
      throw redirect({ to: "/overview" });
    }
  },
  component: Logs,
});

const diagnosticsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/diagnostics",
  beforeLoad: () => {
    if (!useUiStore.getState().developerToolsEnabled) {
      throw redirect({ to: "/overview" });
    }
    throw redirect({ to: "/logs" });
  },
});

const booksRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/books",
  component: Books,
});

const birdsEyeRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/books/$workspaceId/birds-eye",
  component: BirdsEye,
});

const profilesRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/profiles",
  beforeLoad: () => {
    throw redirect({ to: "/books" });
  },
});

const connectionsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/connections",
  component: Connections,
});

const connectionDetailRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/connections/$connectionId",
  component: ConnectionDetail,
});

const importsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/imports",
  component: Imports,
});

/**
 * Settings is a section of routes, one URL per category, so a category can be
 * bookmarked, deep-linked, and shown in the side nav as a real link.
 *
 * A bare `/settings` visit is never rendered: it redirects to the category its
 * (legacy) `#hash` names, or to the default category. That keeps every existing
 * `navigate({ to: "/settings", hash: "market" })` call site working without
 * change — the hash still selects the panel, it just resolves to a route now.
 */
const settingsIndexRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings",
  beforeLoad: ({ location }) => {
    throw redirect({ to: settingsSectionRoute(location.hash), replace: true });
  },
});

/*
 * The category routes are spelled out one by one rather than generated from
 * `SETTINGS_SECTION_SLUG`: TanStack Router infers its typed `to` union from
 * literal `path` strings, and a loop or a path-taking helper widens them to
 * `string`, which would silently un-type every `<Link to="/settings/…">` in the
 * app. `SETTINGS_SECTION_ROUTE` is `satisfies`-checked against the same slugs,
 * so a section added there without a route here fails to compile at its call
 * sites instead of 404-ing at runtime.
 */
const settingsAppearanceRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/appearance",
  component: () => <Settings section="general-appearance" />,
});

const settingsMarketRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/market",
  component: () => <Settings section="network-market" />,
});

const settingsBitcoinRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/bitcoin",
  component: () => <Settings section="network-bitcoin" />,
});

const settingsLightningRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/lightning",
  component: () => <Settings section="network-lightning" />,
});

const settingsLiquidRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/liquid",
  component: () => <Settings section="network-liquid" />,
});

const settingsPrivacyRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/privacy",
  component: () => <Settings section="security-privacy" />,
});

const settingsSecurityRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/security",
  component: () => <Settings section="security-lock" />,
});

const settingsAiRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/ai",
  component: () => <Settings section="assistant-ai" />,
});

const settingsSyncRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/sync",
  component: () => <Settings section="data-sync" />,
});

const settingsDataRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/data",
  component: () => <Settings section="data-storage" />,
});

const settingsTerminalRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/terminal",
  component: () => <Settings section="desktop-terminal" />,
});

const settingsDeveloperRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings/developer",
  component: () => <Settings section="desktop-developer" />,
});

const assistantRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/assistant",
  beforeLoad: () => {
    if (!useUiStore.getState().aiFeaturesEnabled) {
      throw redirect({ to: "/overview" });
    }
  },
  component: Assistant,
});

const assistantTypoRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/assitant",
  beforeLoad: () => {
    if (!useUiStore.getState().aiFeaturesEnabled) {
      throw redirect({ to: "/overview" });
    }
    throw redirect({ to: "/assistant" });
  },
});

const importsAliasRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/Imports",
  beforeLoad: () => {
    throw redirect({ to: "/connections" });
  },
});

const proofFundsAliasRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/proof-funds",
  beforeLoad: () => {
    throw redirect({ to: "/source-of-funds" });
  },
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  assistantTypoRoute,
  importsAliasRoute,
  proofFundsAliasRoute,
  appLayoutRoute.addChildren([
    overviewRoute,
    transactionsRoute,
    activityRoute,
    reportsRoute,
    privacyMirrorRoute,
    exitTaxRoute,
    sourceFundsRoute,
    journalsRoute,
    swapMatchingRoute,
    custodyGapsRoute,
    transferMatchingRoute,
    taxEventsRoute,
    quarantineRoute,
    reconcileRoute,
    egressRoute,
    logsRoute,
    diagnosticsRoute,
    booksRoute,
    birdsEyeRoute,
    profilesRoute,
    connectionsRoute,
    connectionDetailRoute,
    importsRoute,
    settingsIndexRoute,
    settingsAppearanceRoute,
    settingsMarketRoute,
    settingsBitcoinRoute,
    settingsLightningRoute,
    settingsLiquidRoute,
    settingsPrivacyRoute,
    settingsSecurityRoute,
    settingsAiRoute,
    settingsSyncRoute,
    settingsDataRoute,
    settingsTerminalRoute,
    settingsDeveloperRoute,
    assistantRoute,
  ]),
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  defaultPendingComponent: RoutePending,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
