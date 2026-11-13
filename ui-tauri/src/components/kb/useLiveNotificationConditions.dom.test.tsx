// @vitest-environment happy-dom
//
// The live conditions re-read after a mutation that changes them, bound to
// the open book, and reconcile what the re-read proves.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  sourceStatus: "failed",
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.freshness.status") {
          return {
            kind,
            schema_version: 1,
            data: {
              sources: [
                {
                  source_key: "onchain_wallet:w1",
                  source_type: "onchain_wallet",
                  source_label: "Cold",
                  status: daemon.sourceStatus,
                  last_error_message: "timeout",
                },
              ],
            },
          };
        }
        return {
          kind,
          schema_version: 1,
          data: { quarantine: 0, journals_needs_processing: false, swaps: 0, review: 0 },
        };
      },
    }),
  };
});

import { invalidateDaemonQueriesForMutation } from "@/daemon/client";
import type { ActiveBook } from "@/lib/activeBook";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

import { useLiveNotificationConditions } from "./useLiveNotificationConditions";

const BOOK: ActiveBook = {
  key: '["db:/p.sqlite3","ws","pf"]',
  expected: { workspace_id: "ws", profile_id: "pf" },
};
const KEY = conditionKey("source.refresh-failed", {
  book: BOOK.key,
  source: "onchain_wallet:w1",
});

function Probe({ book }: { book: ActiveBook | null }) {
  useLiveNotificationConditions(true, book);
  return null;
}

async function flush() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  daemon.sourceStatus = "failed";
  daemon.calls = [];
  resetNotificationStore();
});

afterEach(cleanup);

describe("live notification conditions", () => {
  it("reconciles the re-read that follows a refresh", async () => {
    const client = new QueryClient();
    render(
      <QueryClientProvider client={client}>
        <Probe book={BOOK} />
      </QueryClientProvider>,
    );
    await waitFor(() =>
      expect(useNotificationStore.getState().conditions[KEY]).toBeDefined(),
    );
    // Both reads are bound to the open book.
    expect(daemon.calls.map((call) => call.args.expected_scope)).toEqual([
      BOOK.expected,
      BOOK.expected,
    ]);

    // The refresh fixed the source; its success invalidates the read.
    daemon.sourceStatus = "fresh";
    invalidateDaemonQueriesForMutation(
      client,
      useUiStore.getState().dataMode,
      "ui.freshness.run",
    );
    await waitFor(() =>
      expect(useNotificationStore.getState().conditions[KEY]).toBeUndefined(),
    );
  });

  it("reads nothing while no book is known", async () => {
    const client = new QueryClient();
    render(
      <QueryClientProvider client={client}>
        <Probe book={null} />
      </QueryClientProvider>,
    );
    await flush();
    expect(daemon.calls).toEqual([]);
    expect(useNotificationStore.getState().conditions).toEqual({});
  });
});
