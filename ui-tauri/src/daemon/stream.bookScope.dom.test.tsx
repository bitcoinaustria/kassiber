// @vitest-environment happy-dom
//
// A turn the daemon refuses before it starts, because it names a book that is
// no longer open (`stale_context` against its `expected_scope`), never ran:
// the hook drops it from the transcript and tells the caller, instead of
// showing it as a failed answer.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

const transport = vi.hoisted(() => ({
  stream: vi.fn(),
  args: [] as unknown[],
}));

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/transport")>()),
  getTransport: () => ({
    stream: (request: unknown, options: unknown) => {
      transport.args.push(request);
      return transport.stream(request, options);
    },
  }),
}));

import { useAiChatStream, type AiChatRequest } from "./stream";

const REQUEST: AiChatRequest = {
  provider: "copilot",
  model: "default",
  messages: [{ role: "user", content: "Investigate case tx-old-book." }],
  expectedScope: { workspace_id: "ws-1", profile_id: "profile-1" },
};

function hook() {
  const client = new QueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useAiChatStream(), { wrapper });
}

describe("a turn refused for another book", () => {
  afterEach(() => {
    cleanup();
    transport.stream.mockReset();
    transport.args.length = 0;
  });

  it("is dropped from the transcript and reported as refused", async () => {
    transport.stream.mockResolvedValue({
      kind: "error",
      error: { code: "stale_context", message: "The active book changed" },
    });
    const { result } = hook();
    let outcome: { refused: boolean } | undefined;
    await act(async () => {
      outcome = await result.current.send(REQUEST, "Investigate case tx-old-book.");
    });

    expect(outcome).toEqual({ refused: true });
    expect(result.current.messages).toEqual([]);
    expect(result.current.error).toBeNull();
    expect(transport.args[0]).toMatchObject({
      kind: "ai.chat",
      args: { expected_scope: REQUEST.expectedScope },
    });
  });

  it("still shows a failure after the turn started", async () => {
    transport.stream.mockImplementation(
      async (_request: unknown, options: { onRecord: (record: unknown) => void }) => {
        options.onRecord({ kind: "ai.chat.status", data: { phase: "preparing" } });
        return {
          kind: "error",
          error: { code: "stale_context", message: "The active book changed" },
        };
      },
    );
    const { result } = hook();
    let outcome: { refused: boolean } | undefined;
    await act(async () => {
      outcome = await result.current.send(REQUEST, "Investigate case tx-old-book.");
    });
    expect(outcome).toEqual({ refused: false });
    expect(result.current.error?.code).toBe("stale_context");
    expect(result.current.messages.at(-1)?.status).toBe("error");
  });
});
