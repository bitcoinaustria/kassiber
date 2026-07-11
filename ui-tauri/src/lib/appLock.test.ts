import { describe, expect, it } from "vitest";

import {
  canEnrollTouchIdPassphrase,
  lockScreenConfig,
  shouldAutoPromptTouchId,
  shouldLockEncryptedWorkspaceOnLaunch,
  shouldRefreshTouchIdPassphrase,
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

  it("locks encrypted books on launch until this daemon session is unlocked", () => {
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        hasSessionUnlock: false,
      }),
    ).toBe(true);
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        hasSessionUnlock: false,
      }),
    ).toBe(true);
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: true,
        hasSessionUnlock: true,
      }),
    ).toBe(false);
    expect(
      shouldLockEncryptedWorkspaceOnLaunch({
        encryptedWorkspace: false,
        hasSessionUnlock: false,
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

  it("refreshes every enrolled desktop credential even when lock-screen offering is disabled", () => {
    expect(
      shouldRefreshTouchIdPassphrase({
        platformSupported: true,
        touchIdStatusConfigured: true,
      }),
    ).toBe(true);
    expect(
      shouldRefreshTouchIdPassphrase({
        platformSupported: true,
        touchIdStatusConfigured: false,
      }),
    ).toBe(false);
    expect(
      shouldRefreshTouchIdPassphrase({
        platformSupported: false,
        touchIdStatusConfigured: true,
      }),
    ).toBe(false);
  });

  it("offers re-enrollment for a stale credential even while the old policy is enabled", () => {
    expect(
      canEnrollTouchIdPassphrase({
        platformSupported: true,
        passphraseRequired: true,
        touchIdEnabled: true,
        touchIdAvailable: true,
        touchIdStale: true,
      }),
    ).toBe(true);
    expect(
      canEnrollTouchIdPassphrase({
        platformSupported: true,
        passphraseRequired: true,
        touchIdEnabled: true,
        touchIdAvailable: true,
        touchIdStale: false,
      }),
    ).toBe(false);
  });

  it("auto-prompts Touch ID only for foreground lock screens", () => {
    expect(
      shouldAutoPromptTouchId({
        autoPromptRequested: true,
        canUseTouchId: true,
        appVisible: true,
        windowFocused: true,
      }),
    ).toBe(true);
    expect(
      shouldAutoPromptTouchId({
        autoPromptRequested: true,
        canUseTouchId: true,
        appVisible: false,
        windowFocused: true,
      }),
    ).toBe(false);
    expect(
      shouldAutoPromptTouchId({
        autoPromptRequested: true,
        canUseTouchId: true,
        appVisible: true,
        windowFocused: false,
      }),
    ).toBe(false);
    expect(
      shouldAutoPromptTouchId({
        autoPromptRequested: false,
        canUseTouchId: true,
        appVisible: true,
        windowFocused: true,
      }),
    ).toBe(false);
  });
});
