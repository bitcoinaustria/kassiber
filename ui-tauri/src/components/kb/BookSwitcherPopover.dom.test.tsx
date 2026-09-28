// @vitest-environment happy-dom
//
// Mounted, not static: the switcher is keyboard-driven, and the highlight,
// filtering and Enter-to-switch only exist once it is open in a live DOM.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ProfilesSnapshot } from "@/mocks/profiles";

const daemon = vi.hoisted(() => ({
  snapshot: undefined as unknown,
  mutate: vi.fn(),
}));

vi.hoisted(() => {
  // Radix positions the popover with a ResizeObserver this DOM does not ship.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({
    data: { data: daemon.snapshot },
    error: null,
    isLoading: false,
  }),
  useDaemonMutation: () => ({
    mutate: daemon.mutate,
    isPending: false,
    error: null,
  }),
}));

vi.mock("@tanstack/react-router", () => ({
  // `params` is dropped: an anchor has no such attribute.
  Link: ({
    children,
    to,
    ...props
  }: React.ComponentProps<"a"> & { to: string; params?: unknown }) => {
    delete props.params;
    return (
      <a href={to} {...props}>
        {children}
      </a>
    );
  },
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { BookSwitcherPopover } from "./BookSwitcherPopover";

const SNAPSHOT: ProfilesSnapshot = {
  activeProfileId: "private",
  workspaces: [
    {
      id: "home",
      name: "Home",
      currency: "EUR",
      jurisdiction: "Austria",
      created: "2026-01-01",
      profiles: [
        { id: "private", name: "Private", taxPolicy: "Austria", accounts: 1, wallets: 2, lastOpened: "today" },
        { id: "savings", name: "Savings", taxPolicy: "Austria", accounts: 1, wallets: 1, lastOpened: "today" },
      ],
    },
    {
      id: "company",
      name: "Company",
      currency: "EUR",
      jurisdiction: "Austria",
      created: "2026-01-01",
      profiles: [
        { id: "gmbh", name: "GmbH", taxPolicy: "Austria", accounts: 1, wallets: 3, lastOpened: "today" },
      ],
    },
  ],
};

function mount() {
  const onOpenChange = vi.fn();
  render(
    <BookSwitcherPopover open onOpenChange={onOpenChange}>
      <button type="button">crumb</button>
    </BookSwitcherPopover>,
  );
  return { onOpenChange, field: screen.getByRole("combobox") };
}

function highlighted() {
  return screen
    .getAllByRole("option")
    .find((option) => option.getAttribute("aria-selected") === "true")
    ?.textContent;
}

describe("BookSwitcherPopover", () => {
  beforeEach(() => {
    daemon.snapshot = SNAPSHOT;
    daemon.mutate.mockReset();
  });
  afterEach(cleanup);

  it("opens on the current book, grouped under its book set", () => {
    mount();
    expect(screen.getAllByRole("group").map((group) => group.getAttribute("aria-label"))).toEqual([
      "Home",
      "Company",
    ]);
    expect(highlighted()).toContain("Private");
    expect(
      screen.getAllByRole("option").find((option) => option.getAttribute("aria-current") === "true")
        ?.textContent,
    ).toContain("Private");
  });

  it("moves with the arrows across book sets and wraps around", () => {
    const { field } = mount();
    fireEvent.keyDown(field, { key: "ArrowDown" });
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(highlighted()).toContain("GmbH");
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(highlighted()).toContain("Private");
  });

  it("filters as you type and switches to the highlighted book on Enter", () => {
    const { field } = mount();
    fireEvent.change(field, { target: { value: "gmb" } });
    expect(screen.getAllByRole("option")).toHaveLength(1);
    fireEvent.keyDown(field, { key: "Enter" });
    expect(daemon.mutate).toHaveBeenCalledWith(
      { profile_id: "gmbh" },
      expect.any(Object),
    );
  });

  it("closes without switching when Enter picks the open book", () => {
    const { field, onOpenChange } = mount();
    fireEvent.keyDown(field, { key: "Enter" });
    expect(daemon.mutate).not.toHaveBeenCalled();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
