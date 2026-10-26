// @vitest-environment happy-dom
//
// A provider's stored default effort seeds the session when that provider is
// selected and the user has not chosen a level; like `kassiber chat`, a level
// the user picked survives provider switches unless the new model lacks it.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook } from "@testing-library/react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  AssistantModelSelection,
  AssistantThinkingEffort,
} from "@/components/ai/assistantSession";

const PROVIDERS = [
  { name: "claude", base_url: "claude-cli://default", kind: "remote", default_reasoning_effort: "high" },
  { name: "claude-work", base_url: "claude-cli://default", kind: "remote", default_reasoning_effort: null },
  { name: "codex", base_url: "codex-cli://default", kind: "remote", default_reasoning_effort: null },
  { name: "copilot", base_url: "copilot-cli://default", kind: "remote", default_reasoning_effort: "max" },
  { name: "codex-pro", base_url: "codex-cli://default", kind: "remote", default_reasoning_effort: "xhigh" },
  { name: "openrouter", base_url: "https://openrouter.ai/api/v1", kind: "remote", default_reasoning_effort: "max" },
];

// The UI store persists to localStorage when a book switch bumps its session.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

// The open book's provider list; a test swaps it to switch books.
// "loading" stands for a list that has not arrived yet (app start, or the
// query cache cleared by a book switch); "error" for one that failed.
const book = vi.hoisted(() => ({ providers: null as unknown }));

vi.mock("@/daemon/client", () => ({
  useDaemon: () =>
    book.providers === "loading"
      ? { data: undefined, isError: false }
      : book.providers === "error"
        ? { data: undefined, isError: true }
        : {
            data: {
              kind: "ai.providers.list",
              data: { default: "claude", providers: book.providers ?? PROVIDERS },
            },
            isError: false,
          },
  daemonQueryKey: (...parts: unknown[]) => ["daemon", ...parts],
  DaemonRequestError: class extends Error {},
}));

import { selectedModelReasoningEfforts, type AiProviderRow } from "@/lib/aiCapabilities";
import { useUiStore } from "@/store/ui";

import { providerModelsQueryKey, useProviderModelsSnapshot } from "./providerModelQuery";
import { nearestOfferedEffort, useProviderDefaultEffort } from "./useProviderDefaultEffort";

/** Opens another book whose `claude` row has `effort` as its default. */
function switchBook(effort: string | null) {
  book.providers = PROVIDERS.map((row) =>
    row.name === "claude" ? { ...row, default_reasoning_effort: effort } : row,
  );
  useUiStore.getState().bumpDaemonSession();
}

// The levels the picker can label, as in ProviderModelPicker.
const KNOWN_LEVELS = ["low", "medium", "high", "xhigh", "max", "ultra"];

function useHarness(initial: AssistantModelSelection | null) {
  const [selection, setSelection] = React.useState(initial);
  const [effort, setRawEffort] = React.useState<AssistantThinkingEffort>("auto");
  const setEffortRef = React.useRef<(effort: AssistantThinkingEffort) => void>(setRawEffort);
  // Stands in for the picker, a child whose effect runs first: it offers what
  // the selected model advertises (the CLI's fixed list for Claude/Copilot,
  // Check models for Codex, everything while unknown) and resets any other
  // level to auto through the same setter the user's picks use.
  const providers = (
    Array.isArray(book.providers) ? book.providers : PROVIDERS
  ) as AiProviderRow[];
  const row = providers.find((provider) => provider.name === selection?.provider);
  const snapshot = useProviderModelsSnapshot(row);
  const advertised = selectedModelReasoningEfforts({
    selection,
    providers,
    models: snapshot?.models ?? [],
  });
  const options = KNOWN_LEVELS.filter((level) => advertised.includes(level));
  const offeredKey = (options.length > 0 ? options : KNOWN_LEVELS).join(",");
  React.useEffect(() => {
    if (effort !== "auto" && !offeredKey.split(",").includes(effort)) {
      setEffortRef.current("auto");
    }
  }, [offeredKey, effort]);
  // The user's picks go through the hook's setter, as in the session context.
  const { setThinkingEffort: setEffort, effortSeeded } = useProviderDefaultEffort({
    selection,
    thinkingEffort: effort,
    setThinkingEffort: setRawEffort,
  });
  setEffortRef.current = setEffort;
  return { effort, effortSeeded, setEffort, setSelection };
}

