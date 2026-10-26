// @vitest-environment happy-dom
//
// A fast-mode notice is part of the stored answer: reopening a saved chat
// shows it again, from the provenance `ui.chat.sessions.get` returns.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { aiChatNotices, storedChatEntries, useAiChatStream } from "@/daemon/stream";
import { ChatMessage } from "./ChatMessage";

const storedRows = [
  { role: "user", content: "Reply with just: ok", provenance: null },
  {
    role: "assistant",
    content: "ok",
    // Unknown phases are provider-chosen and never shown.
    provenance: { notices: ["fast_mode_unavailable", "provider_says_anything"] },
  },
  { role: "user", content: "Again", provenance: { notices: ["fast_mode_unavailable"] } },
  { role: "assistant", content: "ok", provenance: { notices: [] } },
  { role: "assistant", content: "", provenance: { notices: ["fast_mode_unavailable"] } },
];

describe("restored fast-mode notices", () => {
  afterEach(cleanup);

  it("keeps only recognised notices, and only on assistant answers", () => {
    expect(aiChatNotices(["provider_says_anything", "fast_mode_unavailable"])).toEqual([
      "fast_mode_unavailable",
    ]);
    expect(aiChatNotices("fast_mode_unavailable")).toEqual([]);
    expect(storedChatEntries(storedRows)).toEqual([
      { role: "user", content: "Reply with just: ok" },
      { role: "assistant", content: "ok", notices: ["fast_mode_unavailable"] },
      { role: "user", content: "Again" },
      { role: "assistant", content: "ok" },
    ]);
  });

  it("renders the notice under a reopened answer", () => {
    const client = new QueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { result } = renderHook(() => useAiChatStream(), { wrapper });
    act(() => result.current.loadConversation(storedChatEntries(storedRows), "session-1"));

    const [, declined, , standard] = result.current.messages;
    expect(declined?.notices).toEqual(["fast_mode_unavailable"]);
    expect(standard?.notices).toBeUndefined();
    expect(renderToStaticMarkup(<ChatMessage message={declined!} />)).toContain(
      "Fast mode was unavailable for this answer; it ran at standard speed.",
    );
  });
});
