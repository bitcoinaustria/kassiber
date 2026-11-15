import { createMemoryHistory } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";

import { router } from "./routeTree";

describe("imports route", () => {
  it.each(["/imports", "/Imports"])("mounts the imports page for %s", (path) => {
    router.update({ history: createMemoryHistory({ initialEntries: [path] }) });
    expect(router.matchRoutes(path).at(-1)?.fullPath).toBe("/imports");
  });
});

describe("transfers route aliases", () => {
  it("keeps the legacy gap guard and redirects the selected gap to the advanced tools", () => {
    const route = router.routesByPath["/custody-gaps"];
    const load = route.options.loader as unknown as (ctx: { deps: Record<string, unknown> }) => never;
    try {
      load({ deps: { gap_id: "gap-123" } });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/transfers", search: { section: "advanced", tab: "gaps", gap: "gap-123" }, replace: true } });
    }
  });

  it.each(["gaps", "components"])("drops a legacy %s tab link to the overview while developer mode is off", (tab) => {
    const guard = router.routesByPath["/swaps"].options.beforeLoad as unknown as (ctx: { search: Record<string, unknown> }) => void;
    const validate = router.routesByPath["/swaps"].options.validateSearch as (raw: Record<string, unknown>) => Record<string, unknown>;
    try {
      guard({ search: validate({ tab }) });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/transfers", search: {}, replace: true } });
    }
  });

  it("blocks the advanced tools on the canonical route while developer mode is off", () => {
    const guard = router.routesByPath["/transfers"].options.beforeLoad as unknown as (ctx: { search: Record<string, unknown> }) => void;
    try {
      guard({ search: { section: "advanced", tab: "components" } });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/transfers", search: {}, replace: true } });
    }
    expect(() => guard({ search: { section: "tools" } })).not.toThrow();
  });

  it("validates the route before selecting its view", () => {
    const validate = router.routesByPath["/transfers"].options.validateSearch as (raw: Record<string, unknown>) => Record<string, unknown>;
    // A focused candidate from an old link opens "Link manually" on it.
    expect(validate({ tab: "gaps", mode: "transfers", view: "paired", focus: "tx-1" })).toEqual({ link: "tx-1" });
    expect(validate({ link: "tx-2" })).toEqual({ link: "tx-2" });
  });

  it("redirects the old route with its navigation state", () => {
    const guard = router.routesByPath["/swaps"].options.beforeLoad as unknown as (ctx: { search: Record<string, unknown> }) => void;
    try {
      guard({ search: { link: "tx-1" } });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/transfers", search: { link: "tx-1" }, replace: true } });
    }
  });
});
