// @vitest-environment happy-dom
//
// The model picker's docked traits (after T3 Code's composer picker): the
// reasoning column lists only what the selected model advertises, the arrow
// keys move between the list and the column, picking a model keeps the picker
// open on that model's levels, and the footer carries the permission mode.
// None of it contacts a provider.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useUiStore } from "@/store/ui";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

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

const PROVIDERS = [
  { name: "codex", display_name: "Codex", base_url: "codex-cli://default", kind: "remote", default_model: "gpt-5.4", acknowledged_at: "2026-01-01T00:00:00Z", has_api_key: false, is_default: true },
];

vi.mock("@/daemon/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/client")>();
  return {
    ...actual,
    useDaemon: (kind: string) => {
      if (kind !== "ai.providers.list") throw new Error(`unexpected daemon read ${kind}`);
      return {
        data: { kind, data: { default: "codex", providers: PROVIDERS } },
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

function mount(
  value: { provider: string; model: string },
  props: Partial<React.ComponentProps<typeof ProviderModelPicker>> = {},
) {
  const client = new QueryClient();
  const { dataMode, daemonSession } = useUiStore.getState();
  // An earlier explicit Check models for Codex, already in the cache.
  client.setQueryData(
    providerModelsQueryKey(dataMode, daemonSession, {
      name: "codex",
      base_url: "codex-cli://default",
    }),
    {
      kind: "ai.list_models",
      schema_version: 1,
      data: {
        provider: "codex",
        models: [
          { id: "gpt-5.4", display_name: "GPT-5.4", reasoning_efforts: ["low", "medium", "high"] },
          { id: "gpt-5.4-mini", reasoning_efforts: ["low", "high", "minimal"] },
        ],
      },
    },
  );
  const onChange = vi.fn();
  const onOpenChange = vi.fn();
  const onThinkingEffortChange = vi.fn();
  const onPermissionModeChange = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ProviderModelPicker
        value={value}
        onChange={onChange}
        open
        onOpenChange={onOpenChange}
        thinkingEffort="auto"
        onThinkingEffortChange={onThinkingEffortChange}
        showThinkingEffort
        permissionMode="auto"
        onPermissionModeChange={onPermissionModeChange}
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onChange, onOpenChange, onThinkingEffortChange, onPermissionModeChange };
}

function column(): HTMLElement | null {
  return document.querySelector<HTMLElement>("[data-model-picker-effort]");
}

function levels(): string[] {
  return within(column()!)
    .getAllByRole("radio")
    .map((row) => row.textContent?.replace(/Default$/, "").trim() ?? "");
}

function checkedLevel(): HTMLElement {
  return within(column()!)
    .getAllByRole("radio")
    .find((row) => row.getAttribute("aria-checked") === "true")!;
}

async function nextFrame() {
  await act(async () => {
    await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));
  });
}

