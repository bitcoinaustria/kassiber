// @vitest-environment happy-dom
//
// The real job hooks against a fake daemon that enforces `expected_scope`
// the way the Rust dispatcher does: a request bound to a book that is no
// longer open is refused with `stale_context` before anything runs.
import {
  QueryClient,
  QueryClientProvider,
  useQueryClient,
} from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

type Scope = { workspace_id: string; profile_id: string };
type Call = { kind: string; args: Record<string, unknown> };

const daemon = vi.hoisted(() => ({
  /** The book the daemon has open, which can run ahead of the renderer. */
  open: { workspace_id: "ws", profile_id: "pf-a" } as Scope,
  calls: [] as Call[],
  /** Runs held open until the test settles them. */
  pending: [] as Array<{ kind: string; settle: (envelope: unknown) => void }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  const refuse = (args: Record<string, unknown>) => {
    const expected = args.expected_scope as Scope | undefined;
    if (
      expected &&
      (expected.workspace_id !== daemon.open.workspace_id ||
        expected.profile_id !== daemon.open.profile_id)
    ) {
      return {
        kind: "error",
        schema_version: 1,
        error: { code: "stale_context", message: "another book", retryable: true },
      };
    }
    return null;
  };
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.profiles.snapshot") {
          return {
            kind,
            schema_version: 1,
            data: {
              activeWorkspaceId: daemon.open.workspace_id,
              activeProfileId: daemon.open.profile_id,
            },
          };
        }
        return refuse(args) ?? { kind, schema_version: 1, data: {} };
      },
      stream: ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        const refused = refuse(args);
        if (refused) return Promise.resolve(refused);
        return new Promise((settle) => daemon.pending.push({ kind, settle }));
      },
    }),
  };
});

const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { NotificationCenter } from "@/components/kb/NotificationCenter";
import { daemonQueryKey } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { bookKeyFor, readActiveBook } from "@/lib/activeBook";
import { clearFeedback, useFeedbackStore } from "@/lib/feedback";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

const A: Scope = { workspace_id: "ws", profile_id: "pf-a" };
const B: Scope = { workspace_id: "ws", profile_id: "pf-b" };
const KEY_A = bookKeyFor(null, A);

let client: QueryClient;

/** What the renderer believes is open: this session's cached snapshot. */
function rendererSees(scope: Scope) {
  const { dataMode, daemonSession } = useUiStore.getState();
  client.setQueryData(daemonQueryKey(dataMode, daemonSession, "ui.profiles.snapshot"), {
    kind: "ui.profiles.snapshot",
    schema_version: 1,
    data: { activeWorkspaceId: scope.workspace_id, activeProfileId: scope.profile_id },
  });
}

async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function press(button: HTMLElement) {
  act(() => void fireEvent.click(button));
  await flush();
}

function runs(kind: string) {
  return daemon.calls.filter((call) => call.kind === kind);
}

async function settle(envelope: unknown) {
  const run = daemon.pending.shift();
  if (!run) throw new Error("no run in flight");
  await act(async () => {
    run.settle(envelope);
    await Promise.resolve();
  });
  // Let the mutation promise chain land.
  await flush();
}

function Starter({ which }: { which: "journals" | "refresh" }) {
  const { runJournalProcessing } = useJournalProcessingAction();
  const { syncAll } = useWalletSyncAction();
  return (
    <button
      type="button"
      onClick={() => (which === "journals" ? runJournalProcessing() : syncAll())}
    >
      start
    </button>
  );
}

/** The center wired to the real hook, as the shell wires it. */
function Shell() {
  const queryClient = useQueryClient();
  const { syncAll } = useWalletSyncAction();
  return (
    <NotificationCenter
      currentBook={readActiveBook(queryClient)}
      resolveCurrentBook={() => readActiveBook(queryClient)}
      runOperation={(operation, expectedScope) => {
        if (operation.kind === "refresh-book") {
          syncAll({ forceFull: operation.forceFull, expectedScope });
        }
      }}
    />
  );
}

function mount(node: React.ReactNode) {
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  daemon.open = A;
  daemon.calls = [];
  daemon.pending = [];
  navigate.mockReset();
  resetNotificationStore();
  clearFeedback();
  useUiStore.setState({ identity: null, privacyMode: "normal", lang: "en" });
});

afterEach(cleanup);

describe("tracked jobs", () => {
  it("records a journal outcome after the starting surface unmounts", async () => {
    rendererSees(A);
    const view = mount(<Starter which="journals" />);
    await press(screen.getByRole("button", { name: "start" }));
    expect(runs("ui.journals.process")[0].args).toEqual({ expected_scope: A });
    expect(useNotificationStore.getState().jobs["journal-processing"]).toBeDefined();

    view.unmount();
    await settle({
      kind: "ui.journals.process",
      schema_version: 1,
      data: { processed_transactions: 2, entries_created: 4 },
    });

    const state = useNotificationStore.getState();
    expect(state.jobs).toEqual({});
    expect(state.results[0]).toMatchObject({
      code: "journals.processed",
      scope: { book: KEY_A, expected: A },
      params: { transactions: 2, entries: 4 },
    });
  });

  it("binds a fresh run to the open book, reading it when not yet known", async () => {
    mount(<Starter which="refresh" />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "start" }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(runs("ui.profiles.snapshot")).toHaveLength(1);
    expect(runs("ui.freshness.run")[0].args).toMatchObject({ expected_scope: A });
  });
});

describe("a stale Retry after a book switch", () => {
  async function failRefreshInA() {
    rendererSees(A);
    const view = mount(<Starter which="refresh" />);
    await press(screen.getByRole("button", { name: "start" }));
    await settle({
      kind: "error",
      schema_version: 1,
      error: { code: "backend_error", message: "timeout", retryable: true },
    });
    view.unmount();
    const key = conditionKey("book-refresh.failed", { book: KEY_A });
    expect(useNotificationStore.getState().conditions[key]?.occurrenceCount).toBe(1);
    daemon.calls = [];
    clearFeedback();
    return key;
  }

  async function clickRetry() {
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^Notifications/ }));
    });
    const row = screen
      .getAllByTestId("notification-attention")
      .find((element) => element.textContent?.includes("Book refresh failed"));
    if (!row) throw new Error("no refresh failure row");
    await press(within(row).getByRole("button", { name: "Retry" }));
  }

  it("sends A's scope and is refused when the daemon already opened B", async () => {
    const key = await failRefreshInA();
    // The daemon has switched; the renderer has not caught up yet.
    daemon.open = B;
    mount(<Shell />);
    await clickRetry();

    const [retry] = runs("ui.freshness.run");
    expect(retry.args).toMatchObject({ expected_scope: A, force_full: false });
    // Refused before it ran: A's failure stands, unrepeated, and nothing
    // was recorded against B.
    const conditions = useNotificationStore.getState().conditions;
    expect(conditions[key]?.occurrenceCount).toBe(1);
    expect(Object.keys(conditions)).toEqual([key]);
    expect(useNotificationStore.getState().results).toEqual([]);
    expect(useFeedbackStore.getState().items.map((item) => item.key)).toEqual([
      "stale-book",
    ]);
  });

  it("sends nothing once the renderer knows B is open", async () => {
    const key = await failRefreshInA();
    daemon.open = B;
    rendererSees(B);
    mount(<Shell />);
    // A's failure is not B's to show; it is kept for A.
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /^Notifications/ }));
    });
    expect(screen.queryByText("Book refresh failed")).toBeNull();
    expect(runs("ui.freshness.run")).toEqual([]);
    expect(useNotificationStore.getState().conditions[key]).toBeDefined();
  });
});
