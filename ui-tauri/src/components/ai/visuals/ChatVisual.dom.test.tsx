// @vitest-environment happy-dom
//
// Mounted: a visual's own controls work without a new read, and its follow-up
// and action chips ask the assistant through the session's normal send path.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiChatToolCall } from "@/daemon/stream";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { TransactionGraphPayload } from "@/components/transactions/TransactionGraphModel";
import { AssistantSessionContext, type AssistantSessionContextValue } from "../assistantSession";
import fixtures from "./__fixtures__/daemon-visuals.json";
import { ChatVisual } from "./ChatVisual";
import { chatVisualFromToolCall } from "./model";

const graphReads = vi.hoisted(() => [] as Array<{ kind: string; args: unknown }>);
const bookGraph: TransactionGraphPayload = {
  transaction: { id: "tx-demo-out-1" },
  supportLevel: "full",
  inputs: [{ id: "in-0", valueSats: 3_000_000, valueBtc: 0.03, valueState: "known", ownership: "owned" }],
  outputs: [{ id: "out-0", valueSats: 2_998_000, valueBtc: 0.02998, valueState: "known" }],
  fee: { id: "fee", valueSats: 2_000, valueBtc: 0.00002 },
};

vi.mock("@/daemon/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/client")>()),
  useDaemon: (kind: string, args: unknown) => {
    graphReads.push({ kind, args });
    return { data: { kind, data: bookGraph }, isLoading: false };
  },
}));

function visualFrom(result: unknown) {
  const call: AiChatToolCall = {
    callId: "call-visual",
    name: "ui.visuals.render",
    arguments: {},
    kindClass: "read_only",
    needsConsent: false,
    status: "done",
    result: structuredClone(result),
  };
  const visual = chatVisualFromToolCall(call);
  if (!visual) throw new Error("fixture did not parse");
  return visual;
}

function mount(result: unknown, sendPrompt = vi.fn(() => true)) {
  const session = { sendPrompt } as unknown as AssistantSessionContextValue;
  render(
    <TooltipProvider>
      <AssistantSessionContext.Provider value={session}>
        <ChatVisual visual={visualFrom(result)} />
      </AssistantSessionContext.Provider>
    </TooltipProvider>,
  );
  return sendPrompt;
}

afterEach(() => cleanup());

describe("ChatVisual controls", () => {
  it("switches to the table twin and back", () => {
    mount(fixtures.portfolio);
    fireEvent.click(screen.getByRole("button", { name: "Show data as a table" }));
    expect(screen.getByRole("table")).toBeTruthy();
    expect(screen.getByRole("columnheader", { name: "Wallet" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show the chart" }));
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("regroups holdings without asking the daemon again", () => {
    mount(fixtures.portfolio);
    fireEvent.click(screen.getByRole("radio", { name: "Assets" }));
    expect(screen.getByRole("radio", { name: "Assets" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getAllByText("BTC").length).toBeGreaterThan(0);
  });

  it("asks a follow-up through the session", () => {
    const sendPrompt = mount(fixtures.cash_flow);
    fireEvent.click(screen.getByRole("button", { name: "Show my largest outgoing transactions" }));
    expect(sendPrompt).toHaveBeenCalledWith("Show my largest outgoing transactions");
  });

  it("offers each open readiness check as an action", () => {
    const blocked = structuredClone(fixtures.readiness) as {
      data: { rows: Array<Record<string, unknown>>; summary: Record<string, unknown> };
    };
    blocked.data.rows[2] = { check: "quarantine", status: "blocked", count: 2, detail: null, action: "review_quarantine" };
    blocked.data.summary = { ready: false, passed: 5, total: 6 };
    const sendPrompt = mount(blocked);
    expect(screen.getByText("1 thing needs attention")).toBeTruthy();
    expect(screen.getByText("2 transactions in quarantine")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Review with me" }));
    expect(sendPrompt).toHaveBeenCalledWith("Help me review the transactions in quarantine.");
  });

  it("copies the table as CSV", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    mount(fixtures.tax_summary);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Copy as CSV" }));
    });
    expect(writeText).toHaveBeenCalledWith("Year,Proceeds,Cost basis,Gain/loss\n2025,12600,11701.8,898.2");
  });
});

describe("transaction flow", () => {
  it("draws the detail panel's flat graph from a local read, with the summary above it", () => {
    const flow = structuredClone(fixtures.transaction_flow) as { data: Record<string, unknown> };
    flow.data.rows = [
      { side: "input", label: "Cold storage", label_kind: "wallet", position: 0, ownership: "owned", value_sats: 3_000_000, value_state: "known" },
      { side: "output", label: "Output 1", label_kind: "generic", position: 0, ownership: "unknown", value_sats: 2_998_000, value_state: "known" },
      { side: "fee", label: "Miner fee", label_kind: "fee", position: 0, ownership: "network_fee", value_sats: 2_000, value_state: "known" },
    ];
    graphReads.length = 0;
    mount(flow);
    // The same read as the detail panel, never a public lookup.
    expect(graphReads).toContainEqual({
      kind: "ui.transactions.graph",
      args: { transaction: "tx-demo-out-1", allowPublicLookup: false },
    });
    expect(document.querySelector('[data-slot="chat-visual-graph"] svg')).toBeTruthy();
    expect(screen.getByText("Change to your wallets").parentElement?.textContent).toContain("0.03");
  });
});
