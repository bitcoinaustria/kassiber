/**
 * Hand-built route tree + router.
 *
 * Once more screens are translated we'll switch to TanStack Router's
 * file-based routing (`@tanstack/router-plugin/vite`) which generates
 * `routeTree.gen.ts` automatically.
 *
 * `/` chooses Welcome or Overview based on whether onboarding has run.
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

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/overview",
  beforeLoad: () => {
    if (!useUiStore.getState().identity) {
      throw redirect({ to: "/" });
    }
  },
  component: Overview,
});

const routeTree = rootRoute.addChildren([indexRoute, overviewRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
