// @vitest-environment happy-dom
//
// "Ask about this" pins an item as typed screen context for the next turns,
// prefills (never sends) a question that does not contain the item's id, and
// opens the panel.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ send: vi.fn(), reset: vi.fn() }));

vi.hoisted(() => {
  const makeStorage = () => {
    const storage = new Map<string, string>();
    return {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => void storage.set(key, value),
      removeItem: (key: string) => void storage.delete(key),
    };
  };
  vi.stubGlobal("localStorage", makeStorage());
  vi.stubGlobal("sessionStorage", makeStorage());
});

vi.mock("@/daemon/stream", () => ({
  useAiChatStream: () => ({
    messages: [],
    isStreaming: false,
    error: null,
    pendingConsent: null,
    sessionId: null,
    send: mocks.send,
    sendConsent: vi.fn(),
    abort: vi.fn(),
    reset: mocks.reset,
    loadConversation: vi.fn(),
    forgetSession: vi.fn(),
  }),
}));

vi.mock("@/components/ai/useFastModeSupport", () => ({
  useFastModeSupport: () => ({ supported: false, resolved: true, runtime: null }),
}));

import {
  useAssistantSession,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useAssistantScreenMetricsStore } from "@/store/assistantScreenMetrics";
import { useUiStore } from "@/store/ui";
import { AssistantSessionProvider } from "./AssistantSessionProvider";
import { useAskAssistantAbout, type AskAboutTarget } from "./useAskAssistantAbout";

let session: AssistantSessionContextValue;
let ask: ((target: AskAboutTarget) => void) | null;
function Probe() {
  session = useAssistantSession();
  ask = useAskAssistantAbout();
  return null;
}

function mount() {
  return render(
    <AssistantSessionProvider
      screenContext={{ route: "/overview", capabilities: ["core"] }}
    >
      <Probe />
    </AssistantSessionProvider>,
  );
}

function lastScreenContext(): unknown {
  return mocks.send.mock.calls.at(-1)?.[0]?.screenContext;
}

describe("Ask about this", () => {
  beforeEach(() => {
    mocks.send.mockReset();
    mocks.reset.mockReset();
    window.history.replaceState(null, "", "/transactions?period=1year");
    useUiStore.setState({
      assistantModelSelection: { provider: "copilot", model: "default" },
      assistantPanelOpen: false,
    });
    useAssistantDraftStore.setState({ draft: "" });
    useAssistantScreenMetricsStore.setState({ snapshot: null });
  });
  afterEach(cleanup);

  it("pins the transaction as typed context and only prefills the question", () => {
    mount();
    const focusRequest = useUiStore.getState().assistantPanelFocusRequest;
    act(() =>
      ask!({ kind: "transaction", transactionId: "tx-7f3a", date: "2025-12-08 10:00" }),
    );

    expect(mocks.send).not.toHaveBeenCalled();
    const draft = useAssistantDraftStore.getState().draft;
    expect(draft).toBe(
      "Explain this transaction and whether anything about it needs my attention.",
    );
    expect(draft).not.toContain("tx-7f3a");
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    expect(useUiStore.getState().assistantPanelFocusRequest).toBe(focusRequest + 1);
    expect(session.focus?.label).toBe("Transaction · 2025-12-08");

    act(() => session.sendPrompt(draft));
    const sent = mocks.send.mock.calls[0];
    expect(sent[0].screenContext).toEqual({
      route: "/transactions",
      entityType: "transaction",
      entityId: "tx-7f3a",
      capabilities: ["transactions"],
    });
    expect(JSON.stringify(sent[0].messages)).not.toContain("tx-7f3a");

    // The pin holds for follow-ups until the user removes it.
    act(() => session.sendPrompt("And the fee?"));
    expect(lastScreenContext()).toMatchObject({ entityId: "tx-7f3a" });

    act(() => session.clearFocus!());
    act(() => session.sendPrompt("Anything else?"));
    expect(lastScreenContext()).toEqual({
      route: "/transactions",
      capabilities: ["transactions"],
    });
  });

  it("refuses an id the screen-context allowlist would reject", () => {
    mount();
    act(() =>
      ask!({ kind: "transaction", transactionId: "https://explorer.example/tx/1" }),
    );
    expect(session.focus).toBeNull();
    expect(useAssistantDraftStore.getState().draft).toBe("");
    expect(useUiStore.getState().assistantPanelOpen).toBe(false);
  });

  it("drops the pin with the conversation", () => {
    mount();
    act(() => ask!({ kind: "quarantine", transactionId: "tx-held" }));
    expect(session.focus?.context).toMatchObject({
      route: "/quarantine",
      capabilities: ["review"],
    });
    act(() => session.reset());
    expect(session.focus).toBeNull();
  });

  it("sends the page's visible counts with the route context", () => {
    mount();
    act(() =>
      useAssistantScreenMetricsStore.getState().publish({
        route: "/transactions",
        period: "1year",
        metrics: { review_queue_review: 0, review_queue_pending: 11 },
      }),
    );
    act(() => session.sendPrompt("Why 11 pending?"));
    expect(lastScreenContext()).toEqual({
      route: "/transactions",
      capabilities: ["transactions"],
      filters: { period: "1year" },
      metrics: { review_queue_review: 0, review_queue_pending: 11 },
    });

    // Another page's snapshot never rides along.
    window.history.replaceState(null, "", "/overview");
    act(() => session.sendPrompt("And here?"));
    expect(lastScreenContext()).not.toHaveProperty("metrics");
  });
});
