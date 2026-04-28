import { describe, expect, it } from "vitest";

import { ThinkParser, splitThinkOnce } from "./thinkParser";

describe("ThinkParser", () => {
  it("passes content through when no tags are present", () => {
    const out = splitThinkOnce("plain answer with no reasoning");
    expect(out.content).toBe("plain answer with no reasoning");
    expect(out.thinking).toBe("");
  });

  it("splits a single complete <think> block", () => {
    const out = splitThinkOnce("<think>let me think</think>here is the answer");
    expect(out.thinking).toBe("let me think");
    expect(out.content).toBe("here is the answer");
  });

  it("handles multiple think blocks in one stream", () => {
    const out = splitThinkOnce(
      "intro<think>first</think>middle<think>second</think>tail",
    );
    expect(out.content).toBe("intromiddletail");
    expect(out.thinking).toBe("firstsecond");
  });

  it("buffers tag boundaries split across chunks", () => {
    const parser = new ThinkParser();
    const a = parser.feed("hello <th");
    const b = parser.feed("ink>thoughts</thi");
    const c = parser.feed("nk> world");
    const tail = parser.flush();
    const content =
      a.content + b.content + c.content + tail.content;
    const thinking =
      a.thinking + b.thinking + c.thinking + tail.thinking;
    expect(content).toBe("hello  world");
    expect(thinking).toBe("thoughts");
  });

  it("routes unclosed think content to the thinking channel", () => {
    const out = splitThinkOnce("<think>incomplete reasoning");
    expect(out.thinking).toBe("incomplete reasoning");
    expect(out.content).toBe("");
  });

  it("flushes unclosed content with pending tag-prefix on stream end", () => {
    const parser = new ThinkParser();
    parser.feed("hello <th");
    const tail = parser.flush();
    expect(tail.content).toBe("<th");
    expect(tail.thinking).toBe("");
  });

  it("handles a chunk that ends right at <think>", () => {
    const parser = new ThinkParser();
    const a = parser.feed("intro<think>");
    const b = parser.feed("reasoning</think>tail");
    const tail = parser.flush();
    expect(a.content + b.content + tail.content).toBe("introtail");
    expect(a.thinking + b.thinking + tail.thinking).toBe("reasoning");
  });

  it("does not buffer a non-tag suffix", () => {
    // The `<` that arrives mid-text should not be permanently held; if the
    // next chars are clearly not part of a tag, the parser must flush.
    const parser = new ThinkParser();
    const a = parser.feed("price < 5 sats");
    const tail = parser.flush();
    expect(a.content + tail.content).toBe("price < 5 sats");
    expect(parser.isInsideThink).toBe(false);
  });

  it("handles single-character feeds", () => {
    const parser = new ThinkParser();
    let content = "";
    let thinking = "";
    for (const ch of "<think>x</think>y") {
      const out = parser.feed(ch);
      content += out.content;
      thinking += out.thinking;
    }
    const tail = parser.flush();
    expect(content + tail.content).toBe("y");
    expect(thinking + tail.thinking).toBe("x");
  });
});
