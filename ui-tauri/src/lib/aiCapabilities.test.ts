import { describe, expect, it } from "vitest";

import {
  isLoopbackAiHttpUrl,
  nativeAiProviderRuntime,
  modelSupportsReasoningEffort,
  providerSupportsReasoningEffort,
  selectedModelReasoningEfforts,
  selectedModelSupportsReasoningEffort,
} from "./aiCapabilities";

describe("native AI provider locator registry", () => {
  it("recognizes Codex, Claude, and OpenCode locators", () => {
    expect(nativeAiProviderRuntime(" CODEX-CLI://DEFAULT ")).toBe("codex");
    expect(nativeAiProviderRuntime("claude-cli://default")).toBe("claude");
    expect(nativeAiProviderRuntime("opencode-cli://default")).toBe("opencode");
    expect(nativeAiProviderRuntime("https://example.test/v1")).toBeNull();
  });
});

describe("loopback AI base URLs", () => {
  // The daemon's `_is_loopback_ai_http_url` gives the same answers; the URL
  // is read as written, not as a browser would normalise it.
  it.each([
    ["http://localhost:11434/v1", true],
    ["http://LOCALHOST/v1", true],
    ["https://localhost/v1", true],
    ["http://127.0.0.1:8000/v1", true],
    ["http://127.255.255.255/", true],
    ["http://user@127.0.0.1/", true],
    ["http://[::1]:8000/v1", true],
    ["http://[0:0:0:0:0:0:0:1]/", true],
    ["http://[::ffff:127.0.0.1]/", true],
    [" http://127.0.0.1:8000/v1 ", true],
    ["http://127.1/v1", false],
    ["http://127.000.0.1/", false],
    ["http://0x7f000001/", false],
    ["http://127.256.0.1/", false],
    ["http://128.0.0.1/", false],
    ["http://127.0.0.1.:80/", false],
    ["http://localhost.:1/", false],
    ["http://localhost.example.test/v1", false],
    ["http://127.0.0.1@evil.example.test/v1", false],
    ["http://[::ffff:10.0.0.1]/", false],
    ["http://[::1", false],
    ["HTTP://localhost/", false],
    ["https://api.openai.com/v1", false],
    ["http://192.168.1.5/v1", false],
    ["claude-cli://default", false],
    ["http://", false],
  ])("%s → %s", (url, expected) => {
    expect(isLoopbackAiHttpUrl(url)).toBe(expected);
  });
});

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

  it("reads the advertised reasoning-effort levels for the selected model", () => {
    const providers = [
      {
        name: "ollama",
        base_url: "http://localhost:11434/v1",
        kind: "local" as const,
        has_api_key: false,
        is_default: true,
      },
    ];
    const models = [
      {
        id: "qwen",
        supports_reasoning_effort: true,
        reasoning_efforts: ["Low", "HIGH", "low"],
      },
      { id: "plain" },
    ];

    // Normalized to lower-case, de-duplicated, advertised order preserved.
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "ollama", model: "qwen" },
        providers,
        models,
      }),
    ).toEqual(["low", "high"]);

    // No advertised list → empty (callers fall back to their default set).
    expect(
      selectedModelReasoningEfforts({
        selection: { provider: "ollama", model: "plain" },
        providers,
        models,
      }),
    ).toEqual([]);
  });

  describe("for an HTTP model", () => {
    const providers = [
      {
        name: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        kind: "remote" as const,
        has_api_key: true,
        is_default: true,
      },
    ];
    const pick = (model: Record<string, unknown>) => {
      const args = {
        selection: { provider: "openrouter", model: "m" },
        providers,
        models: [{ id: "m", ...model }],
      };
      return {
        supported: selectedModelSupportsReasoningEffort(args),
        levels: selectedModelReasoningEfforts(args),
      };
    };

    it("lights up from OpenRouter's reasoning metadata, with its levels only", () => {
      // What the daemon makes of OpenRouter's `reasoning` and parameters.
      expect(
        pick({
          supports_reasoning_effort: true,
          reasoning_efforts: ["medium", "high"],
          supported_parameters: ["tools", "reasoning", "include_reasoning"],
        }),
      ).toEqual({ supported: true, levels: ["medium", "high"] });
    });

    it("offers only the wire levels, whatever else a listing names", () => {
      expect(
        pick({
          supports_reasoning_effort: true,
          reasoning_efforts: ["low", "high"],
          capabilities: { reasoning_efforts: ["xhigh", "max"] },
        }).levels,
      ).toEqual(["low", "high"]);
      // Takes an effort without naming levels: the three HTTP levels.
      expect(pick({ supports_reasoning_effort: true }).levels).toEqual(["low", "medium", "high"]);
    });

    it("offers none when the listing does not say the model takes an effort", () => {
      expect(pick({})).toEqual({ supported: false, levels: [] });
      expect(pick({ supports_reasoning_effort: false, reasoning_efforts: ["low"] })).toEqual({
        supported: false,
        levels: [],
      });
      expect(pick({ supported_parameters: ["reasoning_effort"] }).supported).toBe(false);
      // Not checked yet: nothing known, nothing offered.
      expect(
        selectedModelReasoningEfforts({
          selection: { provider: "openrouter", model: "m" },
          providers,
          models: [],
        }),
      ).toEqual([]);
    });

    it("leaves CLI agents' levels as they were", () => {
      const cli = [
        { name: "codex", base_url: "codex-cli://default", kind: "remote" as const, has_api_key: false, is_default: false },
        { name: "claude", base_url: "claude-cli://default", kind: "remote" as const, has_api_key: false, is_default: false },
      ];
      expect(
        selectedModelReasoningEfforts({
          selection: { provider: "codex", model: "gpt-5.4" },
          providers: cli,
          models: [{ id: "gpt-5.4", reasoning_efforts: ["low", "xhigh"] }],
        }),
      ).toEqual(["low", "xhigh"]);
      expect(
        selectedModelReasoningEfforts({
          selection: { provider: "claude", model: "opus" },
          providers: cli,
          models: [],
        }),
      ).toEqual(["low", "medium", "high", "xhigh", "max"]);
      expect(
        selectedModelSupportsReasoningEffort({
          selection: { provider: "codex", model: "gpt-5.4" },
          providers: cli,
          models: [{ id: "gpt-5.4", reasoning_efforts: ["low"] }],
        }),
      ).toBe(true);
    });
  });
});
