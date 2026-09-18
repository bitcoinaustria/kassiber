// @vitest-environment happy-dom
//
// Mounted, not static. Every other test in this route renders with
// `renderToStaticMarkup`, where effects never run -- which is exactly how two
// lifecycle bugs in the auto-assembly effect shipped past the suite. This file
// mounts the case hook in a real DOM so the effect actually fires.
import { act, renderHook } from "@testing-library/react";
import { createContext } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSourceFundsCase } from "./useSourceFundsCase";

type Envelope<T> = { kind: string; data: T };

// The UI store persists through `localStorage`, which this DOM does not wire
// up as a global, and zustand resolves its storage while the store module is
// being evaluated. Hoisted so it precedes the imports above.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const state = vi.hoisted(() => ({
  preview: { data: undefined as Envelope<unknown> | undefined, isFetching: false },
  mutations: new Map<string, { mutateAsync: ReturnType<typeof vi.fn>; isPending: boolean; reset: () => void }>(),
}));

function mutation(kind: string) {
  let entry = state.mutations.get(kind);
  if (!entry) {
    entry = { mutateAsync: vi.fn().mockResolvedValue({ kind, data: {} }), isPending: false, reset: () => {} };
    state.mutations.set(kind, entry);
  }
  return entry;
}

vi.mock("@/daemon/client", () => ({
  DaemonScopeContext: createContext(null),
  useDaemon: (kind: string) =>
    kind === "ui.source_funds.review_context"
      ? { ...state.preview, refetch: vi.fn() }
      : { data: undefined, isFetching: false, refetch: vi.fn() },
  useDaemonInfinite: () => ({ data: { pages: [] }, isFetching: false, hasNextPage: false, fetchNextPage: vi.fn() }),
  useDaemonMutation: (kind: string) => mutation(kind),
}));

function packet(targetId: string, edges: number) {
  return {
    kind: "ui.source_funds.review_context",
    data: {
      schema_version: 1,
      workspace_id: "ws",
      profile_id: "profile",
      input_version: 7,
      review_fingerprint: "fp",
      case_id: "case",
      target: { transaction_id: targetId, wallet_id: "w", direction: "outbound", asset: "BTC", occurred_at: "2026-01-01T00:00:00Z" },
      recipe: {},
      report: { graph: { nodes: [], edges: Array.from({ length: edges }, (_, i) => ({ id: `e${i}` })) }, findings: [], sources: [], explain_gates: { blockers: [], warnings: [] } },
      links: [],
      sources: [],
      evidence: [],
      input_needs: [],
    },
  };
}

const assemble = () => mutation("ui.source_funds.assemble").mutateAsync;

beforeEach(() => {
  state.preview = { data: undefined, isFetching: false };
  state.mutations.clear();
});

describe("chain evidence assembles itself, once, and never under a draft", () => {
  it("assembles a case that has no reviewed history exactly once", async () => {
    state.preview = { data: packet("tx-fresh", 0), isFetching: false };
    const hook = renderHook(() => useSourceFundsCase("profile:fresh", "tx-fresh"));
    await act(async () => {});
    expect(assemble()).toHaveBeenCalledTimes(1);
    expect(assemble()).toHaveBeenCalledWith({ target_transaction: "tx-fresh" });
    // A refetch of the same packet is not a new case.
    hook.rerender();
    await act(async () => {});
    expect(assemble()).toHaveBeenCalledTimes(1);
  });

  it("leaves a case with reviewed edges alone", async () => {
    state.preview = { data: packet("tx-owned", 3), isFetching: false };
    renderHook(() => useSourceFundsCase("profile:owned", "tx-owned"));
    await act(async () => {});
    expect(assemble()).not.toHaveBeenCalled();
  });

  it("waits while the user is mid-decision, then fires once the draft is gone", async () => {
    const hook = renderHook(() => useSourceFundsCase("profile:draft", "tx-draft"));
    await act(async () => {});
    // The user started documenting an origin before the packet arrived.
    act(() => hook.result.current.setSourceForm((current) => ({ ...current, label: "Exchange sale" })));
    state.preview = { data: packet("tx-draft", 0), isFetching: false };
    hook.rerender();
    await act(async () => {});
    expect(assemble()).not.toHaveBeenCalled();
    // Draft cleared: nothing authored is at risk any more.
    act(() => hook.result.current.setSourceForm((current) => ({ ...current, label: "" })));
    await act(async () => {});
    expect(assemble()).toHaveBeenCalledTimes(1);
  });

  it("does not assemble while the packet is still loading", async () => {
    state.preview = { data: packet("tx-loading", 0), isFetching: true };
    const hook = renderHook(() => useSourceFundsCase("profile:loading", "tx-loading"));
    await act(async () => {});
    expect(assemble()).not.toHaveBeenCalled();
    state.preview = { data: packet("tx-loading", 0), isFetching: false };
    hook.rerender();
    await act(async () => {});
    expect(assemble()).toHaveBeenCalledTimes(1);
  });
});
