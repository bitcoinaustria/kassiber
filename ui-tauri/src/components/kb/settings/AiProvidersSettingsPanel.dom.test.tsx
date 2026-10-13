// @vitest-environment happy-dom
//
// Mounted against a fake daemon: Settings → AI providers must contact no
// provider on its own. Its only automatic reads are the provider list, the
// offline switch, and cached runtime snapshots (`cached_only`, never a probe);
// a provider is checked only by an explicit click, only that provider, and
// only after the off-device acknowledgement.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

type Request = { kind: string; args?: Record<string, unknown> };
type Provider = Record<string, unknown> & { name: string };

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  confirm: vi.fn(),
  providers: [] as Provider[],
  offline: false,
  cachedRuntime: {} as Record<string, Record<string, unknown> | undefined>,
  probeResult: {} as Record<string, Record<string, unknown>>,
  probeError: {} as Record<string, string>,
  ackError: null as string | null,
  // A refresh that fails after an earlier good check: the daemon answers with
  // that last good snapshot, marked stale with this error.
  probeStale: {} as Record<string, string>,
}));

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
vi.mock("@/lib/confirmAction", () => ({
  confirmAction: (message: string) => Promise.resolve(mocks.confirm(message)),
}));
// Separate settings surfaces with their own daemon reads.
vi.mock("@/components/kb/settings/ChatHistorySettingsCard", () => ({
  ChatHistorySettingsCard: () => null,
}));
vi.mock("@/components/kb/settings/ExternalAgentsSettings", () => ({
  ExternalAgentsSettings: () => null,
}));

import { providerModelsQueryKey } from "@/components/ai/providerModelQuery";
import { useUiStore } from "@/store/ui";

import { AiProvidersSettingsPanel } from "./AiProvidersSettingsPanel";

const NOW = "2026-10-09T10:00:00Z";

function envelope(kind: string, data: unknown) {
  return { kind, schema_version: 1, data };
}

// Like the daemon, runtime snapshots belong to the runtime a row points at
// (`codex-cli://default` → codex), not to the row's name.
function runtimeOf(name: string): string {
  const row = mocks.providers.find((provider) => provider.name === name);
  return String(row?.base_url ?? name).replace(/-cli:\/\/default$/, "");
}

function daemon(request: Request) {
  const args = request.args ?? {};
  switch (request.kind) {
    case "ai.providers.list":
      return envelope(request.kind, { default: "ollama", providers: mocks.providers });
    case "ui.network.offline":
      return envelope(request.kind, { offline: mocks.offline, environment_blocked: false });
    case "ai.providers.acknowledge": {
      if (mocks.ackError) {
        return {
          kind: "error",
          schema_version: 1,
          error: { code: "project_unavailable", message: mocks.ackError, retryable: false },
        };
      }
      const row = mocks.providers.find((p) => p.name === args.name);
      if (row) row.acknowledged_at = NOW;
      return envelope(request.kind, { name: args.name, acknowledged_at: NOW });
    }
    case "ai.providers.update":
      return envelope(request.kind, { name: args.name });
    case "ai.provider_runtime.status": {
      const name = String(args.provider);
      if (args.cached_only) {
        const cached = mocks.cachedRuntime[runtimeOf(name)];
        return envelope(request.kind, {
          provider: name,
          providers: cached ? [cached] : [],
          checked_at: cached ? NOW : null,
          stale: false,
          error: null,
        });
      }
      if (mocks.probeError[name]) {
        return {
          kind: "error",
          schema_version: 1,
          error: { code: "ai_unavailable", message: mocks.probeError[name], retryable: false },
        };
      }
      if (mocks.probeStale[name]) {
        return envelope(request.kind, {
          provider: name,
          providers: [mocks.cachedRuntime[runtimeOf(name)]],
          checked_at: "2026-10-09T09:00:00Z",
          stale: true,
          error: { code: "ai_unavailable", message: mocks.probeStale[name] },
        });
      }
      const result = mocks.probeResult[name];
      mocks.cachedRuntime[runtimeOf(name)] = result;
      return envelope(request.kind, {
        provider: name,
        providers: [result],
        checked_at: NOW,
        stale: false,
        error: null,
      });
    }
    case "ai.list_models":
      if (args.provider === "omlx") {
        return {
          kind: "error",
          schema_version: 1,
          error: { code: "ai_unavailable", message: "Local model server unavailable", retryable: true },
        };
      }
      return envelope(request.kind, {
        provider: args.provider,
        models: [{ id: "qwen3:8b" }, { id: "gemma3:4b" }],
        checked_at: NOW,
        stale: false,
        error: null,
      });
    default:
      throw new Error(`unexpected daemon call ${request.kind}`);
  }
}

