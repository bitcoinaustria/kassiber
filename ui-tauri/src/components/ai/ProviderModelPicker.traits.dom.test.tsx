// @vitest-environment happy-dom
//
// The model picker's docked traits (after T3 Code's composer picker): the
// reasoning column lists only what the selected model advertises, the arrow
// keys move between the list and the column, picking a model keeps the picker
// open on that model's levels, and the footer carries the permission mode.
// None of it contacts a provider.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import * as React from "react";
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
  // The daemon flags every CLI agent as taking an effort (providers.py).
  { name: "codex", display_name: "Codex", base_url: "codex-cli://default", kind: "remote", default_model: "gpt-5.4", acknowledged_at: "2026-01-01T00:00:00Z", has_api_key: false, is_default: true, supports_reasoning_effort: true },
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

import type { AssistantThinkingEffort } from "./assistantSession";
import { ProviderModelPicker } from "./ProviderModelPicker";
import { providerModelsQueryKey } from "./providerModelQuery";
import { useSupportedReasoningEffort } from "./useReasoningEffortSupport";

type PickerProps = React.ComponentProps<typeof ProviderModelPicker>;

/**
 * Holds the selection and level like the composer does, so a pick commits.
 * With `hostEffort` the column shows where the composer would show it.
 */
function Host({
  initial,
  initialEffort,
  hostEffort,
  props,
}: {
  initial: { provider: string; model: string };
  initialEffort: AssistantThinkingEffort;
  hostEffort: boolean;
  props: PickerProps;
}) {
  const [value, setValue] = React.useState<{ provider: string; model: string } | null>(initial);
  const [effort, setEffort] = React.useState(initialEffort);
  const { onChange, onThinkingEffortChange } = props;
  const setThinkingEffort = React.useCallback(
    (next: AssistantThinkingEffort) => {
      onThinkingEffortChange?.(next);
      setEffort(next);
    },
    [onThinkingEffortChange],
  );
  const supported = useSupportedReasoningEffort({
    selection: value,
    thinkingEffort: effort,
    setThinkingEffort,
    enabled: hostEffort,
  });
  return (
    <ProviderModelPicker
      {...props}
      value={value}
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
      thinkingEffort={effort}
      onThinkingEffortChange={onThinkingEffortChange ? setThinkingEffort : undefined}
      showThinkingEffort={hostEffort ? supported : props.showThinkingEffort}
    />
  );
}

function mount(
  value: { provider: string; model: string },
  props: Partial<PickerProps> = {},
  { hostEffort = false }: { hostEffort?: boolean } = {},
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
          // A checked model that takes no effort, as Codex reports one.
          { id: "gpt-5.4-nano", supports_reasoning_effort: false, reasoning_efforts: [] },
        ],
      },
    },
  );
  const onChange = vi.fn();
  const onOpenChange = vi.fn();
  const onThinkingEffortChange = vi.fn();
  const onPermissionModeChange = vi.fn();
  const { thinkingEffort = "auto", ...rest } = props;
  render(
    <QueryClientProvider client={client}>
      <Host
        initial={value}
        initialEffort={thinkingEffort}
        hostEffort={hostEffort}
        props={{
          value,
          onChange,
          open: true,
          onOpenChange,
          onThinkingEffortChange,
          showThinkingEffort: true,
          permissionMode: "auto",
          onPermissionModeChange,
          ...rest,
        }}
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
    expect(column()!.textContent).toContain("Use Check models to see this model's levels.");
    expect(onThinkingEffortChange).not.toHaveBeenCalled();
  });

  it("hides the column for a checked agent model that takes no effort", () => {
    const { onThinkingEffortChange } = mount(
      { provider: "codex", model: "gpt-5.4-nano" },
      { thinkingEffort: "high" },
      { hostEffort: true },
    );
    expect(column()).toBeNull();
    expect(onThinkingEffortChange).toHaveBeenCalledWith("auto");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("drops the Check models hint once the model was checked", () => {
    mount({ provider: "codex", model: "gpt-5.4-nano" });
    expect(levels()).toEqual(["Auto"]);
    expect(column()!.textContent).not.toContain("Check models");
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

  it("moves between the search field and the levels with the arrow keys, choosing as it goes", () => {
    const { onThinkingEffortChange } = mount(
      { provider: "codex", model: "gpt-5.4" },
      { thinkingEffort: "medium" },
    );
    const search = screen.getByRole("combobox");
    expect(document.activeElement).toBe(search);

    fireEvent.keyDown(search, { key: "ArrowRight" });
    expect(document.activeElement).toBe(checkedLevel());
    expect(checkedLevel().textContent).toContain("Medium");

    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(checkedLevel());
    expect(checkedLevel().textContent).toContain("High");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    expect(checkedLevel().textContent).toContain("Low");
    expect(document.activeElement).toBe(checkedLevel());
    expect(onThinkingEffortChange.mock.calls.map(([level]) => level)).toEqual([
      "high",
      "medium",
      "low",
    ]);

    fireEvent.keyDown(document.activeElement!, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(search);
  });

  it("jumps to the first and last level with Home and End", () => {
    const { onThinkingEffortChange } = mount({ provider: "codex", model: "gpt-5.4" });
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "ArrowRight" });
    fireEvent.keyDown(document.activeElement!, { key: "End" });
    expect(checkedLevel().textContent).toContain("High");
    expect(document.activeElement).toBe(checkedLevel());
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    expect(checkedLevel().textContent).toContain("Auto");
    expect(onThinkingEffortChange.mock.calls.map(([level]) => level)).toEqual(["high", "auto"]);
  });

  it("describes Auto to screen readers, not only in a tooltip", () => {
    mount({ provider: "codex", model: "gpt-5.4" });
    const auto = within(column()!).getByRole("radio", { name: /Auto/ });
    const description = document.getElementById(auto.getAttribute("aria-describedby")!);
    expect(description?.textContent).toBe("Let the model decide");
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

  it("moves to the selected model's levels with ArrowRight on a highlight the mouse left behind", async () => {
    const { onChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox");
    const other = screen.getByText("gpt-5.4-mini").closest("[data-model-picker-row], [role=option], li, button")!;
    fireEvent.mouseMove(other);
    await act(async () => {
      fireEvent.keyDown(search, { key: "ArrowRight" });
    });
    expect(onChange).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(checkedLevel());
  });

  it("keeps the picker open after Enter picks a model", async () => {
    const { onChange, onOpenChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const search = screen.getByRole("combobox");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4-mini" });
    await nextFrame();
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    expect(column()!.contains(document.activeElement)).toBe(true);
    expect(column()!.textContent).toContain("gpt-5.4-mini");
  });

  it("closes after picking a checked model that takes no effort, footer docked", async () => {
    const { onChange, onOpenChange } = mount(
      { provider: "codex", model: "gpt-5.4" },
      {},
      { hostEffort: true },
    );
    expect(column()).not.toBeNull();
    expect(document.querySelector("[data-model-picker-footer]")).not.toBeNull();
    const search = screen.getByRole("combobox");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await act(async () => {
      fireEvent.keyDown(search, { key: "Enter" });
    });
    await nextFrame();
    expect(onChange).toHaveBeenCalledWith({ provider: "codex", model: "gpt-5.4-nano" });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
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
    const indicator = trigger.querySelector("[data-full-access-indicator]");
    // Named next to the lock where the composer toolbar has room.
    expect(indicator?.textContent).toBe("Full access");
    expect(indicator?.querySelector("span")?.className).toContain("@sm/composer-toolbar:inline");
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
