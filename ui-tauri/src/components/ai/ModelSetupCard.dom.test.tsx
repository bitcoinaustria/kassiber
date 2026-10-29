// @vitest-environment happy-dom
//
// The empty chat's model setup: it contacts no agent on its own, checks one
// only on a click (after the off-device confirmation), and makes a ready one
// the default and this chat's model. One that is not ready says why.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

type Request = { kind: string; args?: Record<string, unknown> };

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  confirm: vi.fn(),
  runtime: {} as Record<string, Record<string, unknown>>,
}));

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

vi.mock("@/daemon/transport", () => ({
  getTransport: () => ({ invoke: mocks.invoke }),
}));
vi.mock("@/lib/confirmAction", () => ({
  confirmAction: (message: string) => Promise.resolve(mocks.confirm(message)),
}));

import { useUiStore } from "@/store/ui";

import { ModelSetupCard } from "./ModelSetupCard";

const NOW = "2026-10-29T10:00:00Z";
const agent = (name: string, display_name: string) => ({
  name,
  display_name,
  base_url: `${name}-cli://default`,
  kind: "remote",
  default_model: "default",
  acknowledged_at: null,
  has_api_key: false,
  is_default: false,
  enabled: name !== "codex",
});
const PROVIDERS = [
  { ...agent("ollama", "Ollama"), base_url: "http://localhost:11434/v1", kind: "local", is_default: true },
  agent("claude", "Claude"),
  agent("codex", "Codex"),
];

function envelope(kind: string, data: unknown) {
  return { kind, schema_version: 1, data };
}

function daemon(request: Request) {
  const args = request.args ?? {};
  switch (request.kind) {
    case "ai.providers.list":
      return envelope(request.kind, { default: "ollama", providers: PROVIDERS });
    case "ui.network.offline":
      return envelope(request.kind, { offline: false, environment_blocked: false });
    case "ai.providers.acknowledge":
    case "ai.providers.update":
    case "ai.providers.set_default":
      return envelope(request.kind, { name: args.name });
    case "ai.provider_runtime.status": {
      const result = args.cached_only ? undefined : mocks.runtime[String(args.provider)];
      return envelope(request.kind, {
        provider: args.provider,
        providers: result ? [result] : [],
        checked_at: result ? NOW : null,
        stale: false,
        error: null,
      });
    }
    case "ai.list_models":
      return envelope(request.kind, { provider: args.provider, models: [], checked_at: null, stale: false, error: null });
    default:
      throw new Error(`unexpected daemon call ${request.kind}`);
  }
}

const requests = () => mocks.invoke.mock.calls.map(([request]) => request as Request);
const probes = () =>
  requests().filter(
    (request) =>
      (request.kind === "ai.provider_runtime.status" || request.kind === "ai.list_models") &&
      !request.args?.cached_only,
  );

async function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <ModelSetupCard />
    </QueryClientProvider>,
  );
  await screen.findByText("Claude");
}

const row = (name: string) =>
  document.querySelector<HTMLElement>(`[data-model-setup-row="${name}"]`)!;

describe("ModelSetupCard", () => {
  beforeEach(() => {
    mocks.invoke.mockImplementation((request: Request) => Promise.resolve(daemon(request)));
    mocks.confirm.mockReturnValue(true);
    mocks.runtime = {};
    useUiStore.setState({ aiFeaturesEnabled: true, assistantModelSelection: null });
  });
  afterEach(() => {
    cleanup();
    mocks.invoke.mockReset();
    mocks.confirm.mockReset();
  });

  it("lists the coding agents and contacts none of them on its own", async () => {
    await mount();
    expect(screen.getByText("Codex")).toBeTruthy();
    // HTTP providers are set up in Settings, not here.
    expect(screen.queryByText("Ollama")).toBeNull();
    expect(probes()).toEqual([]);
  });

  it("makes a ready agent the default and this chat's model", async () => {
    mocks.runtime.claude = { provider: "claude", state: "ready", version: "2.1.0" };
    await mount();
    await act(async () => {
      fireEvent.click(within(row("claude")).getByRole("button", { name: "Use Claude" }));
    });
    await waitFor(() =>
      expect(useUiStore.getState().assistantModelSelection).toEqual({ provider: "claude", model: "default" }),
    );
    expect(mocks.confirm).toHaveBeenCalledOnce();
    expect(requests()).toContainEqual({ kind: "ai.providers.set_default", args: { name: "claude" } });
    // Only that agent was started.
    expect(probes().map((request) => request.args?.provider)).toEqual(["claude"]);
  });

  it("switches a switched-off agent on before checking it", async () => {
    mocks.runtime.codex = { provider: "codex", state: "ready" };
    await mount();
    await act(async () => {
      fireEvent.click(within(row("codex")).getByRole("button", { name: "Use Codex" }));
    });
    await waitFor(() => expect(useUiStore.getState().assistantModelSelection?.provider).toBe("codex"));
    const kinds = requests().map((request) => request.kind);
    expect(kinds.indexOf("ai.providers.update")).toBeLessThan(
      requests().findIndex((request) => request.kind === "ai.provider_runtime.status" && !request.args?.cached_only),
    );
  });

  it("says why an agent is not ready and chooses nothing", async () => {
    mocks.runtime.claude = {
      provider: "claude",
      state: "authentication_required",
      message: "Run `claude login` outside Kassiber.",
    };
    await mount();
    await act(async () => {
      fireEvent.click(within(row("claude")).getByRole("button", { name: "Use Claude" }));
    });
    await waitFor(() => expect(within(row("claude")).getByText(/Sign-in needed/)).toBeTruthy());
    expect(within(row("claude")).getByText("claude login").tagName).toBe("CODE");
    expect(within(row("claude")).getByRole("button", { name: "Use Claude" }).textContent).toBe("Check again");
    expect(useUiStore.getState().assistantModelSelection).toBeNull();
    expect(requests().some((request) => request.kind === "ai.providers.set_default")).toBe(false);
  });

  it("does not check when the confirmation is declined", async () => {
    mocks.confirm.mockReturnValue(false);
    await mount();
    await act(async () => {
      fireEvent.click(within(row("claude")).getByRole("button", { name: "Use Claude" }));
    });
    expect(probes()).toEqual([]);
    expect(useUiStore.getState().assistantModelSelection).toBeNull();
  });
});
