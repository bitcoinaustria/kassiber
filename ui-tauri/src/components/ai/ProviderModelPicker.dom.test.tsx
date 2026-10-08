// @vitest-environment happy-dom
//
// Mounted, not static: the picker is keyboard-driven, and its highlight,
// search, rail switching, favorites and shortcuts only exist in a live DOM.
// Every test also pins the privacy contract: none of those interactions may
// ask a provider for its models; only the explicit Check models button can.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useUiStore } from "@/store/ui";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  acknowledge: vi.fn(),
  providers: [] as Array<Record<string, unknown>>,
}));

vi.hoisted(() => {
  // The UI store (favorites) persists through `localStorage`, which this DOM
  // does not wire up as a global; zustand resolves it at module evaluation.
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
  // Radix positions popovers with a ResizeObserver this DOM does not ship.
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
      if (kind !== "ai.providers.list") {
        throw new Error(`unexpected daemon read ${kind}`);
      }
      return {
        data: {
          kind,
          data: { default: "ollama", providers: mocks.providers },
        },
        refetch: vi.fn(async () => undefined),
      };
    },
    useDaemonMutation: () => ({
      error: null,
      isPending: false,
      mutateAsync: mocks.acknowledge,
      reset: vi.fn(),
    }),
  };
});

import { ProviderModelPicker } from "./ProviderModelPicker";

const PROVIDERS = [
  {
    name: "ollama",
    display_name: "Ollama",
    base_url: "http://127.0.0.1:11434/v1",
    kind: "local",
    default_model: "qwen3:8b",
    has_api_key: false,
    is_default: true,
  },
  {
    name: "codex",
    display_name: "Codex",
    base_url: "codex-cli://default",
    kind: "remote",
    default_model: "gpt-5.4",
    acknowledged_at: "2026-01-01T00:00:00Z",
    has_api_key: false,
    is_default: false,
  },
  {
    name: "gemini",
    display_name: "Gemini API",
    base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    kind: "remote",
    default_model: "gemini-2.5-pro",
    acknowledged_at: null,
    has_api_key: false,
    is_default: false,
  },
];

function mount(
  props: Partial<React.ComponentProps<typeof ProviderModelPicker>> = {},
) {
  const onChange = vi.fn();
  const onOpenChange = vi.fn();
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={client}>
      <ProviderModelPicker
        value={{ provider: "ollama", model: "qwen3:8b" }}
        onChange={onChange}
        open
        onOpenChange={onOpenChange}
        {...props}
      />
    </QueryClientProvider>,
  );
  return {
    ...view,
    onChange,
    onOpenChange,
    get search() {
      return screen.getByRole("combobox");
    },
  };
}

function highlighted(): string | undefined {
  return screen
    .getAllByRole("option")
    .find((option) => option.getAttribute("aria-selected") === "true")
    ?.textContent ?? undefined;
}

function optionNames(): string[] {
  return screen.getAllByRole("option").map((option) => option.textContent ?? "");
}

