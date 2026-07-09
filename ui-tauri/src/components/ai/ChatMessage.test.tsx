import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatMessage } from "./ChatMessage";
import type { AiChatMessage } from "@/daemon/stream";

describe("ChatMessage", () => {
  it("keeps tool usage details collapsed by default", () => {
    const message: AiChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "",
      status: "streaming",
      toolCalls: [
        {
          callId: "call-1",
          name: "ui.workspace.health",
          arguments: { include_reports: true },
          kindClass: "read_only",
          needsConsent: false,
          status: "running",
        },
      ],
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain("Tool usage");
    expect(html).not.toContain("include_reports");
  });

  it("shows the copy/more actions on a completed assistant answer", () => {
    const message: AiChatMessage = {
      id: "assistant-2",
      role: "assistant",
      content: "Here is your answer.",
      status: "done",
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain('aria-label="Copy"');
    expect(html).toContain('aria-label="More options"');
  });

  it("offers an edit action on a user message when enabled", () => {
    const message: AiChatMessage = {
      id: "user-1",
      role: "user",
      content: "original question",
      status: "done",
    };

    const withEdit = renderToStaticMarkup(
      <ChatMessage message={message} onEdit={() => {}} />,
    );
    const withoutEdit = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(withEdit).toContain('aria-label="Edit message"');
    expect(withoutEdit).not.toContain('aria-label="Edit message"');
  });

  it("hides the actions while the answer is still streaming", () => {
    const message: AiChatMessage = {
      id: "assistant-3",
      role: "assistant",
      content: "Partial answer so far",
      status: "streaming",
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).not.toContain('aria-label="Copy"');
    expect(html).not.toContain('aria-label="More options"');
  });
});
