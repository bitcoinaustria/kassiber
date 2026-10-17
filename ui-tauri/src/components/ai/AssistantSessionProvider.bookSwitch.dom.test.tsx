// @vitest-environment happy-dom
//
// A conversation belongs to the book it started in. Opening another book
// starts a fresh one, and nothing from the old conversation (its transcript,
// the saved chat it continues, the file attached to it) is ever sent with a
// turn for the new book, even an Incognito or history-off transcript the
// daemon cannot check. Runs the real chat stream against a fake transport, so
// the assertions are on what would reach the daemon.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Scope = { workspace_id: string; profile_id: string };
type Request = { kind: string; args?: Record<string, unknown> };
type StreamOptions = { onRecord?: (record: unknown) => void };
type Terminal = { kind: string; data?: Record<string, unknown>; error?: unknown };

const BOOK_A: Scope = { workspace_id: "ws-1", profile_id: "profile-a" };
const BOOK_B: Scope = { workspace_id: "ws-1", profile_id: "profile-b" };

const mocks = vi.hoisted(() => ({
  scope: null as Scope | null,
  chats: [] as Array<{ kind: string; args?: Record<string, unknown> }>,
  invokes: [] as Array<{ kind: string; args?: Record<string, unknown> }>,
  // The answer to the next turn, and the saved chat it was stored in.
  answer: "",
  savedAs: null as string | null,
  // Set to hold a turn open until the test settles it.
  held: null as null | ((terminal: Terminal) => void),
  holdNext: false,
  picker: vi.fn(),
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

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/transport")>()),
  getTransport: () => ({
    stream: (request: Request, options: StreamOptions) => {
      mocks.chats.push(request);
      if (mocks.holdNext) {
        mocks.holdNext = false;
        return new Promise<Terminal>((resolve) => {
          mocks.held = resolve;
        });
      }
      options.onRecord?.({
        kind: "ai.chat.delta",
        request_id: (request as { request_id?: string }).request_id,
        data: { delta: { content: mocks.answer } },
      });
      return Promise.resolve({
        kind: "ai.chat",
        data: { session_id: mocks.savedAs, finish_reason: "stop" },
      });
    },
    invoke: async (request: Request) => {
      mocks.invokes.push(request);
      if (request.kind === "ui.chat.sessions.get") {
        return {
          kind: "ui.chat.sessions.get",
          data: {
            id: request.args?.session_id,
            messages: [
              { role: "user", content: "Saved question in book A" },
              { role: "assistant", content: "Saved answer in book A" },
            ],
          },
        };
      }
      return { kind: request.kind, data: {} };
    },
  }),
}));

