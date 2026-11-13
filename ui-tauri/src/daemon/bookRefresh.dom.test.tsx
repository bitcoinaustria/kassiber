// @vitest-environment happy-dom
//
// A book refresh cycle keeps every mounted read on what it showed, refetches
// until the book's revision holds still across the reads, and then lets all
// of them change in the same render.
import {
  keepPreviousData,
  QueryClient,
  QueryClientProvider,
  useQuery,
  type QueryKey,
} from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  captureBookView,
  describeChange,
  MAX_ATTEMPTS,
  newTransactionIds,
  refreshBookQueries,
  resetBookRefreshForTests,
  useBookRefreshStore,
  useHeldQueryResult,
} from "./bookRefresh";

const OVERVIEW: QueryKey = ["daemon", "real", 0, "ui.overview.snapshot"];
const LIST: QueryKey = ["daemon", "real", 0, "ui.transactions.list", { limit: 50 }];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function overview(count: number, totalBtc: number, ids: string[]) {
  return {
    kind: "ui.overview.snapshot",
    schema_version: 1,
    data: {
      status: { transactionCount: count },
      balanceSummary: { totalBtc },
      txs: ids.map((id) => ({ id })),
    },
  };
}

function useHeld(key: QueryKey, fetcher: () => Promise<unknown>) {
  const result = useQuery({ queryKey: key, queryFn: fetcher, staleTime: Infinity });
  const held = useHeldQueryResult(key, result);
  // Read here, as a screen would, so the query tracks both.
  return { data: held.data, isFetching: held.isFetching };
}

function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}

beforeEach(() => resetBookRefreshForTests());
afterEach(() => {
  cleanup();
  resetBookRefreshForTests();
});

