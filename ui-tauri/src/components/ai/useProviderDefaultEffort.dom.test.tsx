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
];

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({
    data: { kind: "ai.providers.list", data: { default: "claude", providers: PROVIDERS } },
  }),
  daemonQueryKey: (...parts: unknown[]) => ["daemon", ...parts],
  DaemonRequestError: class extends Error {},
}));

import { nearestOfferedEffort, useProviderDefaultEffort } from "./useProviderDefaultEffort";

// Levels the Claude and Copilot CLIs offer (NATIVE_RUNTIME_REASONING_EFFORTS).
const FIXED_LEVELS = ["low", "medium", "high", "xhigh", "max"];

function useHarness(initial: AssistantModelSelection | null) {
  const [selection, setSelection] = React.useState(initial);
  const [effort, setRawEffort] = React.useState<AssistantThinkingEffort>("auto");
  const setEffortRef = React.useRef<(effort: AssistantThinkingEffort) => void>(setRawEffort);
  // Stands in for the picker, a child whose effect runs first: it resets a
  // level the selected Claude/Copilot model does not offer to auto through
  // the same setter the user's picks use.
  React.useEffect(() => {
    const fixed = selection && selection.provider !== "codex";
    if (fixed && effort !== "auto" && !FIXED_LEVELS.includes(effort)) {
      setEffortRef.current("auto");
    }
  }, [selection, effort]);
  // The user's picks go through the hook's setter, as in the session context.
  const setEffort = useProviderDefaultEffort({
    selection,
    thinkingEffort: effort,
    setThinkingEffort: setRawEffort,
  });
  setEffortRef.current = setEffort;
  return { effort, setEffort, setSelection };
}

function renderHarness(initial: AssistantModelSelection | null) {
  const client = new QueryClient();
  return renderHook(() => useHarness(initial), {
    wrapper: ({ children }: { children: React.ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

describe("useProviderDefaultEffort", () => {
  afterEach(cleanup);

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

describe("nearestOfferedEffort", () => {
  it("picks the closest offered level, the cheaper one on a tie", () => {
    expect(nearestOfferedEffort("ultra", ["low", "medium", "high"])).toBe("high");
    expect(nearestOfferedEffort("medium", ["low", "high"])).toBe("low");
    expect(nearestOfferedEffort("high", ["custom"])).toBe("auto");
  });
});
