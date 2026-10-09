// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { QuarantineQueue } from "./QuarantineQueue";
import type { QuarantineItem } from "./types";

const waiting: QuarantineItem = {
  transaction_id: "later-sale",
  external_id: "",
  occurred_at: "2026-01-01T00:00:00Z",
  confirmed_at: null,
  wallet: "C",
  direction: "outbound",
  asset: "BTC",
  amount: 1,
  amount_msat: 100_000_000_000,
  fee: 0,
  fee_msat: 0,
  reason: "custody_basis_barrier",
  detail: {},
  created_at: "2026-09-30T00:00:00Z",
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
    items: [waiting],
    offset: 0,
    pageSize: 100,
    total: 1,
    loading: false,
    hideSensitive: false,
    onOffsetChange: vi.fn(),
    onOpenTransaction: vi.fn(),
    onHide: vi.fn(),
    ...overrides,
  };
  render(<QuarantineQueue {...props} />);
  return props;
}

describe("waiting list", () => {
  it("shows each waiting row by what it waits on, and opens either", () => {
    const props = mount({ total: 382 });
    expect(screen.getByRole("heading", { name: "Waiting on a cause" })).toBeTruthy();
    expect(screen.getByText("382")).toBeTruthy();
    // No scope selector: what needs the user sits on its cause's card.
    expect(screen.queryAllByRole("tab")).toHaveLength(0);
    fireEvent.click(screen.getByText("2026-01-01 · C"));
    expect(props.onOpenTransaction).toHaveBeenCalledWith("later-sale", "details", expect.objectContaining({ reason: "custody_basis_barrier" }));
    fireEvent.click(screen.getByText("Waits on 2024-01-01, A"));
    expect(props.onOpenTransaction).toHaveBeenLastCalledWith("out", "details", null);
    fireEvent.click(screen.getByRole("button", { name: "Hide the list" }));
    expect(props.onHide).toHaveBeenCalledOnce();
  });

  it("pages through a long list and says when it is empty", () => {
    const props = mount({ total: 382 });
    expect(screen.getByText("Rows 1–1 of 382")).toBeTruthy();
    fireEvent.click(screen.getByText("Next"));
    expect(props.onOffsetChange).toHaveBeenCalledWith(100);
    cleanup();
    mount({ items: [], total: 0 });
    expect(screen.getByText("No transaction is waiting on another.")).toBeTruthy();
  });
});
