import { describe, expect, it } from "vitest";

import {
  connectionProbeRequest,
  connectionRowFromBackend,
} from "./NetworkStatusIndicatorModel";
import {
  regtestBackendConnections,
  visibleConnectionBackends,
} from "./backendConnectionRows";
import type { Backend } from "./settings/SettingsModel";

function backend(overrides: Partial<Backend>): Backend {
  return {
    id: "backend",
    name: "backend",
    url: "https://example.invalid",
    net: "BTC",
    kind: "esplora",
    chain: "bitcoin",
    network: "main",
    health: "configured",
    on: true,
    auth: "none",
    isDefault: false,
    walletRefs: [],
    ...overrides,
  };
}

describe("visibleConnectionBackends", () => {
  it("hides unused public defaults when regtest backends are configured", () => {
    const rows = visibleConnectionBackends([
      backend({
        id: "core-regtest",
        name: "core-regtest",
        kind: "bitcoinrpc",
        network: "regtest",
        url: "http://127.0.0.1:18454",
        isDefault: true,
        walletRefs: ["Regtest Demo/Full Accounting/Treasury"],
      }),
      backend({
        id: "liquid-mempool-regtest",
        name: "liquid-mempool-regtest",
        net: "LIQUID",
        chain: "liquid",
        network: "elementsregtest",
        url: "http://127.0.0.1:18560/api",
      }),
      backend({
        id: "mempool",
        name: "mempool",
        url: "https://mempool.bitcoin-austria.at/api",
      }),
      backend({
        id: "liquid",
        name: "liquid",
        net: "LIQUID",
        chain: "liquid",
        network: "liquidv1",
        url: "ssl://les.bullbitcoin.com:995",
      }),
    ]);

    expect(rows.map((row) => row.id)).toEqual([
      "core-regtest",
      "liquid-mempool-regtest",
    ]);
  });

  it("maps local regtest backends to first-party connection rows", () => {
    const rows = regtestBackendConnections([
      backend({
        id: "bitcoin-mempool-regtest",
        name: "Bitcoin Mempool Regtest",
        kind: "mempool",
        network: "regtest",
        url: "http://127.0.0.1:18555/api",
      }),
      backend({
        id: "mempool",
        name: "mempool",
        url: "https://mempool.bitcoin-austria.at/api",
      }),
    ]);

    expect(rows).toMatchObject([
      {
        id: "backend:bitcoin-mempool-regtest",
        role: "backend",
        kind: "backend",
        label: "Bitcoin Mempool Regtest",
        endpoint: "http://127.0.0.1:18555/api",
        settingsHash: "bitcoin",
      },
    ]);
  });

  it("keeps normal configured endpoints outside regtest mode", () => {
    const rows = visibleConnectionBackends([
      backend({ id: "mempool", name: "mempool", isDefault: true }),
      backend({
        id: "liquid",
        name: "liquid",
        net: "LIQUID",
        chain: "liquid",
        network: "liquidv1",
        url: "ssl://les.bullbitcoin.com:995",
      }),
    ]);

    expect(rows.map((row) => row.id)).toEqual(["mempool", "liquid"]);
  });

  it("keeps saved HTTP TLS settings on background health probes", () => {
    const row = connectionRowFromBackend(
      backend({
        id: "private-esplora",
        name: "Private Esplora",
        kind: "esplora",
        url: "https://private.example/api",
        urlSafeForHttpProbe: true,
        certificate: "/private/esplora-ca.pem",
        trustSsl: false,
      }),
      "backend:private-esplora",
      "private-esplora",
    );

    expect(connectionProbeRequest(row)).toEqual({
      kind: "http",
      args: {
        url: "https://private.example/api",
        proxy: undefined,
        insecure: false,
        certificate: "/private/esplora-ca.pem",
        timeout: 5,
      },
    });
  });

  it("passes saved Core proxy and TLS settings to the health probe", () => {
    const row = connectionRowFromBackend(
      backend({
        id: "private-core",
        name: "Private Core",
        kind: "bitcoinrpc",
        url: "https://core.example",
        proxy: { host: "127.0.0.1", port: "9050" },
        certificate: "/private/core-ca.pem",
        trustSsl: false,
      }),
      "backend:private-core",
      "private-core",
    );

    expect(connectionProbeRequest(row)).toEqual({
      kind: "bitcoinrpc",
      args: {
        backend: "private-core",
        url: undefined,
        proxy: "127.0.0.1:9050",
        timeout: 5,
        config: {
          insecure: false,
          certificate: "/private/core-ca.pem",
        },
      },
    });
  });
});
