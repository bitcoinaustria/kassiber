import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AiProvidersSettingsPanel } from "@/components/kb/settings/AiProvidersSettingsPanel";
import { ProviderModelPicker } from "./ProviderModelPicker";
import { useReasoningEffortSupport } from "./useReasoningEffortSupport";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  runtimeRefetch: vi.fn(),
  openPicker: null as ((open: boolean) => void) | null,
  checkModels: null as (() => void | Promise<void>) | null,
  daemonHooks: [] as Array<{ kind: string; enabled?: boolean }>,
  provider: { name: "codex", base_url: "codex-cli://default", kind: "remote" },
  modelError: null as Error | null,
}));

vi.mock("@/components/ui/popover", () => ({
  Popover: ({
    children,
    onOpenChange,
  }: {
    children: ReactNode;
    onOpenChange: (open: boolean) => void;
  }) => {
    mocks.openPicker = onOpenChange;
    return children;
  },
  PopoverContent: ({ children }: { children: ReactNode }) => children,
  PopoverTrigger: ({ children }: { children: ReactNode }) => children,
}));

vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke: mocks.invoke }),
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    onClick,
    ...props
  }: {
    children: ReactNode;
    onClick?: () => void | Promise<void>;
    "data-model-picker-check"?: boolean;
  }) => {
    if (props["data-model-picker-check"] !== undefined) {
      mocks.checkModels = onClick ?? null;
    }
    return <button onClick={onClick}>{children}</button>;
  },
}));

vi.mock("@/daemon/client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/client")>();
  return {
    ...actual,
    useDaemon: (
      kind: string,
      _args?: unknown,
      options?: { enabled?: boolean },
    ) => {
      mocks.daemonHooks.push({ kind, enabled: options?.enabled });
      if (kind === "ai.providers.list") {
        return {
          data: {
            kind,
            data: {
              default: mocks.provider.name,
              providers: [
                {
                  ...mocks.provider,
                  default_model: "default",
                  acknowledged_at: "2026-01-01T00:00:00Z",
                  has_api_key: false,
                  is_default: true,
                },
                {
                  name: "custom",
                  base_url: "https://example.invalid/v1",
                  kind: "remote",
                  default_model: "configured-model",
                  acknowledged_at: "2026-01-01T00:00:00Z",
                  has_api_key: true,
                  is_default: false,
                },
              ],
            },
          },
          refetch: vi.fn(),
        };
      }
      return {
        data: undefined,
        isError: false,
        isLoading: false,
        refetch: mocks.runtimeRefetch,
      };
    },
    useDaemonMutation: () => ({
      error: null,
      isPending: false,
      mutate: vi.fn(),
      mutateAsync: vi.fn(),
      reset: vi.fn(),
    }),
  };
});

vi.mock("@tanstack/react-query", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@tanstack/react-query")>();
  return {
    ...actual,
    useQueries: ({ queries }: { queries: Array<{ enabled?: boolean; queryFn: () => unknown }> }) =>
      queries.map((query) => {
        if (query.enabled) void query.queryFn();
        return { data: undefined, error: mocks.modelError, refetch: query.queryFn };
      }),
  };
});

describe("ProviderModelPicker network consent", () => {
  beforeEach(() => {
    mocks.invoke.mockClear();
    mocks.runtimeRefetch.mockClear();
    mocks.daemonHooks.length = 0;
    mocks.openPicker = null;
    mocks.checkModels = null;
    mocks.provider = { name: "codex", base_url: "codex-cli://default", kind: "remote" };
    mocks.modelError = null;
    mocks.invoke.mockResolvedValue({
      kind: "ai.list_models",
      data: { provider: "codex", models: [] },
    });
  });

  it("does not discover providers merely because the picker mounted", () => {
    renderToStaticMarkup(<ProviderModelPicker value={null} onChange={vi.fn()} />);

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.runtimeRefetch).not.toHaveBeenCalled();
  });

  it("does not discover providers merely because the picker opened", () => {
    renderToStaticMarkup(<ProviderModelPicker value={null} onChange={vi.fn()} />);

    mocks.openPicker?.(true);

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(mocks.runtimeRefetch).not.toHaveBeenCalled();
  });

  it.each([
    { name: "codex", base_url: "codex-cli://default", kind: "remote" },
    { name: "claude", base_url: "claude-cli://default", kind: "remote" },
    { name: "opencode", base_url: "opencode-cli://default", kind: "remote" },
    { name: "ollama", base_url: "http://127.0.0.1:11434/v1", kind: "local" },
  ])("checks only the selected $name provider after an explicit click", async (provider) => {
    mocks.provider = provider;
    renderToStaticMarkup(<ProviderModelPicker value={null} onChange={vi.fn()} />);

    expect(mocks.invoke).not.toHaveBeenCalled();
    mocks.openPicker?.(true);
    expect(mocks.invoke).not.toHaveBeenCalled();
    await mocks.checkModels?.();

    expect(mocks.invoke).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith({
      kind: "ai.list_models",
      args: { provider: provider.name, refresh: true },
    });
    // The broker already probes the selected native provider for its models.
    // Global runtime status would also launch unrelated provider executables.
    expect(mocks.runtimeRefetch).not.toHaveBeenCalled();
    expect(mocks.daemonHooks.map(({ kind }) => kind)).not.toContain(
      "ai.provider_runtime.status",
    );
  });

  it("does not list provider models when Settings mounts", () => {
    renderToStaticMarkup(
      <AiProvidersSettingsPanel
        aiFeaturesEnabled
        setAiFeaturesEnabled={vi.fn()}
      />,
    );

    expect(mocks.daemonHooks.map(({ kind }) => kind)).not.toContain(
      "ai.list_models",
    );
  });

  it("shows selected discovery errors even when a configured default model remains", () => {
    mocks.modelError = new Error("The selected provider requires authentication.");
    const html = renderToStaticMarkup(<ProviderModelPicker value={null} onChange={vi.fn()} />);
    expect(html).toContain("The selected provider requires authentication.");
    expect(mocks.runtimeRefetch).not.toHaveBeenCalled();
  });

  it("only reads cached model metadata for reasoning support", () => {
    function Probe() {
      useReasoningEffortSupport({ provider: "codex", model: "default" });
      return null;
    }

    renderToStaticMarkup(<Probe />);

    expect(mocks.daemonHooks).toContainEqual({
      kind: "ai.list_models",
      enabled: false,
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("leaves reasoning support unresolved until models are actually known", () => {
    // Not discovering models is the point, but it must not be read as
    // "this model has no reasoning effort". `useSupportedReasoningEffort`
    // resets the user's chosen effort to auto once `resolved` is true, and
    // only CLI providers advertise support on the provider row -- for local
    // Ollama/oMLX and remote OpenAI-compatible providers it lives on the
    // model, which is empty until an explicit check.
    let support: ReturnType<typeof useReasoningEffortSupport> | null = null;
    function Probe() {
      support = useReasoningEffortSupport({
        provider: "custom",
        model: "configured-model",
      });
      return null;
    }

    renderToStaticMarkup(<Probe />);

    expect(support).not.toBeNull();
    expect(support!.resolved).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