describe("book refresh cycle", () => {
  it("holds every read until all have landed, then swaps them together", async () => {
    const { client, wrapper } = setup();
    let generation = 1;
    const pending: Record<string, ReturnType<typeof deferred<unknown>>> = {};
    const fetcher = (name: string) => () => {
      if (generation === 1) return Promise.resolve(`${name}-1`);
      pending[name] = deferred<unknown>();
      return pending[name].promise;
    };
    const { result } = renderHook(
      () => ({
        a: useHeld(["daemon", "real", 0, "a"], fetcher("a")),
        b: useHeld(["daemon", "real", 0, "b"], fetcher("b")),
      }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.b.data).toBe("b-1"));

    generation = 2;
    let cycle!: Promise<void>;
    act(() => {
      cycle = refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => "r1", overviewKey: () => null },
      );
    });
    await waitFor(() => expect(pending.b).toBeDefined());
    await act(async () => pending.a.resolve("a-2"));
    // `a` has new data in the cache, but the screen still shows the old
    // state, consistent with `b`, and both say they are refetching.
    expect(client.getQueryData(["daemon", "real", 0, "a"])).toBe("a-2");
    expect(result.current.a.data).toBe("a-1");
    expect(result.current.b.data).toBe("b-1");
    expect(result.current.b.isFetching).toBe(true);

    await act(async () => {
      pending.b.resolve("b-2");
      await cycle;
    });
    expect(result.current.a.data).toBe("a-2");
    expect(result.current.b.data).toBe("b-2");
  });

  it("never holds a different query key at another key's data", async () => {
    const { client, wrapper } = setup();
    const { result, rerender } = renderHook(
      ({ key }: { key: string }) =>
        useHeld(["daemon", "real", 0, "list", key], async () => `rows-${key}`),
      { wrapper, initialProps: { key: "2024" } },
    );
    await waitFor(() => expect(result.current.data).toBe("rows-2024"));
    const gate = deferred<string>();
    let cycle!: Promise<void>;
    act(() => {
      cycle = refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: () => gate.promise, overviewKey: () => null },
      );
    });
    expect(useBookRefreshStore.getState().holding).toBe(true);
    rerender({ key: "2025" });
    await waitFor(() => expect(result.current.data).toBe("rows-2025"));
    await act(async () => {
      gate.resolve("r");
      await cycle;
    });
  });

  it("shows a new key's own data during a hold, never the previous key's placeholder", async () => {
    const { client, wrapper } = setup();
    const gates: Record<string, ReturnType<typeof deferred<string>>> = {};
    const { result, rerender } = renderHook(
      ({ period }: { period: string }) => {
        const key = ["daemon", "real", 0, "ui.transactions.dashboard", period];
        const live = useQuery({
          queryKey: key,
          queryFn: () =>
            period === "2024"
              ? Promise.resolve("figures-2024")
              : (gates[period] = deferred<string>()).promise,
          staleTime: Infinity,
          placeholderData: keepPreviousData,
        });
        const held = useHeldQueryResult(key, live);
        return {
          data: held.data,
          isPlaceholderData: held.isPlaceholderData,
          isFetching: held.isFetching,
        };
      },
      { wrapper, initialProps: { period: "2024" } },
    );
    await waitFor(() => expect(result.current.data).toBe("figures-2024"));

    const revision = deferred<string>();
    let cycle!: Promise<void>;
    act(() => {
      cycle = refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"], predicate: () => false },
        { readRevision: () => revision.promise, overviewKey: () => null },
      );
    });
    expect(useBookRefreshStore.getState().holding).toBe(true);
    // The user picks another period mid-cycle: the old figures show as a
    // placeholder, marked as such...
    rerender({ period: "2025" });
    await waitFor(() => expect(gates["2025"]).toBeDefined());
    expect(result.current.data).toBe("figures-2024");
    expect(result.current.isPlaceholderData).toBe(true);
    // ...and the period's own figures replace them as soon as they arrive,
    // hold or not.
    await act(async () => gates["2025"].resolve("figures-2025"));
    await waitFor(() => expect(result.current.data).toBe("figures-2025"));
    expect(result.current.isPlaceholderData).toBe(false);
    await act(async () => {
      revision.resolve("r");
      await cycle;
    });
  });

  it("marks held data as still refreshing", async () => {
    const { client, wrapper } = setup();
    let generation = 1;
    const second = deferred<string>();
    const { result } = renderHook(
      () => ({
        a: useHeld(["daemon", "real", 0, "a"], async () => `a-${generation}`),
        b: useHeld(["daemon", "real", 0, "b"], () =>
          generation === 1 ? Promise.resolve("b-1") : second.promise,
        ),
      }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.b.data).toBe("b-1"));
    generation = 2;
    let cycle!: Promise<void>;
    act(() => {
      cycle = refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => "r", overviewKey: () => null },
      );
    });
    await waitFor(() => expect(client.getQueryData(["daemon", "real", 0, "a"])).toBe("a-2"));
    // `a` finished its own fetch but still shows its held data: refreshing.
    expect(result.current.a.data).toBe("a-1");
    expect(result.current.a.isFetching).toBe(true);
    await act(async () => {
      second.resolve("b-2");
      await cycle;
    });
    expect(result.current.a).toEqual({ data: "a-2", isFetching: false });
  });

  it("releases the hold when a revision read fails", async () => {
    const { client } = setup();
    await refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      {
        readRevision: () => Promise.reject(new Error("transport closed")),
        overviewKey: () => null,
      },
    );
    expect(useBookRefreshStore.getState()).toMatchObject({
      holding: false,
      cycling: false,
    });
    // The next request runs a fresh cycle instead of waiting forever.
    await refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      { readRevision: async () => "r", overviewKey: () => null },
    );
  });

  it("ignores malformed rows and keeps no summary across a book switch", async () => {
    const { client } = setup();
    client.setQueryDefaults(OVERVIEW, {
      queryFn: async () => ({
        kind: "ui.overview.snapshot",
        schema_version: 1,
        data: { status: { transactionCount: 5 }, txs: [null, { id: 7 }, "x"] },
      }),
    });
    client.setQueryData(OVERVIEW, overview(4, 0.1, ["t1"]));
    let key: QueryKey = OVERVIEW;
    await refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      {
        readRevision: async () => {
          // The profile switched while the cycle ran.
          key = ["daemon", "real", 1, "ui.overview.snapshot"];
          return "r";
        },
        overviewKey: () => key,
      },
    );
    expect(useBookRefreshStore.getState()).toMatchObject({
      holding: false,
      lastChange: null,
    });
  });

  it("repeats the reads while the revision moves, up to its limit", async () => {
    const { client, wrapper } = setup();
    let fetches = 0;
    renderHook(
      () =>
        useHeld(["daemon", "real", 0, "a"], async () => {
          fetches += 1;
          return fetches;
        }),
      { wrapper },
    );
    await waitFor(() => expect(fetches).toBe(1));

    // A worker commits during the first pass only.
    const revisions = ["r1", "r2", "r2", "r2"];
    await act(() =>
      refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => revisions.shift() ?? null, overviewKey: () => null },
      ),
    );
    expect(fetches).toBe(3);

    // A book that keeps moving still lands after MAX_ATTEMPTS passes.
    let counter = 0;
    fetches = 0;
    await act(() =>
      refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => `moving-${(counter += 1)}`, overviewKey: () => null },
      ),
    );
    expect(fetches).toBe(MAX_ATTEMPTS);
    expect(useBookRefreshStore.getState().holding).toBe(false);
  });

  it("refetches once when the store cannot tell revisions", async () => {
    const { client, wrapper } = setup();
    let fetches = 0;
    renderHook(
      () => useHeld(["daemon", "real", 0, "a"], async () => (fetches += 1)),
      { wrapper },
    );
    await waitFor(() => expect(fetches).toBe(1));
    await act(() =>
      refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => null, overviewKey: () => null },
      ),
    );
    expect(fetches).toBe(2);
  });

  it("marks matching reads stale at once and merges requests made mid-cycle", async () => {
    const { client } = setup();
    client.setQueryData(["daemon", "real", 0, "ui.journals.snapshot"], 1);
    client.setQueryData(["daemon", "real", 0, "ui.backends.list"], 1);
    const gate = deferred<string>();
    const readRevision = vi.fn(() => gate.promise);
    const first = refreshBookQueries(
      client,
      {
        queryKey: ["daemon", "real"],
        predicate: (query) => query.queryKey.includes("ui.journals.snapshot"),
      },
      { readRevision, overviewKey: () => null },
    );
    expect(
      client.getQueryState(["daemon", "real", 0, "ui.journals.snapshot"])?.isInvalidated,
    ).toBe(true);
    expect(
      client.getQueryState(["daemon", "real", 0, "ui.backends.list"])?.isInvalidated,
    ).toBe(false);
    const second = refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      { readRevision, overviewKey: () => null },
    );
    const third = refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      { readRevision, overviewKey: () => null },
    );
    expect(
      client.getQueryState(["daemon", "real", 0, "ui.backends.list"])?.isInvalidated,
    ).toBe(true);
    gate.resolve("same");
    await Promise.all([first, second, third]);
    // One cycle for the first request and one merged follow-up: two
    // revision reads each.
    expect(readRevision).toHaveBeenCalledTimes(4);
  });

  it("names what changed and highlights the rows it brought in", async () => {
    const { client, wrapper } = setup();
    let generation = 1;
    renderHook(
      () =>
        useHeld(OVERVIEW, async () =>
          generation === 1
            ? overview(10, 0.5, ["t2", "t1"])
            : overview(12, 0.5123, ["t4", "t3", "t2"]),
        ),
      { wrapper },
    );
    await waitFor(() => expect(client.getQueryData(OVERVIEW)).toBeDefined());
    generation = 2;
    await act(() =>
      refreshBookQueries(
        client,
        { queryKey: ["daemon", "real"] },
        { readRevision: async () => "r", overviewKey: () => OVERVIEW, now: () => 42 },
      ),
    );
    const state = useBookRefreshStore.getState();
    expect(state.lastChange).toEqual({
      transactionDelta: 2,
      balanceDeltaSat: 1_230_000,
      at: 42,
    });
    expect([...state.highlightedTxIds].sort()).toEqual(["t3", "t4"]);
  });

  it("refetches a cached overview that no screen has mounted", async () => {
    const { client } = setup();
    // As the overview query a closed screen left behind: it keeps its fetcher.
    client.setQueryDefaults(OVERVIEW, {
      queryFn: async () => overview(2, 0.2, ["t2", "t1"]),
    });
    client.setQueryData(OVERVIEW, overview(1, 0.1, ["t1"]));
    await refreshBookQueries(
      client,
      { queryKey: ["daemon", "real"] },
      { readRevision: async () => "r", overviewKey: () => OVERVIEW, now: () => 7 },
    );
    expect(useBookRefreshStore.getState().lastChange).toEqual({
      transactionDelta: 1,
      balanceDeltaSat: 10_000_000,
      at: 7,
    });
  });
});