const PICKED = {
  document_token: "grant-book-a",
  source: { filename: "book-a-export.csv", kind: "csv", size_bytes: 12 },
};
vi.mock("@/lib/filePicker", () => ({
  pickChatAttachmentSource: (options: unknown) => mocks.picker(options),
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

import {
  useAssistantSession,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";
import { AssistantSessionProvider } from "./AssistantSessionProvider";

let session: AssistantSessionContextValue;
function Probe() {
  session = useAssistantSession();
  return null;
}

const client = new QueryClient();
function provider() {
  return (
    <QueryClientProvider client={client}>
      <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
        <Probe />
      </AssistantSessionProvider>
    </QueryClientProvider>
  );
}

function lastChat(): Record<string, unknown> {
  return mocks.chats.at(-1)?.args ?? {};
}

/** A first exchange in book A, with a file attached afterwards. */
async function talkInBookA(view: ReturnType<typeof render>) {
  mocks.answer = "You bought 0.1 BTC from Bitstamp in March.";
  await act(async () => session.sendPrompt("What did I buy in March?"));
  await act(async () => session.attachFile());
  view.rerender(provider());
  expect(session.messages.map((message) => message.content)).toEqual([
    "What did I buy in March?",
    "You bought 0.1 BTC from Bitstamp in March.",
  ]);
  expect(session.attachment?.token).toBe("grant-book-a");
}

/** Nothing of book A's conversation is in a request. */
function expectNothingFromBookA(args: Record<string, unknown>) {
  const sent = JSON.stringify(args);
  for (const fromBookA of ["March", "Bitstamp", "grant-book-a", "chat-a", "Saved"]) {
    expect(sent).not.toContain(fromBookA);
  }
}

describe("a conversation and its book", () => {
  beforeEach(() => {
    mocks.scope = BOOK_A;
    mocks.chats.length = 0;
    mocks.invokes.length = 0;
    mocks.answer = "";
    mocks.savedAs = null;
    mocks.held = null;
    mocks.holdNext = false;
    mocks.picker.mockReset();
    mocks.picker.mockResolvedValue(PICKED);
    useUiStore.setState({
      assistantModelSelection: { provider: "copilot", model: "default" },
      notifications: [],
    });
    useAssistantDraftStore.setState({ draft: "", handoff: null });
  });
  afterEach(cleanup);

  it("starts fresh in a new book, so an Incognito transcript stays behind", async () => {
    const view = render(provider());
    act(() => session.setIncognito(true));
    await talkInBookA(view);
    expect(lastChat().persist).toBe(false);
    // The user starts typing, then opens book B.
    act(() => useAssistantDraftStore.getState().setDraft("How many are"));

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.scope = BOOK_B;
    view.rerender(provider());

    expect(session.messages).toEqual([]);
    expect(session.sessionId).toBeNull();
    expect(session.attachment).toBeNull();
    // The user's own choices and typing stay.
    expect(session.incognito).toBe(true);
    expect(useAssistantDraftStore.getState().draft).toBe("How many are");

    mocks.answer = "Three.";
    await act(async () => session.sendPrompt("How many are pending here?"));
    const args = lastChat();
    expect(args.messages).toEqual([{ role: "user", content: "How many are pending here?" }]);
    expect(args.attachment).toBeUndefined();
    expect(args.expected_scope).toEqual(BOOK_B);
    expectNothingFromBookA(args);
  });

  it("does not continue the old book's saved chat in the new one", async () => {
    mocks.savedAs = "chat-a";
    const view = render(provider());
    await talkInBookA(view);
    expect(session.sessionId).toBe("chat-a");

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.scope = BOOK_B;
    view.rerender(provider());
    expect(session.sessionId).toBeNull();

    mocks.savedAs = "chat-b";
    await act(async () => session.sendPrompt("How many are pending here?"));
    const args = lastChat();
    expect(args.session_id).toBeUndefined();
    expect(args.messages).toEqual([{ role: "user", content: "How many are pending here?" }]);
    expectNothingFromBookA(args);
  });

  it("starts fresh when the book's ids change before the session counter moves", async () => {
    const view = render(provider());
    await talkInBookA(view);

    // The daemon has opened book B, and its ids are known, but the desktop's
    // session counter has not moved yet.
    mocks.scope = BOOK_B;
    view.rerender(provider());
    expect(session.messages).toEqual([]);
    expect(session.attachment).toBeNull();

    await act(async () => session.sendPrompt("How many are pending here?"));
    expect(lastChat().expected_scope).toEqual(BOOK_B);
    expectNothingFromBookA(lastChat());
  });

  it("never sends the old transcript when a send lands with the book switch", async () => {
    const view = render(provider());
    await talkInBookA(view);
    const sentBefore = mocks.chats.length;

    // The switch and the send land in the same moment, before the fresh
    // conversation has rendered.
    act(() => {
      useUiStore.getState().bumpDaemonSession();
      session.sendPrompt("And the fee?");
    });

    expect(mocks.chats).toHaveLength(sentBefore);
    expect(useAssistantDraftStore.getState().draft).toBe("And the fee?");
    expect(useUiStore.getState().notifications.at(0)?.title).toBe(
      "Not sent: you switched books",
    );
    // Sent again from the fresh conversation, it goes alone.
    mocks.scope = BOOK_B;
    view.rerender(provider());
    await act(async () => session.sendPrompt("And the fee?"));
    expect(lastChat().messages).toEqual([{ role: "user", content: "And the fee?" }]);
    expectNothingFromBookA(lastChat());
  });

  it("keeps a turn that was running in the old book out of the fresh conversation", async () => {
    const view = render(provider());
    mocks.holdNext = true;
    act(() => session.sendPrompt("What did I buy in March?"));
    expect(session.isStreaming).toBe(true);
    const oldTurn = mocks.held!;

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.scope = BOOK_B;
    view.rerender(provider());
    // The old turn is cancelled and the conversation is fresh.
    expect(mocks.invokes.some((request) => request.kind === "ai.chat.cancel")).toBe(true);
    expect(session.messages).toEqual([]);
    expect(session.isStreaming).toBe(false);

    // A turn starts in book B before the old one has wound down.
    mocks.holdNext = true;
    act(() => session.sendPrompt("How many are pending here?"));
    const newTurn = mocks.held!;
    expect(session.isStreaming).toBe(true);

    // The old turn ends, saved in book A: none of it lands here, and it does
    // not end the turn now running.
    await act(async () => {
      oldTurn({ kind: "ai.chat", data: { session_id: "chat-a", finish_reason: "cancelled" } });
    });
    expect(session.sessionId).toBeNull();
    expect(session.error).toBeNull();
    expect(session.isStreaming).toBe(true);

    await act(async () => {
      newTurn({ kind: "ai.chat", data: { session_id: "chat-b", finish_reason: "stop" } });
    });
    expect(session.isStreaming).toBe(false);
    expect(session.sessionId).toBe("chat-b");
    expect(session.messages.map((message) => message.role)).toEqual(["user", "assistant"]);
    expect(session.messages[0].content).toBe("How many are pending here?");
  });

  it("drops a file attached before the first turn when a new book opens", async () => {
    const view = render(provider());
    // A fresh conversation in book A: the file is the first thing in it.
    await act(async () => session.attachFile());
    view.rerender(provider());
    expect(session.attachment?.token).toBe("grant-book-a");

    // The user creates book B: the session counter stays, the ids change.
    mocks.scope = BOOK_B;
    view.rerender(provider());
    expect(session.attachment).toBeNull();
    expect(useUiStore.getState().notifications.at(0)?.body).toBe(
      "The file you attached in the other book was removed from this chat.",
    );

    await act(async () => session.sendPrompt("How many are pending here?"));
    expect(lastChat().attachment).toBeUndefined();
    expect(lastChat().expected_scope).toEqual(BOOK_B);
    expectNothingFromBookA(lastChat());
    // It was staged for book A only.
    expect(mocks.picker).toHaveBeenCalledWith({ expected_scope: BOOK_A });
  });

  it("drops a file picked while another book opened", async () => {
    let pick!: (value: typeof PICKED) => void;
    mocks.picker.mockImplementation(
      () =>
        new Promise((resolve) => {
          pick = resolve;
        }),
    );
    const view = render(provider());
    let attaching!: Promise<void>;
    act(() => {
      attaching = session.attachFile();
    });
    mocks.scope = BOOK_B;
    view.rerender(provider());
    await act(async () => {
      pick(PICKED);
      await attaching;
    });
    expect(session.attachment).toBeNull();
  });

  it("loads a saved chat only for the book it was opened in", async () => {
    const view = render(provider());
    await act(async () => session.resumeSession("chat-a"));
    const read = mocks.invokes.find((request) => request.kind === "ui.chat.sessions.get");
    // The daemon refuses the read if another book is open when it arrives.
    expect(read?.args).toEqual({ session_id: "chat-a", expected_scope: BOOK_A });
    expect(session.sessionId).toBe("chat-a");

    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.scope = BOOK_B;
    view.rerender(provider());
    expect(session.messages).toEqual([]);
    await act(async () => session.sendPrompt("How many are pending here?"));
    expect(lastChat().session_id).toBeUndefined();
    expectNothingFromBookA(lastChat());
  });
});