function renderHarness(initial: AssistantModelSelection | null) {
  const client = new QueryClient();
  const rendered = renderHook(() => useHarness(initial), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  return { ...rendered, client };
}

/**
 * Check models answers for a Codex-runtime row: gpt-5.4 offers `levels`.
 * Resolves once the query has told its observers (on a timer, not a
 * microtask).
 */
async function resolveModels(client: QueryClient, provider: string, levels: string[]) {
  const { dataMode, daemonSession } = useUiStore.getState();
  client.setQueryData(
    providerModelsQueryKey(dataMode, daemonSession, {
      name: provider,
      base_url: "codex-cli://default",
    }),
    {
      kind: "ai.list_models",
      schema_version: 1,
      data: {
        provider,
        models: [{ id: "gpt-5.4", reasoning_efforts: levels }],
        checked_at: "2026-10-13T10:00:00Z",
        stale: false,
        error: null,
      },
    },
  );
  await new Promise((resolve) => setTimeout(resolve, 0));
}

describe("useProviderDefaultEffort", () => {
  afterEach(() => {
    cleanup();
    book.providers = null;
  });

  it("seeds the selected provider's default and follows provider switches", () => {
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effort).toBe("high");

    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("max");

    // No default: an effort that only came from a default goes back to auto.
    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("auto");
  });

  it("keeps a level the user chose", () => {
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    act(() => result.current.setEffort("low"));
    act(() => result.current.setSelection({ provider: "claude", model: "sonnet" }));
    expect(result.current.effort).toBe("low");

    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("low");
  });

  it("keeps a chosen level when the next provider has a default", () => {
    const { result } = renderHarness({ provider: "claude-work", model: "opus" });
    expect(result.current.effort).toBe("auto");
    act(() => result.current.setEffort("low"));
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("low");

    // Overriding a seeded default counts as a choice too.
    act(() => result.current.setSelection({ provider: "claude", model: "opus" }));
    expect(result.current.effort).toBe("low");
  });

  it("keeps a re-picked level even when it equals the seeded default", () => {
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effort).toBe("high");
    act(() => result.current.setEffort("low"));
    act(() => result.current.setEffort("high"));
    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("high");
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("high");
  });

  it("treats a pick of auto as leaving the level open", () => {
    const { result } = renderHarness({ provider: "claude-work", model: "opus" });
    act(() => result.current.setEffort("low"));
    act(() => result.current.setEffort("auto"));
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("max");
    // That level came from the default, so a provider without one drops it.
    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("auto");
  });

  it("falls back to the new default when the model does not offer the chosen level", () => {
    // Codex advertises nothing before Check models, so any level is allowed.
    const { result } = renderHarness({ provider: "codex", model: "default" });
    act(() => result.current.setEffort("ultra"));
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("max");
  });

  it("uses the nearest offered level when there is no default to fall back to", () => {
    const { result } = renderHarness({ provider: "codex", model: "default" });
    act(() => result.current.setEffort("ultra"));
    act(() => result.current.setSelection({ provider: "claude-work", model: "opus" }));
    expect(result.current.effort).toBe("max");
    // That level is still the user's, not a default to replace.
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("max");
    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("max");
  });
});

