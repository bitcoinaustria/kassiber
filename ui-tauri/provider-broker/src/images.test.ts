import { describe, expect, it } from "vitest";

import { promptBlocks } from "./acp.js";
import { chatArgs, claudeStreamInput } from "./claude.js";
import { promptImages } from "./prompt.js";
import type { ChatRequest } from "./protocol.js";

const PNG = "iVBORw0KGgo=";
const image = { media_type: "image/png", data: PNG };

function request(messages: ChatRequest["messages"], options: ChatRequest["options"] = {}): ChatRequest {
  return { command: "chat", request_id: "r", provider: "claude", model: "default", messages, options };
}

describe("attached images", () => {
  const latest = { role: "user", content: "second", images: [{ ...image, media_type: "image/jpeg" }] };
  const messages = [
    { role: "user", content: "first", images: [image] },
    { role: "assistant", content: "answer" },
    latest,
  ];

  it("carries the images of the messages the prompt replays", () => {
    expect(promptImages(messages, false)).toHaveLength(2);
    // Resuming replays the newest user message only, with its images.
    expect(promptImages(messages, true)).toEqual([{ media_type: "image/jpeg", data: PNG }]);
    expect(promptImages([{ role: "user", content: "x" }], false)).toEqual([]);
  });

  it("rejects an image that is not inline base64 of a known type", () => {
    for (const bad of [
      { media_type: "image/svg+xml", data: PNG },
      { media_type: "image/png", data: "https://example.com/a.png" },
      { media_type: "image/png", data: "" },
      { media_type: "text/plain", data: PNG },
    ]) {
      expect(() => promptImages([{ role: "user", content: "x", images: [bad] }], false)).toThrow(
        "Invalid attached image.",
      );
    }
  });

  it("gives Claude one structured message only when there are images", async () => {
    const withImages = await chatArgs(request([latest]), "/tmp/empty");
    expect(withImages.join(" ")).toContain("--input-format stream-json");
    // The variadic flag stays last.
    expect(withImages.indexOf("--input-format")).toBeLessThan(withImages.indexOf("--disallowed-tools"));
    const plain = await chatArgs(request([{ role: "user", content: "x" }]), "/tmp/empty");
    expect(plain).not.toContain("--input-format");

    const line = claudeStreamInput("USER:\nsecond", [image]);
    expect(line.endsWith("\n")).toBe(true);
    expect(JSON.parse(line)).toEqual({
      type: "user",
      message: {
        role: "user",
        content: [
          { type: "text", text: "USER:\nsecond" },
          { type: "image", source: { type: "base64", media_type: "image/png", data: PNG } },
        ],
      },
    });
  });

  it("sends ACP images only to an agent that takes them, and says so otherwise", () => {
    const turn = request([latest]);
    const blocks = promptBlocks(turn, true, true);
    expect(blocks.map((block) => block.type)).toEqual(["text", "image"]);
    expect(blocks[1]).toMatchObject({ type: "image", mimeType: "image/jpeg", data: PNG });

    const textOnly = promptBlocks(turn, true, false);
    expect(textOnly).toHaveLength(1);
    expect(textOnly[0]).toMatchObject({ type: "text" });
    expect((textOnly[0] as { text: string }).text).toContain("1 attached image was not shown");
  });
});
