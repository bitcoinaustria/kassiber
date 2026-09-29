// @vitest-environment happy-dom
//
// Mounted, not static: the connection panel only exists once the dropdown is
// open, and what it may send depends on the offline switch's live state.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
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

const daemon = vi.hoisted(() => ({
  reads: {} as Record<string, unknown>,
  mutations: {} as Record<string, ReturnType<typeof vi.fn>>,
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string) => {
    const data = daemon.reads[kind];
    return {
      data: data === undefined ? undefined : { kind, data },
      error: null,
      isLoading: false,
      isSuccess: data !== undefined,
    };
  },
  useDaemonMutation: (kind: string) => {
    daemon.mutations[kind] ??= vi.fn(async () => ({ kind, data: { ok: true } }));
    return { mutateAsync: daemon.mutations[kind], isPending: false };
  },
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { useConnectionHealthStore } from "@/store/connectionHealth";
import { useUiStore } from "@/store/ui";

import { NetworkStatusIndicator } from "./NetworkStatusIndicator";

const PROBE_KINDS = [
  "ui.backends.electrum.test",
  "ui.backends.http.test",
  "ui.backends.bitcoinrpc.test",
  "ui.backends.lightning.test",
  "ui.backends.btcpay.test",
];

function probeCalls() {
  return PROBE_KINDS.reduce(
    (total, kind) => total + (daemon.mutations[kind]?.mock.calls.length ?? 0),
    0,
  );
}

async function openPanel() {
  render(<NetworkStatusIndicator daemonEnabled />);
  const trigger = screen.getByRole("button");
  await act(async () => {
    fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false });
  });
  return trigger;
}

function resetStores() {
  useConnectionHealthStore.setState({
    records: {},
    checking: false,
    lastAutomaticCheckAt: null,
  });
  useUiStore.setState({ connectionAutoCheck: false });
}

describe("NetworkStatusIndicator offline mode", () => {
  beforeEach(() => {
    resetStores();
    daemon.mutations = {};
    daemon.reads = {
      "ui.network.offline": { offline: false, environment_blocked: false },
      "ui.backends.settings.list": {
        backends: [
          { name: "fulcrum", kind: "electrum", url: "ssl://fulcrum.example:50002", has_url: true },
        ],
        summary: { count: 1, default_backend: "fulcrum" },
      },
    };
  });
  afterEach(cleanup);

  it("turns offline mode on from the panel", async () => {
    await openPanel();
    const toggle = screen.getByRole("switch", { name: "network.offlineMode.switch" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");

    await act(async () => {
      fireEvent.click(toggle);
    });

    expect(daemon.mutations["ui.network.offline.set"]).toHaveBeenCalledWith({ enabled: true });
    expect(probeCalls()).toBe(0);
  });

  it("shows every connection as offline and refuses checks while offline", async () => {
    daemon.reads["ui.network.offline"] = { offline: true, environment_blocked: false };
    const trigger = await openPanel();

    expect(trigger.getAttribute("aria-label")).toBe("network.indicator.offlineMode");
    expect(screen.getByText("network.offlineMode.on")).toBeTruthy();
    expect(screen.getByLabelText("network.health.offline")).toBeTruthy();
    const check = screen.getByRole("button", { name: "network.checkConnections" });
    expect(check.hasAttribute("disabled")).toBe(true);

    await act(async () => {
      fireEvent.click(check);
    });
    expect(probeCalls()).toBe(0);
  });

  it("keeps the switch usable under the operator's process override", async () => {
    // KASSIBER_NO_EGRESS blocks backends but not AI providers, device sync or
    // update checks, so the switch must still be able to block those.
    daemon.reads["ui.network.offline"] = { offline: false, environment_blocked: true };
    await openPanel();

    const toggle = screen.getByRole("switch", { name: "network.offlineMode.switch" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.hasAttribute("disabled")).toBe(false);
    expect(screen.getByText("network.offlineMode.environment")).toBeTruthy();
    expect(screen.getByLabelText("network.health.offline")).toBeTruthy();
    const check = screen.getByRole("button", { name: "network.checkConnections" });
    expect(check.hasAttribute("disabled")).toBe(true);

    await act(async () => {
      fireEvent.click(toggle);
    });
    expect(daemon.mutations["ui.network.offline.set"]).toHaveBeenCalledWith({ enabled: true });
    expect(probeCalls()).toBe(0);
  });

  it("says so when the switch cannot be changed", async () => {
    daemon.mutations["ui.network.offline.set"] = vi.fn(async () => {
      throw new Error("refused");
    });
    await openPanel();

    await act(async () => {
      fireEvent.click(screen.getByRole("switch", { name: "network.offlineMode.switch" }));
    });
    expect(screen.getByRole("alert").textContent).toBe("network.offlineMode.failed");
  });

  it("checks only when asked while online", async () => {
    await openPanel();
    expect(probeCalls()).toBe(0);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "network.checkConnections" }));
    });
    expect(daemon.mutations["ui.backends.electrum.test"]).toHaveBeenCalledTimes(1);
  });
});

