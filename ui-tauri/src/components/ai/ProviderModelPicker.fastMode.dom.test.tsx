// @vitest-environment happy-dom
//
// The Fast mode toggle in the composer's traits menu: shown only for a model
// that advertises fast mode, worded from the model, marked on the trigger when
// on, and never a reason to contact a provider.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { daemonQueryKey } from "@/daemon/client";
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

function mount(
  value: { provider: string; model: string },
  props: Partial<React.ComponentProps<typeof ProviderModelPicker>> = {},
) {
  const client = new QueryClient();
  const { dataMode, daemonSession } = useUiStore.getState();
  // An earlier explicit Check models for Codex, already in the cache.
  client.setQueryData(
    daemonQueryKey(dataMode, daemonSession, "ai.list_models", { provider: "codex" }),
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
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onFastModeChange };
}

async function openTraits(name: RegExp) {
  const trigger = screen.getByRole("button", { name });
  await act(async () => {
    fireEvent.pointerDown(trigger, { button: 0, pointerType: "mouse" });
  });
  return trigger;
}

describe("Fast mode toggle", () => {
  beforeEach(() => mocks.invoke.mockReset());
  afterEach(cleanup);

  it("offers fast mode with the model's own wording and reports the choice", async () => {
    const { onFastModeChange } = mount({ provider: "codex", model: "gpt-5.4" });
    await openTraits(/Reasoning effort/);
    const toggle = screen.getByRole("menuitemcheckbox", { name: /Fast mode/ });
    expect(toggle.textContent).toContain("2x speed, increased usage");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await act(async () => {
      fireEvent.click(toggle);
    });
    expect(onFastModeChange).toHaveBeenCalledWith(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("marks the trigger when fast mode is on", () => {
    mount({ provider: "codex", model: "gpt-5.4" }, { fastMode: true });
    const trigger = screen.getByRole("button", { name: /Reasoning effort: Auto, Fast/ });
    expect(trigger.querySelector("[data-fast-mode-indicator]")).not.toBeNull();
  });

  it("hides the toggle for a model without fast mode, even if asked on", async () => {
    mount({ provider: "codex", model: "gpt-5.4-mini" }, { fastMode: true });
    const trigger = await openTraits(/Reasoning effort/);
    expect(screen.queryByRole("menuitemcheckbox")).toBeNull();
    expect(trigger.querySelector("[data-fast-mode-indicator]")).toBeNull();
  });

  it("uses Claude's own wording on Opus and offers nothing on other aliases or Copilot", async () => {
    mount({ provider: "claude", model: "opus" });
    await openTraits(/Reasoning effort/);
    expect(
      screen.getByRole("menuitemcheckbox", { name: /Fast mode/ }).textContent,
    ).toContain("Faster Opus output, billed at a higher rate");
    cleanup();

    for (const value of [
      { provider: "claude", model: "sonnet" },
      { provider: "copilot", model: "default" },
    ]) {
      mount(value);
      await openTraits(/Reasoning effort/);
      expect(screen.queryByRole("menuitemcheckbox")).toBeNull();
      cleanup();
    }
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("shows the control for fast mode alone when the model has no effort levels", async () => {
    const { onFastModeChange } = mount(
      { provider: "codex", model: "gpt-5.4" },
      { showThinkingEffort: false, onThinkingEffortChange: undefined },
    );
    await openTraits(/Fast mode: Standard/);
    expect(screen.queryByRole("menuitemradio")).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitemcheckbox", { name: /Fast mode/ }));
    });
    expect(onFastModeChange).toHaveBeenCalledWith(true);
  });

  it("limits Copilot's undiscovered default to its published effort levels", async () => {
    mount({ provider: "copilot", model: "default" });
    await openTraits(/Reasoning effort/);
    const levels = screen.getAllByRole("menuitemradio").map((item) => item.textContent ?? "");
    expect(levels.some((label) => label.includes("Ultra"))).toBe(false);
    expect(levels.some((label) => label.includes("Max"))).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
