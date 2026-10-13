// @vitest-environment happy-dom
//
// "Ask about this" pins an item as typed screen context for the next turns,
// prefills (never sends) a question that does not contain the item's id, and
// opens the panel.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  send: vi.fn(),
  reset: vi.fn(),
  streaming: false,
  // Whether this book's provider settings have seeded the effort.
  seeded: true,
}));

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
    isStreaming: mocks.streaming,
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

// Stands in for the seeding hook (which reads the provider list): the test
// decides when this book's effort is seeded.
vi.mock("@/components/ai/useProviderDefaultEffort", () => ({
  useProviderDefaultEffort: ({
    setThinkingEffort,
  }: {
    setThinkingEffort: (effort: string) => void;
  }) => ({ setThinkingEffort, effortSeeded: mocks.seeded }),
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
import { handOffToAssistant } from "./assistantHandoff";
import { useAskAssistantAbout, type AskAboutTarget } from "./useAskAssistantAbout";

let session: AssistantSessionContextValue;
let ask: ((target: AskAboutTarget) => void) | null;
function Probe() {
  session = useAssistantSession();
  ask = useAskAssistantAbout();
  return null;
}

function provider() {
  return (
    <AssistantSessionProvider
      screenContext={{ route: "/overview", capabilities: ["core"] }}
    >
      <Probe />
    </AssistantSessionProvider>
  );
}

function mount() {
  return render(provider());
}

function lastScreenContext(): unknown {
  return mocks.send.mock.calls.at(-1)?.[0]?.screenContext;
}

describe("Ask about this", () => {
  beforeEach(() => {
    mocks.send.mockReset();
    mocks.reset.mockReset();
    mocks.streaming = false;
    mocks.seeded = true;
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

  it("runs a queued question with the pin it was asked under", () => {
    const view = mount();
    // An answer is streaming, so the next questions queue.
    mocks.streaming = true;
    view.rerender(provider());

    act(() => ask!({ kind: "transaction", transactionId: "tx-a" }));
    act(() => session.sendPrompt("Why is A pending?"));
    // Before the first answer finishes, the user pins B and asks about it,
    // then removes the pin and asks a general question.
    act(() => ask!({ kind: "transaction", transactionId: "tx-b" }));
    act(() => session.sendPrompt("And B?"));
    act(() => session.clearFocus!());
    act(() => session.sendPrompt("Anything else?"));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(session.queuedPrompts).toEqual([
      "Why is A pending?",
      "And B?",
      "Anything else?",
    ]);

    mocks.streaming = false;
    act(() => view.rerender(provider()));

    const sent = mocks.send.mock.calls.map(([request]) => ({
      prompt: request.messages.at(-1)?.content,
      entityId: request.screenContext?.entityId,
    }));
    expect(sent).toEqual([
      { prompt: "Why is A pending?", entityId: "tx-a" },
      { prompt: "And B?", entityId: "tx-b" },
      { prompt: "Anything else?", entityId: undefined },
    ]);
  });

  it("drops a queued question's pin when the book changes", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => ask!({ kind: "transaction", transactionId: "tx-a" }));
    act(() => session.sendPrompt("Why is A pending?"));

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.streaming = false;
    act(() => view.rerender(provider()));

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(lastScreenContext()).not.toHaveProperty("entityId");
  });

  it("drops a queued pin when the answer ends in the same render as a book switch", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => ask!({ kind: "transaction", transactionId: "tx-old-book" }));
    act(() => session.sendPrompt("Why is this pending?"));

    // Stream completion and the book switch commit together, so the drain
    // runs before any effect that clears pins could re-render.
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.streaming = false;
      view.rerender(provider());
    });

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(lastScreenContext()).not.toHaveProperty("entityId");
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-old-book");
  });

  it("does not send a live pin taken in the previous book", () => {
    const view = mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-old-book" }));
    // The book switch and the submit land before the pin-clearing effect
    // has re-rendered.
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      session.sendPrompt("Why is this pending?");
    });
    view.rerender(provider());
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-old-book");
  });

  it("keeps the pin on a question held until the effort is seeded", () => {
    // Right after a book switch the provider settings are still loading, so
    // a question waits in the queue.
    mocks.seeded = false;
    const view = mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-a" }));
    act(() => session.sendPrompt("Why is A pending?"));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(session.queuedPrompts).toEqual(["Why is A pending?"]);

    // Pinning another item before the hold ends does not retarget it.
    act(() => ask!({ kind: "transaction", transactionId: "tx-b" }));
    mocks.seeded = true;
    act(() => view.rerender(provider()));

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(lastScreenContext()).toMatchObject({ entityId: "tx-a" });
  });

  it("drops a held question's pin when the book changes before seeding ends", () => {
    mocks.seeded = false;
    const view = mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-old-book" }));
    act(() => session.sendPrompt("Why is this pending?"));

    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.seeded = true;
      view.rerender(provider());
    });

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-old-book");
  });

  it("lets a page's handoff replace the pin instead of being narrowed by it", () => {
    mount();
    act(() => ask!({ kind: "quarantine", transactionId: "tx-held" }));
    expect(session.focus).not.toBeNull();

    // Source of Funds hands its own investigation to the assistant.
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-target");
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    expect(lastScreenContext()).toEqual({
      route: "/source-of-funds",
      entityType: "transaction",
      entityId: "tx-target",
      capabilities: ["source_funds", "transactions"],
    });
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-held");
    // The pin is gone, so the chip and the next turns agree with what was sent.
    expect(session.focus).toBeNull();
    act(() => session.sendPrompt("And the fee?"));
    expect(lastScreenContext()).not.toHaveProperty("entityId", "tx-held");
  });

  it("runs a queued handoff with the page it came from", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-target");
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    expect(mocks.send).not.toHaveBeenCalled();

    // Before it runs, the user moves on and pins a quarantine cause.
    window.history.replaceState(null, "", "/quarantine");
    act(() => ask!({ kind: "quarantine", transactionId: "tx-held" }));
    mocks.streaming = false;
    act(() => view.rerender(provider()));

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(lastScreenContext()).toMatchObject({
      route: "/source-of-funds",
      capabilities: ["source_funds", "transactions"],
    });
  });

  it("drops the pin when a handoff waits in the composer for a model", () => {
    mount();
    act(() => ask!({ kind: "quarantine", transactionId: "tx-held" }));
    act(() =>
      useUiStore.setState({ assistantModelSelection: null }),
    );
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toBe(
      "Investigate this source of funds case.",
    );
    expect(session.focus).toBeNull();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
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
    const scope = {
      workspaceId: "ws-1",
      profileId: "profile-1",
      inputVersion: 4,
      wallet: "Cold storage",
    };
    act(() =>
      useAssistantScreenMetricsStore.getState().publish({
        route: "/transactions",
        period: "1year",
        scope,
        metrics: { review_queue_review: 0, review_queue_pending: 11 },
      }),
    );
    act(() => session.sendPrompt("Why 11 pending?"));
    expect(lastScreenContext()).toEqual({
      route: "/transactions",
      capabilities: ["transactions"],
      filters: { period: "1year" },
      metrics: { review_queue_review: 0, review_queue_pending: 11 },
      metricsScope: scope,
    });

    // Counts never travel without the book state they were counted on.
    act(() =>
      useAssistantScreenMetricsStore.getState().publish({
        route: "/transactions",
        scope: { ...scope, profileId: "/home/user/book" },
        metrics: { review_queue_pending: 11 },
      }),
    );
    act(() => session.sendPrompt("Still 11?"));
    expect(lastScreenContext()).not.toHaveProperty("metrics");

    // Another page's snapshot never rides along.
    window.history.replaceState(null, "", "/overview");
    act(() => session.sendPrompt("And here?"));
    expect(lastScreenContext()).not.toHaveProperty("metrics");
  });
});
