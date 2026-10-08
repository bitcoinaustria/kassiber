import { describe, expect, it } from "vitest";

import {
  dedupeProviderRows,
  filterModelsByPrivacy,
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
import { providerIconKey, providerInitials } from "./providerIdentity";
import { initialModelPickerView } from "./modelPickerKeys";

describe("provider brand icons", () => {
  it("matches each native runtime to the corresponding T3-style mark", () => {
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.codex).toBe(OpenAIIcon);
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.claude).toBe(ClaudeIcon);
    expect(PROVIDER_BRAND_ICON_BY_RUNTIME.opencode).toBe(OpenCodeIcon);
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

describe("provider identity", () => {
  const row = (name: string, base_url: string) => ({
    name,
    base_url,
    kind: "remote" as const,
    has_api_key: false,
    is_default: false,
  });

  it("keys brand marks by CLI runtime, including the ACP agents", () => {
    expect(providerIconKey(row("codex", "codex-cli://default"))).toBe("codex");
    expect(providerIconKey(row("gemini", "gemini-cli://default"))).toBe("gemini");
    expect(providerIconKey(row("copilot", "Copilot-CLI://default"))).toBe("copilot");
    expect(providerIconKey(row("ollama", "http://127.0.0.1:11434/v1"))).toBeNull();
  });

  it("falls back to initials for providers without a mark", () => {
    expect(providerInitials("Ollama")).toBe("OL");
    expect(providerInitials("GitHub Copilot")).toBe("GC");
    expect(providerInitials("gemini-cli")).toBe("GC");
    expect(providerInitials("  ")).toBe("?");
  });
});

describe("initialModelPickerView", () => {
  const groups = [
    { provider: { name: "ollama", is_default: true } },
    { provider: { name: "codex", is_default: false } },
  ];

  it("opens on the current model's provider", () => {
    expect(
      initialModelPickerView({
        groups,
        value: { provider: "codex", model: "gpt-5.4" },
        favorites: [],
      }),
    ).toEqual({ provider: "codex" });
  });

  it("opens on Favorites when the current model is starred", () => {
    expect(
      initialModelPickerView({
        groups,
        value: { provider: "codex", model: "gpt-5.4" },
        favorites: [{ provider: "codex", model: "gpt-5.4" }],
      }),
    ).toBe("favorites");
  });

  it("falls back to the default provider, then Favorites", () => {
    expect(initialModelPickerView({ groups, value: null, favorites: [] })).toEqual({
      provider: "ollama",
    });
    expect(initialModelPickerView({ groups: [], value: null, favorites: [] })).toBe(
      "favorites",
    );
  });
});
