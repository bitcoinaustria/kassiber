import { describe, expect, it } from "vitest";

import {
  backendPayload,
  backendRowToSettingsBackend,
  type Backend,
} from "./SettingsModel";
import { backendTypeIdForSettingsBackend } from "./SyncBackendSettingsModel";

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
});