const BASE_PROVIDERS: Provider[] = [
  {
    name: "ollama",
    display_name: "Ollama",
    base_url: "http://localhost:11434/v1",
    kind: "local",
    default_model: null,
    acknowledged_at: NOW,
    has_api_key: false,
    secret_ref: { store_id: "sqlcipher_inline", state: "missing" },
    is_default: true,
    enabled: true,
    default_reasoning_effort: null,
  },
  ...(
    [
      ["claude", "Claude", "claude-cli://default"],
      ["codex", "Codex", "codex-cli://default"],
      ["copilot", "GitHub Copilot", "copilot-cli://default"],
      ["opencode", "OpenCode", "opencode-cli://default"],
    ] as const
  ).map(([name, display_name, base_url]) => ({
    name,
    display_name,
    base_url,
    kind: "remote",
    default_model: "default",
    acknowledged_at: name === "copilot" ? null : NOW,
    has_api_key: false,
    secret_ref: { store_id: "sqlcipher_inline", state: "missing" },
    is_default: false,
    enabled: true,
    default_reasoning_effort: name === "claude" ? "high" : null,
  })),
];

function requests(): Request[] {
  return mocks.invoke.mock.calls.map(([request]) => request as Request);
}

function probes(): Request[] {
  return requests().filter(
    (request) =>
      request.kind === "ai.list_models" ||
      (request.kind === "ai.provider_runtime.status" && !request.args?.cached_only),
  );
}

function card(name: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(`[data-ai-provider-card="${name}"]`);
  if (!element) throw new Error(`no card for ${name}`);
  return element;
}

async function mount(aiFeaturesEnabled = true, seed?: (client: QueryClient) => void) {
  // Library defaults (retry included), so a check that retried would show.
  const client = new QueryClient();
  seed?.(client);
  render(
    <QueryClientProvider client={client}>
      <AiProvidersSettingsPanel
        aiFeaturesEnabled={aiFeaturesEnabled}
        setAiFeaturesEnabled={vi.fn()}
      />
    </QueryClientProvider>,
  );
  await screen.findByText("GitHub Copilot");
  // Let the cached snapshot reads settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return client;
}

