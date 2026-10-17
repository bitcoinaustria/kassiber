import { describe, expect, it } from "vitest";

import { aiProviderTestOutcome, aiTestConnectionArgs } from "./aiProviderTest";

describe("provider form Test connection", () => {
  it("sends the form's model and format, so the reply test asks them", () => {
    expect(
      aiTestConnectionArgs({
        baseUrl: " https://openrouter.ai/api/v1 ",
        kind: "remote",
        apiKey: " sk-typed ",
        model: " anthropic/claude ",
        provider: "openrouter",
        apiFormat: "chat_completions",
        omitStoreField: false,
        acknowledged: true,
      }),
    ).toEqual({
      base_url: "https://openrouter.ai/api/v1",
      kind: "remote",
      api_key: "sk-typed",
      model: "anthropic/claude",
      provider: "openrouter",
      api_format: "chat_completions",
      omit_store_field: false,
      acknowledged: true,
    });
    // Nothing typed: the daemon falls back to the saved key, model and format.
    expect(
      aiTestConnectionArgs({
        baseUrl: "http://127.0.0.1:1234/v1",
        kind: "local",
        apiKey: "",
        model: "",
        acknowledged: false,
      }),
    ).toEqual({ base_url: "http://127.0.0.1:1234/v1", kind: "local" });
  });

  it("keeps the detected format for the row to save, and what was tested", () => {
    expect(
      aiProviderTestOutcome({
        model_count: 3,
        check_kind: "models",
        models: [{ id: "m" }],
        api_format: "auto",
        detected_api_format: "chat_completions",
        tools_accepted: false,
        store_accepted: false,
        probe: { status: "ok", reason: null, model: "m", finish_reason: "stop" },
      }),
    ).toEqual({
      state: "ok",
      modelCount: 3,
      checkKind: "models",
      detectedApiFormat: "chat_completions",
      testedApiFormat: "auto",
      toolsAccepted: false,
      storeAccepted: false,
      probeModel: "m",
      models: [{ id: "m" }],
    });
    const odd = aiProviderTestOutcome({
      model_count: 1,
      api_format: "made-up" as never,
      detected_api_format: "made-up" as never,
    });
    expect(odd).toMatchObject({
      state: "ok",
      detectedApiFormat: null,
      testedApiFormat: null,
      toolsAccepted: null,
      storeAccepted: null,
      probeModel: null,
    });
  });

  it("does not call a model listing alone a success", () => {
    expect(
      aiProviderTestOutcome({
        model_count: 12,
        models: [{ id: "a" }],
        probe: { status: "skipped", reason: "no_model", model: null },
      }),
    ).toEqual({ state: "no_model", modelCount: 12, models: [{ id: "a" }] });
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
