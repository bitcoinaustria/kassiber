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
  Outlet,
  redirect,
} from "@tanstack/react-router";
import { Welcome } from "./routes/Welcome";
import { Overview } from "./routes/Overview";
import { Transactions } from "./routes/Transactions";
import { Reports } from "./routes/Reports";
import { Journals } from "./routes/Journals";
import { TaxEvents } from "./routes/TaxEvents";
import { Quarantine } from "./routes/Quarantine";
import { Profiles } from "./routes/Profiles";
import { Connections } from "./routes/Connections";
import { ConnectionDetail } from "./routes/ConnectionDetail";
import { Assistant } from "./routes/Assistant";
import { AppShell } from "./components/kb/AppShell";
import { useUiStore } from "./store/ui";

const rootRoute = createRootRoute({
  component: () => <Outlet />,
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
  beforeLoad: () => {
    if (!useUiStore.getState().identity) {
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

const reportsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/reports",
  component: Reports,
});

const journalsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/journals",
  component: Journals,
});

const taxEventsRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/tax-events",
  component: TaxEvents,
});

const quarantineRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/quarantine",
  component: Quarantine,
});

const profilesRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/profiles",
  component: Profiles,
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

const assistantRoute = createRoute({
  getParentRoute: () => appLayoutRoute,
  path: "/assistant",
  component: Assistant,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  appLayoutRoute.addChildren([
    overviewRoute,
    transactionsRoute,
    reportsRoute,
    journalsRoute,
    taxEventsRoute,
    quarantineRoute,
    profilesRoute,
    connectionsRoute,
    connectionDetailRoute,
    assistantRoute,
  ]),
]);

export const router = createRouter({ routeTree, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
