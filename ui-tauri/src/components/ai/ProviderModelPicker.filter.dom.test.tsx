// @vitest-environment happy-dom
//
// The picker hides an HTTP provider's models that cannot run the assistant,
// from the cached listing alone: no tools, no text output, or (where the
// listing says nothing, as OpenAI's does) an id that names an embedding,
// speech, image or moderation model. Show all models undoes it; the selected,
// default and favorite models always stay; nothing here contacts a provider.
// The effort menu offers an HTTP model only the levels its listing names.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
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
  props: Partial<React.ComponentProps<typeof ProviderModelPicker>> = {},
) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  seed(client, PROVIDERS[0], models.openai ?? OPENAI_MODELS);
  seed(client, PROVIDERS[1], models.openrouter ?? OPENROUTER_MODELS);
  if (models.codex) seed(client, PROVIDERS[2], models.codex);
  const onChange = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ProviderModelPicker value={value} onChange={onChange} open {...props} />
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

  it("picks nothing when no listed model can run the assistant, and says so", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "openai" ? { ...provider, default_model: null } : provider,
    );
    const { onChange } = mount(null, {
      openai: [{ id: "text-embedding-3-small" }, { id: "tts-1" }, { id: "whisper-1" }],
    });
    // No unusable model is chosen for the composer on its own.
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "OpenAI · remote" }));
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("No model from this provider can run the assistant.")).toBeTruthy();
    expect(screen.getByText("Use Show all models to pick one anyway.")).toBeTruthy();
    expect(screen.queryByText("No models found")).toBeNull();

    // Still possible to choose one deliberately.
    fireEvent.click(showAll());
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(screen.queryByText("No model from this provider can run the assistant.")).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole("option", { name: /whisper-1/ }));
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "openai", model: "whisper-1" });
    expect(mocks.invoke).not.toHaveBeenCalled();
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

describe("ProviderModelPicker effort menu for HTTP models", () => {
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

  async function effortLevels(
    model: Record<string, unknown>,
    thinkingEffort: "auto" | "high" = "auto",
  ): Promise<{ levels: string[]; onThinkingEffortChange: ReturnType<typeof vi.fn> }> {
    const onThinkingEffortChange = vi.fn();
    mount(
      { provider: "openrouter", model: "vendor/reasoner" },
      { openrouter: [{ id: "vendor/reasoner", supports_tools: true, ...model }] },
      { open: false, showThinkingEffort: true, thinkingEffort, onThinkingEffortChange },
    );
    await act(async () => {
      fireEvent.pointerDown(screen.getByRole("button", { name: /Reasoning effort/ }), {
        button: 0,
        pointerType: "mouse",
      });
    });
    const levels = screen
      .getAllByRole("menuitemradio")
      .map((item) => item.textContent?.replace(/Default.*$/, "").trim() ?? "");
    return { levels, onThinkingEffortChange };
  }

  it("offers the levels OpenRouter's listing names", async () => {
    const { levels } = await effortLevels({
      supports_reasoning_effort: true,
      reasoning_efforts: ["medium", "high"],
      supported_parameters: ["tools", "reasoning", "include_reasoning"],
    });
    expect(levels).toEqual(["Auto", "Medium", "High"]);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("offers no level above high, even where the listing names one", async () => {
    const { levels } = await effortLevels({
      supports_reasoning_effort: true,
      reasoning_efforts: ["low", "high"],
      capabilities: { reasoning_efforts: ["xhigh", "max"] },
    });
    expect(levels).toEqual(["Auto", "Low", "High"]);
    const unnamed = await (async () => {
      cleanup();
      return effortLevels({ supports_reasoning_effort: true });
    })();
    expect(unnamed.levels).toEqual(["Auto", "Low", "Medium", "High"]);
  });
});
