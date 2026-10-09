// @vitest-environment happy-dom
//
// Mounted, not static: the explanation only exists once the label is clicked.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { UNKNOWN_FIAT_COMPLETENESS } from "@/lib/fiatCompleteness";
import type { FiatCompleteness } from "@/mocks/seed";

vi.hoisted(() => {
  // Radix positions the popover with a ResizeObserver this DOM does not ship.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, ...props }: React.ComponentProps<"a"> & { to: string }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
}));

import { BasisGapPopover } from "./BasisGapPopover";

const incomplete: FiatCompleteness = {
  ...UNKNOWN_FIAT_COMPLETENESS,
  state: "incomplete",
  reasons: ["quarantines", "missing_prices"],
  quarantineCount: 385,
  missingPriceCount: 1,
  earliestIncompleteAt: "2022-12-12T10:00:00Z",
};

function open(completeness: FiatCompleteness) {
  render(
    <BasisGapPopover completeness={completeness}>
      <button type="button">label</button>
    </BasisGapPopover>,
  );
  fireEvent.click(screen.getByRole("button", { name: "label" }));
  return screen.getByRole("dialog");
}

afterEach(cleanup);

describe("cost basis gap explanation", () => {
  it("says what is exact, from when the basis is not, and links each cause", () => {
    const dialog = open(incomplete);
    expect(dialog.textContent).toContain("Your BTC balances are exact.");
    expect(dialog.textContent).toContain("From Dec 12, 2022 on");
    const rows = [...dialog.querySelectorAll("li")].map((row) => ({
      text: row.textContent,
      href: row.querySelector("a")?.getAttribute("href"),
    }));
    expect(rows).toEqual([
      { text: "385 transactions in quarantineQuarantine", href: "/quarantine" },
      { text: "1 transaction without a priceQuarantine", href: "/quarantine" },
    ]);
  });

  it("sends stale journals and custody gaps to the journals and drops the date it lacks", () => {
    const dialog = open({
      ...incomplete,
      state: "stale",
      reasons: ["journals_stale", "custody_unresolved"],
      earliestIncompleteAt: null,
    });
    expect(dialog.textContent).not.toContain("From ");
    expect([...dialog.querySelectorAll("a")].map((link) => link.getAttribute("href"))).toEqual([
      "/journals",
      "/journals",
    ]);
  });
});
