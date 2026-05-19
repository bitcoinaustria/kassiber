import { describe, expect, test } from "vitest";

import {
  coreLightningBackendModeValid,
  type CoreLightningBackendFormState,
} from "./lightning";

function makeState(
  overrides: Partial<CoreLightningBackendFormState> = {},
): CoreLightningBackendFormState {
  return {
    commandoPeerId: "",
    rune: "",
    lightningDir: "",
    rpcFile: "",
    hadRune: false,
    hadCommandoPeerId: false,
    hadLightningDir: false,
    hadRpcFile: false,
    ...overrides,
  };
}

describe("coreLightningBackendModeValid (M-2)", () => {
  test("blank create is invalid", () => {
    expect(coreLightningBackendModeValid(makeState())).toBe(false);
  });

  test("create with both commando fields is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({ commandoPeerId: "02abc", rune: "rune-string" }),
      ),
    ).toBe(true);
  });

  test("create with only the peer id is invalid", () => {
    expect(
      coreLightningBackendModeValid(makeState({ commandoPeerId: "02abc" })),
    ).toBe(false);
  });

  test("create with only the rune is invalid", () => {
    expect(
      coreLightningBackendModeValid(makeState({ rune: "rune-string" })),
    ).toBe(false);
  });

  test("local RPC mode: rpc_file alone is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({ rpcFile: "/tmp/lightning-rpc" }),
      ),
    ).toBe(true);
  });

  test("local RPC mode: lightning_dir alone is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({ lightningDir: "/home/op/.lightning" }),
      ),
    ).toBe(true);
  });

  test("edit with redacted commando fields and no input is valid", () => {
    // Editing an existing working commando backend: the form clears the
    // sentinel placeholders so the inputs are blank, but the existing
    // values stay on the row until the user overrides them.
    expect(
      coreLightningBackendModeValid(
        makeState({ hadRune: true, hadCommandoPeerId: true }),
      ),
    ).toBe(true);
  });

  test("edit with redacted local-rpc fields and no input is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({ hadLightningDir: true }),
      ),
    ).toBe(true);
    expect(
      coreLightningBackendModeValid(makeState({ hadRpcFile: true })),
    ).toBe(true);
  });

  test("edit overriding peer id while rune remains stored is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({
          commandoPeerId: "02new",
          hadRune: true,
          hadCommandoPeerId: true,
        }),
      ),
    ).toBe(true);
  });

  test("edit clearing all commando state and adding local rpc is valid", () => {
    expect(
      coreLightningBackendModeValid(
        makeState({
          rpcFile: "/tmp/lightning-rpc",
          // Existing rune/commando state was wiped (admin replaced
          // remote commando with local RPC); should still be valid.
        }),
      ),
    ).toBe(true);
  });

  test("edit with no input AND no prior config is invalid", () => {
    // Defensive: an inconsistent initial state (auth!=apikey AND no
    // local config) should not silently let the user save an empty
    // backend.
    expect(coreLightningBackendModeValid(makeState())).toBe(false);
  });
});
