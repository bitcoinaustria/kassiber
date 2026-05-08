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
        content: "Summarize my books.",
        status: "done",
      },
      {
        id: "assistant-1",
        role: "assistant",
        content: "Books look clean.",
        status: "done",
        provenance: {
          generated_at: "2026-04-28T12:34:55Z",
          provider: "ollama",
          model: "gemma4:e4b",
          tools_used: ["ui.reports.summary"],
          active_transactions: 1,
          quarantines: 0,
          auto_journal_processed: true,
        },
        toolCalls: [
          {
            callId: "tool-1",
            name: "ui.reports.summary",
            arguments: { wallet: "Cold" },
            kindClass: "read_only",
            needsConsent: false,
            status: "done",
            result: {
              kind: "ui.reports.summary",
              data: {
                asset_flow: [{ asset: "BTC", inbound_amount_msat: 1000 }],
              },
            },
          },
        ],
      },
    ];

    expect(chatExportFilename(exportedAt)).toBe("kassiber-chat-2026-04-28.md");
    expect(buildChatExportMarkdown(messages, exportedAt)).toMatchInlineSnapshot(`
      "# Kassiber chat export

      Exported: 2026-04-28T12:34:56.000Z

      ## You

      Summarize my books.

      ---

      ## Kassiber

      Books look clean.
      Tools:
      - ui.reports.summary: done
        Arguments: {"wallet":"Cold"}
        Result: {"kind":"ui.reports.summary","data":{"asset_flow":[{"asset":"BTC","inbound_amount_msat":1000}]}}
      Provenance:
      {"generated_at":"2026-04-28T12:34:55Z","provider":"ollama","model":"gemma4:e4b","tools_used":["ui.reports.summary"],"active_transactions":1,"quarantines":0,"auto_journal_processed":true}
      "
    `);
  });
});
