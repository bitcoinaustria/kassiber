/**
 * Hand-built route tree + router.
 *
 * Once more screens are translated we'll switch to TanStack Router's
 * file-based routing (`@tanstack/router-plugin/vite`) which generates
 * `routeTree.gen.ts` automatically.
 *
 * Layout: `/` shows Welcome (no chrome). Authenticated routes mount
 * under the AppShell layout (header + outlet + footer) and require
 * a persisted identity; otherwise the layout redirects to `/`.
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
import { Profiles } from "./routes/Profiles";
import { Connections } from "./routes/Connections";
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

const routeTree = rootRoute.addChildren([
  indexRoute,
  appLayoutRoute.addChildren([
    overviewRoute,
    transactionsRoute,
    reportsRoute,
    profilesRoute,
    connectionsRoute,
  ]),
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
