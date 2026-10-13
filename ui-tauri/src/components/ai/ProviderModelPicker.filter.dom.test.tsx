// @vitest-environment happy-dom
//
// The picker hides an HTTP provider's models that cannot run the assistant,
// from the cached listing alone: no tools, no text output, or (where the
// listing says nothing, as OpenAI's does) an id that names an embedding,
// speech, image or moderation model. Show all models undoes it; the selected,
// default and favorite models always stay; nothing here contacts a provider.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useUiStore } from "@/store/ui";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  providers: [] as Array<Record<string, unknown>>,
  defaultProvider: "openai",
}));

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke: mocks.invoke }),
}));

vi.mock("@/daemon/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/client")>();
  return {
    ...actual,
    useDaemon: (kind: string) => {
      if (kind !== "ai.providers.list") throw new Error(`unexpected daemon read ${kind}`);
      return {
        data: { kind, data: { default: mocks.defaultProvider, providers: mocks.providers } },
        refetch: vi.fn(async () => undefined),
      };
    },
    useDaemonMutation: () => ({
      error: null,
      isPending: false,
      mutateAsync: vi.fn(),
      reset: vi.fn(),
    }),
  };
});

import { ProviderModelPicker } from "./ProviderModelPicker";
import { providerModelsQueryKey } from "./providerModelQuery";

const ACK = "2026-01-01T00:00:00Z";

const PROVIDERS = [
  {
    name: "openai",
    display_name: "OpenAI",
    base_url: "https://api.openai.com/v1",
    kind: "remote",
    default_model: "gpt-5",
    acknowledged_at: ACK,
    has_api_key: true,
    is_default: true,
  },
  {
    name: "openrouter",
    display_name: "OpenRouter",
    base_url: "https://openrouter.ai/api/v1",
    kind: "remote",
    default_model: null,
    acknowledged_at: ACK,
    has_api_key: true,
    is_default: false,
  },
  {
    name: "codex",
    display_name: "Codex",
    base_url: "codex-cli://default",
    kind: "remote",
    default_model: "gpt-5.4",
    acknowledged_at: ACK,
    has_api_key: false,
    is_default: false,
  },
];

// OpenAI's listing says nothing about tools or modalities.
const OPENAI_MODELS = [
  "gpt-5",
  "text-embedding-3-small",
  "tts-1",
  "whisper-1",
  "gpt-4o",
  "dall-e-3",
  "gpt-image-1",
  "omni-moderation-latest",
].map((id) => ({ id, owned_by: "openai", supports_tools: null }));

const OPENROUTER_MODELS = [
  {
    id: "anthropic/claude-sonnet",
    supports_tools: true,
    input_modalities: ["text", "image"],
    output_modalities: ["text"],
  },
  { id: "acme/no-tools", supports_tools: false, output_modalities: ["text"] },
  { id: "acme/painter", supports_tools: null, output_modalities: ["image"] },
  { id: "openai/gpt-4o-audio-preview", supports_tools: true, output_modalities: ["text", "audio"] },
];

function seed(client: QueryClient, provider: (typeof PROVIDERS)[number], models: unknown[]) {
  const { dataMode, daemonSession } = useUiStore.getState();
  client.setQueryData(providerModelsQueryKey(dataMode, daemonSession, provider), {
    kind: "ai.list_models",
    schema_version: 1,
    data: { provider: provider.name, models, checked_at: ACK, stale: false, error: null },
  });
}

function mount(
  value: { provider: string; model: string } | null,
  models: { openai?: unknown[]; openrouter?: unknown[]; codex?: unknown[] } = {},
) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  seed(client, PROVIDERS[0], models.openai ?? OPENAI_MODELS);
  seed(client, PROVIDERS[1], models.openrouter ?? OPENROUTER_MODELS);
  if (models.codex) seed(client, PROVIDERS[2], models.codex);
  const onChange = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ProviderModelPicker value={value} onChange={onChange} open />
    </QueryClientProvider>,
  );
  return { onChange };
}

function listed(id: string): boolean {
  return screen
    .queryAllByRole("option")
    .some((option) => (option.textContent ?? "").includes(id));
}

function filterBar(): HTMLElement | null {
  return document.querySelector<HTMLElement>("[data-model-filter]");
}

