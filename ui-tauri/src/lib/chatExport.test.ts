import { describe, expect, it } from "vitest";

import {
  buildChatExportMarkdown,
  chatExportFilename,
} from "@/lib/chatExport";
import type { AiChatMessage } from "@/daemon/stream";

describe("chat export", () => {
  it("builds a markdown transcript with user, assistant, and tool status", () => {
    const exportedAt = new Date("2026-04-28T12:34:56.000Z");
    const messages: AiChatMessage[] = [
      {
        id: "user-1",
        role: "user",
        content: "Summarize my ledger.",
        status: "done",
      },
      {
        id: "assistant-1",
        role: "assistant",
        content: "Ledger looks clean.",
        status: "done",
        toolCalls: [
          {
            callId: "tool-1",
            name: "ui.workspace.health",
            arguments: {},
            kindClass: "read_only",
            needsConsent: false,
            status: "done",
          },
        ],
      },
    ];

    expect(chatExportFilename(exportedAt)).toBe("kassiber-chat-2026-04-28.md");
    expect(buildChatExportMarkdown(messages, exportedAt)).toMatchInlineSnapshot(`
      "# Kassiber chat export

      Exported: 2026-04-28T12:34:56.000Z

      ## You

      Summarize my ledger.

      ---

      ## Kassiber

      Ledger looks clean.
      Tools:
      - ui.workspace.health: done
      "
    `);
  });
});
