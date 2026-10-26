import { describe, expect, it } from "vitest";

import {
  dedupeProviderRows,
  filterModelsByPrivacy,
  httpModelRunsAssistant,
  modelIdNamesNonChatModel,
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

describe("which listed HTTP models can run the assistant", () => {
  it("hides only ids with a clear non-chat mark when a listing says nothing", () => {
    // OpenAI's /models: no tool or modality metadata at all.
    const nonChat = [
      "text-embedding-3-small",
      "text-embedding-ada-002",
      "nomic-embed-text:latest",
      "tts-1",
      "tts-1-hd",
      "gpt-4o-mini-tts",
      "whisper-1",
      "dall-e-3",
      "gpt-image-1",
      "omni-moderation-latest",
      "gpt-4o-transcribe",
      "gpt-4o-mini-transcribe",
      "gpt-realtime",
      "gpt-4o-realtime-preview",
      "gpt-audio",
      "gpt-4o-mini-audio-preview",
      "GPT-4o-Audio-Preview",
    ];
    const chat = [
      "gpt-5",
      "gpt-5-mini",
      "gpt-5-codex",
      "gpt-4.1",
      "gpt-4o",
      "chatgpt-4o-latest",
      "o3",
      "o4-mini",
      "o1-pro",
      "gpt-4o-search-preview",
      "gpt-3.5-turbo",
      "codex-mini-latest",
      "gpt-oss-120b",
      "qwen3:8b",
      "llama3.3:70b",
      "deepseek-r1:32b",
      // Marks inside a longer word are not marks.
      "watts-chat",
      "audiocraft-instruct",
    ];
    for (const id of nonChat) {
      expect(modelIdNamesNonChatModel(id), id).toBe(true);
      expect(httpModelRunsAssistant({ id }), id).toBe(false);
      expect(httpModelRunsAssistant({ id, supports_tools: null }), id).toBe(false);
    }
    for (const id of chat) {
      expect(modelIdNamesNonChatModel(id), id).toBe(false);
      expect(httpModelRunsAssistant({ id }), id).toBe(true);
    }
  });

  it("follows the listing where it says what a model takes and gives", () => {
    // OpenRouter-style rows.
    expect(httpModelRunsAssistant({ id: "vendor/chat", supports_tools: false })).toBe(false);
    expect(
      httpModelRunsAssistant({ id: "vendor/painter", supports_tools: null, output_modalities: ["image"] }),
    ).toBe(false);
    expect(
      httpModelRunsAssistant({ id: "vendor/painter", supports_tools: true, output_modalities: ["image"] }),
    ).toBe(false);
    expect(
      httpModelRunsAssistant({
        id: "openai/gpt-4o-audio-preview",
        supports_tools: true,
        output_modalities: ["text", "audio"],
      }),
    ).toBe(true);
    // Known metadata wins over the name.
    expect(httpModelRunsAssistant({ id: "vendor/embed-chat", supports_tools: true })).toBe(true);
    expect(
      httpModelRunsAssistant({ id: "gpt-audio", supports_tools: null, output_modalities: ["text"] }),
    ).toBe(true);
    expect(httpModelRunsAssistant({ id: "vendor/x", output_modalities: [] })).toBe(true);
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
    expect(providerIconKey(row("gemini", "gemini-cli://default"))).toBeNull();
    expect(providerIconKey(row("ollama", "http://localhost:11434/v1"))).toBe("ollama");
    expect(providerIconKey(row("omlx", "http://127.0.0.1:8000/v1"))).toBe("omlx");
    expect(providerIconKey({ ...row("my-box", "http://10.0.0.5:11434/v1"), display_name: "Ollama (studio)" })).toBe("ollama");
    expect(providerIconKey(row("copilot", "Copilot-CLI://default"))).toBe("copilot");
  });

  it("falls back to initials for providers without a mark", () => {
    expect(providerInitials("Ollama")).toBe("OL");
    expect(providerInitials("GitHub Copilot")).toBe("GC");
    expect(providerInitials("Gemini CLI")).toBe("GE");
    expect(providerInitials("gemini-cli")).toBe("GE");
    expect(providerInitials("CLI")).toBe("CL");
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
