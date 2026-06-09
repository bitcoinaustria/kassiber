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
const SourceFunds = lazyRouteComponent(
  () => import("./routes/SourceFunds"),
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
const Quarantine = lazyRouteComponent(
  () => import("./routes/Quarantine"),
  "Quarantine",
);
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

const settingsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/settings",
  component: Settings,
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
    sourceFundsRoute,
    journalsRoute,
    swapMatchingRoute,
    transferMatchingRoute,
    taxEventsRoute,
    quarantineRoute,
    logsRoute,
    diagnosticsRoute,
    booksRoute,
    birdsEyeRoute,
    profilesRoute,
    connectionsRoute,
    connectionDetailRoute,
    importsRoute,
    settingsRoute,
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
