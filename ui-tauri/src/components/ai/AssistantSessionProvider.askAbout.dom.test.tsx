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
  // The open book's ids, as the daemon's profile snapshot reports them.
  scope: { workspace_id: "ws-1", profile_id: "profile-1" } as
    | { workspace_id: string; profile_id: string }
    | null,
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
vi.mock("@/components/ai/useActiveBookScope", () => ({
  useActiveBookScope: () => mocks.scope,
}));

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
import i18n from "@/i18n";
import { mutationAdvancesDaemonSession } from "@/daemon/client";
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
    mocks.scope = { workspace_id: "ws-1", profile_id: "profile-1" };
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

  it("returns a pinned question queued in the previous book to the composer", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => ask!({ kind: "transaction", transactionId: "tx-a" }));
    act(() => session.sendPrompt("Why is A pending?"));

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.streaming = false;
    act(() => view.rerender(provider()));

    // Asked in the previous book, so not sent in this one: back in the
    // composer, and the old pin goes nowhere.
    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toContain("Why is A pending?");
  });

  it("holds back a queued question when the answer ends in the same render as a book switch", () => {
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

    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toContain("Why is this pending?");
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

  it("holds back a question waiting for seeding when the book changes", () => {
    mocks.seeded = false;
    const view = mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-old-book" }));
    act(() => session.sendPrompt("Why is this pending?"));

    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.seeded = true;
      view.rerender(provider());
    });

    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toContain("Why is this pending?");
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

  it.each([
    ["behind a streaming answer", "streaming"],
    ["until the effort is seeded", "seeding"],
  ] as const)("runs a question queued %s with the page it was asked on", (_, held) => {
    if (held === "seeding") mocks.seeded = false;
    const view = mount();
    if (held === "streaming") {
      mocks.streaming = true;
      view.rerender(provider());
    }
    const scope = {
      workspaceId: "ws-1",
      profileId: "profile-1",
      inputVersion: 4,
      wallet: null,
      pairsGrouped: true,
    };
    act(() =>
      useAssistantScreenMetricsStore.getState().publish({
        route: "/transactions",
        period: "1year",
        scope,
        metrics: { review_queue_review: 0, review_queue_pending: 11 },
      }),
    );
    act(() => session.sendPrompt("Why are 11 pending?"));
    expect(mocks.send).not.toHaveBeenCalled();

    // Before it runs, the user leaves Transactions for Reports, which clears
    // the Transactions counts.
    window.history.replaceState(null, "", "/reports?year=2025");
    act(() => useAssistantScreenMetricsStore.getState().clear("/transactions"));
    mocks.streaming = false;
    mocks.seeded = true;
    act(() => view.rerender(provider()));

    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(lastScreenContext()).toEqual({
      route: "/transactions",
      capabilities: ["transactions"],
      filters: { period: "1year" },
      metrics: { review_queue_review: 0, review_queue_pending: 11 },
      metricsScope: scope,
    });
    expect(mocks.send.mock.calls[0][0].expectedScope).toEqual({
      workspace_id: "ws-1",
      profile_id: "profile-1",
    });

    // A question asked now belongs to the page on screen.
    act(() => session.sendPrompt("And this year's report?"));
    expect(lastScreenContext()).toEqual({
      route: "/reports",
      capabilities: ["reports"],
      filters: { year: 2025 },
    });
  });

  it("returns a question queued on another page in the previous book to the composer", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => session.sendPrompt("Why are 11 pending?"));

    window.history.replaceState(null, "", "/reports");
    useUiStore.setState({ notifications: [] });
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.streaming = false;
      view.rerender(provider());
    });

    // Its page context was the previous book's: nothing is sent, and the
    // user's own text, not a page's, goes back to the composer.
    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toBe("Why are 11 pending?");
    expect(useUiStore.getState().notifications.at(0)?.body).toBe(
      "You switched books before it was sent. Your text is back in the composer.",
    );
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

  it("sends a parked handoff with the page it came from once a model is chosen", () => {
    useUiStore.setState({ assistantModelSelection: null });
    mount();
    window.history.replaceState(null, "", "/chain-analysis");
    act(() => handOffToAssistant(session, "Analyze this PSBT."));
    expect(mocks.send).not.toHaveBeenCalled();

    // Later, on another page, the user edits the draft, picks a model and
    // sends it.
    window.history.replaceState(null, "", "/quarantine");
    act(() =>
      useAssistantDraftStore.getState().setDraft("Analyze this PSBT, please."),
    );
    act(() =>
      useUiStore.setState({
        assistantModelSelection: { provider: "copilot", model: "default" },
      }),
    );
    act(() => session.sendPrompt(useAssistantDraftStore.getState().draft));
    expect(lastScreenContext()).toMatchObject({
      route: "/chain-analysis",
      capabilities: ["privacy", "transactions", "transfers"],
    });

    // It is used once: the next prompt belongs to the page on screen.
    act(() => session.sendPrompt("And this cause?"));
    expect(lastScreenContext()).toMatchObject({ route: "/quarantine" });
  });

  it("drops a parked handoff when its draft is cleared or replaced", () => {
    useUiStore.setState({ assistantModelSelection: null });
    mount();
    window.history.replaceState(null, "", "/chain-analysis");
    act(() => handOffToAssistant(session, "Analyze this PSBT."));
    window.history.replaceState(null, "", "/quarantine");
    act(() =>
      useUiStore.setState({
        assistantModelSelection: { provider: "copilot", model: "default" },
      }),
    );

    // Cleared, then retyped: an ordinary question about this page.
    act(() => useAssistantDraftStore.getState().setDraft(""));
    act(() => session.sendPrompt("What is held here?"));
    expect(lastScreenContext()).toMatchObject({ route: "/quarantine" });

    // Replaced by Ask about this: the pin applies, not the old handoff.
    act(() => useUiStore.setState({ assistantModelSelection: null }));
    window.history.replaceState(null, "", "/chain-analysis");
    act(() => handOffToAssistant(session, "Analyze this PSBT."));
    window.history.replaceState(null, "", "/quarantine");
    act(() => ask!({ kind: "quarantine", transactionId: "tx-held" }));
    act(() =>
      useUiStore.setState({
        assistantModelSelection: { provider: "copilot", model: "default" },
      }),
    );
    act(() => session.sendPrompt(useAssistantDraftStore.getState().draft));
    expect(lastScreenContext()).toMatchObject({
      route: "/quarantine",
      entityId: "tx-held",
    });
  });

  it("does not send a queued request into another book", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    // A Source of Funds handoff queued behind the running answer.
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-old-book");
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    act(() => session.sendPrompt("And the fee?"));
    useUiStore.setState({ notifications: [] });

    // The book switches before the answer ends.
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.streaming = false;
      view.rerender(provider());
    });

    expect(mocks.send).not.toHaveBeenCalled();
    expect(session.queuedPrompts).toEqual([]);
    // The user's own text is back in the composer. The handoff's text was
    // made from the old book's page and carries its data, so it is dropped,
    // and the notice says both.
    expect(useAssistantDraftStore.getState().draft).toBe("And the fee?");
    expect(useAssistantDraftStore.getState().handoff).toBeNull();
    const notice = useUiStore.getState().notifications.at(0);
    expect(notice?.title).toBe("Not sent: you switched books");
    expect(notice?.body).toContain("Your own text is back in the composer");
    expect(JSON.stringify(useUiStore.getState().notifications)).not.toContain(
      "source of funds case",
    );
  });

  it("does not send a handoff parked in another book", () => {
    useUiStore.setState({ assistantModelSelection: null });
    mount();
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-old-book");
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    expect(useAssistantDraftStore.getState().handoff).not.toBeNull();
    useUiStore.setState({ notifications: [] });

    // A model is chosen and the parked request sent in the same moment the
    // book switches, before anything could clear the composer.
    act(() => {
      useUiStore.setState({
        assistantModelSelection: { provider: "copilot", model: "default" },
      });
    });
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      session.sendPrompt("Investigate this source of funds case.");
    });

    expect(mocks.send).not.toHaveBeenCalled();
    // Its text carries the old book's data: not kept for a manual resend.
    expect(useAssistantDraftStore.getState().draft).toBe("");
    expect(useAssistantDraftStore.getState().handoff).toBeNull();
    expect(useUiStore.getState().notifications.at(0)?.body).toContain(
      "referred to its data",
    );
  });

  it("clears a parked handoff from the composer when the book changes", () => {
    useUiStore.setState({ assistantModelSelection: null });
    mount();
    window.history.replaceState(null, "", "/chain-analysis");
    act(() => handOffToAssistant(session, "Analyze this PSBT."));
    useUiStore.setState({ notifications: [] });

    act(() => useUiStore.getState().bumpDaemonSession());

    expect(useAssistantDraftStore.getState().draft).toBe("");
    expect(useAssistantDraftStore.getState().handoff).toBeNull();
    expect(useUiStore.getState().notifications).toHaveLength(1);
  });

  it("does not send a question held for effort seeding into another book", () => {
    mocks.seeded = false;
    const view = mount();
    act(() => session.sendPrompt("Why is this pending?"));
    expect(mocks.send).not.toHaveBeenCalled();
    useAssistantDraftStore.setState({ draft: "Why is this pending?" });

    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.seeded = true;
      view.rerender(provider());
    });

    expect(mocks.send).not.toHaveBeenCalled();
    // Already in the composer: kept once, not duplicated.
    expect(useAssistantDraftStore.getState().draft).toBe("Why is this pending?");
  });

  it("keeps the pin when the language changes", async () => {
    mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-7f3a" }));
    expect(session.focus).not.toBeNull();
    await act(async () => {
      await i18n.changeLanguage("de");
    });
    try {
      expect(session.focus?.context.entityId).toBe("tx-7f3a");
    } finally {
      await act(async () => {
        await i18n.changeLanguage("en");
      });
    }
  });

  it("sends every turn with the book it was asked in", () => {
    mount();
    act(() => session.sendPrompt("How many pending?"));
    expect(mocks.send.mock.calls[0][0].expectedScope).toEqual({
      workspace_id: "ws-1",
      profile_id: "profile-1",
    });
  });

  // Creating a book ("New book") opens it without a new daemon session: only
  // the book's ids change. Everything made in the old book stays there.
  const BOOK_B = { workspace_id: "ws-1", profile_id: "profile-b" };

  it("is a new book without a new daemon session", () => {
    expect(mutationAdvancesDaemonSession("ui.profiles.create")).toBe(false);
    expect(mutationAdvancesDaemonSession("ui.workspace.create")).toBe(false);
  });

  it("clears a handoff parked in the old book when a new book opens", () => {
    useUiStore.setState({ assistantModelSelection: null });
    const view = mount();
    // Source of Funds hands its case over before a model is chosen.
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-old-book");
    act(() => handOffToAssistant(session, "Investigate this source of funds case."));
    expect(useAssistantDraftStore.getState().handoff).not.toBeNull();
    useUiStore.setState({ notifications: [] });

    // On Books, the user creates book B, which opens it.
    window.history.replaceState(null, "", "/books");
    mocks.scope = BOOK_B;
    act(() => view.rerender(provider()));

    // Its text carries book A's case: gone, and the notice says so.
    expect(useAssistantDraftStore.getState().draft).toBe("");
    expect(useAssistantDraftStore.getState().handoff).toBeNull();
    expect(useUiStore.getState().notifications.at(0)?.body).toContain("referred to its data");
    act(() =>
      useUiStore.setState({ assistantModelSelection: { provider: "copilot", model: "default" } }),
    );
    act(() => session.sendPrompt("What is open here?"));
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-old-book");
    expect(mocks.send.mock.calls.at(-1)?.[0].expectedScope).toEqual(BOOK_B);
  });

  it("drops an Ask about this pin from the old book when a new book opens", () => {
    const view = mount();
    act(() => ask!({ kind: "transaction", transactionId: "tx-old-book" }));
    expect(session.focus).not.toBeNull();

    mocks.scope = BOOK_B;
    act(() => view.rerender(provider()));
    expect(session.focus).toBeNull();
    act(() => session.sendPrompt("Why is this pending?"));
    expect(JSON.stringify(mocks.send.mock.calls)).not.toContain("tx-old-book");
  });

  it("does not run a queued question in a new book", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => session.sendPrompt("Investigate case tx-old-book."));
    useUiStore.setState({ notifications: [] });

    // Book B opens while the answer streams; the session counter stays.
    mocks.scope = BOOK_B;
    mocks.streaming = false;
    act(() => view.rerender(provider()));

    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toBe("Investigate case tx-old-book.");
    expect(useUiStore.getState().notifications.at(0)?.title).toBe("Not sent: you switched books");
  });

  it("keeps a question asked before the book's ids loaded in that book", () => {
    // Right after launch the ids are not known yet, and an answer streams.
    mocks.scope = null;
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => session.sendPrompt("Investigate case tx-old-book."));

    // The ids load (book A), then the user creates book B before it runs.
    mocks.scope = { workspace_id: "ws-1", profile_id: "profile-1" };
    act(() => view.rerender(provider()));
    mocks.scope = BOOK_B;
    mocks.streaming = false;
    act(() => view.rerender(provider()));

    // It was asked in book A: never sent as book B's question.
    expect(mocks.send).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toBe("Investigate case tx-old-book.");
  });

  it("returns a queued prompt the draft only mentions", () => {
    const view = mount();
    mocks.streaming = true;
    view.rerender(provider());
    act(() => session.sendPrompt("explain fees"));
    // Meanwhile the user drafts something that contains those words.
    act(() =>
      useAssistantDraftStore.getState().setDraft("don't explain fees yet; check basis"),
    );

    act(() => {
      useUiStore.getState().bumpDaemonSession();
      mocks.streaming = false;
      view.rerender(provider());
    });

    expect(useAssistantDraftStore.getState().draft).toBe(
      "don't explain fees yet; check basis\n\nexplain fees",
    );
  });

  it("holds sends until the open book is known", () => {
    mocks.scope = null;
    const view = mount();
    expect(session.sendBlockedReason).toBe("Loading this book…");
    act(() => session.sendPrompt("How many pending?"));
    expect(mocks.send).not.toHaveBeenCalled();

    mocks.scope = { workspace_id: "ws-1", profile_id: "profile-1" };
    act(() => view.rerender(provider()));
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(mocks.send.mock.calls[0][0].expectedScope).toEqual(mocks.scope);
  });

  it("handles the daemon refusing a turn asked in another book", async () => {
    mocks.send.mockResolvedValue({ refused: true });
    mount();
    useUiStore.setState({ notifications: [] });

    // The user's own question comes back to the composer.
    await act(async () => {
      session.sendPrompt("Why is this pending?");
    });
    expect(useAssistantDraftStore.getState().draft).toBe("Why is this pending?");
    expect(useUiStore.getState().notifications.at(0)?.title).toBe(
      "Not sent: you switched books",
    );

    // A page's handoff carries that book's data: it is not kept.
    useAssistantDraftStore.setState({ draft: "" });
    useUiStore.setState({ notifications: [] });
    window.history.replaceState(null, "", "/source-of-funds?tx=tx-old-book");
    await act(async () => {
      handOffToAssistant(session, "Investigate this source of funds case.");
    });
    expect(useAssistantDraftStore.getState().draft).toBe("");
    expect(useUiStore.getState().notifications.at(0)?.body).toContain(
      "referred to its data",
    );
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
      pairsGrouped: true,
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

    // Nor without saying how the page counted reviewed pairs.
    act(() =>
      useAssistantScreenMetricsStore.getState().publish({
        route: "/transactions",
        scope: { ...scope, pairsGrouped: undefined as unknown as boolean },
        metrics: { review_queue_pending: 11 },
      }),
    );
    act(() => session.sendPrompt("And now?"));
    expect(lastScreenContext()).not.toHaveProperty("metrics");

    // Another page's snapshot never rides along.
    window.history.replaceState(null, "", "/overview");
    act(() => session.sendPrompt("And here?"));
    expect(lastScreenContext()).not.toHaveProperty("metrics");
  });
});
