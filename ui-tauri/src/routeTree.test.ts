import { createMemoryHistory } from "@tanstack/react-router";
import { describe, expect, it } from "vitest";

import { router } from "./routeTree";

describe("imports route", () => {
  it.each(["/imports", "/Imports"])("mounts the imports page for %s", (path) => {
    router.update({ history: createMemoryHistory({ initialEntries: [path] }) });
    expect(router.matchRoutes(path).at(-1)?.fullPath).toBe("/imports");
  });
});

describe("custody route aliases", () => {
  it("keeps the legacy gap guard and redirects the selected gap", () => {
    const route = router.routesByPath["/custody-gaps"];
    const load = route.options.loader as unknown as (ctx: { deps: Record<string, unknown> }) => never;
    try {
      load({ deps: { gap_id: "gap-123" } });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/swaps", search: { tab: "gaps", gap: "gap-123" }, replace: true } });
    }
  });

  it.each(["gaps", "components"])("blocks a direct %s tab link while developer mode is off", (tab) => {
    const guard = router.routesByPath["/swaps"].options.beforeLoad as unknown as (ctx: { search: { tab: string } }) => void;
    try {
      guard({ search: { tab } });
      throw new Error("expected redirect");
    } catch (redirect) {
      expect(redirect).toMatchObject({ options: { to: "/swaps", search: {}, replace: true } });
    }
  });

  it("validates the unified route before selecting its view", () => {
    const validate = router.routesByPath["/swaps"].options.validateSearch as (raw: Record<string, unknown>) => Record<string, unknown>;
    expect(validate({ tab: "gaps", mode: "swaps", view: "paired", focus: "tx-1" })).toEqual({ tab: "review", mode: "transfers", view: "review", focus: "tx-1" });
  });
});
