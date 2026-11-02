// @vitest-environment happy-dom
//
// Integration: the composer with the real ProviderModelPicker and its docked
// reasoning column (the other composer tests mock the picker). Covers the
// composer's Mod+Shift+M and Mod+Shift+K shortcuts, and picking a model with
// the pointer while the picker stays open on its levels.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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
          data: {
            default: "ollama",
            providers: [
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
                name: "lmstudio",
                display_name: "LM Studio",
                base_url: "http://127.0.0.1:1234/v1",
                kind: "local",
                default_model: "gemma-3-12b",
                has_api_key: false,
                is_default: false,
              },
            ],
          },
        },
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

import Ai02 from "@/components/ai-02";

function Harness({ onSelectionChange }: { onSelectionChange: (next: unknown) => void }) {
  const [selection, setSelection] = React.useState({
    provider: "ollama",
    model: "qwen3:8b",
  });
  const [effort, setEffort] = React.useState<"auto" | "low">("auto");
  return (
    <QueryClientProvider client={new QueryClient()}>
      <Ai02
        selection={selection}
        onSelectionChange={(next) => {
          onSelectionChange(next);
          if (next) setSelection(next);
        }}
        onSubmit={() => {}}
        thinkingEffort={effort}
        onThinkingEffortChange={(next) => setEffort(next as "auto" | "low")}
        showThinkingEffort
      />
    </QueryClientProvider>
  );
}

describe("Ai02 with the real model picker", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    useUiStore.setState({ assistantModelFavorites: [] });
  });
  afterEach(cleanup);

  it("ignores key repeats of a held Mod+Shift+M", async () => {
    render(<Harness onSelectionChange={vi.fn()} />);
    const composer = screen.getByRole("textbox");
    await act(async () => {
      fireEvent.keyDown(composer, { key: "m", ctrlKey: true, shiftKey: true });
    });
    expect(screen.getByRole("combobox")).toBeTruthy();
    await act(async () => {
      fireEvent.keyDown(document.activeElement ?? composer, {
        key: "m",
        ctrlKey: true,
        shiftKey: true,
        repeat: true,
      });
    });
    expect(screen.getByRole("combobox")).toBeTruthy();
  });

  it("opens on the reasoning levels with Mod+Shift+K, and moves there when open", async () => {
    render(<Harness onSelectionChange={vi.fn()} />);
    const composer = screen.getByRole("textbox");
    await act(async () => {
      fireEvent.keyDown(composer, { key: "k", ctrlKey: true, shiftKey: true });
    });
    const column = document.querySelector<HTMLElement>("[data-model-picker-effort]")!;
    expect(column).not.toBeNull();
    expect(document.activeElement?.getAttribute("aria-checked")).toBe("true");
    expect(column.contains(document.activeElement)).toBe(true);

    // From the search field of the open picker, the shortcut moves, not closes.
    const search = screen.getByRole("combobox");
    search.focus();
    await act(async () => {
      fireEvent.keyDown(search, { key: "k", ctrlKey: true, shiftKey: true });
    });
    expect(screen.getByRole("combobox")).toBeTruthy();
    expect(column.contains(document.activeElement)).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("picks a model with the pointer and stays open on its levels", async () => {
    const onSelectionChange = vi.fn();
    render(<Harness onSelectionChange={onSelectionChange} />);
    const composer = screen.getByRole("textbox");
    await act(async () => {
      fireEvent.keyDown(composer, { key: "m", ctrlKey: true, shiftKey: true });
    });
    expect(document.activeElement).toBe(screen.getByRole("combobox"));

    fireEvent.click(screen.getByRole("button", { name: "LM Studio · local" }));
    // The rail switch hands focus back to the search field on the next frame.
    await act(async () => {
      await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));
    });
    const row = screen
      .getAllByRole("option")
      .find((option) => option.textContent?.includes("gemma-3-12b"));
    expect(row).toBeDefined();
    await act(async () => {
      fireEvent.mouseDown(row!);
      fireEvent.click(row!);
    });
    expect(onSelectionChange).toHaveBeenCalledWith({
      provider: "lmstudio",
      model: "gemma-3-12b",
    });
    await act(async () => {
      await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));
    });
    expect(screen.getByRole("combobox")).toBeTruthy();
    expect(
      document.querySelector("[data-model-picker-effort]")?.contains(document.activeElement),
    ).toBe(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
