import { describe, expect, it } from "vitest";
import { scopedDaemonArgs, type DaemonScopeBoundary } from "./client";
import { useUiStore } from "@/store/ui";

describe("scoped setup daemon requests", () => {
  it("overrides caller scope with the captured book and rejects switches before dispatch", () => {
    const session = useUiStore.getState().daemonSession;
    const boundary: DaemonScopeBoundary = { daemonSession: session,
      expectedScope: { workspace_id: "workspace", profile_id: "original-profile" } };
    expect(scopedDaemonArgs({ wallet: "Wallet", expected_scope: { profile_id: "other" } }, boundary)).toEqual({
      wallet: "Wallet", expected_scope: boundary.expectedScope,
    });
    expect(() => scopedDaemonArgs({}, { ...boundary, daemonSession: session - 1 })).toThrow("different book");
    expect(() => scopedDaemonArgs({}, { ...boundary, isCurrent: () => false })).toThrow("conversation");
    expect(scopedDaemonArgs(undefined, null)).toBeUndefined();
  });
});
