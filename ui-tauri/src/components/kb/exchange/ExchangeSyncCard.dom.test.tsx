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

// The book open when the work starts, as the app reads it from
// `ui.profiles.snapshot`.
const openBook = vi.hoisted(() => ({
  scope: { book: "book-a", expected: { workspace_id: "ws", profile_id: "pf-a" } } as {
    book: string;
    expected: { workspace_id: string; profile_id: string };
  } | null,
}));
vi.mock("@/lib/notificationScope", () => ({ activeBookScope: () => openBook.scope }));

const mocks = vi.hoisted(() => ({
  status: { data: undefined as unknown, error: null as unknown },
  statusArgs: [] as unknown[],
  refetch: vi.fn(),
  sync: vi.fn(),
  update: vi.fn(),
  encrypted: false,
}));

vi.mock("@/daemon/client", async () => {
  const actual = await vi.importActual<typeof import("@/daemon/client")>("@/daemon/client");
  return {
    DaemonRequestError: actual.DaemonRequestError,
    useDaemon: (kind: string, args: unknown, options: unknown) => {
      if (kind === "status") {
        return {
          data: { kind: "status", data: { database_encrypted: mocks.encrypted } },
          error: null,
        };
      }
      if (kind !== "ui.connections.exchange.status") throw new Error(`unexpected ${kind}`);
      mocks.statusArgs.push([args, options]);
      return { ...mocks.status, refetch: mocks.refetch };
    },
    useDaemonMutation: (kind: string) => {
      // The asset scope toggle is a wallet change (docs/plan/24-crypto-assets.md).
      if (kind === "ui.wallets.update") return { mutateAsync: mocks.update, isPending: false };
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
  mocks.update.mockReset();
  mocks.encrypted = false;
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

  it("shows Bitpanda's balance against the book, masked when amounts are hidden", () => {
    const lastSync = (provider_balance: object) => ({
      status: "ok" as const,
      at: "2024-01-08T00:00:00Z",
      imported: 6,
      fetched: 6,
      provider_balance,
    });
    mocks.status = {
      data: {
        data: {
          connections: [
            row({
              last_sync: lastSync({
                status: "matches",
                asset: "BTC",
                provider_msat: 991_000_000,
                book_msat: 991_000_000,
                difference_msat: 0,
              }) as ExchangeConnectionStatus["last_sync"],
            }),
          ],
        },
      },
      error: null,
    };
    useUiStore.setState({ hideSensitive: true });
    const { rerender } = render(<Card />);
    const matches = screen.getByText("₿ 0.00991000 · matches the book");
    expect(matches.className).toContain("sensitive");
    expect(screen.queryByText(/disagree/)).toBeNull();

    useUiStore.setState({ hideSensitive: false });
    mocks.status = {
      data: {
        data: {
          connections: [
            row({
              last_sync: lastSync({
                status: "mismatch",
                asset: "BTC",
                provider_msat: 1_091_000_000,
                book_msat: 991_000_001,
                difference_msat: -99_999_999,
              }) as ExchangeConnectionStatus["last_sync"],
            }),
          ],
        },
      },
      error: null,
    };
    rerender(<Card />);
    const mismatch = screen.getByText("₿ 0.01091000 · the book shows ₿ 0.00991000001");
    expect(mismatch.className).not.toContain("sensitive");
    expect(screen.getByText(/The book and Bitpanda disagree/)).toBeTruthy();

    mocks.status = {
      data: {
        data: {
          connections: [
            row({
              last_sync: lastSync({
                status: "unavailable",
                asset: "BTC",
                reason: "a BTC transaction names no Bitpanda wallet",
              }) as ExchangeConnectionStatus["last_sync"],
            }),
          ],
        },
      },
      error: null,
    };
    rerender(<Card />);
    expect(screen.getByText("Not stated unambiguously in the history")).toBeTruthy();
    // The daemon's English reason is not shown as UI text.
    expect(screen.queryByText(/names no Bitpanda wallet/)).toBeNull();
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
      scope: { book: "book-a", expected: { profile_id: "pf-a" }, source: "connection:w-1" },
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
      scope: { book: "book-a", expected: { profile_id: "pf-a" }, source: "connection:w-1" },
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

  describe("other crypto assets", () => {
    const synced = (extra: object, rowExtra: Partial<ExchangeConnectionStatus> = {}) => {
      mocks.status = {
        data: {
          data: {
            connections: [
              row({
                ...rowExtra,
                last_sync: {
                  status: "ok",
                  at: "2026-11-12T00:00:00Z",
                  imported: 3,
                  fetched: 45,
                  ...extra,
                } as ExchangeConnectionStatus["last_sync"],
              }),
            ],
          },
        },
        error: null,
      };
    };
    const updated = {
      kind: "ui.wallets.update",
      schema_version: 1,
      data: { wallet: { id: "w-1" } },
    };

    it("offers to track what the bitcoin scope skipped, then syncs again", async () => {
      synced({
        assets: {
          scope: "bitcoin",
          skipped: { BEST: 2, ETH: 30, SOL: 10 },
          excluded: {},
          next_step: "kassiber wallets update …",
        },
      });
      mocks.update.mockResolvedValue(updated);
      mocks.sync.mockResolvedValue({
        kind: "ui.connections.exchange.sync",
        schema_version: 1,
        data: { imported: 42, fetched: 45 },
      });
      render(<Card />);
      expect(screen.getByText("Also found ETH, SOL, BEST: 42 entries")).toBeTruthy();
      // The CLI's English next step is not UI text.
      expect(screen.queryByText(/kassiber wallets update/)).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Also track these assets" }));
      expect(screen.getByText(/syncs this exchange again/)).toBeTruthy();

      // Nothing changes before the typed acknowledgement.
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Track and sync now" }));
      });
      expect(screen.getByRole("alert").textContent).toContain("CHANGE LOCAL DATA");
      expect(mocks.update).not.toHaveBeenCalled();

      fireEvent.change(screen.getByLabelText(/Type CHANGE LOCAL DATA/), {
        target: { value: "CHANGE LOCAL DATA" },
      });
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Track and sync now" }));
      });
      expect(mocks.update).toHaveBeenCalledWith({
        wallet: "w-1",
        asset_scope: "crypto",
        auth_response: { plaintext_change_ack: "CHANGE LOCAL DATA" },
      });
      expect(mocks.sync).toHaveBeenCalledWith({ wallet: "w-1" });
      expect(mocks.update.mock.invocationCallOrder[0]).toBeLessThan(
        mocks.sync.mock.invocationCallOrder[0],
      );
    });

    it("asks an encrypted book for its passphrase", async () => {
      mocks.encrypted = true;
      synced({ assets: { scope: "bitcoin", skipped: { ETH: 1 }, excluded: {} } });
      mocks.update.mockResolvedValue(updated);
      mocks.sync.mockResolvedValue({ kind: "ui.connections.exchange.sync", schema_version: 1, data: {} });
      render(<Card />);
      expect(screen.getByText("Also found ETH: 1 entry")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "Also track these assets" }));
      const input = screen.getByLabelText("Database passphrase") as HTMLInputElement;
      expect(input.type).toBe("password");
      fireEvent.change(input, { target: { value: "synthetic passphrase" } });
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Track and sync now" }));
      });
      expect(mocks.update).toHaveBeenCalledWith({
        wallet: "w-1",
        asset_scope: "crypto",
        auth_response: { passphrase_secret: "synthetic passphrase" },
      });
    });

    it("switches back without syncing and keeps the imported rows", async () => {
      synced(
        { assets: { scope: "crypto", skipped: {}, excluded: {} }, swap_pairs: 2 },
        { asset_scope: "crypto" },
      );
      mocks.update.mockResolvedValue(updated);
      render(<Card />);
      expect(screen.getByText("Other crypto assets are tracked for this wallet.")).toBeTruthy();
      expect(screen.getByText("2 crypto-to-crypto swaps paired")).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Also track these assets" })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Track Bitcoin only" }));
      expect(screen.getByText(/Entries already imported stay in the book/)).toBeTruthy();
      fireEvent.change(screen.getByLabelText(/Type CHANGE LOCAL DATA/), {
        target: { value: "CHANGE LOCAL DATA" },
      });
      await act(async () => {
        fireEvent.click(screen.getByRole("button", { name: "Track Bitcoin only" }));
      });
      expect(mocks.update).toHaveBeenCalledWith({
        wallet: "w-1",
        asset_scope: "bitcoin",
        auth_response: { plaintext_change_ack: "CHANGE LOCAL DATA" },
      });
      expect(mocks.sync).not.toHaveBeenCalled();
    });

    it("names excluded holdings without a toggle", () => {
      synced({ assets: { scope: "bitcoin", skipped: {}, excluded: { TSLA: 2, GLD: 1 } } });
      render(<Card />);
      expect(
        screen.getByText(
          "Not imported: TSLA, GLD (3 entries). Stocks, ETFs and metals stay out of the book.",
        ),
      ).toBeTruthy();
      expect(screen.queryByRole("button", { name: "Also track these assets" })).toBeNull();
      expect(screen.queryByRole("button", { name: "Track Bitcoin only" })).toBeNull();
    });

    it("shows nothing about assets for a sync that did not report them", () => {
      synced({});
      render(<Card />);
      expect(screen.queryByTestId("asset-scope")).toBeNull();
    });

    it("speaks Austrian German", async () => {
      await i18n.changeLanguage("de");
      synced({ assets: { scope: "bitcoin", skipped: { ETH: 3, SOL: 1 }, excluded: {} } });
      render(<Card />);
      expect(screen.getByText("Außerdem gefunden: ETH, SOL: 4 Einträge")).toBeTruthy();
      fireEvent.click(screen.getByRole("button", { name: "Diese Assets auch erfassen" }));
      expect(screen.getByRole("button", { name: "Erfassen und Sync starten" })).toBeTruthy();
      expect(screen.getByLabelText(/Tippe CHANGE LOCAL DATA/)).toBeTruthy();
    });
  });

  it("speaks Austrian German", async () => {
    await i18n.changeLanguage("de");
    render(<Card />);
    expect(screen.getByRole("button", { name: "Sync starten" })).toBeTruthy();
    expect(screen.getByText("Seit dem Start von Kassiber nicht synced")).toBeTruthy();
  });
});
