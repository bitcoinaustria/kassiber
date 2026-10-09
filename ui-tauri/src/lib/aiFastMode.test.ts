import { describe, expect, it } from "vitest";

import {
  selectedModelFastMode,
  selectedModelReasoningEfforts,
  type AiProviderRow,
} from "./aiCapabilities";

const provider = (name: string, base_url: string, kind: AiProviderRow["kind"] = "remote") => ({
  name,
  base_url,
  kind,
  has_api_key: false,
  is_default: false,
});

const codex = provider("codex", "codex-cli://default");
const claude = provider("claude", "claude-cli://default");
const copilot = provider("copilot", "copilot-cli://default");
const ollama = provider("ollama", "http://127.0.0.1:11434/v1", "local");
const providers = [codex, claude, copilot, ollama];

describe("fast mode support", () => {
  it("follows a discovered model's advertisement and description", () => {
    const models = [
      {
        id: "gpt-5.4",
        supports_fast_mode: true,
        fast_mode_description: "2x speed, increased usage",
      },
      { id: "gpt-5.4-mini", supports_fast_mode: false },
    ];
    expect(
      selectedModelFastMode({ selection: { provider: "codex", model: "gpt-5.4" }, providers, models }),
    ).toEqual({ supported: true, runtime: "codex", description: "2x speed, increased usage" });
    expect(
      selectedModelFastMode({ selection: { provider: "codex", model: "gpt-5.4-mini" }, providers, models })
        .supported,
    ).toBe(false);
  });

  it("fails closed for unknown models, except the broker's fixed Claude Opus entry", () => {
    expect(
      selectedModelFastMode({ selection: { provider: "codex", model: "gpt-5.4" }, providers, models: [] })
        .supported,
    ).toBe(false);
    expect(
      selectedModelFastMode({ selection: { provider: "claude", model: "opus" }, providers, models: [] }),
    ).toEqual({ supported: true, runtime: "claude" });
    for (const model of ["default", "sonnet", "haiku", "fable"]) {
      expect(
        selectedModelFastMode({ selection: { provider: "claude", model }, providers, models: [] })
          .supported,
      ).toBe(false);
    }
  });

  it("is never offered by Copilot or HTTP providers", () => {
    expect(
      selectedModelFastMode({ selection: { provider: "copilot", model: "default" }, providers, models: [] })
        .supported,
    ).toBe(false);
    // Even a row claiming support cannot route fast mode over HTTP.
    expect(
      selectedModelFastMode({
        selection: { provider: "ollama", model: "qwen3:8b" },
        providers,
        models: [{ id: "qwen3:8b", supports_fast_mode: true }],
      }).supported,
    ).toBe(false);
  });
});

describe("reasoning effort fallback before discovery", () => {
  it("offers the runtime's fixed levels for the stored default row", () => {
    // Copilot's "CLI default" is all the picker knows before Check models.
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "copilot", model: "default" },
        providers,
        models: [],
      }),
    ).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "claude", model: "opus" },
        providers,
        models: [],
      }),
    ).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });

  it("keeps model-dependent runtimes discovery-only", () => {
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "codex", model: "gpt-5.4" },
        providers,
        models: [],
      }),
    ).toEqual([]);
  });

  it("prefers a discovered model's own list", () => {
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "copilot", model: "gpt-4.1" },
        providers,
        models: [{ id: "gpt-4.1", reasoning_efforts: ["low", "high"] }],
      }),
    ).toEqual(["low", "high"]);
  });
});
