// @vitest-environment happy-dom
//
// Mounted: "Hide all" reaches the widget through the UI store, which a static
// render does not follow.
import { cleanup, render } from "@testing-library/react";
import type * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: React.ComponentProps<"a"> & { to?: string }) => <a href={to}>{children}</a>,
}));

import "@/i18n";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import { FlowsWidget } from "./OverviewWidgets";

const now = new Date();
const thisMonth = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-01 10:00`;
const received = {
  ...MOCK_OVERVIEW.txs[0],
  id: "in",
  date: thisMonth,
  occurredAt: undefined,
  amountSat: 50_000_000,
  eur: 30_000,
  feeSat: 0,
  internal: false,
  type: "Income" as const,
};
const snapshot: OverviewSnapshot = { ...MOCK_OVERVIEW, activityTxs: [received], txs: [received] };
const bars = (container: HTMLElement) => container.querySelectorAll(".bg-emerald-500\\/70, .bg-red-500\\/60").length;

describe("money in and out under the privacy modes", () => {
  afterEach(() => {
    cleanup();
    useUiStore.setState({ privacyMode: "normal" });
  });

  it("draws a bar pair per month while nothing is hidden", () => {
    const { container } = render(<FlowsWidget snapshot={snapshot} hideSensitive={false} currency="eur" />);
    expect(bars(container)).toBe(24);
    expect(container.textContent).toContain("30.000");
  });

  it("masks the totals and drops the bars under Hide all", () => {
    useUiStore.setState({ privacyMode: "hideAll" });
    const { container } = render(<FlowsWidget snapshot={snapshot} hideSensitive currency="eur" />);
    expect(bars(container)).toBe(0);
    expect(container.textContent).toContain("••••");
    expect(container.textContent).not.toContain("30.000");
    // The month titles keep the names, not the amounts.
    expect(container.querySelector("[title*='0.5000']")).toBeNull();
  });

  it("keeps the bars blurred but drawn under Hide amounts", () => {
    useUiStore.setState({ privacyMode: "hideAmounts" });
    const { container } = render(<FlowsWidget snapshot={snapshot} hideSensitive currency="eur" />);
    expect(bars(container)).toBe(24);
    expect(container.textContent).not.toContain("30.000");
  });
});