describe("AiProvidersSettingsPanel (mounted)", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (request: Request) => daemon(request));
    mocks.confirm.mockReset();
    mocks.providers = BASE_PROVIDERS.map((provider) => ({ ...provider }));
    mocks.offline = false;
    mocks.cachedRuntime = {};
    mocks.probeResult = {};
    mocks.probeError = {};
    mocks.ackError = null;
    mocks.probeStale = {};
  });
  afterEach(() => {
    cleanup();
  });

  it("contacts no provider when it opens", async () => {
    await mount();
    expect(probes()).toEqual([]);
    const kinds = new Set(requests().map((request) => request.kind));
    expect([...kinds].sort()).toEqual([
      "ai.provider_runtime.status",
      "ai.providers.list",
      "ui.network.offline",
    ]);
    for (const request of requests()) {
      if (request.kind === "ai.provider_runtime.status") {
        expect(request.args).toEqual({ provider: expect.any(String), cached_only: true });
      }
    }
    expect(within(card("copilot")).getByText("Not checked yet")).toBeTruthy();
  });

  it("asks nothing at all while AI features are off", async () => {
    await mount(false);
    expect(requests().map((request) => request.kind)).not.toContain(
      "ai.provider_runtime.status",
    );
    const check = within(card("claude")).getByRole("button", { name: "Check Claude" });
    expect((check as HTMLButtonElement).disabled).toBe(true);
  });

  it("checks only that CLI provider, after the off-device acknowledgement", async () => {
    mocks.confirm.mockReturnValue(true);
    mocks.probeResult.copilot = {
      provider: "copilot",
      display_name: "GitHub Copilot",
      state: "authentication_required",
      version: "1.0.4",
      message: "Run `copilot login` outside Kassiber.",
      models: [],
    };
    await mount();
    await act(async () => {
      fireEvent.click(within(card("copilot")).getByRole("button", { name: "Check GitHub Copilot" }));
    });
    await waitFor(() =>
      expect(within(card("copilot")).getByText(/Sign-in needed/)).toBeTruthy(),
    );

    expect(mocks.confirm).toHaveBeenCalledOnce();
    const ackIndex = requests().findIndex((r) => r.kind === "ai.providers.acknowledge");
    const probeIndex = requests().findIndex(
      (r) => r.kind === "ai.provider_runtime.status" && r.args?.refresh === true,
    );
    expect(requests()[ackIndex].args).toEqual({ name: "copilot" });
    expect(ackIndex).toBeLessThan(probeIndex);
    expect(probes()).toEqual([
      { kind: "ai.provider_runtime.status", args: { provider: "copilot", refresh: true } },
    ]);
    const status = within(card("copilot"));
    expect(status.getByText("Installed v1.0.4 · Sign-in needed")).toBeTruthy();
    expect(status.getByText("copilot login").tagName).toBe("CODE");
  });

  it("reports a failed acknowledgement on the card instead of rejecting", async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (reason: unknown) => void unhandled.push(reason);
    process.on("unhandledRejection", onUnhandled);
    try {
      mocks.confirm.mockReturnValue(true);
      mocks.ackError = "The book is not available.";
      await mount();
      await act(async () => {
        fireEvent.click(within(card("copilot")).getByRole("button", { name: "Check GitHub Copilot" }));
      });
      await waitFor(() =>
        expect(within(card("copilot")).getByText("The book is not available.")).toBeTruthy(),
      );
      expect(within(card("copilot")).queryByText("Not checked yet")).toBeNull();
      expect(probes()).toEqual([]);
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });

  it("still checks the acknowledged providers when Check all cannot acknowledge one", async () => {
    mocks.confirm.mockReturnValue(true);
    mocks.ackError = "The book is not available.";
    for (const name of ["claude", "codex", "opencode"]) {
      mocks.probeResult[name] = { provider: name, state: "ready", message: "Ready.", models: [] };
    }
    await mount();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Check all" }));
    });
    await waitFor(() =>
      expect(within(card("copilot")).getByText("The book is not available.")).toBeTruthy(),
    );
    await waitFor(() => expect(probes()).toHaveLength(4));
    expect(probes().some((r) => r.args?.provider === "copilot")).toBe(false);
  });

  it("does not check when the acknowledgement is declined", async () => {
    mocks.confirm.mockReturnValue(false);
    await mount();
    await act(async () => {
      fireEvent.click(within(card("copilot")).getByRole("button", { name: "Check GitHub Copilot" }));
    });
    expect(mocks.confirm).toHaveBeenCalledOnce();
    expect(requests().map((r) => r.kind)).not.toContain("ai.providers.acknowledge");
    expect(probes()).toEqual([]);
  });

  it("refuses off-device checks in offline mode before asking anything", async () => {
    mocks.offline = true;
    await mount();
    await act(async () => {
      fireEvent.click(within(card("claude")).getByRole("button", { name: "Check Claude" }));
    });
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(probes()).toEqual([]);
    expect(
      within(card("claude")).getByText(
        "Offline mode is on. Only providers marked local can be checked.",
      ),
    ).toBeTruthy();
  });

  it("mentions no fast mode for a CLI that has not been checked", async () => {
    await mount();
    expect(within(card("claude")).queryByText(/Fast mode is available/)).toBeNull();
  });

  it("shows a CLI's models from the picker's Check models without probing", async () => {
    const { dataMode, daemonSession } = useUiStore.getState();
    // What the picker's Check models leaves in the shared query for Claude.
    const seed = (client: QueryClient) =>
      client.setQueryData(
        providerModelsQueryKey(dataMode, daemonSession, {
          name: "claude",
          base_url: "claude-cli://default",
        }),
        envelope("ai.list_models", {
          provider: "claude",
          models: [
            { id: "opus", supports_fast_mode: true },
            { id: "sonnet" },
          ],
          checked_at: NOW,
          stale: false,
          error: null,
        }),
      );
    await mount(true, seed);
    const claude = within(card("claude"));
    expect(claude.getByText("Models listed · 2 models")).toBeTruthy();
    expect(claude.getByText(/Fast mode is available/)).toBeTruthy();
    expect(claude.queryByText("Not checked yet")).toBeNull();
    expect(probes()).toEqual([]);
  });

  it("lets a newer not-ready runtime check outrank an older model list", async () => {
    const { dataMode, daemonSession } = useUiStore.getState();
    mocks.cachedRuntime.claude = {
      provider: "claude",
      state: "authentication_required",
      message: "Run `claude login` outside Kassiber.",
      models: [],
    };
    const seed = (client: QueryClient) =>
      client.setQueryData(
        providerModelsQueryKey(dataMode, daemonSession, {
          name: "claude",
          base_url: "claude-cli://default",
        }),
        envelope("ai.list_models", {
          provider: "claude",
          models: [{ id: "opus", supports_fast_mode: true }],
          checked_at: "2026-10-09T09:00:00Z",
          stale: false,
          error: null,
        }),
      );
    await mount(true, seed);
    const claude = within(card("claude"));
    await waitFor(() => expect(claude.getByText("Sign-in needed")).toBeTruthy());
    expect(claude.queryByText(/Fast mode is available/)).toBeNull();
    expect(probes()).toEqual([]);
  });

  it("renders each runtime state from the daemon's cached snapshot", async () => {
    mocks.cachedRuntime = {
      claude: { provider: "claude", state: "ready", version: "2.1.0", message: "Ready.", models: [] },
      codex: {
        provider: "codex",
        state: "missing_executable",
        message: "Install Codex, then run `codex login` outside Kassiber.",
        models: [],
      },
      copilot: {
        provider: "copilot",
        state: "authentication_required",
        message: "Run `copilot login` outside Kassiber.",
        models: [],
      },
      opencode: { provider: "opencode", state: "error", message: "OpenCode crashed.", models: [] },
    };
    await mount();
    await waitFor(() =>
      expect(within(card("claude")).getByText("Installed v2.1.0 · Signed in")).toBeTruthy(),
    );
    expect(within(card("codex")).getByText("Not installed")).toBeTruthy();
    expect(within(card("copilot")).getByText("Sign-in needed")).toBeTruthy();
    expect(within(card("opencode")).getByText("Error")).toBeTruthy();
    expect(within(card("opencode")).getByText(/OpenCode crashed\./)).toBeTruthy();
    expect(within(card("claude")).getAllByText(/checked/).length).toBeGreaterThan(0);
    expect(probes()).toEqual([]);
  });

  it("hides key rows for CLI agents and mentions fast mode only where it exists", async () => {
    // Only a checked model row says so: the broker gates fast mode on the
    // installed CLI's version.
    mocks.cachedRuntime = {
      claude: {
        provider: "claude",
        state: "ready",
        version: "2.1.287 (Claude Code)",
        message: "Ready.",
        models: [{ id: "opus", supports_fast_mode: true }],
      },
    };
    await mount();
    const claude = within(card("claude"));
    expect(claude.queryByText("Key storage")).toBeNull();
    expect(claude.getByText(/Fast mode is available/)).toBeTruthy();
    expect(within(card("copilot")).queryByText(/Fast mode is available/)).toBeNull();
    expect(within(card("ollama")).getByText("Key storage")).toBeTruthy();
    // The stored default effort is shown on the card.
    expect(claude.getByText("High")).toBeTruthy();
  });

  it("checks an HTTP provider through its model list", async () => {
    await mount();
    await act(async () => {
      fireEvent.click(within(card("ollama")).getByRole("button", { name: "Check Ollama" }));
    });
    await waitFor(() =>
      expect(within(card("ollama")).getByText("Reachable · 2 models")).toBeTruthy(),
    );
    // Local providers need no acknowledgement.
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(probes()).toEqual([
      { kind: "ai.list_models", args: { provider: "ollama", refresh: true } },
    ]);
  });

  it("checks every enabled provider one by one with a single confirmation", async () => {
    mocks.confirm.mockReturnValue(true);
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "opencode" ? { ...provider, enabled: false } : provider,
    );
    for (const name of ["claude", "codex", "copilot"]) {
      mocks.probeResult[name] = { provider: name, state: "ready", message: "Ready.", models: [] };
    }
    await mount();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Check all" }));
    });
    await waitFor(() => expect(probes()).toHaveLength(4));
    // Only Copilot lacked the acknowledgement.
    expect(mocks.confirm).toHaveBeenCalledOnce();
    expect(mocks.confirm.mock.calls[0][0]).toContain("GitHub Copilot");
    expect(probes()).toEqual(
      expect.arrayContaining([
        { kind: "ai.list_models", args: { provider: "ollama", refresh: true } },
        { kind: "ai.provider_runtime.status", args: { provider: "claude", refresh: true } },
        { kind: "ai.provider_runtime.status", args: { provider: "codex", refresh: true } },
        { kind: "ai.provider_runtime.status", args: { provider: "copilot", refresh: true } },
      ]),
    );
    // A switched-off provider is skipped, and nothing asks for every runtime.
    expect(probes().some((r) => r.args?.provider === "opencode")).toBe(false);
    expect(probes().every((r) => typeof r.args?.provider === "string")).toBe(true);
  });

  it("shows an edited endpoint as not checked, without checking it", async () => {
    const client = await mount();
    await act(async () => {
      fireEvent.click(within(card("ollama")).getByRole("button", { name: "Check Ollama" }));
    });
    await waitFor(() =>
      expect(within(card("ollama")).getByText("Reachable · 2 models")).toBeTruthy(),
    );

    // The URL is edited (the form saves it; the provider list refetches).
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "ollama"
        ? { ...provider, base_url: "http://localhost:11999/v1" }
        : provider,
    );
    await act(async () => {
      await client.invalidateQueries({ queryKey: ["daemon"] });
    });
    await waitFor(() =>
      expect(within(card("ollama")).getByText("Not checked yet")).toBeTruthy(),
    );
    expect(within(card("ollama")).queryByText(/Reachable/)).toBeNull();
    expect(probes()).toEqual([
      { kind: "ai.list_models", args: { provider: "ollama", refresh: true } },
    ]);
  });

  it("shows a CLI row moved to another runtime as that runtime's result, without a probe", async () => {
    mocks.probeResult.codex = {
      provider: "codex",
      state: "ready",
      version: "0.9.0",
      message: "Ready.",
      models: [{ id: "gpt-5.4" }],
    };
    const client = await mount();
    await act(async () => {
      fireEvent.click(within(card("codex")).getByRole("button", { name: "Check Codex" }));
    });
    await waitFor(() =>
      expect(within(card("codex")).getByText("Installed v0.9.0 · Signed in")).toBeTruthy(),
    );

    // Pointed at the Claude CLI from `kassiber ai providers update`; the
    // desktop only refetches its provider list.
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "codex" ? { ...provider, base_url: "claude-cli://default" } : provider,
    );
    await act(async () => {
      await client.invalidateQueries({
        predicate: (query) => query.queryKey.includes("ai.providers.list"),
      });
    });
    await waitFor(() =>
      expect(within(card("codex")).getByText("Not checked yet")).toBeTruthy(),
    );
    expect(within(card("codex")).queryByText(/0\.9\.0/)).toBeNull();
    expect(probes()).toEqual([
      { kind: "ai.provider_runtime.status", args: { provider: "codex", refresh: true } },
    ]);
  });

  it("drops the picker's models once a CLI check stops reporting ready", async () => {
    mocks.probeResult.codex = {
      provider: "codex",
      state: "ready",
      version: "0.9.0",
      message: "Ready.",
      models: [{ id: "gpt-5.4", supports_fast_mode: true }],
    };
    const client = await mount();
    const { dataMode, daemonSession } = useUiStore.getState();
    const pickerKey = providerModelsQueryKey(dataMode, daemonSession, {
      name: "codex",
      base_url: "codex-cli://default",
    });
    const pickerModels = () =>
      (client.getQueryData(pickerKey) as { data?: { models?: unknown[] } } | undefined)
        ?.data?.models;
    const checkCodex = async () => {
      await act(async () => {
        fireEvent.click(within(card("codex")).getByRole("button", { name: "Check Codex" }));
      });
    };

    await checkCodex();
    await waitFor(() => expect(pickerModels()).toHaveLength(1));

    for (const state of ["authentication_required", "missing_executable", "error"]) {
      await act(async () => {
        client.setQueryData(pickerKey, {
          kind: "ai.list_models",
          schema_version: 1,
          data: { provider: "codex", models: [{ id: "gpt-5.4" }], checked_at: NOW, stale: false, error: null },
        });
      });
      mocks.probeResult.codex = { provider: "codex", state, message: "Not ready.", models: [] };
      await checkCodex();
      await waitFor(() => expect(pickerModels()).toBeUndefined());
    }
    // Clearing the picker's copy is not a probe of its own.
    expect(probes().every((r) => r.kind === "ai.provider_runtime.status")).toBe(true);
    expect(probes()).toHaveLength(4);
  });

  it("keeps a failed refresh's stale marking on the picker's copy of a CLI's models", async () => {
    mocks.probeResult.codex = {
      provider: "codex",
      state: "ready",
      version: "0.9.0",
      message: "Ready.",
      models: [{ id: "gpt-5.4", supports_fast_mode: true }],
    };
    const client = await mount();
    const { dataMode, daemonSession } = useUiStore.getState();
    const pickerKey = providerModelsQueryKey(dataMode, daemonSession, {
      name: "codex",
      base_url: "codex-cli://default",
    });
    type Listed = { data?: { stale?: boolean; error?: { message?: string } | null } };
    const listed = () => client.getQueryData(pickerKey) as Listed | undefined;
    const checkCodex = async () => {
      await act(async () => {
        fireEvent.click(within(card("codex")).getByRole("button", { name: "Check Codex" }));
      });
    };

    await checkCodex();
    await waitFor(() => expect(listed()?.data?.stale).toBe(false));

    mocks.probeStale.codex = "Codex CLI timed out.";
    await checkCodex();
    await waitFor(() => expect(listed()?.data?.stale).toBe(true));
    expect(listed()?.data?.error?.message).toBe("Codex CLI timed out.");
  });

  it("does not show a failed check's error on an endpoint it never checked", async () => {
    mocks.probeError.codex = "Codex CLI crashed.";
    const client = await mount();
    await act(async () => {
      fireEvent.click(within(card("codex")).getByRole("button", { name: "Check Codex" }));
    });
    await waitFor(() =>
      expect(within(card("codex")).getByText("Codex CLI crashed.")).toBeTruthy(),
    );

    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "codex" ? { ...provider, base_url: "claude-cli://default" } : provider,
    );
    await act(async () => {
      await client.invalidateQueries({
        predicate: (query) => query.queryKey.includes("ai.providers.list"),
      });
    });
    await waitFor(() =>
      expect(within(card("codex")).getByText("Not checked yet")).toBeTruthy(),
    );
    expect(within(card("codex")).queryByText("Codex CLI crashed.")).toBeNull();
  });

  it("does not carry a failed check's error into another book", async () => {
    mocks.probeError.claude = "Claude CLI crashed.";
    await mount();
    await act(async () => {
      fireEvent.click(within(card("claude")).getByRole("button", { name: "Check Claude" }));
    });
    await waitFor(() =>
      expect(within(card("claude")).getByText("Claude CLI crashed.")).toBeTruthy(),
    );

    // Another book with a `claude` row of its own.
    await act(async () => {
      useUiStore.getState().bumpDaemonSession();
    });
    await waitFor(() =>
      expect(within(card("claude")).getByText("Not checked yet")).toBeTruthy(),
    );
    expect(within(card("claude")).queryByText("Claude CLI crashed.")).toBeNull();
  });

  it("cannot check a switched-off provider", async () => {
    mocks.providers = mocks.providers.map((provider) =>
      provider.name === "codex" || provider.name === "ollama"
        ? { ...provider, enabled: false, is_default: false }
        : provider,
    );
    await mount();
    for (const [name, label] of [
      ["codex", "Check Codex"],
      ["ollama", "Check Ollama"],
    ] as const) {
      const check = within(card(name)).getByRole("button", { name: label });
      expect((check as HTMLButtonElement).disabled).toBe(true);
      expect(check.getAttribute("title")).toBe("Disabled. Turn it on to check it.");
      await act(async () => {
        fireEvent.click(check);
      });
    }
    // Enabled providers keep their check.
    const claude = within(card("claude")).getByRole("button", { name: "Check Claude" });
    expect((claude as HTMLButtonElement).disabled).toBe(false);
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(probes()).toEqual([]);
  });

  it("reports a failed check once instead of asking again", async () => {
    mocks.providers.push({
      ...mocks.providers[0],
      name: "omlx",
      display_name: "oMLX",
      base_url: "http://127.0.0.1:8000/v1",
      is_default: false,
    });
    await mount();
    await act(async () => {
      fireEvent.click(within(card("omlx")).getByRole("button", { name: "Check oMLX" }));
    });
    await waitFor(() =>
      expect(within(card("omlx")).getByText("Local model server unavailable")).toBeTruthy(),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1500));
    });
    expect(probes()).toEqual([
      { kind: "ai.list_models", args: { provider: "omlx", refresh: true } },
    ]);
  });

  it("switches a provider off through the daemon, but never the default", async () => {
    await mount();
    const ollamaSwitch = within(card("ollama")).getByRole("switch");
    expect((ollamaSwitch as HTMLButtonElement).disabled).toBe(true);
    await act(async () => {
      fireEvent.click(within(card("codex")).getByRole("switch", { name: "Offer Codex in chat" }));
    });
    await waitFor(() =>
      expect(requests()).toContainEqual({
        kind: "ai.providers.update",
        args: { name: "codex", enabled: false },
      }),
    );
    expect(probes()).toEqual([]);
  });
});
