// @vitest-environment happy-dom
//
// A graph handoff takes a projection round trip before its prompt exists.
// Its book is captured at the click, so when the user creates book B while
// book A's projection is pending, A's prompt is never sent under B's
// `expected_scope`: the session refuses it, and the daemon would too.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Scope = { workspace_id: string; profile_id: string };
const BOOK_A: Scope = { workspace_id: "ws-1", profile_id: "profile-a" };
const BOOK_B: Scope = { workspace_id: "ws-1", profile_id: "profile-b" };

const mocks = vi.hoisted(() => ({
  scope: null as { workspace_id: string; profile_id: string } | null,
  send: vi.fn(),
  project: vi.fn(),
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

vi.mock("@/daemon/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/client")>()),
  useDaemonMutation: () => ({ mutateAsync: mocks.project, isPending: false }),
}));
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
    reset: vi.fn(),
    loadConversation: vi.fn(),
    forgetSession: vi.fn(),
  }),
}));
vi.mock("@/components/ai/useActiveBookScope", () => ({
  useActiveBookScope: () => mocks.scope,
}));
vi.mock("@/components/ai/useProviderDefaultEffort", () => ({
  useProviderDefaultEffort: ({
    setThinkingEffort,
  }: {
    setThinkingEffort: (effort: string) => void;
  }) => ({ setThinkingEffort, effortSeeded: true }),
}));
vi.mock("@/components/ai/useFastModeSupport", () => ({
  useFastModeSupport: () => ({ supported: false, resolved: true, runtime: null }),
}));

import { AssistantSessionProvider } from "@/components/ai/AssistantSessionProvider";
import { DaemonScopeContext, type DaemonScopeBoundary } from "@/daemon/client";
import { DEFAULT_ANALYSIS_QUERY } from "@/lib/chainAnalysis";
import { useUiStore } from "@/store/ui";
import { useChainAnalysisAssistant } from "./useChainAnalysisAssistant";

let action: ReturnType<typeof useChainAnalysisAssistant>;
function Workbench() {
  action = useChainAnalysisAssistant(() => {});
  return null;
}

// The real page boundary checks the session and database, not the book ids,
// so creating a book leaves it "current".
function boundaryFor(scope: Scope): DaemonScopeBoundary {
  const daemonSession = useUiStore.getState().daemonSession;
  return {
    expectedScope: scope,
    daemonSession,
    isCurrent: () => useUiStore.getState().daemonSession === daemonSession,
  };
}

function tree(boundary: DaemonScopeBoundary) {
  return (
    <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
      <DaemonScopeContext.Provider value={boundary}>
        <Workbench />
      </DaemonScopeContext.Provider>
    </AssistantSessionProvider>
  );
}

const investigation = {
  query: { ...DEFAULT_ANALYSIS_QUERY, observer: "public" as const, subject: "raw-chain-identity" },
  snapshot_id: "snapshot-a",
};

describe("a graph handoff across a new book", () => {
  beforeEach(() => {
    mocks.scope = BOOK_A;
    mocks.send.mockReset();
    mocks.project.mockReset();
    window.history.replaceState(null, "", "/chain-analysis");
    useUiStore.setState({
      assistantModelSelection: { provider: "copilot", model: "default" },
      notifications: [],
    });
  });
  afterEach(cleanup);

  it("is never sent under the book created while its projection was pending", async () => {
    let project!: (value: unknown) => void;
    mocks.project.mockImplementation(
      () =>
        new Promise((resolve) => {
          project = resolve;
        }),
    );
    const view = render(tree(boundaryFor(BOOK_A)));
    let asked!: Promise<void>;
    act(() => {
      asked = action.ask(investigation);
    });

    // The user creates book B: the session sees B's ids, the page remounts on
    // B's boundary, and the pending ask still holds book A's.
    mocks.scope = BOOK_B;
    act(() => view.rerender(tree(boundaryFor(BOOK_B))));
    await act(async () => {
      project({
        data: {
          query: { subject: "opaque-node", observer: "public" },
          subject: "opaque-node",
          snapshot_id: "snapshot-a",
        },
      });
      await asked;
    });

    expect(mocks.send.mock.calls.map(([request]) => request.expectedScope)).not.toContainEqual(
      BOOK_B,
    );
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("is sent with the book it was asked in when nothing changed", async () => {
    mocks.project.mockResolvedValue({
      data: { query: { subject: "opaque-node" }, subject: "opaque-node", snapshot_id: "snapshot-a" },
    });
    render(tree(boundaryFor(BOOK_A)));
    await act(async () => {
      await action.ask(investigation);
    });
    expect(mocks.send).toHaveBeenCalledTimes(1);
    expect(mocks.send.mock.calls[0][0].expectedScope).toEqual(BOOK_A);
    expect(mocks.send.mock.calls[0][0].screenContext.route).toBe("/chain-analysis");
  });
});
