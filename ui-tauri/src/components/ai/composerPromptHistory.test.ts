import { describe, expect, it } from "vitest";

import {
  buildComposerPromptHistoryEntries,
  isCaretOnTextEdge,
  stepComposerPromptHistory,
  type ComposerPromptHistoryPosition,
} from "./composerPromptHistory";

const entries = buildComposerPromptHistoryEntries([
  { id: "m1", role: "user", content: "first" },
  { id: "a1", role: "assistant", content: "reply" },
  { id: "m2", role: "user", content: "second" },
  { id: "m3", role: "user", content: "third" },
]);

function backward(position: ComposerPromptHistoryPosition | null, currentPrompt: string) {
  return stepComposerPromptHistory({ direction: "backward", entries, position, currentPrompt });
}

function forward(position: ComposerPromptHistoryPosition | null, currentPrompt: string) {
  return stepComposerPromptHistory({ direction: "forward", entries, position, currentPrompt });
}

describe("buildComposerPromptHistoryEntries", () => {
  it("keeps user prompts oldest first and skips other roles and blanks", () => {
    expect(
      buildComposerPromptHistoryEntries([
        { id: "s", role: "system", content: "sys" },
        { id: "u1", role: "user", content: "  hello  " },
        { id: "u2", role: "user", content: "   " },
      ]),
    ).toEqual([{ id: "u1", prompt: "hello" }]);
  });

  it("collapses consecutive duplicates into the newest id", () => {
    expect(
      buildComposerPromptHistoryEntries([
        { id: "u1", role: "user", content: "again" },
        { id: "u2", role: "user", content: "again" },
        { id: "u3", role: "user", content: "other" },
        { id: "u4", role: "user", content: "again" },
      ]),
    ).toEqual([
      { id: "u2", prompt: "again" },
      { id: "u3", prompt: "other" },
      { id: "u4", prompt: "again" },
    ]);
  });
});

describe("stepComposerPromptHistory", () => {
  it("walks back from an empty composer and stops at the oldest prompt", () => {
    const third = backward(null, "");
    expect(third?.prompt).toBe("third");
    const second = backward(third!.position, "third");
    expect(second?.prompt).toBe("second");
    const first = backward(second!.position, "second");
    expect(first?.prompt).toBe("first");
    expect(backward(first!.position, "first")).toBeNull();
  });

  it("does not replace a typed draft", () => {
    expect(backward(null, "draft")).toBeNull();
  });

  it("walks forward and clears past the newest prompt", () => {
    const second = backward(backward(null, "")!.position, "third")!;
    const third = forward(second.position, "second");
    expect(third?.prompt).toBe("third");
    expect(forward(third!.position, "third")).toEqual({ position: null, prompt: "" });
  });

  it("ends browsing once the recalled text was edited", () => {
    const third = backward(null, "")!;
    expect(backward(third.position, "third, edited")).toBeNull();
    expect(forward(third.position, "third, edited")).toBeNull();
  });

  it("does nothing forward without an active recall", () => {
    expect(forward(null, "")).toBeNull();
  });

  it("returns null for an empty history", () => {
    expect(
      stepComposerPromptHistory({
        direction: "backward",
        entries: [],
        position: null,
        currentPrompt: "",
      }),
    ).toBeNull();
  });
});

describe("isCaretOnTextEdge", () => {
  it("detects the first and last logical line", () => {
    const value = "one\ntwo";
    expect(isCaretOnTextEdge(value, 2, 2, "start")).toBe(true);
    expect(isCaretOnTextEdge(value, 5, 5, "start")).toBe(false);
    expect(isCaretOnTextEdge(value, 5, 5, "end")).toBe(true);
    expect(isCaretOnTextEdge(value, 2, 2, "end")).toBe(false);
  });

  it("never treats a selection as an edge", () => {
    expect(isCaretOnTextEdge("one", 0, 3, "start")).toBe(false);
  });
});
