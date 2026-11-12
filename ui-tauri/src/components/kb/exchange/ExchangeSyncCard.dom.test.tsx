// @vitest-environment happy-dom
//
// Mounted: Sync now is the wallet page's only exchange request, and its last
// result (or its error, explained) is what the card shows.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";
import { useUiStore } from "@/store/ui";

import type { ExchangeConnectionStatus, ExchangeKind } from "./exchangeApiModel";

vi.hoisted(() => {
  // The UI store (notifications) persists through `localStorage`, which this
  // DOM does not wire up as a global; zustand resolves it at module evaluation.
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const mocks = vi.hoisted(() => ({
  status: { data: undefined as unknown, error: null as unknown },
  statusArgs: [] as unknown[],
  refetch: vi.fn(),
  sync: vi.fn(),
}));

vi.mock("@/daemon/client", async () => {
  const actual = await vi.importActual<typeof import("@/daemon/client")>("@/daemon/client");
  return {
    DaemonRequestError: actual.DaemonRequestError,
    useDaemon: (kind: string, args: unknown, options: unknown) => {
      if (kind !== "ui.connections.exchange.status") throw new Error(`unexpected ${kind}`);
      mocks.statusArgs.push([args, options]);
      return { ...mocks.status, refetch: mocks.refetch };
    },
    useDaemonMutation: (kind: string) => {
      if (kind !== "ui.connections.exchange.sync") throw new Error(`unexpected ${kind}`);
      return { mutateAsync: mocks.sync, isPending: false };
    },
  };
});

import { DaemonRequestError } from "@/daemon/client";
import { ExchangeSyncCard } from "./ExchangeSyncCard";
import { useExchangeSync } from "./useExchangeSync";

function row(overrides: Partial<ExchangeConnectionStatus> = {}): ExchangeConnectionStatus {
  return {
    wallet_id: "w-1",
    wallet: "Bitpanda",
    exchange: "bitpanda",
    provider: "Bitpanda",
    backend: "bitpanda",
    backend_configured: true,
    has_api_key: true,
    has_api_secret: null,
    last_sync: null,
    ...overrides,
  };
}

function Card({ exchange = "bitpanda" }: { exchange?: ExchangeKind }) {
  const state = useExchangeSync({ walletId: "w-1", label: "Bitpanda", exchange });
  return <ExchangeSyncCard exchange={exchange} state={state} />;
}

function daemonError(code: string, message: string) {
  return new DaemonRequestError("ui.connections.exchange.sync", {
    kind: "error",
    schema_version: 1,
    error: { code, message, hint: null, details: null, retryable: false },
  });
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  mocks.status = { data: { data: { connections: [row()] } }, error: null };
  mocks.statusArgs.length = 0;
  mocks.refetch.mockReset();
  mocks.sync.mockReset();
  useUiStore.setState({ identity: null });
  resetNotificationStore();
});

afterEach(() => {
  cleanup();
});

describe("ExchangeSyncCard", () => {
  it("reads only the local status and says when it last synced", () => {
    render(<Card />);
    // The status is a local read the card never retries.
    expect(mocks.statusArgs[0]).toEqual([{ wallet: "w-1" }, { enabled: true, retry: false }]);
    expect(screen.getByRole("heading", { name: "Bitpanda API" })).toBeTruthy();
    expect(screen.getByText(/only when you press Sync now/)).toBeTruthy();
    expect(screen.getByText("Not synced since Kassiber started")).toBeTruthy();
    expect(screen.getByText("Stored in backend “bitpanda”")).toBeTruthy();
    expect(mocks.sync).not.toHaveBeenCalled();
  });

  it("syncs on Sync now and reports what it imported", async () => {
    mocks.sync.mockResolvedValue({
      kind: "ui.connections.exchange.sync",
      schema_version: 1,
      data: { imported: 5, fetched: 7 },
    });
    render(<Card />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    });
    expect(mocks.sync).toHaveBeenCalledWith({ wallet: "w-1" });
    expect(mocks.refetch).toHaveBeenCalled();
    // One Recent result that opens this exchange account; the job is over.
    const { jobs, results } = useNotificationStore.getState();
    expect(jobs).toEqual({});
    expect(results).toHaveLength(1);
    expect(results[0]).toMatchObject({
      code: "connection.synced",
      scope: { book: null, source: "connection:w-1" },
      params: { transactions: 5 },
      action: { type: "open-connection", connectionId: "w-1" },
    });
  });

  it("explains a refusal and asks for a report of an unrecognised type", async () => {
    mocks.sync.mockRejectedValue(
      daemonError("validation", "Bitpanda operation op-1 (STAKE) is not a type Kassiber recognises"),
    );
    render(<Card />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    });
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("please report it");
    expect(alert.textContent).toContain("Details: Bitpanda operation op-1 (STAKE)");
    // Needs attention until a sync of this account succeeds; its one
    // action opens the account, where Sync now and the hint are.
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition).toMatchObject({
      code: "connection.sync-failed",
      scope: { book: null, source: "connection:w-1" },
      action: { type: "open-connection", connectionId: "w-1" },
    });
    expect(condition.details?.[0]).toMatchObject({ label: "Bitpanda" });
    expect(condition.details?.[0]?.message).toContain("STAKE");

    mocks.sync.mockResolvedValue({
      kind: "ui.connections.exchange.sync",
      schema_version: 1,
      data: { imported: 0, fetched: 0 },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    });
    expect(useNotificationStore.getState().conditions).toEqual({});
  });

  it("shows the session's last result and error from the status", () => {
    mocks.status = {
      data: {
        data: {
          connections: [
            row({
              last_sync: {
                status: "error",
                at: "2026-11-08T12:00:00Z",
                error: { code: "rate_limited", message: "Backend rate limited the request" },
              },
            }),
          ],
        },
      },
      error: null,
    };
    render(<Card />);
    expect(screen.getByText(/· failed$/)).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("Bitpanda is limiting requests right now.");
  });

  it("cannot sync without a stored key or on an older backend", () => {
    mocks.status = { data: { data: { connections: [row({ has_api_key: false })] } }, error: null };
    render(<Card />);
    expect((screen.getByRole("button", { name: "Sync now" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Missing — add the exchange again")).toBeTruthy();
    cleanup();

    mocks.status = {
      data: undefined,
      error: new DaemonRequestError("ui.connections.exchange.status", {
        kind: "error",
        schema_version: 1,
        error: { code: "daemon_unavailable", message: "not wired", retryable: true },
      }),
    };
    render(<Card />);
    expect((screen.getByRole("button", { name: "Sync now" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("need a newer Kassiber backend");
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    render(<Card />);
    expect(screen.getByRole("button", { name: "Sync starten" })).toBeTruthy();
    expect(screen.getByText("Seit dem Start von Kassiber nicht synced")).toBeTruthy();
  });
});
