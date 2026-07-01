import { describe, expect, it } from "vitest";

import {
  lockScreenConfig,
  shouldLockEncryptedWorkspaceOnLaunch,
  shouldStoreTouchIdPassphrase,
  shouldUseDaemonUnlock,
} from "./appLock";

describe("app lock decisions", () => {
  it("routes real workspaces through the daemon unlock path", () => {
    expect(
      shouldUseDaemonUnlock({
        dataMode: "real",
        hasIdentity: true,
        daemonAuthRequired: false,
      }),
    ).toBe(true);
    expect(
      shouldUseDaemonUnlock({
        dataMode: "regtest",
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

  it("does not proactively lock encrypted books on launch unless the user enabled it", () => {
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        requirePassphraseOnLaunch: false,
        hasSessionUnlock: false,
      }),
    ).toBe(false);
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        requirePassphraseOnLaunch: true,
        hasSessionUnlock: false,
      }),
    ).toBe(true);
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        requirePassphraseOnLaunch: true,
        hasSessionUnlock: true,
      }),
    ).toBe(false);
  });

  it("stores Touch ID passphrases only after explicit enrollment or an existing current-root entry", () => {
    expect(
      shouldStoreTouchIdPassphrase({
        platformSupported: true,
        rememberWithTouchId: true,
        touchIdStatusConfigured: false,
      }),
    ).toBe(true);
    expect(
      shouldStoreTouchIdPassphrase({
        platformSupported: true,
        rememberWithTouchId: undefined,
        touchIdStatusConfigured: true,
      }),
    ).toBe(true);
    expect(
      shouldStoreTouchIdPassphrase({
        platformSupported: true,
        rememberWithTouchId: undefined,
        touchIdStatusConfigured: false,
      }),
    ).toBe(false);
    expect(
      shouldStoreTouchIdPassphrase({
        platformSupported: true,
        rememberWithTouchId: false,
        touchIdStatusConfigured: true,
      }),
    ).toBe(false);
  });
});
