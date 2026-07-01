import { describe, expect, it } from "vitest";

import { visibleConnectionBackends } from "./NetworkStatusIndicator";
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
  it("hides unused public defaults when the active backend is regtest", () => {
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

    expect(rows.map((row) => row.id)).toEqual(["core-regtest"]);
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
});
