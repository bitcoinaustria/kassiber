import { describe, expect, it } from "vitest";

import {
  backendPayload,
  backendTrust,
  backendRowToSettingsBackend,
  deriveExplorerSettings,
  marketRateBackends,
  type Backend,
} from "./SettingsModel";
import {
  backendTypeIdForConnectionSetup,
  backendTypeIdForSettingsBackend,
} from "./SyncBackendSettingsModel";

describe("backend settings model", () => {
  it("keeps the stable backend id separate from the editable display name", () => {
    const backend = backendRowToSettingsBackend({
      name: "liquid",
      display_name: "My Liquid node",
      kind: "liquid-esplora",
      chain: "liquid",
      network: "liquidv1",
      url: "https://liquid.network/api",
      has_url: true,
      wallet_refs: ["Main/Default/Treasury"],
    });

    expect(backend.id).toBe("liquid");
    expect(backend.name).toBe("My Liquid node");
    expect(backend.walletRefs).toEqual(["Main/Default/Treasury"]);
  });

  it("carries the daemon health-probe safety flag", () => {
    const backend = backendRowToSettingsBackend({
      name: "mempool",
      display_name: "mempool",
      kind: "esplora",
      chain: "bitcoin",
      network: "main",
      url: "https://mempool.example.com/api",
      has_url: true,
      url_safe_for_http_probe: true,
    });

    expect(backend.urlSafeForHttpProbe).toBe(true);
  });

  it("serializes Silent Payments capability and scanner replacements", () => {
    const backend = backendRowToSettingsBackend({
      name: "sp-local",
      display_name: "SP scanner",
      kind: "custom",
      chain: "bitcoin",
      network: "main",
      url: "local://silent-payments",
      has_url: true,
      silent_payments: true,
    });

    expect(backend.silentPayments).toBe(true);
    expect(backend).not.toHaveProperty("silentPaymentScanFile");
    expect(backend).not.toHaveProperty("silentPaymentScanPath");

    const payload = backendPayload({
      ...backend,
      silentPaymentScanFile: "/tmp/sp-scan.json",
      silentPaymentScanPath: "/silent-payments/scan",
    });

    expect(payload.config).toMatchObject({
      silent_payments: true,
      silent_payment_scan_file: "/tmp/sp-scan.json",
      silent_payment_scan_path: "/silent-payments/scan",
    });

    const disabledPayload = backendPayload({
      ...backend,
      silentPayments: false,
      silentPaymentScanFile: "/tmp/kept-hidden.json",
      silentPaymentScanPath: "/silent-payments/disabled",
    });

    expect(disabledPayload.config).toMatchObject({
      silent_payments: false,
    });
    expect(disabledPayload.config).not.toHaveProperty(
      "silent_payment_scan_file",
    );
    expect(disabledPayload.config).not.toHaveProperty(
      "silent_payment_scan_path",
    );
  });

  it("updates display_name without renaming the backend key", () => {
    const payload = backendPayload({
      id: "liquid",
      name: "Desk Liquid indexer",
      url: "https://liquid.network/api",
      net: "LIQUID",
      kind: "liquid-esplora",
      chain: "liquid",
      network: "liquidv1",
      health: "configured",
      on: true,
      auth: "none",
    } satisfies Backend);

    expect(payload.name).toBe("liquid");
    expect(payload.config).toMatchObject({
      display_name: "Desk Liquid indexer",
    });
  });

  it("keeps a stored BTCPay API key when the edit field is left blank", () => {
    const payload = backendPayload({
      id: "shop-btcpay",
      name: "Shop BTCPay",
      url: "https://btcpay.example.com",
      net: "BTC",
      kind: "btcpay",
      chain: "bitcoin",
      network: "main",
      health: "configured",
      on: true,
      auth: "apikey",
    } satisfies Backend);

    expect(payload.name).toBe("shop-btcpay");
    expect(payload.kind).toBe("btcpay");
    expect(payload).not.toHaveProperty("token");
    expect(payload.clear).toEqual(["auth_header", "username", "password"]);
  });

  it("overwrites a BTCPay API key when a new value is entered", () => {
    const payload = backendPayload({
      id: "shop-btcpay",
      name: "Shop BTCPay",
      url: "https://btcpay.example.com",
      net: "BTC",
      kind: "btcpay",
      chain: "bitcoin",
      network: "main",
      health: "configured",
      on: true,
      auth: "apikey",
      token: "new-btcpay-key",
    } satisfies Backend);

    expect(payload.token).toBe("new-btcpay-key");
    expect(payload.clear).toEqual(["auth_header", "username", "password"]);
  });

  it("opens stored Liquid Electrum backends in the Liquid edit path", () => {
    const backend = backendRowToSettingsBackend({
      name: "desk-liquid",
      kind: "electrum",
      chain: "liquid",
      network: "liquidv1",
      url: "ssl://liquid.example:995",
      has_url: true,
    });

    expect(backendTypeIdForSettingsBackend(backend)).toBe("liquid");
  });

  it("opens graph backend diagnostics in the matching network setup path", () => {
    expect(
      backendTypeIdForConnectionSetup({
        sourceId: "liquid",
        reason: "Transaction graph needs a Liquid backend",
        backendKind: "liquid",
      }),
    ).toBe("liquid");
    expect(
      backendTypeIdForConnectionSetup({
        sourceId: "bitcoin",
        reason: "Transaction graph needs a Bitcoin backend",
        backendKind: "bitcoin",
      }),
    ).toBe("bitcoin");
  });

  it("preserves redacted proxy credentials when saving unrelated backend edits", () => {
    const backend = backendRowToSettingsBackend({
      name: "mempool",
      display_name: "Mempool",
      kind: "esplora",
      chain: "bitcoin",
      network: "main",
      url: "https://mempool.example/api",
      has_url: true,
      tor_proxy: "socks5h://redacted@127.0.0.1:9050",
    });

    expect(backend.proxy).toEqual({
      host: "127.0.0.1",
      port: "9050",
      redactedCredentials: true,
    });
    const payload = backendPayload(backend);
    expect(payload).not.toHaveProperty("tor_proxy");
    expect(payload.clear).not.toContain("tor_proxy");
  });

  it("lets the built-in liquid backend recover from an accidental bitcoin chain", () => {
    const backend = backendRowToSettingsBackend({
      name: "liquid",
      kind: "electrum",
      chain: "bitcoin",
      network: "main",
      url: "ssl://les.bullbitcoin.com:995",
      has_url: true,
    });

    expect(backendTypeIdForSettingsBackend(backend)).toBe("liquid");
  });

  it("only treats proxy settings as shielding for transports that use them", () => {
    const electrum = backendRowToSettingsBackend({
      name: "fulcrum",
      kind: "electrum",
      chain: "bitcoin",
      network: "main",
      url: "ssl://fulcrum.example:50002",
      has_url: true,
      tor_proxy: "127.0.0.1:9050",
    });
    const lnd = backendRowToSettingsBackend({
      name: "lnd",
      kind: "lnd",
      chain: "lightning",
      network: "main",
      url: "https://lnd.example",
      has_url: true,
      tor_proxy: "127.0.0.1:9050",
    });

    expect(backendTrust(electrum).posture).toBe("shielded");
    expect(backendTrust(lnd).posture).toBe("remote");
  });

  it("shows the local mempool backend as the active market-price endpoint", () => {
    const backends = [
      backendRowToSettingsBackend({
        name: "mempool",
        kind: "mempool",
        chain: "bitcoin",
        network: "main",
        url: "https://mempool.bitcoin-austria.at/api",
        is_default: true,
        has_url: true,
      }),
      backendRowToSettingsBackend({
        name: "desk-mempool",
        kind: "mempool",
        chain: "bitcoin",
        network: "main",
        url: "http://127.0.0.1:3006/api",
        infrastructure_owner: "self",
        has_url: true,
      }),
    ];

    const [marketBackend] = marketRateBackends(
      {
        background_enabled: true,
        report_read_sync: false,
        source_classes: { market_rates: true },
        market_rate_provider: "mempool",
        market_rate_providers: ["mempool"],
      },
      backends,
    );

    expect(marketBackend.url).toBe("http://127.0.0.1:3006/api");
    expect(marketBackend.health).toBe("via desk-mempool");
    expect(marketBackend.infrastructureOwner).toBe("self");
    expect(marketBackend.on).toBe(true);
  });

  it("disables public explorer fallbacks when the active backend is regtest", () => {
    const settings = deriveExplorerSettings([
      backendRowToSettingsBackend({
        name: "core-regtest",
        kind: "bitcoinrpc",
        chain: "bitcoin",
        network: "regtest",
        url: "http://127.0.0.1:18456",
        is_default: true,
        has_url: true,
      }),
    ]);

    expect(settings).toEqual({
      bitcoinBaseUrl: "",
      liquidBaseUrl: "",
      publicFallbacks: false,
    });
  });
});
