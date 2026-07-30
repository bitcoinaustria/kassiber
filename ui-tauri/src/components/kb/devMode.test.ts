import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { buildAppSearchResults } from "./search";
import { DEV_HIDDEN_ROUTES, DEV_LOCKED_ROUTES } from "./devMode";
import {
  migrateUiState,
  useUiStore,
  UI_STATE_VERSION,
} from "@/store/ui";
import { router } from "@/routeTree";

const fixedT = i18n.getFixedT("en", null);
const t = (key: string, options?: Record<string, unknown>) =>
  fixedT(key as never, options as never) as unknown;

const devOnly = [...DEV_LOCKED_ROUTES, ...DEV_HIDDEN_ROUTES];


describe("pre-release dev mode", () => {
  it("ships off, so a fresh install never lands on an unfinished page", () => {
    expect(useUiStore.getState().developerToolsEnabled).toBe(false);
  });

  it("guards every gated route in the router, not just the nav", () => {
    // The nav greys these rows out, but the greying is a signpost: Back after
    // switching the flag off, a deep link, or a bookmark all arrive here.
    for (const route of devOnly) {
      const match = router.routesByPath[route as "/overview"];
      expect(match, `no route registered for ${route}`).toBeDefined();
      expect(
        typeof match.options.beforeLoad,
        `${route} has no beforeLoad guard`,
      ).toBe("function");
    }
  });

  // Every install from before this switch existed carries `true` from the old
  // "Enable Logs page" default, which was never a deliberate choice, so the v1
  // migration drops it once. zustand only runs `migrate` for older stored
  // versions, which is what lets a later opt-in survive.
  it("drops a pre-v1 stored opt-in exactly once", () => {
    expect(UI_STATE_VERSION).toBe(1);
    expect(
      migrateUiState({ developerToolsEnabled: true, theme: "light" }),
    ).toMatchObject({ developerToolsEnabled: false, theme: "light" });
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

      expect(on.some((r) => dev(r.route?.to) || r.id === "setting:data-sync")).toBe(
        true,
      );
      expect(off.some((r) => dev(r.route?.to))).toBe(false);
      expect(off.some((r) => r.id === "setting:data-sync")).toBe(false);
    }
  });
});
