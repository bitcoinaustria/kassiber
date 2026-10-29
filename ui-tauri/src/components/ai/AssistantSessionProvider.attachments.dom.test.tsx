// @vitest-environment happy-dom
//
// Files pasted into the chat (or dropped in the browser preview) are handed
// to the daemon in chunks and come back as the same kind of token a picked
// file gets. Sending waits for them; the message that first carries a file
// shows it and marks it new; later turns keep using it without showing it. Runs the real chat
// stream against a fake transport, so the assertions are on what would reach
// the daemon.
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
  // Set to hold the next upload chunk open until the test settles it.
  uploadHeld: null as null | (() => void),
  holdUpload: false,
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
      if (request.kind === "ui.chat.attachments.upload") {
        if (mocks.holdUpload) {
          mocks.holdUpload = false;
          await new Promise<void>((resolve) => {
            mocks.uploadHeld = resolve;
          });
        }
        return {
          kind: request.kind,
          data: {
            complete: true,
            document_token: `grant-${request.args?.filename}`,
            source: { filename: request.args?.filename, kind: "delimited", size_bytes: request.args?.total_bytes },
          },
        };
      }
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

// The real composer, without the model picker's provider reads.
vi.mock("@/components/ai/ProviderModelPicker", () => ({ ProviderModelPicker: () => null }));
vi.mock("@/components/ai/useReasoningEffortSupport", () => ({
  useSupportedReasoningEffort: () => false,
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

const csv = (name = "export.csv") => new File(["date,amount\n2024-01-01,0.1\n"], name, { type: "text/csv" });
const uploads = () => mocks.invokes.filter((request) => request.kind === "ui.chat.attachments.upload");

describe("a pasted chat file", () => {
  beforeEach(() => {
    mocks.scope = BOOK_A;
    mocks.chats.length = 0;
    mocks.invokes.length = 0;
    mocks.answer = "Read it.";
    mocks.savedAs = null;
    mocks.held = null;
    mocks.holdNext = false;
    mocks.uploadHeld = null;
    mocks.holdUpload = false;
    useUiStore.setState({
      assistantModelSelection: { provider: "copilot", model: "default" },
      notifications: [],
    });
    useAssistantDraftStore.setState({ draft: "", handoff: null });
  });
  afterEach(cleanup);

  it("is staged for its book, holds sending meanwhile, and shows on the message that sends it", async () => {
    const view = render(provider());
    mocks.holdUpload = true;
    let attaching!: Promise<void>;
    await act(async () => {
      attaching = session.attachFiles!([csv()]);
    });
    view.rerender(provider());
    expect(session.attachments).toMatchObject([{ filename: "export.csv", state: "uploading" }]);
    expect(session.sendBlockedReason).toBe("Attaching…");
    await act(async () => {
      mocks.uploadHeld?.();
      await attaching;
    });
    view.rerender(provider());
    expect(uploads()[0].args).toMatchObject({
      filename: "export.csv", offset: 0, expected_scope: BOOK_A,
    });
    expect(session.attachments).toMatchObject([{ token: "grant-export.csv", state: "ready" }]);
    expect(session.sendBlockedReason).toBeNull();

    await act(async () => session.sendPrompt("Import this export"));
    view.rerender(provider());
    expect(lastChat().attachments).toEqual([{ token: "grant-export.csv", new: true }]);
    expect(session.messages[0].attachments).toMatchObject([{ filename: "export.csv", kind: "delimited" }]);
    expect(session.attachments[0].state).toBe("sent");

    // The chat keeps the file; the next message does not show it again.
    await act(async () => session.sendPrompt("And the fees?"));
    expect(lastChat().attachments).toEqual([{ token: "grant-export.csv" }]);
    expect(session.messages[2].content).toBe("And the fees?");
    expect(session.messages[2].attachments).toBeUndefined();
  });

  it("attaches several files, in order, and a later one rides with its own message", async () => {
    const view = render(provider());
    await act(async () => session.attachFiles!([csv("a.csv"), csv("b.png")]));
    view.rerender(provider());
    expect(session.attachments.map((file) => file.filename)).toEqual(["a.csv", "b.png"]);
    await act(async () => session.sendPrompt("Compare these"));
    view.rerender(provider());
    expect(lastChat().attachments).toEqual([
      { token: "grant-a.csv", new: true },
      { token: "grant-b.png", new: true },
    ]);
    expect(session.messages[0].attachments).toHaveLength(2);

    await act(async () => session.attachFiles!([csv("c.csv")]));
    view.rerender(provider());
    await act(async () => session.sendPrompt("And this one?"));
    view.rerender(provider());
    expect(lastChat().attachments).toEqual([
      { token: "grant-a.csv" },
      { token: "grant-b.png" },
      { token: "grant-c.csv", new: true },
    ]);
    expect(session.messages[2].attachments).toMatchObject([{ filename: "c.csv" }]);

    // Removing one takes it off the next turn.
    act(() => session.removeAttachment("grant-b.png"));
    view.rerender(provider());
    await act(async () => session.sendPrompt("Without the image"));
    expect(lastChat().attachments).toEqual([{ token: "grant-a.csv" }, { token: "grant-c.csv" }]);
  });

  it("skips what it cannot take and stops at the chat's limit", async () => {
    const view = render(provider());
    await act(async () => session.attachFiles!([new File(["x"], "notes.zip"), csv("b.csv")]));
    view.rerender(provider());
    expect(session.attachments.map((file) => file.filename)).toEqual(["b.csv"]);
    expect(useUiStore.getState().notifications.at(-1)?.title).toBe("This file can't be attached");

    const many = Array.from({ length: 9 }, (_, index) => csv(`f${index}.csv`));
    await act(async () => session.attachFiles!(many));
    view.rerender(provider());
    expect(session.attachments).toHaveLength(8);
    expect(uploads()).toHaveLength(8);
    // Newest first.
    expect(useUiStore.getState().notifications[0]?.title).toBe("A chat holds up to 8 files");
  });

  it("refuses a file the chat cannot take without sending anything", async () => {
    const view = render(provider());
    await act(async () => session.attachFiles!([new File(["x"], "wallet.dat")]));
    view.rerender(provider());
    expect(session.attachments).toEqual([]);
    expect(uploads()).toHaveLength(0);
    expect(useUiStore.getState().notifications.at(-1)?.title).toBe("This file can't be attached");
  });

  it("drops a paste that finishes after another book opened", async () => {
    const view = render(provider());
    mocks.holdUpload = true;
    let attaching!: Promise<void>;
    await act(async () => {
      attaching = session.attachFiles!([csv()]);
    });
    act(() => useUiStore.getState().bumpDaemonSession());
    mocks.scope = BOOK_B;
    view.rerender(provider());
    await act(async () => {
      mocks.uploadHeld?.();
      await attaching;
    });
    view.rerender(provider());
    expect(session.attachments).toEqual([]);
    await act(async () => session.sendPrompt("Anything pending?"));
    expect(lastChat().attachments).toBeUndefined();
  });
});

function lastChat(): Record<string, unknown> {
  return mocks.chats.at(-1)?.args ?? {};
}
