import { describe, expect, it } from "vitest";

import { lockScreenConfig, shouldUseDaemonUnlock } from "./appLock";

describe("app lock decisions", () => {
  it("routes real workspaces through the daemon unlock path", () => {
    expect(
      shouldUseDaemonUnlock({
        dataMode: "real",
        hasIdentity: true,
        daemonAuthRequired: false,
      }),
    ).toBe(true);
  });

  it("keeps mock sessions out of daemon unlock unless auth is explicitly required", () => {
    expect(
      shouldUseDaemonUnlock({
        dataMode: "mock",
        hasIdentity: true,
        daemonAuthRequired: false,
      }),
    ).toBe(false);
    expect(
      shouldUseDaemonUnlock({
        dataMode: "mock",
        hasIdentity: true,
        daemonAuthRequired: true,
      }),
    ).toBe(true);
  });

  it("does not ask plaintext daemon sessions for a passphrase", () => {
    expect(
      lockScreenConfig({
        daemonAuthRequired: false,
        encryptedWorkspace: false,
      }),
    ).toEqual({
      reason: "Open the local daemon session to continue.",
      passphraseRequired: false,
    });
  });

  it("requires a passphrase when the daemon reports auth is required", () => {
    expect(
      lockScreenConfig({
        daemonAuthRequired: true,
        encryptedWorkspace: false,
      }),
    ).toEqual({
      reason:
        "The daemon needs the database passphrase before it can return live books data.",
      passphraseRequired: true,
    });
  });
});
