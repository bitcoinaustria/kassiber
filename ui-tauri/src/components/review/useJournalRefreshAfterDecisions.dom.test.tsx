// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => undefined, removeItem: () => undefined });
});

import "@/i18n";
import { useUiStore } from "@/store/ui";

const daemon = vi.hoisted(() => ({ streams: [] as Array<{ kind: string; args?: Record<string, unknown> }> }));
vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return { ...actual, getTransport: () => ({
    invoke: async ({ kind }: { kind: string }) => ({ kind, schema_version: 1, data: { activeWorkspaceId: "ws", activeProfileId: "book" } }),
    stream: async (request: { kind: string; args?: Record<string, unknown> }) => {
      daemon.streams.push(request);
      return { kind: request.kind, schema_version: 1, data: {} };
    },
  }) };
});
import { useJournalRefreshAfterDecisions } from "./useJournalRefreshAfterDecisions";
let request: () => void = () => undefined;
function Probe() { request = useJournalRefreshAfterDecisions().requestRefresh; return null; }
async function mount() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(<QueryClientProvider client={client}><Probe /></QueryClientProvider>);
  await waitFor(() => expect(client.getQueryCache().getAll().some((q) => q.state.status === "success")).toBe(true));
  // Start fake timers only after the book identity has loaded.
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  return view;
}
beforeEach(() => { daemon.streams = []; useUiStore.setState({ daemonSession: 1 }); });
afterEach(() => { cleanup(); vi.clearAllTimers(); vi.useRealTimers(); });
const expected = { kind: "ui.journals.process", args: { auto_pair: false, expected_scope: { workspace_id: "ws", profile_id: "book" } } };
describe("journal refresh after review decisions", () => {
  it("coalesces decisions and processes only their originating book without auto pairing", async () => {
    await mount();
    act(() => { request(); request(); });
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(daemon.streams).toEqual([expected]);
  });
  it("recalculates when navigation precedes the queued run", async () => {
    const view = await mount();
    act(() => request());
    view.unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(daemon.streams).toEqual([expected]);
  });
  it("recalculates when the decision response arrives after unmount", async () => {
    const view = await mount();
    const completedDecision = request;
    view.unmount();
    completedDecision();
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(daemon.streams).toEqual([expected]);
  });
  it("abandons queued work after switching books", async () => {
    await mount();
    act(() => request());
    act(() => useUiStore.getState().bumpDaemonSession());
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(daemon.streams).toEqual([]);
  });
  it("does not retarget a decision response received after switching books", async () => {
    await mount();
    const completedDecision = request;
    act(() => useUiStore.getState().bumpDaemonSession());
    act(() => completedDecision());
    await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    expect(daemon.streams).toEqual([]);
  });
});