describe("NetworkStatusIndicator automatic checks", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetStores();
    daemon.mutations = {};
    daemon.reads = {
      "ui.network.offline": { offline: false, environment_blocked: false },
      "ui.backends.settings.list": {
        backends: [
          { name: "fulcrum", kind: "electrum", url: "ssl://fulcrum.example:50002", has_url: true },
          { name: "spare", kind: "electrum", url: "ssl://spare.example:50002", has_url: true },
          {
            name: "coingecko",
            kind: "coingecko",
            url: "https://api.coingecko.com/api/v3",
            has_url: true,
            url_safe_for_http_probe: true,
          },
        ],
        summary: { count: 3, default_backend: "fulcrum" },
      },
      // Only the backend this book's wallets sync from.
      "ui.backends.list": { backends: [{ name: "fulcrum" }] },
    };
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  async function advance(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  }

  it("never probes on a timer without the opt-in", async () => {
    render(<NetworkStatusIndicator daemonEnabled />);
    await advance(30 * 60 * 1000);
    expect(probeCalls()).toBe(0);
  });

  it("probes only the book's own connections, then every interval", async () => {
    useUiStore.setState({ connectionAutoCheck: true });
    render(<NetworkStatusIndicator daemonEnabled />);
    await advance(0);

    const electrum = daemon.mutations["ui.backends.electrum.test"];
    expect(electrum).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(electrum.mock.calls[0])).toContain("fulcrum.example");
    expect(daemon.mutations["ui.backends.http.test"]?.mock.calls.length ?? 0).toBe(0);

    await advance(5 * 60 * 1000 - 1000);
    expect(electrum).toHaveBeenCalledTimes(1);
    await advance(1000);
    expect(electrum).toHaveBeenCalledTimes(2);
    expect(probeCalls()).toBe(2);
  });

  it("stays silent while offline mode is on", async () => {
    useUiStore.setState({ connectionAutoCheck: true });
    daemon.reads["ui.network.offline"] = { offline: true, environment_blocked: false };
    render(<NetworkStatusIndicator daemonEnabled />);
    await advance(30 * 60 * 1000);
    expect(probeCalls()).toBe(0);
  });

  it("does not re-probe early when the panel remounts", async () => {
    useUiStore.setState({ connectionAutoCheck: true });
    const first = render(<NetworkStatusIndicator daemonEnabled />);
    await advance(0);
    expect(probeCalls()).toBe(1);
    first.unmount();

    await openPanel();
    await advance(60 * 1000);
    expect(probeCalls()).toBe(1);
    // The cached result still describes the connection after the remount.
    expect(screen.getAllByLabelText("network.health.healthy")).toHaveLength(1);
  });
});
