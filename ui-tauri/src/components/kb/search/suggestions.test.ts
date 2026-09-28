import { describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { buildAppSearchResults, buildAppSearchSuggestions } from "./appSearch";

const t = (key: string, options?: Record<string, unknown>) =>
  i18n.t(key as never, options as never) as unknown;

describe("buildAppSearchSuggestions", () => {
  it("offers the main pages, then everyday actions, each with its shortcut", () => {
    const suggestions = buildAppSearchSuggestions({
      aiFeaturesEnabled: true,
      developerToolsEnabled: false,
      t,
    });

    expect(suggestions.map((result) => result.id)).toEqual([
      "page:overview",
      "page:transactions",
      "page:connections",
      "page:reports",
      "page:assistant",
      "page:quarantine",
      "page:journals",
      "page:settings",
      "action:sync-wallets",
      "action:add-wallet",
      "action:process-journals",
      "action:lock-app",
    ]);
    expect(suggestions.every((result) => result.shortcut)).toBe(true);
    expect(suggestions[0]).toMatchObject({
      title: "Overview",
      shortcut: { keys: ["mod", "1"], nativeMenu: true },
    });
  });

  it("follows the same visibility rules as search", () => {
    const suggestions = buildAppSearchSuggestions({
      aiFeaturesEnabled: false,
      developerToolsEnabled: false,
      t,
    });

    expect(suggestions.map((result) => result.id)).not.toContain(
      "page:assistant",
    );
  });

  it("runs the book refresh and the lock as actions, not page jumps", () => {
    const byId = new Map(
      buildAppSearchSuggestions({
        aiFeaturesEnabled: true,
        developerToolsEnabled: true,
        t,
      }).map((result) => [result.id, result]),
    );

    expect(byId.get("action:sync-wallets")).toMatchObject({
      title: "Refresh book set",
      action: { id: "refresh-book" },
      shortcut: { keys: ["mod", "r"] },
    });
    expect(byId.get("action:sync-wallets")?.route).toBeUndefined();
    expect(byId.get("action:lock-app")).toMatchObject({
      action: { id: "lock-app" },
      shortcut: { keys: ["mod", "l"] },
    });
  });

  it("leaves typed search empty for an empty query", () => {
    expect(
      buildAppSearchResults({
        query: "  ",
        aiFeaturesEnabled: true,
        developerToolsEnabled: true,
        t,
      }),
    ).toEqual([]);
  });

  it("keeps the Wallets page, not the book refresh, on top for wallet words", () => {
    for (const query of ["wallets", "connections", "wallet"]) {
      const [top] = buildAppSearchResults({
        query,
        aiFeaturesEnabled: true,
        developerToolsEnabled: true,
        t,
      });
      expect(top?.id, query).not.toBe("action:sync-wallets");
    }
    const [top] = buildAppSearchResults({
      query: "wallets",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t,
    });
    expect(top?.id).toBe("page:connections");
  });
});
