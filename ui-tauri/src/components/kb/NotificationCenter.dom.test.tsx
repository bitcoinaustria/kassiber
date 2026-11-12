// @vitest-environment happy-dom
//
// Mounted, not static: expanding, dismissing and the action buttons only exist
// once the popover is open, and what reaches `kb.ui` depends on the live store.
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const storage = vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
    removeItem: (key: string) => void map.delete(key),
  });
  // Radix positions the popover with a ResizeObserver this DOM does not ship.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  return map;
});

const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { UI_STATE_VERSION, useUiStore, type AppNotification } from "@/store/ui";

import { NotificationCenter } from "./NotificationCenter";

const MINUTE = 60_000;

function notification(
  overrides: Partial<AppNotification> & Pick<AppNotification, "id" | "title">,
): AppNotification {
  return {
    body: "Body text.",
    tone: "info",
    createdAt: new Date(Date.now() - 2 * MINUTE - 5_000).toISOString(),
    ...overrides,
  };
}

function openBell(props: Partial<React.ComponentProps<typeof NotificationCenter>> = {}) {
  const runCommand = vi.fn();
  render(<NotificationCenter runCommand={runCommand} {...props} />);
  act(() => {
    fireEvent.click(screen.getByRole("button", { name: /^Notifications/ }));
  });
  return { runCommand };
}

function entry(title: string): HTMLElement {
  const match = screen
    .getAllByTestId("notification-entry")
    .find((element) => element.textContent?.includes(title));
  if (!match) throw new Error(`no entry titled ${title}`);
  return match;
}

beforeEach(() => {
  navigate.mockReset();
  useUiStore.setState({
    lang: "en",
    developerToolsEnabled: false,
    notifications: [],
  });
});

afterEach(() => {
  cleanup();
});

