// @vitest-environment happy-dom
//
// Reconcile has two modes. Switching between them must not throw away a
// pasted list or a loaded CSV, and the mode tabs must point at real panels.
import * as React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const router = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  navigate: vi.fn(),
}));

vi.hoisted(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => router.navigate,
  useSearch: () => router.search,
}));

vi.mock("@/daemon/client", () => {
  const idle = { mutate: vi.fn(), mutateAsync: vi.fn(), isPending: false, error: null };
  return {
    useDaemon: () => ({ data: undefined, error: null, isLoading: false, isFetching: false }),
    useDaemonMutation: () => idle,
    useDaemonStreamMutation: () => idle,
  };
});

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

// Stands in for the cross-check with its own local state, like a loaded CSV.
vi.mock("@/components/reconcile/ToolCrossCheck", () => ({
  ToolCrossCheck: () => {
    const [value, setValue] = React.useState("");
    return (
      <input aria-label="cross-check file" value={value} onChange={(event) => setValue(event.target.value)} />
    );
  },
}));

import { Reconcile } from "./Reconcile";

afterEach(() => {
  cleanup();
  router.search = {};
  router.navigate.mockReset();
});

function switchTo(name: RegExp, rerender: (ui: React.ReactElement) => void) {
  // Radix tabs activate on mouse down; the route owns the mode via ?mode=.
  fireEvent.mouseDown(screen.getByRole("tab", { name }), { button: 0 });
  const call = router.navigate.mock.calls.at(-1)?.[0] as { search: Record<string, unknown> };
  router.search = call.search;
  rerender(<Reconcile />);
}

describe("Reconcile modes", () => {
  it("keeps both modes' input when switching back and forth", () => {
    const { rerender } = render(<Reconcile />);
    const list = screen.getByRole("textbox", { name: "reconcile.inputTitle" }) as HTMLTextAreaElement;
    fireEvent.change(list, { target: { value: "bc1qexample" } });

    switchTo(/reconcile.modes.tools/, rerender);
    expect(router.navigate).toHaveBeenLastCalledWith({
      to: "/reconcile",
      search: { mode: "tools" },
      replace: true,
    });
    expect(list.closest("[role=tabpanel]")?.hasAttribute("hidden")).toBe(true);
    const file = screen.getByLabelText("cross-check file") as HTMLInputElement;
    fireEvent.change(file, { target: { value: "blockpit.csv" } });

    switchTo(/reconcile.modes.addresses/, rerender);
    expect(router.search).toEqual({});
    expect((screen.getByRole("textbox", { name: "reconcile.inputTitle" }) as HTMLTextAreaElement).value).toBe(
      "bc1qexample",
    );
    expect(file.closest("[role=tabpanel]")?.hasAttribute("hidden")).toBe(true);

    switchTo(/reconcile.modes.tools/, rerender);
    expect((screen.getByLabelText("cross-check file") as HTMLInputElement).value).toBe("blockpit.csv");
  });

  it("opens the mode named in the URL and wires each tab to its panel", () => {
    router.search = { mode: "tools" };
    render(<Reconcile />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(2);
    for (const tab of tabs) {
      const panel = document.getElementById(tab.getAttribute("aria-controls") ?? "");
      expect(panel?.getAttribute("role")).toBe("tabpanel");
      expect(panel?.getAttribute("aria-labelledby")).toBe(tab.id);
    }
    expect(screen.getByRole("tab", { name: /reconcile.modes.tools/ }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(screen.getByLabelText("cross-check file").closest("[role=tabpanel]")?.hasAttribute("hidden")).toBe(
      false,
    );
  });
});
