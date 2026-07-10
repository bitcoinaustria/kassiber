import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatMessage } from "./ChatMessage";
import { ChatToolCall } from "./ChatToolCall";
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

  it("renders local references and the per-answer privacy receipt", () => {
    const message: AiChatMessage = {
      id: "assistant-review",
      role: "assistant",
      content: "Review complete.",
      status: "done",
      toolCalls: [
        {
          callId: "call-review",
          name: "ui.transactions.review_context",
          arguments: { transaction: "tx-1" },
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: {
            kind: "ui.transactions.review_context",
            schema_version: 1,
            data: {
              local_reference: {
                route: "/transactions",
                transaction: "tx-1",
              },
            },
          },
        },
      ],
      provenance: {
        tools_used: ["ui.transactions.review_context"],
        privacy_receipt: {
          provider_kind: "remote",
          remote_provider: true,
          advertised_tool_count: 18,
          egress_records: 2,
          egress_bytes_out: 1234,
        },
      },
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain("remote provider");
    expect(html).toContain("18 tool schemas");
    expect(html).toContain("2 outbound events");

    const toolHtml = renderToStaticMarkup(
      <ChatToolCall toolCall={message.toolCalls![0]} />,
    );
    expect(toolHtml).toContain("Open transaction");
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

  it("keeps the inline edit controls hidden until editing starts", () => {
    const message: AiChatMessage = {
      id: "user-2",
      role: "user",
      content: "original question",
      status: "done",
    };

    const html = renderToStaticMarkup(
      <ChatMessage message={message} onEdit={() => {}} />,
    );

    // The confirm/cancel affordances only appear once the user opens the
    // inline editor; the default bubble just shows the edit entry point.
    expect(html).not.toContain('aria-label="Confirm edit"');
    expect(html).not.toContain('aria-label="Cancel edit"');
  });

  it("hides the status pill while the reasoning header is visible", () => {
    const message: AiChatMessage = {
      id: "assistant-thinking",
      role: "assistant",
      content: "",
      status: "streaming",
      thinking: "Checking report readiness...",
      thinkingSegments: [
        { id: "seg-1", content: "Checking report readiness..." },
      ],
      activityLabel: "Thinking",
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain("Thinking");
    expect(html).not.toContain('aria-label="Assistant status: Thinking"');
  });

  it("renders separate thinking panes for each model round", () => {
    const message: AiChatMessage = {
      id: "assistant-rounds",
      role: "assistant",
      content: "Done.",
      status: "done",
      thinkingSegments: [
        { id: "seg-1", content: "Plan the tool call." },
        { id: "seg-2", content: "Summarize the tool result." },
      ],
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain("Thoughts · round 1");
    expect(html).toContain("Thoughts · round 2");
  });

  it("shows the status pill before visible reasoning content arrives", () => {
    const message: AiChatMessage = {
      id: "assistant-loading",
      role: "assistant",
      content: "",
      status: "streaming",
      activityLabel: "Loading model",
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain('aria-label="Assistant status: Loading model"');
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

  it("omits provenance metrics already shown in fact cards", () => {
    const message: AiChatMessage = {
      id: "assistant-4",
      role: "assistant",
      content: "Portfolio summary ready.",
      status: "done",
      provenance: {
        tools_used: ["a", "b", "c", "d", "e"],
        active_transactions: 119,
        quarantines: 0,
        missing_price_transactions: null,
        auto_journal_processed: true,
        journals_processed_at: "2026-07-09T11:58:00Z",
        auto_sync_attempted: false,
        auto_sync_ok: null,
        generated_at: "2026-07-09T12:10:00Z",
      },
      toolCalls: [
        {
          callId: "call-summary",
          name: "ui.reports.summary",
          arguments: {},
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: {
            kind: "ui.reports.summary",
            schema_version: 1,
            data: {
              metrics: { active_transactions: 119 },
              asset_flow: [
                {
                  asset: "BTC",
                  inbound_amount_sat: 1_000_000,
                  outbound_amount_sat: 500_000,
                },
              ],
            },
          },
        },
      ],
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html).toContain("119 active tx");
    expect(html).toContain("5 local tools");
    expect(html).toContain("journals refreshed");
    expect(html).toContain("answered");
    expect(html).not.toMatch(/119 active tx[^<]*119 active tx/);
    const activeTxCount = html.match(/119 active tx/g)?.length ?? 0;
    expect(activeTxCount).toBe(1);
  });

  it("deduplicates repeated deterministic fact cards by tool kind", () => {
    const balanceSheetResult = {
      kind: "ui.reports.balance_sheet",
      schema_version: 1,
      data: {
        totals_by_asset: [{ asset: "BTC", quantity_sat: 1_000_000 }],
      },
    };
    const message: AiChatMessage = {
      id: "assistant-5",
      role: "assistant",
      content: "Balance sheet.",
      status: "done",
      toolCalls: [
        {
          callId: "call-balance-1",
          name: "ui.reports.balance_sheet",
          arguments: {},
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: balanceSheetResult,
        },
        {
          callId: "call-balance-2",
          name: "ui.reports.balance_sheet",
          arguments: {},
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: balanceSheetResult,
        },
      ],
    };

    const html = renderToStaticMarkup(<ChatMessage message={message} />);

    expect(html.match(/1 asset total/g)?.length ?? 0).toBe(1);
  });
});
