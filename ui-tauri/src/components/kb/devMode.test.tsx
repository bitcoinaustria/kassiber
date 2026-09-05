import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { buildAppSearchResults } from "./search";
import { renderToStaticMarkup } from "react-dom/server";

import {
  DEV_HIDDEN_ROUTES,
  DEV_LOCK_CLASS,
  DEV_LOCKED_ROUTES,
  devLockProps,
  isDevOnlyConnectionSource,
  isDevOnlySettingsSlug,
} from "./devMode";
import { CONNECTION_SOURCES } from "@/lib/connectionCatalog";
import {
  migrateUiState,
  useUiStore,
  UI_STATE_VERSION,
} from "@/store/ui";
import { requireDeveloperTools, router } from "@/routeTree";

const fixedT = i18n.getFixedT("en", null);
const t = (key: string, options?: Record<string, unknown>) =>
  fixedT(key as never, options as never) as unknown;

const devOnly = [...DEV_LOCKED_ROUTES, ...DEV_HIDDEN_ROUTES];


describe("pre-release dev mode", () => {
  it("ships off, so a fresh install never lands on an unfinished page", () => {
    expect(useUiStore.getState().developerToolsEnabled).toBe(false);
  });

  // Spelled out rather than derived: every other test in this file iterates
  // these arrays, so dropping a route would shrink the loops and stay green
  // while the surface silently went live.
  it("gates exactly the agreed surfaces", () => {
    expect([...DEV_LOCKED_ROUTES]).toEqual([
      "/activity",
      "/custody-gaps",
      "/exit-tax",
      "/privacy-mirror",
      "/source-of-funds",
    ]);
    expect([...DEV_HIDDEN_ROUTES]).toEqual([
      "/egress",
      "/logs",
      "/settings/lightning",
      "/settings/sync",
    ]);
  });

  // Attaching a Lightning node is the settings section plus the two catalog
  // cards that lead into it; hiding one without the other leaves a dead end.
  it("hides the Lightning node connections with their settings section", () => {
    expect(CONNECTION_SOURCES.filter((s) => isDevOnlyConnectionSource(s.id)).map(
      (s) => s.id,
    )).toEqual(["core-ln", "lnd"]);
    expect(isDevOnlySettingsSlug("lightning")).toBe(true);
  });

  describe("devLockProps", () => {
    const hint = "Early-stage feature.";

    it("renders a visible, inert, self-explaining row", () => {
      const lock = devLockProps(true, hint);
      // Spread last, as a caller naturally would: the props must not carry a
      // `className` of their own, or they would drop the row's own layout.
      expect(lock).not.toHaveProperty("className");
      const html = renderToStaticMarkup(
        <a href="/x" className={`base-layout ${DEV_LOCK_CLASS}`} {...lock}>
          row
        </a>,
      );
      expect(html).toContain("base-layout");
      expect(html).toContain("opacity-50");
      expect(html).toContain("cursor-not-allowed");
      expect(html).toContain('aria-disabled="true"');
      expect(html).toContain(hint);
      // Still reachable by keyboard: a row nobody can focus cannot announce
      // itself as unavailable either.
      expect(html).not.toContain("tabindex");
    });

    it("blocks activation, including the keyboard's synthesized click", () => {
      let defaultPrevented = false;
      devLockProps(true, hint).onClick?.({
        preventDefault: () => {
          defaultPrevented = true;
        },
      });
      expect(defaultPrevented).toBe(true);
    });

    it("is inert when unlocked", () => {
      expect(devLockProps(false, hint)).toEqual({});
    });
  });

  it("guards every gated route in the router, not just the nav", () => {
    // The nav greys these rows out, but the greying is a signpost: Back after
    // switching the flag off, a deep link, or a bookmark all arrive here.
    for (const route of devOnly) {
      const match = router.routesByPath[route as "/overview"];
      expect(match, `no route registered for ${route}`).toBeDefined();
      // Identity, not just "has a beforeLoad" — plenty of routes carry an
      // unrelated redirect guard.
      expect(match.options.beforeLoad, `${route} is not dev-gated`).toBe(
        requireDeveloperTools,
      );
    }
  });

  // Every install from before this switch existed carries `true` from the old
  // "Enable Logs page" default, which was never a deliberate choice, so the v1
  // migration drops it once. zustand only runs `migrate` for older stored
  // versions, which is what lets a later opt-in survive.
  it("drops a pre-v1 stored opt-in exactly once", () => {
    expect(UI_STATE_VERSION).toBe(1);
    expect(
      migrateUiState({ developerToolsEnabled: true, theme: "light" }, 0),
    ).toMatchObject({ developerToolsEnabled: false, theme: "light" });
    // A deliberate opt-in must survive the next version bump.
    expect(
      migrateUiState({ developerToolsEnabled: true }, 1),
    ).toMatchObject({ developerToolsEnabled: true });
  });

  it("keeps dev-only pages and device sync out of the app search", () => {
    // One query per surface, so a page dropping out of PAGE_RESULTS cannot make
    // this pass vacuously — each query has to match something when dev mode is on.
    for (const query of [
      "exit tax",
      "custody gaps",
      "network monitor",
      "logs",
      "device sync",
    ]) {
      const options = { query, aiFeaturesEnabled: true, t };
      const off = buildAppSearchResults({
        ...options,
        developerToolsEnabled: false,
      });
      const on = buildAppSearchResults({
        ...options,
        developerToolsEnabled: true,
      });
      const dev = (route: string | undefined) =>
        Boolean(route && devOnly.includes(route as (typeof devOnly)[number]));

      expect(on.some((r) => dev(r.route?.to) || r.id === "page:custody-gaps" || r.id === "page:custody-components" || r.id === "setting:data-sync")).toBe(
        true,
      );
      expect(off.some((r) => dev(r.route?.to) || r.id === "page:custody-gaps" || r.id === "page:custody-components")).toBe(false);
      expect(off.some((r) => r.id === "setting:data-sync")).toBe(false);
    }
  });
});
