import { describe, expect, it } from "vitest";

import { planHeaderRefresh } from "./headerRefresh";

describe("planHeaderRefresh", () => {
  it("does nothing without a workspace (the caller redirects to onboarding)", () => {
    expect(
      planHeaderRefresh({ hasWorkspace: false, bookKey: "ws:profile" }),
    ).toEqual({ reopenSyncCardForBook: null, startRefresh: false });
  });

  it("surfaces the loader for the active book and starts a refresh", () => {
    expect(
      planHeaderRefresh({ hasWorkspace: true, bookKey: "ws:profile" }),
    ).toEqual({ reopenSyncCardForBook: "ws:profile", startRefresh: true });
  });

  it("still refreshes when no book key is resolvable, without re-opening a card", () => {
    expect(planHeaderRefresh({ hasWorkspace: true, bookKey: null })).toEqual({
      reopenSyncCardForBook: null,
      startRefresh: true,
    });
  });
});
