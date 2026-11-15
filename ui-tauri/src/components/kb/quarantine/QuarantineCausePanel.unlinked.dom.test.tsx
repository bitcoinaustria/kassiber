// @vitest-environment happy-dom
//
// A held send whose automatic link the owner removed (D4): the daemon files
// it as a decision and offers to link exactly that pair back. The Review
// page asks the same question the Transfers screen asks, offers linking it
// back or linking it elsewhere, and no longer offers a sync that cannot
// answer it.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const spies = vi.hoisted(() => ({
  navigate: vi.fn(),
  mutate: vi.fn(),
  kinds: [] as string[],
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => spies.navigate }));
vi.mock("@/daemon/client", () => ({
  DaemonRequestError: class extends Error {},
  useDaemonStreamMutation: () => ({ mutate: vi.fn(), isPending: false }),
  useDaemonMutation: (kind: string) => {
    spies.kinds.push(kind);
    return {
      mutateAsync: async (args: unknown) => {
        spies.mutate(kind, args);
        return { data: {} };
      },
      isPending: false,
    };
  },
}));

import i18n from "@/i18n";
import { useFeedbackStore } from "@/lib/feedback";

import { QuarantineCausePanel } from "./QuarantineCausePanel";
import type { QuarantineAction, QuarantineItem, QuarantineSnapshot } from "./types";

const GROUP_KEY = "recorded_transfer_authority_conflict:out-bark";

function snapshot(actions: QuarantineAction[]): QuarantineSnapshot {
  const item: QuarantineItem = {
    transaction_id: "out-bark",
    external_id: "f".repeat(64),
    occurred_at: "2026-11-13T11:51:50Z",
    confirmed_at: null,
    wallet: "BTCPay Bark",
    direction: "outbound",
    asset: "BTC",
    amount: 0.006,
    amount_msat: 600_000_000,
    fee: 0,
    fee_msat: 0,
    reason: "recorded_transfer_authority_conflict",
    detail: {},
    created_at: "2026-11-13T12:16:00Z",
    category: "needs_decision",
    blocks_reports: true,
    is_downstream: false,
    root: null,
    evidence: {},
    actions,
    group_key: GROUP_KEY,
  };
  return {
    summary: {
      workspace: "Store",
      profile: "Shop",
      workspace_id: "ws",
      profile_id: "book",
      count: 1,
      by_reason: [],
      limit: 100,
      offset: 0,
      blocking_count: 1,
      reports_blocked: true,
      freshness: { needs_processing: false, last_processed_at: "2026-11-13T12:16:00Z", last_error: null },
      groups: [
        {
          key: GROUP_KEY,
          category: "needs_decision",
          reason: "recorded_transfer_authority_conflict",
          root_transaction_id: "out-bark",
          root_occurred_at: "2026-11-13T11:51:50Z",
          root_wallet: "BTCPay Bark",
          root_external_id: "f".repeat(64),
          root_amount_msat: 600_000_000,
          root_direction: "outbound",
          root_asset: "BTC",
          count: 1,
          downstream_count: 0,
          blocks_reports: true,
          wallets: ["BTCPay Bark"],
          earliest_occurred_at: "2026-11-13T11:51:50Z",
          evidence: {},
          actions,
          root_transaction_ids: ["out-bark"],
          root_count: 1,
        },
      ],
      group_count: 1,
      attention_count: 1,
      waiting_count: 0,
      scope: "attention",
      scope_count: 1,
    },
    items: [item],
  } as QuarantineSnapshot;
}

const UNDONE: QuarantineAction[] = [
  {
    kind: "restore_pairing",
    transaction_id: "out-bark",
    dismissal_id: "dismissal-9",
    out_transaction_id: "out-bark",
    in_transaction_id: "in-store",
  },
  { kind: "pair_transfer", transaction_id: "out-bark" },
  { kind: "sync_wallet", wallet_id: "w-bark", wallet_label: "BTCPay Bark" },
];

function mount(data: QuarantineSnapshot, onProcessJournals = vi.fn()) {
  render(
    <QuarantineCausePanel
      snapshot={data}
      isProcessingJournals={false}
      onProcessJournals={onProcessJournals}
      onOpenTransaction={vi.fn()}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={() => {}}
      onRefresh={vi.fn(async () => ({ items: data.items, scope: { workspace_id: "ws", profile_id: "book" } }))}
    />,
  );
  return { onProcessJournals };
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  useFeedbackStore.setState({ items: [] });
  spies.navigate.mockReset();
  spies.mutate.mockReset();
});
afterEach(cleanup);

describe("an unlinked automatic link on the Review page", () => {
  it("asks what the send was, and links it back with the dismissal the daemon named", async () => {
    const { onProcessJournals } = mount(snapshot(UNDONE));
    const card = screen.getByTestId("quarantine-cause");
    expect(within(card).getByText("You unlinked this send from BTCPay Bark")).toBeTruthy();
    // The dead end is gone: no sync offered for a link the owner removed.
    expect(within(card).queryByRole("button", { name: /Sync/ })).toBeNull();
    fireEvent.click(within(card).getByRole("button", { name: "Link it back" }));
    expect(spies.mutate).toHaveBeenCalledWith("ui.transfers.dismissals.delete", { dismissal_id: "dismissal-9" });
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalled());
    expect(useFeedbackStore.getState().items[0]?.title).toBe("Linked again");
  });

  it("links it elsewhere with Link manually on Transfers, starting from the send", () => {
    mount(snapshot(UNDONE));
    fireEvent.click(screen.getByRole("button", { name: "Link it to a different receipt…" }));
    expect(spies.navigate).toHaveBeenCalledWith({ to: "/transfers", search: { link: "out-bark" } });
  });

  it("keeps the general reading, and its sync, when no removed link names the pair", () => {
    mount(snapshot(UNDONE.slice(1)));
    const card = screen.getByTestId("quarantine-cause");
    expect(within(card).getByText("One side comes from a file, the other from the chain")).toBeTruthy();
    expect(within(card).queryByRole("button", { name: "Link it back" })).toBeNull();
    expect(within(card).getByRole("button", { name: /Sync BTCPay Bark/ })).toBeTruthy();
    expect(within(card).getByRole("button", { name: "Open matching tools" })).toBeTruthy();
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    mount(snapshot(UNDONE));
    const card = screen.getByTestId("quarantine-cause");
    expect(within(card).getByText("Du hast die Verknüpfung dieser Sendung von BTCPay Bark aufgehoben")).toBeTruthy();
    expect(within(card).getByRole("button", { name: "Wieder verknüpfen" })).toBeTruthy();
  });
});
