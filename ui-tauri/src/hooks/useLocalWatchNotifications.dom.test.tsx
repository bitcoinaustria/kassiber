// @vitest-environment happy-dom
import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const inbox = vi.hoisted(() => ({
  result: { isSuccess: true, data: { data: { unread_count: 0 } } } as {
    isSuccess: boolean;
    data?: { data?: { unread_count: number } };
  },
}));

vi.mock("@/daemon/client", () => ({ useDaemon: () => inbox.result }));

import { resolveEntryAction } from "@/components/kb/notificationRouting";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";
import { useUiStore } from "@/store/ui";

import { useLocalWatchNotifications } from "./useLocalWatchNotifications";

beforeEach(() => {
  resetNotificationStore();
  useUiStore.setState({ identity: null });
});

describe("evidence-watch condition", () => {
  it("waits while the inbox has unread changes, without counting polls, and opens Chain Analysis", () => {
    inbox.result = { isSuccess: true, data: { data: { unread_count: 2 } } };
    const hook = renderHook(() => useLocalWatchNotifications(true));
    hook.rerender();
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition).toMatchObject({ code: "watch.evidence-changed", occurrenceCount: 1 });
    expect(resolveEntryAction(condition.action, condition.scope, null, false)).toEqual({
      kind: "navigate",
      to: "/chain-analysis",
      stale: false,
    });

    // A failed read proves nothing.
    inbox.result = { isSuccess: false };
    hook.rerender();
    expect(Object.keys(useNotificationStore.getState().conditions)).toHaveLength(1);

    inbox.result = { isSuccess: true, data: { data: { unread_count: 0 } } };
    hook.rerender();
    expect(useNotificationStore.getState().conditions).toEqual({});
  });
});
