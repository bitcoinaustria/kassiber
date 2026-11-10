// @vitest-environment happy-dom
//
// The whole Add connection flow for an exchange API: the card opens the
// setup, Test connection is the first request, Create connection stores the
// connection without contacting the exchange, and only the opt-in imports.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

vi.hoisted(() => {
  // The UI store persists through `localStorage`, which this DOM does not
  // wire up as a global; zustand resolves it at module evaluation.
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

const mocks = vi.hoisted(() => ({
  calls: [] as Array<[string, unknown]>,
  answers: {} as Record<string, unknown>,
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/daemon/transport", async () => ({
  ...(await vi.importActual<typeof import("@/daemon/transport")>("@/daemon/transport")),
  openExternalUrl: vi.fn(async () => {}),
}));
vi.mock("@/daemon/client", async () => {
  const actual = await vi.importActual<typeof import("@/daemon/client")>("@/daemon/client");
  const mutation = (kind: string) => ({
    isPending: false,
    mutate: vi.fn(),
    mutateAsync: vi.fn(async (args: unknown) => {
      mocks.calls.push([kind, args]);
      return { kind, schema_version: 1, data: mocks.answers[kind] ?? {} };
    }),
  });
  return {
    ...actual,
    useDaemon: () => ({ data: undefined, isLoading: false, error: null, refetch: vi.fn() }),
    useDaemonMutation: mutation,
    useDaemonStreamMutation: mutation,
  };
});

import { AddConnectionDialog } from "../AddConnectionDialog";

const KEY = "synthetic-pasted-key";
const SECRET = "synthetic-pasted-secret";

function kinds() {
  return mocks.calls.map(([kind]) => kind);
}

async function click(name: string | RegExp) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
  });
}

function open(onOpenChange = vi.fn()) {
  render(<AddConnectionDialog open initialSourceId="kraken" onOpenChange={onOpenChange} />);
  return onOpenChange;
}

async function pasteAndTest() {
  fireEvent.change(screen.getByLabelText("API key"), { target: { value: KEY } });
  fireEvent.change(screen.getByLabelText("Private key"), { target: { value: SECRET } });
  await click("Test connection");
}

beforeEach(async () => {
  await i18n.changeLanguage("en");
  mocks.calls.length = 0;
  mocks.answers = {
    "ui.connections.exchange.test": { exchange: "kraken", ok: true, read_only: null },
    "ui.connections.exchange.create": {
      exchange: "kraken",
      wallet: { id: "w-kraken", label: "Kraken", kind: "kraken" },
      backend: { name: "kraken" },
    },
    "ui.connections.exchange.sync": { imported: 3, fetched: 4 },
  };
});

afterEach(() => {
  cleanup();
});

describe("Add an exchange API connection", () => {
  it("adds the connection only after a passing test, without syncing", async () => {
    const onOpenChange = open();
    expect(screen.getByRole("heading", { name: "Create a read-only API key" })).toBeTruthy();
    const label = screen.getByLabelText("Connection label") as HTMLInputElement;
    expect(label.value).toBe("Kraken");
    // Opening the setup contacts nothing.
    expect(mocks.calls).toEqual([]);

    // Creating before a test is refused locally.
    await click("Create connection");
    expect(mocks.calls).toEqual([]);
    expect(screen.getAllByText("Paste the API key.").length).toBeGreaterThan(0);

    await pasteAndTest();
    expect(mocks.calls).toEqual([
      ["ui.connections.exchange.test", { exchange: "kraken", api_key: KEY, api_secret: SECRET }],
    ]);
    // The import waits for an explicit opt-in.
    const optIn = screen.getByRole("checkbox", {
      name: "Import Kraken history right after adding the connection",
    });
    expect(optIn.getAttribute("aria-checked")).toBe("false");

    await click("Create connection");
    expect(kinds()).toEqual(["ui.connections.exchange.test", "ui.connections.exchange.create"]);
    expect(mocks.calls[1][1]).toEqual({
      exchange: "kraken",
      api_key: KEY,
      api_secret: SECRET,
      label: "Kraken",
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("imports right after adding when the user opts in", async () => {
    open();
    await pasteAndTest();
    await act(async () => {
      fireEvent.click(
        screen.getByRole("checkbox", { name: "Import Kraken history right after adding the connection" }),
      );
    });
    await click("Create connection");
    expect(kinds()).toEqual([
      "ui.connections.exchange.test",
      "ui.connections.exchange.create",
      "ui.connections.exchange.sync",
    ]);
    expect(mocks.calls[2][1]).toEqual({ wallet: "w-kraken" });
  });

  it("asks for a new test when the key changes after one passed", async () => {
    open();
    await pasteAndTest();
    fireEvent.change(screen.getByLabelText("Private key"), { target: { value: `${SECRET}-2` } });
    await click("Create connection");
    expect(kinds()).toEqual(["ui.connections.exchange.test"]);
    expect(screen.getAllByText("Test the connection before adding it.").length).toBeGreaterThan(0);
  });
});
