import { describe, expect, test } from "vitest";

import {
  connectionSupportsLightningCapability,
  coreLightningBackendModeValid,
  coreLightningUrl,
  isCoreLightningCommandoBackend,
  lightningCapabilitiesForConnection,
  type CoreLightningBackendFormState,
} from "./lightning";

function makeState(
  overrides: Partial<CoreLightningBackendFormState> = {},
): CoreLightningBackendFormState {
  return {
    lightningCli: "",
    lightningDir: "",
    rpcFile: "",
    hadLightningDir: false,
    hadRpcFile: false,
    ...overrides,
  };
}

describe("coreLightningBackendModeValid", () => {
  test("blank create is invalid", () => {
    expect(coreLightningBackendModeValid(makeState())).toBe(false);
  });

  test("rpc_file, lightning_dir or a lightning-cli wrapper alone is valid", () => {
    for (const overrides of [
      { rpcFile: "/tmp/lightning-rpc" },
      { lightningDir: "/home/op/.lightning" },
      { lightningCli: "/usr/local/bin/cln-wrapper" },
    ]) {
      expect(coreLightningBackendModeValid(makeState(overrides))).toBe(true);
    }
  });

  test("edit with redacted local-rpc fields and no input is valid", () => {
    expect(
      coreLightningBackendModeValid(makeState({ hadLightningDir: true })),
    ).toBe(true);
    expect(
      coreLightningBackendModeValid(makeState({ hadRpcFile: true })),
    ).toBe(true);
  });
});

describe("coreLightningUrl", () => {
  test("blank and commando URLs become local; others are kept", () => {
    expect(coreLightningUrl("")).toBe("cln://local");
    expect(coreLightningUrl(" CLN://Commando ")).toBe("cln://local");
    expect(coreLightningUrl("cln://local")).toBe("cln://local");
    expect(coreLightningUrl(" cln://node-a ")).toBe("cln://node-a");
  });
});

describe("isCoreLightningCommandoBackend", () => {
  // Same test as the core's `_refuse_commando`: any commando field marks it.
  test.each([
    { kind: "coreln", url: "cln://commando", auth: "none" },
    { kind: "coreln", url: "CLN://Commando", auth: "none" },
    { kind: "coreln", url: "cln://local", auth: "apikey" },
    { kind: "coreln", url: "cln://local", auth: "none", commandoPeerId: "Configured peer" },
  ])("flags %o", (backend) => {
    expect(isCoreLightningCommandoBackend(backend)).toBe(true);
  });

  test.each([
    { kind: "coreln", url: "cln://local", auth: "none" },
    { kind: "coreln", url: "cln://local", auth: "none", commandoPeerId: "  " },
    { kind: "lnd", url: "https://127.0.0.1:8080", auth: "apikey" },
    { kind: "btcpay", url: "cln://commando", auth: "apikey" },
  ])("does not flag %o", (backend) => {
    expect(isCoreLightningCommandoBackend(backend)).toBe(false);
  });
});

describe("Lightning adapter capability helpers", () => {
  test("uses explicit daemon capability metadata when present", () => {
    const caps = lightningCapabilitiesForConnection({
      kind: "lnd",
      lightningCapabilities: {
        nodeSnapshot: true,
        routingProfitability: false,
        channelBalances: true,
      },
    });

    expect(caps.nodeSnapshot).toBe(true);
    expect(caps.routingProfitability).toBe(false);
    expect(caps.channelBalances).toBe(true);
    expect(caps.forwardEvents).toBe(false);
  });

  test("legacy fallback is limited to implemented node adapters", () => {
    expect(
      connectionSupportsLightningCapability(
        { kind: "lnd" },
        "routingProfitability",
      ),
    ).toBe(true);
    expect(
      connectionSupportsLightningCapability(
        { kind: "nwc" },
        "routingProfitability",
      ),
    ).toBe(false);
  });

  test("explicit unsupported capability overrides kind fallback", () => {
    expect(
      connectionSupportsLightningCapability(
        {
          kind: "lnd",
          lightningCapabilities: {
            nodeSnapshot: true,
            routingProfitability: false,
          },
        },
        "routingProfitability",
      ),
    ).toBe(false);
  });
});
