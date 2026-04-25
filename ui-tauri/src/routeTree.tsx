/**
 * Hand-built route tree.
 *
 * Once screens are translated, switch to TanStack Router's file-based
 * routing (`@tanstack/router-plugin/vite`) which generates `routeTree.gen.ts`
 * automatically. For the scaffold we keep a single placeholder index route
 * so the router is wired end-to-end.
 */

import {
  createRootRoute,
  createRoute,
  Outlet,
} from "@tanstack/react-router";
import { ScaffoldHome } from "./routes/ScaffoldHome";

const rootRoute = createRootRoute({
  component: () => <Outlet />,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: ScaffoldHome,
});

export const routeTree = rootRoute.addChildren([indexRoute]);
