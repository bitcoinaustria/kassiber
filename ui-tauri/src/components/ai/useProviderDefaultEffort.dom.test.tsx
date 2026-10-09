// @vitest-environment happy-dom
//
// A provider's stored default effort seeds the session when that provider is
// selected; it never overrides a level the user picked afterwards.
import { act, cleanup, renderHook } from "@testing-library/react";
import * as React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  AssistantModelSelection,
  AssistantThinkingEffort,
} from "@/components/ai/assistantSession";

const PROVIDERS = [
  { name: "claude", base_url: "claude-cli://default", kind: "remote", default_reasoning_effort: "high" },
  { name: "codex", base_url: "codex-cli://default", kind: "remote", default_reasoning_effort: null },
  { name: "copilot", base_url: "copilot-cli://default", kind: "remote", default_reasoning_effort: "max" },
];

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({
    data: { kind: "ai.providers.list", data: { default: "claude", providers: PROVIDERS } },
  }),
}));

import { useProviderDefaultEffort } from "./useProviderDefaultEffort";

function useHarness(initial: AssistantModelSelection | null) {
  const [selection, setSelection] = React.useState(initial);
  const [effort, setEffort] = React.useState<AssistantThinkingEffort>("auto");
  useProviderDefaultEffort({
    selection,
    thinkingEffort: effort,
    setThinkingEffort: setEffort,
  });
  return { effort, setEffort, setSelection };
}

describe("useProviderDefaultEffort", () => {
  afterEach(cleanup);

  it("seeds the selected provider's default and follows provider switches", () => {
    const { result } = renderHook(() =>
      useHarness({ provider: "claude", model: "opus" }),
    );
    expect(result.current.effort).toBe("high");

    act(() => result.current.setSelection({ provider: "copilot", model: "default" }));
    expect(result.current.effort).toBe("max");

    // No default: an effort that only came from a default goes back to auto.
    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("auto");
  });

  it("keeps a level the user chose", () => {
    const { result } = renderHook(() =>
      useHarness({ provider: "claude", model: "opus" }),
    );
    act(() => result.current.setEffort("low"));
    act(() => result.current.setSelection({ provider: "claude", model: "sonnet" }));
    expect(result.current.effort).toBe("low");

    act(() => result.current.setSelection({ provider: "codex", model: "default" }));
    expect(result.current.effort).toBe("low");
  });
});