describe("ProviderModelPicker (mounted)", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.acknowledge.mockReset();
    mocks.providers = PROVIDERS.map((provider) => ({ ...provider }));
    useUiStore.setState({ assistantModelFavorites: [] });
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("opens on the current provider with the search field focused, without discovery", () => {
    const { search } = mount();
    expect(document.activeElement).toBe(search);
    expect(optionNames()).toHaveLength(1);
    expect(highlighted()).toContain("qwen3:8b");
    expect(highlighted()).toContain("local");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("keeps the provider name and posture on the trigger", () => {
    mount({ open: false, value: { provider: "codex", model: "gpt-5.4" } });
    const trigger = screen.getByRole("button", { name: /Models: Codex · gpt-5.4/ });
    expect(trigger.textContent).toContain("Codex · gpt-5.4");
    expect(within(trigger).getByText("remote")).toBeTruthy();
    expect(trigger.getAttribute("aria-label")).toContain("Prompts leave this device");
  });

  it("switches providers from the rail and the keyboard without discovery", () => {
    const { search } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Codex · remote" }));
    expect(optionNames()[0]).toContain("gpt-5.4");

    fireEvent.keyDown(search, { key: "ArrowDown", ctrlKey: true, shiftKey: true });
    expect(optionNames()[0]).toContain("gemini-2.5-pro");

    fireEvent.keyDown(search, { key: "ArrowDown", ctrlKey: true, shiftKey: true });
    // Wraps to Favorites, which is empty until something is starred.
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("No favorites yet")).toBeTruthy();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("searches every provider's known models at once, without discovery", () => {
    const { search } = mount();
    fireEvent.change(search, { target: { value: "gem" } });
    expect(optionNames()).toHaveLength(1);
    expect(optionNames()[0]).toContain("gemini-2.5-pro");
    // Results name the provider that would receive the prompt.
    expect(optionNames()[0]).toContain("Gemini API");
    // The rail steps aside while searching.
    expect(screen.queryByRole("toolbar")).toBeNull();

    fireEvent.change(search, { target: { value: "zzzz" } });
    expect(screen.getByText("No matching models")).toBeTruthy();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("picks the highlighted model with the arrows and Enter", async () => {
    const { search, onChange } = mount();
    fireEvent.change(search, { target: { value: "codex" } });
    fireEvent.keyDown(search, { key: "ArrowDown" });
    // A single result wraps back onto itself.
    expect(highlighted()).toContain("gpt-5.4");
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4" });
  });

  it("asks for the off-device acknowledgement before the first remote pick", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const { search, onChange } = mount();
    fireEvent.change(search, { target: { value: "gemini" } });
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.acknowledge).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(mocks.acknowledge).toHaveBeenCalledWith({ name: "gemini" });
    expect(onChange).toHaveBeenCalledWith({
      provider: "gemini",
      model: "gemini-2.5-pro",
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("jumps to the nth row with Mod+digit", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const { search, onChange } = mount();
    fireEvent.change(search, { target: { value: "e" } });
    const second = optionNames()[1];
    expect(second).toBeDefined();
    await act(async () => {
      fireEvent.keyDown(search, { key: "2", ctrlKey: true });
    });
    expect(onChange).toHaveBeenCalledOnce();
    const picked = onChange.mock.calls[0][0] as { model: string };
    expect(second).toContain(picked.model);
  });

  it("stars a model into Favorites and opens there when it is in use", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Add to favorites" }));
    expect(useUiStore.getState().assistantModelFavorites).toEqual([
      { provider: "ollama", model: "qwen3:8b" },
    ]);
    cleanup();

    mount();
    expect(
      screen.getByRole("button", { name: "Favorites" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(optionNames()[0]).toContain("qwen3:8b");
    expect(optionNames()[0]).toContain("Ollama");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("filters to proven-local models", () => {
    const { search } = mount();
    fireEvent.click(screen.getByRole("button", { name: "Local" }));
    // Remote providers leave the rail; only Ollama is left beside Favorites.
    expect(
      screen.getAllByRole("button", { name: /· (remote|local)$/ }).map(
        (button) => button.getAttribute("aria-label"),
      ),
    ).toEqual(["Ollama · local"]);
    fireEvent.change(search, { target: { value: "gpt" } });
    expect(screen.getByText("No matching models")).toBeTruthy();
  });

  it("checks only the active provider after the explicit click", async () => {
    mocks.invoke.mockResolvedValue({
      kind: "ai.list_models",
      data: {
        provider: "ollama",
        models: [{ id: "qwen3:8b" }, { id: "llama3.3:70b" }],
      },
    });
    mount();
    expect(mocks.invoke).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Check models/ }));
    });
    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith({
      kind: "ai.list_models",
      args: { provider: "ollama", refresh: true },
    });
    expect(optionNames().some((name) => name.includes("llama3.3:70b"))).toBe(true);
  });

  it("asks before discovering an unacknowledged remote provider", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    mount();
    fireEvent.click(screen.getByRole("button", { name: "Gemini API · remote" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Check models/ }));
    });
    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("moves focus between the search field and the rail", () => {
    const { search } = mount();
    fireEvent.keyDown(search, { key: "ArrowLeft" });
    const ollama = screen.getByRole("button", { name: "Ollama · local" });
    expect(document.activeElement).toBe(ollama);
    fireEvent.keyDown(ollama, { key: "ArrowRight" });
    expect(document.activeElement).toBe(search);
  });

  describe("IME input", () => {
    it("does not pick a model with an IME-confirming Enter", async () => {
      const { search, onChange } = mount();
      fireEvent.change(search, { target: { value: "codex" } });
      await act(async () => {
        // WebKit reports the confirming Enter as keyCode 229 with
        // isComposing already false.
        fireEvent.keyDown(search, { key: "Enter", keyCode: 229 });
        fireEvent.keyDown(search, { key: "Enter", isComposing: true });
      });
      expect(onChange).not.toHaveBeenCalled();
      await act(async () => {
        fireEvent.keyDown(search, { key: "Enter" });
      });
      expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4" });
    });

    it("ignores picker shortcuts during a composition", async () => {
      const { search, onChange } = mount();
      await act(async () => {
        fireEvent.keyDown(search, { key: "1", ctrlKey: true, keyCode: 229 });
        fireEvent.keyDown(search, {
          key: "ArrowDown",
          ctrlKey: true,
          shiftKey: true,
          isComposing: true,
        });
        fireEvent.keyDown(search, { key: "d", ctrlKey: true, keyCode: 229 });
      });
      expect(onChange).not.toHaveBeenCalled();
      expect(optionNames()[0]).toContain("qwen3:8b");
      expect(useUiStore.getState().assistantModelFavorites).toEqual([]);
    });
  });

  it("stars and unstars the highlighted model with Mod+D", () => {
    const { search } = mount();
    fireEvent.change(search, { target: { value: "codex" } });
    fireEvent.keyDown(search, { key: "d", ctrlKey: true });
    expect(useUiStore.getState().assistantModelFavorites).toEqual([
      { provider: "codex", model: "gpt-5.4" },
    ]);
    expect(
      screen.getByRole("button", { name: "Remove from favorites" }).getAttribute("title"),
    ).toBe("Remove from favorites (Ctrl+D)");
    fireEvent.keyDown(search, { key: "d", metaKey: true });
    expect(useUiStore.getState().assistantModelFavorites).toEqual([]);
    expect(search.getAttribute("aria-describedby")).toBeTruthy();
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("lists every provider's favorites on the active Favorites entry, without discovery", () => {
    useUiStore.setState({
      assistantModelFavorites: [
        { provider: "codex", model: "gpt-5.4" },
        { provider: "ollama", model: "qwen3:8b" },
        // Never returned by a check: known only from the stored identifiers.
        { provider: "gemini", model: "gemini-2.5-flash" },
      ],
    });
    mount();
    expect(
      screen.getByRole("button", { name: "Favorites" }).getAttribute("aria-pressed"),
    ).toBe("true");
    const names = optionNames();
    expect(names).toHaveLength(3);
    expect(
      names.some((name) => name.includes("gemini-2.5-flash") && name.includes("Gemini API")),
    ).toBe(true);
    expect(names.some((name) => name.includes("gpt-5.4") && name.includes("Codex"))).toBe(true);
    expect(names.some((name) => name.includes("qwen3:8b") && name.includes("Ollama"))).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("resolves the opening view once providers arrive after the picker opened", () => {
    mocks.providers = [];
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const tree = () => (
      <QueryClientProvider client={client}>
        <ProviderModelPicker
          value={{ provider: "codex", model: "gpt-5.4" }}
          onChange={vi.fn()}
          open
        />
      </QueryClientProvider>
    );
    const { rerender } = render(tree());
    expect(screen.queryAllByRole("option")).toHaveLength(0);

    mocks.providers = PROVIDERS.map((provider) => ({ ...provider }));
    rerender(tree());
    expect(
      screen.getByRole("button", { name: "Codex · remote" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(optionNames()[0]).toContain("gpt-5.4");
    expect(screen.queryByText("No favorites yet")).toBeNull();

    // An explicit rail choice is kept across later inventory refreshes.
    fireEvent.click(screen.getByRole("button", { name: "Ollama · local" }));
    rerender(tree());
    expect(optionNames()[0]).toContain("qwen3:8b");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
