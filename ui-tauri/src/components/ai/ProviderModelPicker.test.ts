import { describe, expect, it } from "vitest";

import {
  dedupeProviderRows,
  filterModelsByPrivacy,
  filterModelRows,
  modelPrivacyPosture,
  providerRuntimeSelectable,
  providerRuntimeTone,
  sortModelRowsByPosture,
} from "./providerModelSearch";
import {
  ClaudeIcon,
  OpenAIIcon,
  OpenCodeIcon,
  PROVIDER_BRAND_ICON_BY_RUNTIME,
} from "./providerBrandIcons";

describe("provider brand icons", () => {
  it("matches each native runtime to the corresponding T3-style mark", () => {
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.codex).toBe(OpenAIIcon);
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.claude).toBe(ClaudeIcon);
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.opencode).toBe(OpenCodeIcon);
  });
});

describe("filterModelRows", () => {
  const models = [
    { id: "gpt-5.4", display_name: "GPT 5.4", owned_by: "OpenAI" },
    { id: "claude-opus-4-7", display_name: "Claude Opus 4.7", owned_by: "Anthropic" },
  ];

  it("matches all search tokens across model metadata", () => {
    expect(filterModelRows(models, "claude anthropic")).toEqual([models[1]]);
    expect(filterModelRows(models, "openai 5.4")).toEqual([models[0]]);
    expect(filterModelRows(models, "missing")).toEqual([]);
  });
});

describe("sortModelRowsByPosture", () => {
  const provider = {
    name: "mixed",
    kind: "remote",
    base_url: "https://example.invalid",
  } as Parameters<typeof sortModelRowsByPosture>[0];

  it("puts on-device models above TEE above off-device, order preserved within", () => {
    const models = [
      { id: "cloud-b", privacy_posture: "remote" as const },
      { id: "on-device-a", privacy_posture: "local" as const },
      { id: "cloud-a", privacy_posture: "remote" as const },
      { id: "enclave", privacy_posture: "tee" as const },
      { id: "on-device-b", privacy_posture: "local" as const },
    ];

    expect(sortModelRowsByPosture(provider, models).map((m) => m.id)).toEqual([
      "on-device-a",
      "on-device-b",
      "enclave",
      // Input order inside a posture group is kept: cloud-b came first.
      "cloud-b",
      "cloud-a",
    ]);
  });

  it("falls back to the provider kind when a model declares no posture", () => {
    const models = [{ id: "default" }, { id: "on-device", privacy_posture: "local" as const }];

    expect(sortModelRowsByPosture(provider, models).map((m) => m.id)).toEqual([
      "on-device",
      "default",
    ]);
  });
});

describe("dedupeProviderRows", () => {
  const base = {
    kind: "remote" as const,
    has_api_key: false,
    is_default: false,
  };
  const providers = [
    { ...base, name: "codex", display_name: "Codex", base_url: "codex-cli://default" },
    {
      ...base,
      name: "codex-cli",
      display_name: "Codex CLI",
      base_url: "codex-cli://default",
    },
    {
      ...base,
      name: "custom-http",
      display_name: "Custom",
      base_url: "https://example.test/v1",
    },
  ];

  it("prefers the canonical native provider over a legacy duplicate", () => {
    expect(dedupeProviderRows(providers).map((provider) => provider.name)).toEqual([
      "codex",
      "custom-http",
    ]);
  });

  it("preserves an actively selected legacy provider without showing both", () => {
    expect(
      dedupeProviderRows(providers, "codex-cli").map((provider) => provider.name),
    ).toEqual(["codex-cli", "custom-http"]);
  });
});

describe("provider runtime selector state", () => {
  const runtime = (state: "ready" | "authentication_required" | "error") => ({
    provider: "codex" as const,
    display_name: "Codex",
    state,
    message: state,
    privacy_posture: "remote" as const,
    native_tools: "disabled" as const,
    models: [],
  });

  it("allows switching to ready providers and disables unavailable ones", () => {
    expect(providerRuntimeSelectable(runtime("ready"))).toBe(true);
    expect(providerRuntimeSelectable(runtime("authentication_required"))).toBe(false);
    expect(providerRuntimeSelectable(runtime("error"))).toBe(false);
  });

  it("maps readiness to stable status tones", () => {
    expect(providerRuntimeTone(runtime("ready"))).toBe("ready");
    expect(providerRuntimeTone(runtime("authentication_required"))).toBe("attention");
    expect(providerRuntimeTone(runtime("error"))).toBe("unavailable");
  });
});

describe("model privacy filtering", () => {
  const provider = {
    name: "opencode",
    base_url: "opencode-cli://default",
    kind: "remote" as const,
    has_api_key: false,
    is_default: false,
  };
  const models = [
    { id: "omlx/local-model", source_provider: "omlx" },
    {
      id: "future/proven-local",
      source_provider: "future",
      privacy_posture: "local" as const,
    },
  ];

  it("keeps unverified OpenCode sources remote and filters only proven-local rows", () => {
    expect(modelPrivacyPosture(provider, models[0])).toBe("remote");
    expect(filterModelsByPrivacy(provider, models, true)).toEqual([models[1]]);
    expect(filterModelsByPrivacy(provider, models, false)).toEqual(models);
  });
});
