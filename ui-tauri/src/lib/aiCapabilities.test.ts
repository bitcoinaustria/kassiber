import { describe, expect, it } from "vitest";

import {
  modelSupportsReasoningEffort,
  providerSupportsReasoningEffort,
  selectedModelSupportsReasoningEffort,
} from "./aiCapabilities";

describe("AI reasoning effort capability detection", () => {
  it("uses explicit provider support", () => {
    expect(
      providerSupportsReasoningEffort({
        name: "codex",
        base_url: "codex-cli://default",
        kind: "remote",
        has_api_key: false,
        is_default: true,
        supports_reasoning_effort: true,
      }),
    ).toBe(true);
  });

  it("uses explicit model support metadata", () => {
    expect(
      modelSupportsReasoningEffort({
        id: "reasoner",
        supported_parameters: ["reasoning_effort"],
      }),
    ).toBe(true);
    expect(
      modelSupportsReasoningEffort({
        id: "reasoner",
        capabilities: { reasoning_efforts: ["low", "medium", "high"] },
      }),
    ).toBe(true);
  });

  it("does not infer support from thinking-style model names", () => {
    expect(modelSupportsReasoningEffort({ id: "qwen3.6:35b" })).toBe(false);
    expect(modelSupportsReasoningEffort({ id: "llama3.3:70b" })).toBe(false);
  });

  it("requires the selected provider or selected model to advertise support", () => {
    expect(
      selectedModelSupportsReasoningEffort({
        selection: { provider: "ollama", model: "qwen3.6:35b" },
        providers: [
          {
            name: "ollama",
            base_url: "http://localhost:11434/v1",
            kind: "local",
            has_api_key: false,
            is_default: true,
          },
        ],
        models: [{ id: "qwen3.6:35b" }],
      }),
    ).toBe(false);

    expect(
      selectedModelSupportsReasoningEffort({
        selection: { provider: "openai", model: "reasoner" },
        providers: [
          {
            name: "openai",
            base_url: "https://api.example.test/v1",
            kind: "remote",
            has_api_key: true,
            is_default: true,
          },
        ],
        models: [
          {
            id: "reasoner",
            supports_reasoning_effort: true,
          },
        ],
      }),
    ).toBe(true);
  });
});
