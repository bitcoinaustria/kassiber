// @vitest-environment happy-dom
//
// Undoing an automatic pairing from the custody lineage: a confirmation
// first, then the existing dismissal kind; restoring removes that dismissal.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

import type { CustodyLineageItem } from "./custodyGapsModel";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  items: [] as unknown[],
  failDismiss: false,
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.custody.lineage.snapshot") {
          return {
            kind,
            schema_version: 1,
            data: {
              summary: { total_count: daemon.items.length, truncated: false },
              items: daemon.items,
            },
          };
        }
        if (kind === "ui.custody.gaps.list") {
          return {
            kind,
            schema_version: 1,
            data: {
              summary: {
                total: 0, needs_review: 0, conflicting: 0, resolved: 0, dismissed: 0,
                unresolved_msat: "0", canonical_issue_count: 0,
                derived_state_current: true, search_complete: true,
              },
              gaps: [],
            },
          };
        }
        if (kind === "ui.transfers.dismiss") {
          if (daemon.failDismiss) {
            return {
              kind: "error",
              schema_version: 1,
              error: { code: "not_found", message: "Transaction not found", retryable: false },
            };
          }
          return { kind, schema_version: 1, data: { id: "dismissal-7" } };
        }
        if (kind === "ui.transfers.dismissals.delete") {
          return { kind, schema_version: 1, data: { deleted: true } };
        }
        return {
          kind: "error",
          schema_version: 1,
          error: { code: "not_found", message: "unavailable", retryable: false },
        };
      },
    }),
  };
});

import { CustodyGapsContent } from "./CustodyGaps";

function lineageItem(overrides: Partial<CustodyLineageItem> = {}): CustodyLineageItem {
  return {
    out_transaction_id: "withdrawal-1",
    in_transaction_id: "receipt-1",
    occurred_at: "2024-02-01T10:00:00Z",
    asset: "BTC",
    amount_msat: "600000000",
    from_wallet_id: "wallet-strike",
    from_wallet_label: "Strike",
    to_wallet_id: "wallet-cold",
    to_wallet_label: "Cold storage",
    custody_state: "internal_verified",
    basis_state: "eligible",
    basis_barrier_at: null,
    evidence_reason: "exchange_txid_exact",
    network: "main",
    rail: "bitcoin",
    ...overrides,
  };
}

function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <CustodyGapsContent />
    </QueryClientProvider>,
  );
}

const calls = (kind: string) => daemon.calls.filter((call) => call.kind === kind);

beforeEach(async () => {
  await i18n.changeLanguage("en");
  daemon.items = [lineageItem()];
  daemon.failDismiss = false;
  daemon.calls = [];
});

afterEach(() => cleanup());

describe("Undoing an automatic pairing from the custody lineage", () => {
  it("offers undo only on automatic exchange pairings", async () => {
    daemon.items = [
      lineageItem(),
      lineageItem({
        out_transaction_id: "out-2",
        in_transaction_id: "in-2",
        from_wallet_label: "Hot",
        evidence_reason: "reviewed_transfer_pair",
        custody_state: "internal_reviewed",
      }),
      lineageItem({
        out_transaction_id: "out-3",
        in_transaction_id: "in-3",
        from_wallet_label: "Vault",
        evidence_reason: "recorded_fanout",
      }),
    ];
    mount();
    await screen.findByText("Hot");
    expect(screen.getAllByRole("button", { name: "Undo automatic pairing" })).toHaveLength(1);
  });

  it("asks before dismissing, and cancelling sends nothing", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Undo automatic pairing" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Undo this automatic pairing?")).toBeTruthy();
    expect(within(dialog).getByText("Strike → Cold storage")).toBeTruthy();
    fireEvent.click(within(dialog).getByRole("button", { name: "Keep pairing" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(calls("ui.transfers.dismiss")).toHaveLength(0);
  });

  it("dismisses both legs once confirmed and restores by the dismissal id", async () => {
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Undo automatic pairing" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Undo pairing" }));

    await screen.findByText("Undone automatic pairings");
    expect(calls("ui.transfers.dismiss").map((call) => call.args)).toEqual([
      expect.objectContaining({
        tx_out: "withdrawal-1",
        tx_in: "receipt-1",
        expires_in_days: 3650,
      }),
    ]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("button", { name: "Undo automatic pairing" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Restore pairing" }));
    await waitFor(() => expect(screen.queryByText("Undone automatic pairings")).toBeNull());
    expect(calls("ui.transfers.dismissals.delete").map((call) => call.args)).toEqual([
      expect.objectContaining({ dismissal_id: "dismissal-7" }),
    ]);
    expect(await screen.findByRole("button", { name: "Undo automatic pairing" })).toBeTruthy();
  });

  it("keeps the dialog open with the daemon's error when the dismissal fails", async () => {
    daemon.failDismiss = true;
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Undo automatic pairing" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Undo pairing" }));
    expect(await within(dialog).findByText("Transaction not found")).toBeTruthy();
    expect(screen.queryByText("Undone automatic pairings")).toBeNull();
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Automatische Paarung aufheben" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Diese automatische Paarung aufheben?")).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Paarung behalten" })).toBeTruthy();
  });
});