describe("useProviderDefaultEffort once the model's levels are known", () => {
  afterEach(() => {
    cleanup();
    book.providers = null;
  });

  it("moves an unsupported chosen level to the nearest offered one, not auto", async () => {
    const { result, client } = renderHarness({ provider: "codex", model: "gpt-5.4" });
    // Nothing is known yet, so every level is offered.
    act(() => result.current.setEffort("ultra"));
    expect(result.current.effort).toBe("ultra");

    await act(async () => resolveModels(client, "codex", ["low", "medium", "high"]));
    expect(result.current.effort).toBe("high");
    // Still the user's level: a provider with a default does not replace it.
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("high");
  });

  it("moves an unsupported seeded default to the nearest offered level", async () => {
    const { result, client } = renderHarness({ provider: "codex-pro", model: "gpt-5.4" });
    expect(result.current.effort).toBe("xhigh");

    await act(async () => resolveModels(client, "codex-pro", ["low", "medium", "high"]));
    expect(result.current.effort).toBe("high");
  });

  it("falls back to the provider default when it is offered", async () => {
    const { result, client } = renderHarness({ provider: "codex-pro", model: "gpt-5.4" });
    act(() => result.current.setEffort("ultra"));

    await act(async () => resolveModels(client, "codex-pro", ["low", "medium", "high", "xhigh"]));
    expect(result.current.effort).toBe("xhigh");
  });

  it("moves an HTTP provider's seeded default to the model's nearest wire level", async () => {
    const { result, client } = renderHarness({ provider: "openrouter", model: "vendor/reasoner" });
    // Nothing checked yet: the stored default stands (sent as high).
    expect(result.current.effort).toBe("max");

    const { dataMode, daemonSession } = useUiStore.getState();
    await act(async () => {
      client.setQueryData(
        providerModelsQueryKey(dataMode, daemonSession, {
          name: "openrouter",
          base_url: "https://openrouter.ai/api/v1",
        }),
        {
          kind: "ai.list_models",
          schema_version: 1,
          data: {
            provider: "openrouter",
            models: [
              {
                id: "vendor/reasoner",
                supports_reasoning_effort: true,
                reasoning_efforts: ["low", "medium", "high"],
                // Levels the endpoint does not take over HTTP are not offered.
                capabilities: { reasoning_efforts: ["xhigh"] },
              },
            ],
            checked_at: "2026-10-13T10:00:00Z",
            stale: false,
            error: null,
          },
        },
      );
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(result.current.effort).toBe("high");
  });

  it("leaves a supported level alone when the levels resolve", async () => {
    const { result, client } = renderHarness({ provider: "codex", model: "gpt-5.4" });
    act(() => result.current.setEffort("low"));

    await act(async () => resolveModels(client, "codex", ["low", "medium", "high"]));
    expect(result.current.effort).toBe("low");
    await act(async () => resolveModels(client, "codex", ["low", "high"]));
    expect(result.current.effort).toBe("low");
  });
});

describe("useProviderDefaultEffort across books", () => {
  afterEach(() => {
    cleanup();
    book.providers = null;
  });

  it("seeds again from the new book when the same provider stays selected", () => {
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effort).toBe("high");

    act(() => switchBook("low"));
    expect(result.current.effort).toBe("low");

    act(() => switchBook(null));
    expect(result.current.effort).toBe("auto");
  });

  it("does not carry a level chosen in one book into the next", () => {
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    act(() => result.current.setEffort("medium"));
    // Within the book, the choice holds.
    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    act(() => result.current.setSelection({ provider: "claude", model: "opus" }));
    expect(result.current.effort).toBe("medium");

    act(() => switchBook("low"));
    expect(result.current.effort).toBe("low");

    act(() => result.current.setEffort("xhigh"));
    act(() => switchBook(null));
    expect(result.current.effort).toBe("auto");
  });
});

describe("useProviderDefaultEffort while the provider list loads", () => {
  afterEach(() => {
    cleanup();
    book.providers = null;
  });

  it("is not seeded until the list arrives", () => {
    book.providers = "loading";
    const { result, rerender } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effortSeeded).toBe(false);
    expect(result.current.effort).toBe("auto");

    book.providers = null;
    rerender();
    expect(result.current.effortSeeded).toBe(true);
    expect(result.current.effort).toBe("high");
  });

  it("is not seeded for a new book until that book's list arrives", () => {
    const { result, rerender } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effortSeeded).toBe(true);
    expect(result.current.effort).toBe("high");

    act(() => {
      book.providers = "loading";
      useUiStore.getState().bumpDaemonSession();
    });
    // Still the previous book's effort, so sends must wait.
    expect(result.current.effortSeeded).toBe(false);

    book.providers = PROVIDERS.map((row) =>
      row.name === "claude" ? { ...row, default_reasoning_effort: "low" } : row,
    );
    rerender();
    expect(result.current.effortSeeded).toBe(true);
    expect(result.current.effort).toBe("low");
  });

  it("drops the previous book's level when the new book's list fails to load", () => {
    const { result, rerender } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effort).toBe("high");

    act(() => {
      book.providers = "error";
      useUiStore.getState().bumpDaemonSession();
    });
    rerender();
    expect(result.current.effortSeeded).toBe(true);
    expect(result.current.effort).toBe("auto");

    // The list arrives after all: this book's default seeds then.
    book.providers = PROVIDERS.map((row) =>
      row.name === "claude" ? { ...row, default_reasoning_effort: "low" } : row,
    );
    rerender();
    expect(result.current.effort).toBe("low");
  });

  it("keeps a level picked in the new book while its list was loading", () => {
    const { result, rerender } = renderHarness({ provider: "claude", model: "opus" });
    act(() => {
      book.providers = "loading";
      useUiStore.getState().bumpDaemonSession();
    });
    act(() => result.current.setEffort("medium"));
    expect(result.current.effortSeeded).toBe(false);

    book.providers = "error";
    rerender();
    expect(result.current.effortSeeded).toBe(true);
    expect(result.current.effort).toBe("medium");
  });

  it("does not hold sends forever when the list fails to load", () => {
    book.providers = "error";
    const { result } = renderHarness({ provider: "claude", model: "opus" });
    expect(result.current.effortSeeded).toBe(true);
  });
});

describe("nearestOfferedEffort", () => {
  it("picks the closest offered level, the cheaper one on a tie", () => {
    expect(nearestOfferedEffort("ultra", ["low", "medium", "high"])).toBe("high");
    expect(nearestOfferedEffort("medium", ["low", "high"])).toBe("low");
    expect(nearestOfferedEffort("high", ["custom"])).toBe("auto");
  });
});
