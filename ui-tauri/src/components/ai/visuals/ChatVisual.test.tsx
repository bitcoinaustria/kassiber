import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { AiChatMessage, AiChatToolCall } from "@/daemon/stream";
import { ChatMessage } from "../ChatMessage";
import fixtures from "./__fixtures__/daemon-visuals.json";

type FixtureName = keyof typeof fixtures;

// A static render does not read the book; the flow card then shows the transaction.
vi.mock("@/daemon/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/client")>()),
  useDaemon: () => ({ data: undefined, isLoading: false }),
}));

function visualCall(result: unknown, overrides: Partial<AiChatToolCall> = {}): AiChatToolCall {
  return {
    callId: "call-visual",
    name: "ui.visuals.render",
    arguments: {},
    kindClass: "read_only",
    needsConsent: false,
    status: "done",
    result,
    ...overrides,
  };
}

function render(toolCalls: AiChatToolCall[]) {
  const message: AiChatMessage = {
    id: "assistant-visual",
    role: "assistant",
    content: "Here it is.",
    status: "done",
    toolCalls,
  };
  return renderToStaticMarkup(<ChatMessage message={message} />);
}

describe("ChatVisual in an assistant message", () => {
  it.each(Object.keys(fixtures) as FixtureName[])("renders %s outside the work log", (name) => {
    const html = render([visualCall(fixtures[name])]);
    expect(html).toContain('data-slot="chat-visual"');
    expect(html).toContain(`data-visual-source="${name}"`);
    expect(html).toContain("Drawn by Kassiber from your local books");
    expect(html).toContain("Copy as CSV");
  });

  it("shows the overview's figures with the final values in a static render", () => {
    const html = render([visualCall(fixtures.overview)]);
    expect(html).toContain("Portfolio value");
    expect(html).toContain("Cost basis complete");
    expect(html).toContain("Where your bitcoin is");
    expect(html).toContain("Cold");
  });

  it("lists transactions as buttons that open them", () => {
    const html = render([visualCall(fixtures.transactions)]);
    expect(html).toContain("6 of 6 transactions");
    expect(html).toContain("Open DCA");
  });

  it("falls back to the transaction itself when there is no graph", () => {
    const html = render([visualCall(fixtures.transaction_flow)]);
    expect(html).toContain("no input and output data");
    expect(html).toContain("Sold some");
  });

  it("titles the model's request as text, never markup", () => {
    const data = structuredClone(fixtures.tax_summary) as { data: { visual: Record<string, unknown> } };
    data.data.visual.title = "Gains <script>alert(1)</script>";
    const html = render([visualCall(data)]);
    expect(html).toContain("Gains &lt;script&gt;");
    expect(html).not.toContain("<script>");
  });

  it("shows a placeholder while a visual is being drawn", () => {
    const html = render([
      visualCall(null, { status: "running", arguments: { source: "cash_flow" } }),
    ]);
    expect(html).toContain('data-slot="chat-visual-pending"');
    expect(html).toContain("Drawing Cash flow from your books");
  });

  it("never trusts the model's raw source in the placeholder", () => {
    for (const source of ["constructor", "__proto__", "toString"]) {
      const html = render([visualCall(null, { status: "running", arguments: { source } })]);
      expect(html).toContain("Drawing a visual from your books");
    }
  });

  it("lists a report blocker under its own title", () => {
    const blocked = structuredClone(fixtures.readiness) as {
      data: { rows: Array<Record<string, unknown>>; summary: Record<string, unknown> };
    };
    blocked.data.rows.push({ check: "blocker", status: "blocked", count: null, detail: "Cold could not sync.", action: null, title: "Wallet sync failed" });
    const html = render([visualCall(blocked)]);
    expect(html).toContain("Wallet sync failed");
    expect(html).toContain("Cold could not sync.");
  });

  it("falls back to the tool card when the result is not a visual", () => {
    const html = render([visualCall({ kind: "ui.visuals.render", data: { visual: { version: 1 } } })]);
    expect(html).not.toContain('data-slot="chat-visual"');
  });
});
