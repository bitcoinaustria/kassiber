// @vitest-environment happy-dom
//
// Table sorts survive navigation and restarts: they live in the persisted UI
// store, an older stored blob without them still loads, and a stored sort a
// table no longer offers falls back to that table's default.
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import { normalizeTableSorts } from "./tableSort";
import { migrateUiState, uiStatePartialForStorage, useUiStore } from "./ui";
import { useRememberedTableSort } from "./useRememberedTableSort";

const WALLET_KEYS = ["label", "kind", "balance"] as const;
const WALLET_DEFAULT = { key: "kind", direction: "asc" } as const;

afterEach(() => {
  cleanup();
  useUiStore.setState({ tableSorts: {} });
});

describe("normalizeTableSorts", () => {
  it("keeps known tables with well-formed sorts and explicit nulls", () => {
    expect(
      normalizeTableSorts({
        wallets: { key: "balance", direction: "desc" },
        transactions: null,
        review: { key: "date", direction: "sideways" },
        unknownTable: { key: "x", direction: "asc" },
      }),
    ).toEqual({
      wallets: { key: "balance", direction: "desc" },
      transactions: null,
    });
    expect(normalizeTableSorts("garbage")).toEqual({});
    expect(normalizeTableSorts([{ key: "a", direction: "asc" }])).toEqual({});
  });
});

describe("persisted table sorts", () => {
  it("are part of the stored UI state", () => {
    useUiStore.getState().setTableSort("wallets", { key: "balance", direction: "desc" });
    expect(uiStatePartialForStorage(useUiStore.getState()).tableSorts).toEqual({
      wallets: { key: "balance", direction: "desc" },
    });
  });

  it("restore from storage and load older blobs that never had them", () => {
    const merge = useUiStore.persist.getOptions().merge!;
    const current = useUiStore.getState();
    const restored = merge(
      migrateUiState(
        { tableSorts: { review: { key: "date", direction: "asc" } } },
        2,
      ),
      current,
    );
    expect(restored.tableSorts).toEqual({ review: { key: "date", direction: "asc" } });
    expect(merge(migrateUiState({ theme: "light" }, 2), current).tableSorts).toEqual({});
  });

  it("does not rewrite the store when the sort is unchanged", () => {
    useUiStore.getState().setTableSort("review", { key: "date", direction: "asc" });
    const before = useUiStore.getState().tableSorts;
    useUiStore.getState().setTableSort("review", { key: "date", direction: "asc" });
    expect(useUiStore.getState().tableSorts).toBe(before);
  });
});

describe("useRememberedTableSort", () => {
  it("starts from the default and remembers a change", () => {
    const first = renderHook(() =>
      useRememberedTableSort("wallets", WALLET_KEYS, WALLET_DEFAULT),
    );
    expect(first.result.current[0]).toEqual(WALLET_DEFAULT);
    act(() => first.result.current[1]({ key: "balance", direction: "desc" }));
    first.unmount();

    // A later mount (another visit to the page) picks the sort up again.
    const second = renderHook(() =>
      useRememberedTableSort("wallets", WALLET_KEYS, WALLET_DEFAULT),
    );
    expect(second.result.current[0]).toEqual({ key: "balance", direction: "desc" });
  });

  it("falls back when the stored key is no longer offered", () => {
    useUiStore.setState({
      tableSorts: { wallets: { key: "retiredColumn", direction: "asc" } },
    });
    const { result } = renderHook(() =>
      useRememberedTableSort("wallets", WALLET_KEYS, WALLET_DEFAULT),
    );
    expect(result.current[0]).toEqual(WALLET_DEFAULT);
  });

  it("keeps an explicitly stored 'unsorted'", () => {
    useUiStore.setState({ tableSorts: { transactions: null } });
    const { result } = renderHook(() =>
      useRememberedTableSort("transactions", ["date", "amount"] as const, {
        key: "date",
        direction: "desc",
      }),
    );
    expect(result.current[0]).toBeNull();
  });
});
