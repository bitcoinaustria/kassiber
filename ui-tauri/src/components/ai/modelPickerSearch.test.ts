import { describe, expect, it } from "vitest";

import {
  modelPickerModelKey,
  parseModelPickerModelKey,
} from "./modelPickerKeys";
import {
  buildModelPickerSearchText,
  rankModelPickerItems,
  scoreModelPickerSearch,
  type ModelPickerSearchable,
} from "./modelPickerSearch";

const opus: ModelPickerSearchable = {
  id: "claude-opus-4-7",
  displayName: "Claude Opus 4.7",
  ownedBy: "Anthropic",
  providerName: "claude",
  providerDisplayName: "Claude Code",
};
const gpt: ModelPickerSearchable = {
  id: "gpt-5.4",
  displayName: "GPT 5.4",
  ownedBy: "OpenAI",
  providerName: "codex",
  providerDisplayName: "Codex",
};
const qwen: ModelPickerSearchable = {
  id: "omlx/qwen3.6:35b",
  sourceProvider: "omlx",
  providerName: "opencode",
  providerDisplayName: "OpenCode",
};

describe("buildModelPickerSearchText", () => {
  it("indexes model and provider fields", () => {
    expect(buildModelPickerSearchText(opus)).toBe(
      "claude opus 4.7 claude-opus-4-7 anthropic claude code claude",
    );
  });
});

describe("scoreModelPickerSearch", () => {
  it("requires every token to match some field", () => {
    expect(scoreModelPickerSearch(opus, "opus anthropic")).not.toBeNull();
    expect(scoreModelPickerSearch(gpt, "opus anthropic")).toBeNull();
  });

  it("tolerates typos in longer tokens", () => {
    expect(scoreModelPickerSearch(opus, "anthrpic")).not.toBeNull();
    // Short tokens stay literal, so "zq" does not fuzzy-match everything.
    expect(scoreModelPickerSearch(opus, "zq")).toBeNull();
  });

  it("ranks exact and prefix hits ahead of fuzzy hits", () => {
    const exact = scoreModelPickerSearch(gpt, "codex");
    const prefix = scoreModelPickerSearch(gpt, "cod");
    const fuzzy = scoreModelPickerSearch(gpt, "cdx");
    expect(exact).not.toBeNull();
    expect(prefix).not.toBeNull();
    expect(fuzzy).not.toBeNull();
    expect(exact!).toBeLessThan(prefix!);
    expect(prefix!).toBeLessThan(fuzzy!);
  });

  it("matches word boundaries inside namespaced ids", () => {
    expect(scoreModelPickerSearch(qwen, "qwen")).not.toBeNull();
    expect(scoreModelPickerSearch(qwen, "35b")).not.toBeNull();
    expect(scoreModelPickerSearch(qwen, "omlx")).not.toBeNull();
  });

  it("matches the provider so a query can name where the model runs", () => {
    expect(scoreModelPickerSearch(qwen, "opencode")).not.toBeNull();
  });

  it("boosts favorites", () => {
    const plain = scoreModelPickerSearch(opus, "opus")!;
    const favorite = scoreModelPickerSearch({ ...opus, isFavorite: true }, "opus")!;
    expect(favorite).toBeLessThan(plain);
  });
});

describe("rankModelPickerItems", () => {
  it("drops non-matches and orders by score", () => {
    const ranked = rankModelPickerItems([gpt, qwen, opus], "claude", (item) => item);
    expect(ranked.map((item) => item.id)).toEqual(["claude-opus-4-7"]);
  });

  it("keeps the caller's order for equal scores", () => {
    const a = { ...gpt, id: "a", displayName: "Same" };
    const b = { ...gpt, id: "b", displayName: "Same" };
    expect(
      rankModelPickerItems([b, a], "same", (item) => item).map((item) => item.id),
    ).toEqual(["b", "a"]);
  });

  it("puts a favorite first among equal matches", () => {
    const a = { ...gpt, id: "a", displayName: "Same" };
    const b = { ...gpt, id: "b", displayName: "Same", isFavorite: true };
    expect(
      rankModelPickerItems([a, b], "same", (item) => item).map((item) => item.id),
    ).toEqual(["b", "a"]);
  });
});

describe("modelPickerModelKey", () => {
  it("round-trips provider and model ids containing separators", () => {
    const key = modelPickerModelKey("a:b", "ollama/qwen:35b");
    expect(parseModelPickerModelKey(key)).toEqual({
      provider: "a:b",
      model: "ollama/qwen:35b",
    });
  });

  it("does not collide when the split point moves", () => {
    expect(modelPickerModelKey("a", "b:c")).not.toBe(
      modelPickerModelKey("a:b", "c"),
    );
  });

  it("rejects foreign keys", () => {
    expect(parseModelPickerModelKey("favorites")).toBeNull();
    expect(parseModelPickerModelKey("model:x:abc")).toBeNull();
    expect(parseModelPickerModelKey("model:9:abc")).toBeNull();
  });
});
