// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DaemonEnvelope } from "@/daemon/transport";
import type { AiModelsListData, AiProviderRuntimeStatusData } from "@/lib/aiCapabilities";
import { useUiStore } from "@/store/ui";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});
vi.mock("@/daemon/transport", () => ({ getTransport: () => ({ invoke: mocks.invoke }) }));

import { providerModelsQueryKey, providerModelsQueryOptions, useCachedProviderModels,
  useRestoreProviderModelChecks } from "./providerModelQuery";

const OLD = "2026-10-30T12:00:00Z";
const NEW = "2026-11-01T12:00:00Z";
const LOCAL = { name: "ollama", base_url: "http://127.0.0.1:11434/v1", last_checked_at: NEW };
const CLI = { name: "codex", base_url: "codex-cli://default", last_checked_at: NEW };
type Target = typeof LOCAL;

function listing(at = OLD, id = "saved-local-model"): DaemonEnvelope<AiModelsListData> {
  return { kind: "ai.list_models", schema_version: 1,
    data: { provider: LOCAL.name, models: [{ id }], checked_at: at } };
}
function runtime(state: "ready" | "error", at = NEW): DaemonEnvelope<AiProviderRuntimeStatusData> {
  return { kind: "ai.provider_runtime.status", schema_version: 1, data: {
    checked_at: at, providers: [{ provider: "codex", display_name: "Codex", state,
      message: "Saved check", privacy_posture: "remote", native_tools: "disabled",
      models: [{ id: "saved-codex-model" }] }],
  } };
}
function mount(provider = LOCAL, enabled = true, client = new QueryClient()) {
  const wrapper = ({ children }: { children: ReactNode }) =>
    <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  const view = renderHook(({ target, active }: { target: Target; active: boolean }) => {
    useRestoreProviderModelChecks([target], active);
    return useCachedProviderModels(target);
  }, { wrapper, initialProps: { target: provider, active: enabled } });
  return { ...view, client };
}

describe("saved model inventory restoration", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    useUiStore.setState({ dataMode: "real", daemonSession: 100 });
  });
  afterEach(cleanup);

  it("restores local models into a fresh client using only a saved display read", async () => {
    mocks.invoke.mockResolvedValue(listing());
    const view = mount();
    await waitFor(() => expect(view.result.current.data?.data?.models[0]?.id).toBe("saved-local-model"));
    expect(mocks.invoke.mock.calls).toEqual([[{ kind: "ai.list_models",
      args: { provider: LOCAL.name, base_url: LOCAL.base_url, cached_only: true } }]]);
    expect(providerModelsQueryOptions("real", 100, LOCAL).gcTime).toBe(Infinity);
    // A new app's query client rehydrates from the durable daemon snapshot.
    view.unmount();
    const restarted = mount();
    await waitFor(() => expect(restarted.result.current.data?.data?.models).toEqual([{ id: "saved-local-model" }]));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("restores a CLI runtime-only check without launching the CLI", async () => {
    mocks.invoke.mockImplementation(async ({ kind }) => kind === "ai.list_models"
      ? { ...listing(), data: { provider: CLI.name, models: [], checked_at: null } }
      : runtime("ready"));
    const view = mount(CLI);
    await waitFor(() => expect(view.result.current.data?.data?.models[0]?.id).toBe("saved-codex-model"));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.invoke.mock.calls.every(([request]) => request.args.cached_only === true)).toBe(true);
  });

  it("lets a newer not-ready runtime clear an older listing", async () => {
    mocks.invoke.mockImplementation(async ({ kind }) => kind === "ai.list_models" ? listing() : runtime("error"));
    const view = mount(CLI);
    await waitFor(() => expect(view.result.current.data?.data?.checked_at).toBe(NEW));
    expect(view.result.current.data?.data?.models).toEqual([]);
  });

  it("prefers runtime readiness on tied timestamps regardless of response order", async () => {
    let resolve!: (value: DaemonEnvelope<AiProviderRuntimeStatusData>) => void;
    mocks.invoke.mockImplementation(({ kind }) => kind === "ai.list_models"
      ? Promise.resolve(listing(NEW)) : new Promise((done) => { resolve = done; }));
    const view = mount(CLI);
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));
    await act(async () => { resolve(runtime("error")); });
    await waitFor(() => expect(view.result.current.data?.data?.checked_at).toBe(NEW));
    expect(view.result.current.data?.data?.models).toEqual([]);
  });

  it("preserves a newer listing and its stale error over an older runtime", async () => {
    const stale = listing(NEW);
    stale.data = { ...stale.data!, stale: true, error: { code: "unavailable", message: "Last refresh failed" } };
    mocks.invoke.mockImplementation(async ({ kind }) => kind === "ai.list_models" ? stale : runtime("error", OLD));
    const view = mount(CLI);
    await waitFor(() => expect(view.result.current.data?.data).toEqual(stale.data));
  });

  it("does not let a delayed saved read overwrite a newer explicit check", async () => {
    let resolve!: (value: DaemonEnvelope<AiModelsListData>) => void;
    mocks.invoke.mockReturnValue(new Promise((done) => { resolve = done; }));
    const view = mount();
    const explicit = listing(NEW, "fresh-model");
    explicit.data = { ...explicit.data!, stale: true, error: { code: "unavailable", message: "Failed refresh" } };
    act(() => view.client.setQueryData(providerModelsQueryKey("real", 100, LOCAL), explicit));
    await act(async () => { resolve(listing()); });
    expect(view.client.getQueryData(providerModelsQueryKey("real", 100, LOCAL))).toEqual(explicit);
  });

  it("isolates endpoint edits and book sessions, including late old reads", async () => {
    let resolve!: (value: DaemonEnvelope<AiModelsListData>) => void;
    mocks.invoke.mockImplementation(({ args }) => args.base_url === LOCAL.base_url
      ? new Promise((done) => { resolve = done; }) : Promise.resolve(listing(NEW, "new-endpoint")));
    const view = mount();
    const target = { ...LOCAL, base_url: "http://127.0.0.1:1234/v1" };
    view.rerender({ target, active: true });
    await waitFor(() => expect(view.result.current.data?.data?.models[0]?.id).toBe("new-endpoint"));
    await act(async () => { resolve(listing()); });
    expect(view.result.current.data?.data?.models[0]?.id).toBe("new-endpoint");
    act(() => useUiStore.setState({ daemonSession: 101 }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(view.result.current.data?.data?.models[0]?.id).toBe("new-endpoint"));
    expect(view.client.getQueryData(providerModelsQueryKey("real", 101, LOCAL))).toBeUndefined();
  });

  it("does not read while disabled or for unchecked providers", () => {
    const disabled = mount(LOCAL, false);
    expect(mocks.invoke).not.toHaveBeenCalled();
    disabled.unmount();
    mount({ ...LOCAL, last_checked_at: "" });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