function showAll(): HTMLElement {
  return screen.getByRole("button", { name: "Show all models" });
}

describe("ProviderModelPicker usability filter", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.providers = PROVIDERS.map((provider) => ({ ...provider }));
    mocks.defaultProvider = "openai";
    useUiStore.setState({ assistantModelFavorites: [] });
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("hides OpenAI's non-chat models by their names until Show all models", () => {
    mount({ provider: "openai", model: "gpt-5" });
    expect(screen.getAllByRole("option")).toHaveLength(2);
    expect(listed("gpt-5")).toBe(true);
    expect(listed("gpt-4o")).toBe(true);
    for (const id of ["text-embedding-3-small", "tts-1", "whisper-1", "dall-e-3", "gpt-image-1"]) {
      expect(listed(id), id).toBe(false);
    }
    expect(filterBar()?.textContent).toContain(
      "6 models hidden that can't use tools or answer in text.",
    );
    expect(showAll().getAttribute("aria-pressed")).toBe("false");

    fireEvent.click(showAll());
    expect(screen.getAllByRole("option")).toHaveLength(8);
    expect(listed("text-embedding-3-small")).toBe(true);
    expect(filterBar()?.textContent).toContain(
      "Showing 6 models that can't use tools or answer in text.",
    );
    expect(showAll().getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(showAll());
    expect(screen.getAllByRole("option")).toHaveLength(2);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("hides models a listing says take no tools or give no text", () => {
    mount({ provider: "openrouter", model: "anthropic/claude-sonnet" });
    expect(listed("anthropic/claude-sonnet")).toBe(true);
    // Text and audio out, with tools: still an assistant model.
    expect(listed("openai/gpt-4o-audio-preview")).toBe(true);
    expect(listed("acme/no-tools")).toBe(false);
    expect(listed("acme/painter")).toBe(false);
    expect(filterBar()?.textContent).toContain("2 models hidden");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("keeps the selected, default and favorite models listed", () => {
    useUiStore.setState({ assistantModelFavorites: [{ provider: "openai", model: "whisper-1" }] });
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "openai" ? { ...provider, default_model: "tts-1" } : provider,
    );
    mount({ provider: "openai", model: "text-embedding-3-small" });
    expect(
      screen.getByRole("button", { name: "OpenAI · remote" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(listed("text-embedding-3-small")).toBe(true);
    expect(listed("tts-1")).toBe(true);
    expect(listed("whisper-1")).toBe(true);
    expect(listed("dall-e-3")).toBe(false);
    // Only what is actually hidden is counted.
    expect(filterBar()?.textContent).toContain("3 models hidden");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("searches only usable models until Show all models", () => {
    mount({ provider: "openai", model: "gpt-5" });
    const search = screen.getByRole("combobox");
    fireEvent.change(search, { target: { value: "embedding" } });
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    // Across both HTTP providers.
    expect(filterBar()?.textContent).toContain("8 models hidden");
    fireEvent.click(showAll());
    expect(listed("text-embedding-3-small")).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("leaves CLI agents' models alone", () => {
    mount(
      { provider: "codex", model: "gpt-5.4" },
      { codex: [{ id: "gpt-5.4" }, { id: "gpt-realtime-codex" }] },
    );
    expect(listed("gpt-realtime-codex")).toBe(true);
    expect(filterBar()).toBeNull();
  });

  it("starts a provider without a default on a usable model", () => {
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "openai" ? { ...provider, default_model: null } : provider,
    );
    const { onChange } = mount(null, {
      openai: [{ id: "text-embedding-3-small" }, { id: "tts-1" }, { id: "gpt-5-mini" }],
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "gpt-5-mini" });
  });

  it("filters a large listing from the cache alone", () => {
    const many = Array.from({ length: 400 }, (_, index) => ({
      id: `vendor/model-${index}`,
      supports_tools: index % 4 !== 0,
      output_modalities: index % 10 === 1 ? ["image"] : ["text"],
    }));
    mount({ provider: "openrouter", model: "vendor/model-2" }, { openrouter: many });
    const usable = many.filter(
      (model, index) => model.supports_tools && index % 10 !== 1,
    ).length;
    expect(screen.getAllByRole("option")).toHaveLength(usable);
    expect(filterBar()?.textContent).toContain(`${400 - usable} models hidden`);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