describe("ProviderModelPicker docked traits", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    useUiStore.setState({ assistantModelFavorites: [] });
  });
  afterEach(cleanup);

  it("lists Auto and the model's advertised levels under its name", () => {
    const { onThinkingEffortChange } = mount({ provider: "codex", model: "gpt-5.4" });
    expect(column()!.textContent).toContain("Reasoning");
    expect(column()!.textContent).toContain("GPT-5.4");
    expect(levels()).toEqual(["Auto", "Low", "Medium", "High"]);
    expect(checkedLevel().textContent).toContain("Auto");

    fireEvent.click(within(column()!).getByRole("radio", { name: "High" }));
    expect(onThinkingEffortChange).toHaveBeenCalledWith("high");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("never offers a level the model does not advertise as one Kassiber knows", () => {
    mount({ provider: "codex", model: "gpt-5.4-mini" });
    expect(levels()).toEqual(["Auto", "Low", "High"]);
  });

  it("hides the column for a model without reasoning effort", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { showThinkingEffort: false });
    expect(column()).toBeNull();
  });

  it("keeps a stored level for a model whose levels are not known yet", () => {
    const { onThinkingEffortChange } = mount(
      { provider: "codex", model: "gpt-5.5" },
      { thinkingEffort: "high" },
    );
    expect(levels()).toEqual(["Auto", "High"]);
    expect(checkedLevel().textContent).toContain("High");
    expect(column()!.textContent).toContain("Check models to list this model's levels.");
    expect(onThinkingEffortChange).not.toHaveBeenCalled();
  });

  it("drops a level the newly selected model does not offer back to Auto", () => {
    const { onThinkingEffortChange } = mount(
      { provider: "codex", model: "gpt-5.4-mini" },
      { thinkingEffort: "medium" },
    );
    expect(onThinkingEffortChange).toHaveBeenCalledWith("auto");
  });

  it("locks the levels while a reply streams", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { onThinkingEffortChange: undefined });
    for (const row of within(column()!).getAllByRole("radio")) {
      expect((row as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("moves between the search field and the levels with the arrow keys", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { thinkingEffort: "medium" });
    const search = screen.getByRole("combobox");
    expect(document.activeElement).toBe(search);

    fireEvent.keyDown(search, { key: "ArrowRight" });
    expect(document.activeElement).toBe(checkedLevel());
    expect(checkedLevel().textContent).toContain("Medium");

    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement?.textContent).toContain("High");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(document.activeElement?.textContent).toContain("Low");

    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(search);
  });

  it("leaves ArrowRight to the text while the caret is inside the query", () => {
    mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox") as HTMLInputElement;
    fireEvent.change(search, { target: { value: "gpt" } });
    search.setSelectionRange(1, 1);
    fireEvent.keyDown(search, { key: "ArrowRight" });
    expect(document.activeElement).toBe(search);
  });

  it("picks the highlighted model with ArrowRight and stays open on its levels", async () => {
    const { onChange, onOpenChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await act(async () => {
      fireEvent.keyDown(search, { key: "ArrowRight" });
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4-mini" });
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    await nextFrame();
    expect(column()!.contains(document.activeElement)).toBe(true);
  });

  it("does not switch models with ArrowRight on a highlight the mouse left behind", async () => {
    const { onChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox");
    const other = screen.getByText("gpt-5.4-mini").closest("[data-model-picker-row], [role=option], li, button")!;
    fireEvent.mouseMove(other);
    await act(async () => {
      fireEvent.keyDown(search, { key: "ArrowRight" });
    });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("keeps the picker open after Enter picks a model", async () => {
    const { onChange, onOpenChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4-mini" });
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it("closes after a pick when no trait is docked", async () => {
    const { onChange, onOpenChange } = mount(
      { provider: "codex", model: "gpt-5.4" },
      { showThinkingEffort: false, onPermissionModeChange: undefined },
    );
    expect(document.querySelector("[data-model-picker-footer]")).toBeNull();
    const search = screen.getByRole("combobox");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(onChange).toHaveBeenCalledOnce();
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("picks the permission mode from the footer, each mode described", async () => {
    const { onPermissionModeChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const mode = document.querySelector<HTMLElement>('[data-composer-option="permission-mode"]')!;
    expect(mode.getAttribute("aria-label")).toBe("Permission mode: Auto");
    await act(async () => {
      fireEvent.pointerDown(mode, { button: 0, pointerType: "mouse" });
    });
    const options = screen.getAllByRole("menuitemradio");
    expect(options.map((option) => option.textContent)).toEqual([
      expect.stringContaining("Ask before every change."),
      expect.stringContaining("Ask before network access and anything permanent."),
      expect.stringContaining("syncs and other network actions included"),
    ]);
    expect(options[1]!.getAttribute("aria-checked")).toBe("true");
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitemradio", { name: /Full access/ }));
    });
    expect(onPermissionModeChange).toHaveBeenCalledWith("full_access");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("keeps Full access in sight on the trigger and in a warning tone", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { permissionMode: "full_access", open: false });
    const trigger = document.querySelector<HTMLElement>("[data-chat-provider-model-picker]")!;
    expect(trigger.querySelector("[data-full-access-indicator]")).not.toBeNull();
    expect(trigger.getAttribute("aria-label")).toContain("Permission mode: Full access");
    cleanup();

    mount({ provider: "codex", model: "gpt-5.4" }, { permissionMode: "full_access" });
    const mode = document.querySelector<HTMLElement>('[data-composer-option="permission-mode"]')!;
    expect(mode.className).toContain("text-amber");
  });

  it("leaves the permission mode out when the host does not offer it", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { onPermissionModeChange: undefined });
    expect(document.querySelector('[data-composer-option="permission-mode"]')).toBeNull();
  });

  it("names the chosen level on the trigger", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { thinkingEffort: "high", open: false });
    const trigger = document.querySelector<HTMLElement>("[data-chat-provider-model-picker]")!;
    expect(trigger.querySelector("[data-composer-traits]")?.textContent).toBe("High");
    expect(trigger.getAttribute("aria-label")).toContain("Reasoning effort: High");
  });
});
