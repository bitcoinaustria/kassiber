// @vitest-environment happy-dom
//
// Mounted: the hook routes an activated OS notification only once the shell
// is unlocked, and the bell opens itself when asked to.
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
    removeItem: (key: string) => void map.delete(key),
  });
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { NotificationCenter } from "@/components/kb/NotificationCenter";
import {
  configureOsNotifications,
  randomToken,
  recordOsNotificationActivation,
  resetOsNotifications,
  TOKEN_TTL_MS,
  useOsNotificationActivation,
} from "@/lib/osNotifications";
import {
  resetNotificationStore,
  useNotificationStore,
  type ConditionInput,
} from "@/store/notifications";
import type { ActiveBook } from "@/lib/activeBook";
import { bookIdentityKey, useUiStore, type Identity } from "@/store/ui";

import { useOsNotifications } from "./useOsNotifications";

const BOOK = { name: "Book A", workspace: "Ws", profile: "A" } as unknown as Identity;
const OTHER = { name: "Book B", workspace: "Ws", profile: "B" } as unknown as Identity;
const A = bookIdentityKey(BOOK);

const failure: ConditionInput = {
  code: "book-refresh.failed",
  scope: { book: A },
  severity: "error",
  priority: "attention",
  params: { forceFull: true },
  action: {
    type: "operation",
    operation: { kind: "refresh-book", forceFull: true },
    review: "/connections",
  },
};

/** The open book as the shell's profiles read would name it. */
function activeBookFor(identity: Identity | null): ActiveBook | null {
  if (!identity) return null;
  const { workspace, profile } = identity as unknown as {
    workspace: string;
    profile: string;
  };
  return {
    key: bookIdentityKey(identity)!,
    expected: { workspace_id: workspace, profile_id: profile },
  };
}

function Shell({ unlocked }: { unlocked: boolean }) {
  const activeBook = activeBookFor(useUiStore((s) => s.identity));
  useOsNotifications(unlocked, activeBook);
  return unlocked ? (
    <NotificationCenter runOperation={runOperation} currentBook={activeBook} />
  ) : null;
}

const runOperation = vi.fn();
let clock = 1_000_000;
const invoke = vi.fn(async (command: string) =>
  command === "deliver_os_notification" ? "delivered" : null,
);
const tokens: string[] = [];

/** Mount, then swap in test transport for the live one the hook wires. */
function mount(unlocked: boolean) {
  const view = render(<Shell unlocked={unlocked} />);
  configureOsNotifications({
    enabled: () => useUiStore.getState().desktopNotifications,
    available: () => true,
    windowFocused: async () => false,
    invoke,
    now: () => clock,
    token: () => {
      const token = randomToken();
      tokens.push(token);
      return token;
    },
  });
  return view;
}

async function deliveredToken(): Promise<string> {
  act(() => useNotificationStore.getState().raiseCondition(failure));
  await vi.waitFor(() => expect(tokens).toHaveLength(1));
  await vi.waitFor(() => expect(invoke).toHaveBeenCalled());
  return tokens[0];
}

beforeEach(() => {
  clock = 1_000_000;
  tokens.length = 0;
  invoke.mockClear();
  navigate.mockClear();
  runOperation.mockClear();
  resetNotificationStore();
  resetOsNotifications();
  useUiStore.setState({ identity: BOOK, desktopNotifications: true });
});

afterEach(() => {
  cleanup();
  resetOsNotifications();
  useUiStore.setState({ identity: null, desktopNotifications: false });
});

describe("useOsNotifications activation", () => {
  it("waits for unlock, then opens the item's review page without running it", async () => {
    const view = mount(false);
    const token = await deliveredToken();
    expect(invoke).toHaveBeenCalledWith("deliver_os_notification", {
      category: "needs-attention",
      token,
    });

    act(() => recordOsNotificationActivation(token, clock + 1000));
    expect(navigate).not.toHaveBeenCalled();
    expect(useOsNotificationActivation.getState().pending).not.toBeNull();

    view.rerender(<Shell unlocked />);
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
    expect(runOperation).not.toHaveBeenCalled();
    expect(useOsNotificationActivation.getState().pending).toBeNull();
    // Nothing but the delivery reached the native side.
    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "deliver_os_notification",
    ]);
  });

  it("routes at once when the shell is already unlocked", async () => {
    mount(true);
    const token = await deliveredToken();
    act(() => recordOsNotificationActivation(token, clock));
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
  });

  it("opens the center for an expired token", async () => {
    mount(true);
    const token = await deliveredToken();
    act(() => recordOsNotificationActivation(token, clock + TOKEN_TTL_MS + 1));
    expect(navigate).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(useNotificationStore.getState().centerOpenRequested).toBe(false);
  });

  it("opens the center after a restart (unknown token)", () => {
    const view = mount(false);
    act(() => recordOsNotificationActivation(randomToken()));
    view.rerender(<Shell unlocked />);
    expect(navigate).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("opens the center when another book is open by then", async () => {
    const view = mount(false);
    const token = await deliveredToken();
    act(() => recordOsNotificationActivation(token, clock));
    act(() => useUiStore.setState({ identity: OTHER }));
    view.rerender(<Shell unlocked />);
    expect(navigate).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("offers nothing while the setting is off", async () => {
    useUiStore.setState({ desktopNotifications: false });
    mount(true);
    act(() => useNotificationStore.getState().raiseCondition(failure));
    await Promise.resolve();
    await Promise.resolve();
    expect(invoke).not.toHaveBeenCalled();
  });
});
