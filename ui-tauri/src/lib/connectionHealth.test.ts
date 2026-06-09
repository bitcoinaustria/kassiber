import { describe, expect, it } from "vitest";

import {
  CONNECTION_HEALTH_CHECK_INTERVAL_MS,
  CONNECTION_HEALTH_CHECK_JITTER_MS,
  canRunConnectionHealthChecks,
  connectionHealthTone,
  connectionProbeKind,
  endpointWithPort,
  isConnectionHealthStale,
  nextConnectionHealthCheckDelayMs,
  settingsHashForConnection,
  type ConnectionHealthSnapshot,
} from "./connectionHealth";

describe("connection health model", () => {
  it("renders endpoints with explicit ports", () => {
    expect(endpointWithPort("https://mempool.space/api")).toBe(
      "https://mempool.space:443/api",
    );
    expect(endpointWithPort("http://127.0.0.1:8332")).toBe(
      "http://127.0.0.1:8332",
    );
    expect(endpointWithPort("ssl://index.example.com:50002")).toBe(
      "ssl://index.example.com:50002",
    );
    expect(endpointWithPort("tcp://index.example.com")).toBe(
      "tcp://index.example.com:50001",
    );
  });

  it("routes connection types to the available probe", () => {
    expect(
      connectionProbeKind({
        id: "electrum",
        name: "Electrum",
        url: "ssl://index.example.com:50002",
        kind: "electrum",
        net: "BTC",
      }),
    ).toBe("electrum");
    expect(
      connectionProbeKind({
        id: "esplora",
        name: "Explorer",
        url: "https://mempool.example.com/api",
        kind: "esplora",
        net: "BTC",
      }),
    ).toBe("http");
    expect(
      connectionProbeKind({
        id: "cln",
        name: "Core Lightning",
        url: "cln://commando",
        kind: "coreln",
        net: "LN",
      }),
    ).toBe("unsupported");
  });

  it("maps connections to settings sections", () => {
    expect(
      settingsHashForConnection({
        id: "btc",
        name: "Bitcoin",
        url: "https://mempool.example.com/api",
        net: "BTC",
      }),
    ).toBe("bitcoin");
    expect(
      settingsHashForConnection({
        id: "fx",
        name: "Coinbase",
        url: "https://api.exchange.coinbase.com",
        net: "FX",
      }),
    ).toBe("market");
  });

  it("keeps mixed healthy and failed connections orange", () => {
    const healthy: ConnectionHealthSnapshot = { status: "healthy" };
    const failed: ConnectionHealthSnapshot = { status: "unhealthy" };

    expect(connectionHealthTone("online", [healthy, healthy])).toBe("online");
    expect(connectionHealthTone("online", [healthy, failed])).toBe("warning");
    expect(connectionHealthTone("online", [healthy, failed, failed])).toBe(
      "warning",
    );
    expect(connectionHealthTone("online", [{ status: "checking" }])).toBe(
      "warning",
    );
    expect(connectionHealthTone("online", [failed])).toBe("error");
    expect(connectionHealthTone("online", [failed, failed])).toBe("error");
    expect(connectionHealthTone("offline", [healthy])).toBe("error");
  });

  it("gates automatic checks to unlocked, online, visible, idle app state", () => {
    const ready = {
      checking: false,
      checkableConnectionCount: 1,
      daemonEnabled: true,
      documentVisible: true,
      networkStatus: "online" as const,
    };

    expect(canRunConnectionHealthChecks(ready)).toBe(true);
    expect(
      canRunConnectionHealthChecks({ ...ready, checking: true }),
    ).toBe(false);
    expect(
      canRunConnectionHealthChecks({ ...ready, checkableConnectionCount: 0 }),
    ).toBe(false);
    expect(
      canRunConnectionHealthChecks({ ...ready, daemonEnabled: false }),
    ).toBe(false);
    expect(
      canRunConnectionHealthChecks({ ...ready, documentVisible: false }),
    ).toBe(false);
    expect(
      canRunConnectionHealthChecks({ ...ready, networkStatus: "offline" }),
    ).toBe(false);
  });

  it("adds bounded jitter to the 60 second refresh cadence", () => {
    expect(nextConnectionHealthCheckDelayMs(() => 0)).toBe(
      CONNECTION_HEALTH_CHECK_INTERVAL_MS - CONNECTION_HEALTH_CHECK_JITTER_MS,
    );
    expect(nextConnectionHealthCheckDelayMs(() => 0.5)).toBe(
      CONNECTION_HEALTH_CHECK_INTERVAL_MS,
    );
    expect(nextConnectionHealthCheckDelayMs(() => 1)).toBe(
      CONNECTION_HEALTH_CHECK_INTERVAL_MS + CONNECTION_HEALTH_CHECK_JITTER_MS,
    );
  });

  it("treats missing, invalid, and old checks as stale", () => {
    const now = Date.parse("2026-06-09T12:00:00.000Z");

    expect(isConnectionHealthStale(undefined, now)).toBe(true);
    expect(isConnectionHealthStale("not-a-date", now)).toBe(true);
    expect(isConnectionHealthStale("2026-06-09T11:59:01.000Z", now)).toBe(
      false,
    );
    expect(isConnectionHealthStale("2026-06-09T11:59:00.000Z", now)).toBe(
      true,
    );
  });
});
