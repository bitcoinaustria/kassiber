import { describe, expect, it } from "vitest";

import { aiProviderTestOutcome, aiTestConnectionArgs } from "./aiProviderTest";

describe("provider form Test connection", () => {
  it("sends the form's model, so the reply test asks it", () => {
    expect(
      aiTestConnectionArgs({
        baseUrl: " https://openrouter.ai/api/v1 ",
        kind: "remote",
        apiKey: " sk-typed ",
        model: " anthropic/claude ",
        provider: "openrouter",
      }),
    ).toEqual({
      base_url: "https://openrouter.ai/api/v1",
      kind: "remote",
      api_key: "sk-typed",
      model: "anthropic/claude",
      provider: "openrouter",
    });
    // Nothing typed: the daemon falls back to the saved key and model.
    expect(
      aiTestConnectionArgs({
        baseUrl: "http://127.0.0.1:1234/v1",
        kind: "local",
        apiKey: "",
        model: "",
      }),
    ).toEqual({ base_url: "http://127.0.0.1:1234/v1", kind: "local" });
  });

  it("keeps the detected format for the row to save", () => {
    expect(
      aiProviderTestOutcome({
        model_count: 3,
        check_kind: "models",
        detected_api_format: "chat_completions",
        probe: { status: "ok", reason: null, model: "m" },
      }),
    ).toEqual({
      state: "ok",
      modelCount: 3,
      checkKind: "models",
      detectedApiFormat: "chat_completions",
    });
    expect(
      aiProviderTestOutcome({ model_count: 1, detected_api_format: "made-up" as never })
        .state,
    ).toBe("ok");
  });

  it("does not call a model listing alone a success", () => {
    expect(
      aiProviderTestOutcome({
        model_count: 12,
        probe: { status: "skipped", reason: "no_model", model: null },
      }),
    ).toEqual({ state: "no_model", modelCount: 12 });
    // A CLI agent has no reply test; its check stays a success.
    expect(
      aiProviderTestOutcome({
        model_count: 2,
        check_kind: "binary_presence",
        probe: { status: "skipped", reason: "cli_provider", model: null },
      }).state,
    ).toBe("ok");
  });
});
