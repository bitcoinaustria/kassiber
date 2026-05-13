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
});