describe("NotificationCenter", () => {
  it("shows when each entry arrived, relative and in full", () => {
    useUiStore.setState({
      notifications: [notification({ id: "a", title: "Export finished" })],
    });
    openBell();

    const time = within(entry("Export finished")).getByText("2 min. ago");
    expect(time.tagName).toBe("TIME");
    expect(time.getAttribute("dateTime")).toBe(
      useUiStore.getState().notifications[0].createdAt,
    );
    expect(time.getAttribute("title")).toMatch(/\d{4}/);
  });

  it("expands to the full body and detail lines, and collapses again", () => {
    useUiStore.setState({
      notifications: [
        notification({
          id: "a",
          title: "Book refresh needs attention",
          tone: "warning",
          body: "1 completed, 1 needs attention",
          details: ["Cold storage: Electrum timeout"],
        }),
      ],
    });
    openBell();
    const row = entry("Book refresh needs attention");

    expect(within(row).queryByText("Cold storage: Electrum timeout")).toBeNull();
    const toggle = within(row).getByRole("button", { name: /Show more/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");

    act(() => void fireEvent.click(toggle));
    expect(within(row).getByText("Cold storage: Electrum timeout")).toBeTruthy();
    const collapse = within(row).getByRole("button", { name: /Show less/ });
    expect(collapse.getAttribute("aria-expanded")).toBe("true");

    act(() => void fireEvent.click(collapse));
    expect(within(row).queryByText("Cold storage: Electrum timeout")).toBeNull();
  });

  it("offers no toggle when the body already shows in full", () => {
    useUiStore.setState({
      notifications: [notification({ id: "a", title: "Copied" })],
    });
    openBell();
    expect(
      within(entry("Copied")).queryByRole("button", { name: /Show more/ }),
    ).toBeNull();
  });

  it("dismisses one entry and leaves the others", () => {
    useUiStore.setState({
      notifications: [
        notification({ id: "a", title: "Export finished" }),
        notification({ id: "b", title: "Import rolled back" }),
      ],
    });
    openBell();

    act(() => {
      fireEvent.click(
        screen.getByRole("button", {
          name: "Dismiss notification: Export finished",
        }),
      );
    });

    expect(useUiStore.getState().notifications.map((item) => item.id)).toEqual([
      "b",
    ]);
    expect(screen.queryByText("Export finished")).toBeNull();
    expect(screen.getByText("Import rolled back")).toBeTruthy();
  });

  it("does not offer to dismiss an entry derived from book state", () => {
    openBell({
      systemItems: [
        {
          id: "review",
          title: "Waiting for your review",
          body: "2 decisions left.",
          tone: "warning",
          to: "/review",
        },
      ],
    });
    expect(
      within(entry("Waiting for your review")).queryByRole("button", {
        name: /Dismiss/,
      }),
    ).toBeNull();
  });

  it("navigates from an action button and from the title", () => {
    useUiStore.setState({
      notifications: [
        notification({
          id: "a",
          title: "Backend required",
          tone: "warning",
          target: "/settings/bitcoin",
          action: { label: "Open settings", target: "/settings/bitcoin" },
        }),
        notification({ id: "b", title: "Quarantine", target: "/review" }),
      ],
    });
    openBell();

    act(() => {
      fireEvent.click(
        within(entry("Backend required")).getByRole("button", {
          name: "Open settings",
        }),
      );
    });
    expect(navigate).toHaveBeenCalledWith({ to: "/settings/bitcoin" });

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^Notifications/ }));
    });
    act(() => {
      fireEvent.click(
        within(entry("Quarantine")).getByRole("button", { name: "Quarantine" }),
      );
    });
    expect(navigate).toHaveBeenLastCalledWith({ to: "/review" });
  });

  it("runs a command action, and holds it while it is already running", () => {
    useUiStore.setState({
      notifications: [
        notification({
          id: "a",
          title: "Book refresh failed",
          tone: "error",
          action: { label: "Retry", command: "refresh-book" },
        }),
      ],
    });
    const { runCommand } = openBell();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    });
    expect(runCommand).toHaveBeenCalledWith("refresh-book");
    expect(navigate).not.toHaveBeenCalled();

    cleanup();
    openBell({ pendingCommands: { "refresh-book": true } });
    expect(
      (screen.getByRole("button", { name: "Retry" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("drops a stored action that names an unknown route", () => {
    useUiStore.setState({
      notifications: [
        notification({
          id: "a",
          title: "Tampered",
          action: { label: "Go", target: "https://example.com" },
        }),
      ],
    });
    openBell();
    expect(screen.queryByRole("button", { name: "Go" })).toBeNull();
  });

  it("shows how often the same notification repeated", () => {
    useUiStore.setState({
      notifications: [
        notification({ id: "a", title: "Export failed", tone: "error", repeatCount: 3 }),
      ],
    });
    openBell();
    const row = entry("Export failed");
    expect(within(row).getByText("×3")).toBeTruthy();
    expect(within(row).getByText("Arrived 3 times")).toBeTruthy();
  });
});

describe("notification persistence", () => {
  it("writes no detail lines to kb.ui", () => {
    useUiStore.getState().addNotification({
      title: "Book refresh failed",
      body: "Electrum call failed",
      tone: "error",
      details: ["Cold storage: tcp://10.0.0.7:50001 refused"],
    });

    const stored = storage.get("kb.ui") ?? "";
    expect(stored).toContain("Electrum call failed");
    expect(stored).not.toContain("10.0.0.7");
  });

  it("drops detail lines from a stored blob on restore", async () => {
    storage.set(
      "kb.ui",
      JSON.stringify({
        version: UI_STATE_VERSION,
        state: {
          notifications: [
            {
              id: "old",
              createdAt: "2026-11-12T10:00:00Z",
              title: "Book refresh failed",
              body: "Failed",
              tone: "error",
              details: ["token=written-by-hand"],
              action: { label: "Retry", command: "refresh-book" },
            },
          ],
        },
      }),
    );
    await act(async () => {
      await useUiStore.persist.rehydrate();
    });

    const [restored] = useUiStore.getState().notifications;
    expect(restored).toMatchObject({ id: "old", title: "Book refresh failed" });
    expect(restored.details).toBeUndefined();
    expect(restored.action).toEqual({ label: "Retry", command: "refresh-book" });
  });
});
