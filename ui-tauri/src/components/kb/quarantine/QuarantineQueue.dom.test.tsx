// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { QuarantineQueue } from "./QuarantineQueue";
import type { QuarantineItem } from "./types";

const base: Omit<QuarantineItem, "transaction_id" | "reason"> = {
  external_id: "",
  occurred_at: "2024-01-01T00:00:00Z",
  confirmed_at: null,
  wallet: "A",
  direction: "outbound",
  asset: "BTC",
  amount: 1,
  amount_msat: 100_000_000_000,
  fee: 0,
  fee_msat: 0,
  detail: {},
  created_at: "2026-09-30T00:00:00Z",
};

const root: QuarantineItem = {
  ...base,
  transaction_id: "out",
  reason: "custody_quantity_unresolved",
  category: "needs_decision",
  blocks_reports: true,
  is_downstream: false,
  root: null,
  evidence: { blocker_code: "reviewed_residual_suspense", pair_id: "pair-1" },
};

const waiting: QuarantineItem = {
  ...base,
  transaction_id: "later-sale",
  reason: "custody_basis_barrier",
  occurred_at: "2026-01-01T00:00:00Z",
  wallet: "C",
  category: "downstream",
  blocks_reports: false,
  is_downstream: true,
  root: {
    transaction_id: "out",
    reason: "custody_quantity_unresolved",
    occurred_at: "2024-01-01T00:00:00Z",
    wallet: "A",
    external_id: "out",
  },
  evidence: {},
};

afterEach(cleanup);

function mount(overrides: Partial<Parameters<typeof QuarantineQueue>[0]> = {}) {
  const props = {
    items: [root, waiting],
    scope: "all" as const,
    counts: { attention: 1, waiting: 382, all: 383 },
    offset: 0,
    pageSize: 100,
    total: 2,
    loading: false,
    hideSensitive: false,
    onScopeChange: vi.fn(),
    onOffsetChange: vi.fn(),
    onOpenTransaction: vi.fn(),
    ...overrides,
  };
  render(<QuarantineQueue {...props} />);
  return props;
}

describe("quarantine queue", () => {
  it("offers each scope with its whole-book count", () => {
    const props = mount();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Needs you1", "Waiting382", "All383"]);
    expect(tabs[2].getAttribute("aria-selected")).toBe("true");
    // Radix tabs select on pointer down.
    fireEvent.mouseDown(tabs[1]);
    expect(props.onScopeChange).toHaveBeenCalledWith("waiting");
  });

  it("shows a cause by what is wrong and a waiting row by what it waits on", () => {
    const props = mount();
    expect(screen.getByText("A transfer pair doesn't add up")).toBeTruthy();
    expect(screen.getByText("Blocks reports")).toBeTruthy();
    expect(screen.getByText("Waits on 2024-01-01, A")).toBeTruthy();
    // A pair-made suspense opens where the pair can be unpaired.
    fireEvent.click(screen.getByText("A transfer pair doesn't add up"));
    expect(props.onOpenTransaction).toHaveBeenCalledWith("out", "linked", expect.objectContaining({ reason: "custody_quantity_unresolved" }));
    fireEvent.click(screen.getByText("Waits on 2024-01-01, A"));
    expect(props.onOpenTransaction).toHaveBeenLastCalledWith("out", "details", null);
  });

  it("pages through a long scope and says when one is empty", () => {
    const props = mount({ scope: "waiting", items: [waiting], total: 382, offset: 0 });
    expect(screen.getByText("Rows 1–1 of 382")).toBeTruthy();
    fireEvent.click(screen.getByText("Next"));
    expect(props.onOffsetChange).toHaveBeenCalledWith(100);
    cleanup();
    mount({ scope: "attention", items: [], total: 0 });
    expect(screen.getByText("Nothing needs you right now.")).toBeTruthy();
  });
});
