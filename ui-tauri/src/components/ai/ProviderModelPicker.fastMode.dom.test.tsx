// @vitest-environment happy-dom
//
// The Standard | Fast toggle in the model picker's footer: shown only for a
// model that advertises fast mode, worded from the model, marked on the
// trigger when on, and never a reason to contact a provider.
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
  { name: "claude", display_name: "Claude", base_url: "claude-cli://default", kind: "remote", default_model: "default", acknowledged_at: "2026-01-01T00:00:00Z", has_api_key: false, is_default: false },
  { name: "copilot", display_name: "GitHub Copilot", base_url: "copilot-cli://default", kind: "remote", default_model: "default", acknowledged_at: "2026-01-01T00:00:00Z", has_api_key: false, is_default: false },
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
  claudeModels?: Array<Record<string, unknown>>,
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
          {
            id: "gpt-5.4",
            reasoning_efforts: ["low", "medium", "high"],
            supports_fast_mode: true,
            fast_mode_description: "2x speed, increased usage",
          },
          { id: "gpt-5.4-mini", reasoning_efforts: ["low"], supports_fast_mode: false },
        ],
      },
    },
  );
  if (claudeModels) {
    // A Check models for Claude, whose rows the broker gated on the CLI version.
    client.setQueryData(
      providerModelsQueryKey(dataMode, daemonSession, {
        name: "claude",
        base_url: "claude-cli://default",
      }),
      { kind: "ai.list_models", schema_version: 1, data: { provider: "claude", models: claudeModels } },
    );
  }
  const onFastModeChange = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <ProviderModelPicker
        value={value}
        onChange={vi.fn()}
        thinkingEffort="auto"
        onThinkingEffortChange={vi.fn()}
        showThinkingEffort
        fastMode={false}
        onFastModeChange={onFastModeChange}
        open
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onFastModeChange };
}

function speedToggle(): HTMLElement | null {
  return screen.queryByRole("radiogroup", { name: "Speed" });
}

function trigger(): HTMLElement {
  return document.querySelector<HTMLElement>("[data-chat-provider-model-picker]")!;
}

describe("Fast mode toggle", () => {
  beforeEach(() => mocks.invoke.mockReset());
  afterEach(cleanup);

  it("offers fast mode with the model's own wording and reports the choice", async () => {
    const { onFastModeChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const toggle = speedToggle()!;
    const standard = within(toggle).getByRole("radio", { name: "Standard" });
    const fast = within(toggle).getByRole("radio", { name: "Fast" });
    expect(standard.getAttribute("aria-checked")).toBe("true");
    expect(fast.getAttribute("aria-checked")).toBe("false");
    expect(fast.getAttribute("title")).toBe("2x speed, increased usage");
    await act(async () => {
      fireEvent.click(fast);
    });
    expect(onFastModeChange).toHaveBeenCalledWith(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("keeps the billing note in sight while Fast is on", () => {
    mount({ provider: "codex", model: "gpt-5.4" });
    const note = document.querySelector<HTMLElement>("[data-fast-mode-hint]")!;
    expect(note.hidden).toBe(true);
    const fast = within(speedToggle()!).getByRole("radio", { name: "Fast" });
    expect(fast.getAttribute("aria-describedby")).toBe(note.id);
    cleanup();

    mount({ provider: "codex", model: "gpt-5.4" }, { fastMode: true });
    const shown = document.querySelector<HTMLElement>("[data-fast-mode-hint]")!;
    expect(shown.hidden).toBe(false);
    expect(shown.textContent).toBe("2x speed, increased usage");
  });

  it("is one tab stop whose arrow keys switch the speed", () => {
    const { onFastModeChange } = mount({ provider: "codex", model: "gpt-5.4" });
    const toggle = speedToggle()!;
    const standard = within(toggle).getByRole("radio", { name: "Standard" });
    const fast = within(toggle).getByRole("radio", { name: "Fast" });
    expect(standard.tabIndex).toBe(0);
    expect(fast.tabIndex).toBe(-1);
    standard.focus();
    fireEvent.keyDown(standard, { key: "ArrowRight" });
    expect(onFastModeChange).toHaveBeenCalledWith(true);
    expect(document.activeElement).toBe(fast);
  });

  it("marks the trigger when fast mode is on", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { fastMode: true, open: false });
    expect(trigger().getAttribute("aria-label")).toMatch(/Models: Codex · gpt-5\.4.*, Fast$/);
    expect(trigger().querySelector("[data-fast-mode-indicator]")).not.toBeNull();
  });

  it("hides the toggle for a model without fast mode, even if asked on", () => {
    mount({ provider: "codex", model: "gpt-5.4-mini" }, { fastMode: true });
    expect(speedToggle()).toBeNull();
    expect(trigger().querySelector("[data-fast-mode-indicator]")).toBeNull();
  });

  it("locks the toggle while a reply streams", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { onFastModeChange: undefined });
    for (const radio of within(speedToggle()!).getAllByRole("radio")) {
      expect((radio as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("uses Claude's own wording on a checked Opus and offers nothing on other aliases or Copilot", () => {
    const checked = [
      { id: "opus", reasoning_efforts: ["low", "high"], supports_fast_mode: true },
      { id: "sonnet", reasoning_efforts: ["low", "high"] },
    ];
    mount({ provider: "claude", model: "opus" }, {}, checked);
    expect(
      within(speedToggle()!).getByRole("radio", { name: "Fast" }).getAttribute("title"),
    ).toBe("Faster Opus output, billed at a higher rate");
    cleanup();

    for (const value of [
      { provider: "claude", model: "sonnet" },
      { provider: "copilot", model: "default" },
    ]) {
      mount(value, {}, checked);
      expect(speedToggle()).toBeNull();
      cleanup();
    }
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("offers no Claude fast mode before Check models, or on a CLI too old for it", () => {
    // Before a check the installed CLI's version is unknown; an older CLI's
    // checked Opus row carries no fast flag.
    for (const claudeModels of [undefined, [{ id: "opus", reasoning_efforts: ["low", "high"] }]]) {
      mount({ provider: "claude", model: "opus" }, { fastMode: true }, claudeModels);
      expect(speedToggle()).toBeNull();
      expect(trigger().querySelector("[data-fast-mode-indicator]")).toBeNull();
      cleanup();
    }
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("shows the speed toggle alone when the model takes no reasoning effort", async () => {
    const { onFastModeChange } = mount(
      { provider: "codex", model: "gpt-5.4" },
      { showThinkingEffort: false, onThinkingEffortChange: undefined },
    );
    expect(document.querySelector("[data-model-picker-effort]")).toBeNull();
    await act(async () => {
      fireEvent.click(within(speedToggle()!).getByRole("radio", { name: "Fast" }));
    });
    expect(onFastModeChange).toHaveBeenCalledWith(true);
  });

  it("limits Copilot's undiscovered default to its published effort levels", () => {
    mount({ provider: "copilot", model: "default" });
    const column = document.querySelector<HTMLElement>("[data-model-picker-effort]")!;
    const levels = within(column).getAllByRole("radio").map((item) => item.textContent ?? "");
    expect(levels.some((label) => label.includes("Ultra"))).toBe(false);
    expect(levels.some((label) => label.includes("Max"))).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
