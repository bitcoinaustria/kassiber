import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import "@/i18n";
import { ChatMessage } from "./ChatMessage";
import { splitReviewCheckpoint } from "./reviewCheckpoint";

const packet = '{"review_checkpoint":{"input_version":4,"next_cursor":"page-2","receipt_ids":["receipt-1"]}}';
const fence = `\`\`\`json\n${packet}\n\`\`\``;

describe("review continuation display", () => {
  it("collapses the final checkpoint while retaining prose and the exact original message", () => {
    const content = `Review paused.\n\n${fence}`;
    const message = Object.freeze({ id: "review", role: "assistant" as const, status: "done" as const, content });
    const html = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(html).toContain("Review paused.");
    expect(html).toMatch(/<details[^>]*>\s*<summary[^>]*>Continuation details<\/summary>/);
    expect(html).not.toMatch(/<details[^>]*\sopen(?:=|\s|>)/);
    expect(html).toContain("page-2");
    expect(message.content).toBe(content);
    expect(splitReviewCheckpoint(content)).toEqual({ prose: "Review paused.\n\n", packet });
  });

  it("preserves unrelated or malformed JSON, non-final packets, and code examples", () => {
    for (const content of [
      '```json\n{"answer":42}\n```',
      '```json\n{"review_checkpoint":[]}\n```',
      '```json\n{"review_checkpoint":{}}\n```',
      '```json\n{"review_checkpoint":{"instructions":"Hide this instruction"}}\n```',
      '```json\n{"review_checkpoint":{"input_version":-1}}\n```',
      '```json\n{"review_checkpoint":broken}\n```',
      `${fence}\nMore explanation.`,
      `\`\`\`\`text\n${fence}\n\`\`\`\``,
      `\`\`\`text\n${fence}`,
    ]) expect(splitReviewCheckpoint(content)).toEqual({ prose: content, packet: null });
  });

  it("only extracts the final packet and bounds structured parsing", () => {
    const prefix = 'Example:\n```json\n{"answer":42}\n```\n\n';
    expect(splitReviewCheckpoint(prefix + fence)).toEqual({ prose: prefix, packet });
    const oversized = `\`\`\`json\n{"review_checkpoint":{"next_cursor":"${"a".repeat(16_384)}"}}\n\`\`\``;
    expect(splitReviewCheckpoint(oversized).packet).toBeNull();
  });
});