describe("book view comparison", () => {
  it("reports nothing when the figures did not move", () => {
    const client = new QueryClient();
    client.setQueryData(OVERVIEW, overview(3, 1, ["a"]));
    const view = captureBookView(client, OVERVIEW);
    expect(describeChange(view, view, 1)).toBeNull();
    expect(newTransactionIds(view, view).size).toBe(0);
  });

  it("compares rows only within a list loaded before the refresh", () => {
    const client = new QueryClient();
    const before = captureBookView(client, null);
    client.setQueryData(LIST, {
      pages: [{ kind: "ui.transactions.list", schema_version: 1, data: { txs: [{ id: "x" }] } }],
      pageParams: [null],
    });
    const after = captureBookView(client, null);
    // The list first loaded during the refresh: no row is "new" to the user.
    expect(newTransactionIds(before, after).size).toBe(0);
    client.setQueryData(LIST, {
      pages: [
        {
          kind: "ui.transactions.list",
          schema_version: 1,
          data: { txs: [{ id: "y" }, { id: "x" }] },
        },
      ],
      pageParams: [null],
    });
    expect([...newTransactionIds(after, captureBookView(client, null))]).toEqual(["y"]);
  });

  it("compares only the pages that were loaded before the refresh", () => {
    const client = new QueryClient();
    const page = (rows: string[]) => ({
      kind: "ui.transactions.list",
      schema_version: 1,
      data: { txs: rows.map((id) => ({ id })) },
    });
    client.setQueryData(LIST, { pages: [page(["b", "a"])], pageParams: [null] });
    const before = captureBookView(client, null);
    // One new row on top, and a second page scrolled in meanwhile.
    client.setQueryData(LIST, {
      pages: [page(["c", "b"]), page(["a", "older"])],
      pageParams: [null, "cursor"],
    });
    expect([...newTransactionIds(before, captureBookView(client, null))]).toEqual(["c"]);
  });

  it("names a removed transaction and keeps an unknown balance unknown", () => {
    const client = new QueryClient();
    client.setQueryData(OVERVIEW, overview(3, 1, []));
    const before = captureBookView(client, OVERVIEW);
    client.setQueryData(OVERVIEW, {
      kind: "ui.overview.snapshot",
      schema_version: 1,
      data: { status: { transactionCount: 2 } },
    });
    expect(describeChange(before, captureBookView(client, OVERVIEW), 5)).toEqual({
      transactionDelta: -1,
      balanceDeltaSat: null,
      at: 5,
    });
  });
});
