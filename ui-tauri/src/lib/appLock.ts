export function shouldUseDaemonUnlock({
  hasIdentity,
  daemonAuthRequired,
}: {
  hasIdentity: boolean;
  daemonAuthRequired: boolean;
}) {
  return hasIdentity || daemonAuthRequired;
}

export function shouldLockEncryptedWorkspaceOnLaunch({
  encryptedWorkspace,
  hasSessionUnlock,
}: {
  encryptedWorkspace: boolean;
  hasSessionUnlock: boolean;
}) {
  return encryptedWorkspace && !hasSessionUnlock;
}

/**
 * Why the lock screen is up, for its copy: `encrypted` books ask for their
 * passphrase, `daemon-auth` means the daemon asked for it again (a restarted
 * session), and `session` is a plaintext book behind the app lock alone.
 */
export type LockReason = "encrypted" | "daemon-auth" | "session";

export function lockScreenConfig({
  daemonAuthRequired,
  encryptedWorkspace,
}: {
  daemonAuthRequired: boolean;
  encryptedWorkspace: boolean;
}): { reason: LockReason; passphraseRequired: boolean } {
  return {
    reason: daemonAuthRequired
      ? "daemon-auth"
      : encryptedWorkspace
        ? "encrypted"
        : "session",
    passphraseRequired: encryptedWorkspace || daemonAuthRequired,
  };
}

export function shouldStoreTouchIdPassphrase({
  platformSupported,
  rememberWithTouchId,
  touchIdStatusConfigured,
}: {
  platformSupported: boolean;
  rememberWithTouchId?: boolean;
  touchIdStatusConfigured: boolean;
}) {
  if (!platformSupported || rememberWithTouchId === false) {
    return false;
  }
  return rememberWithTouchId === true || touchIdStatusConfigured;
}

export function shouldRefreshTouchIdPassphrase({
  platformSupported,
  touchIdStatusConfigured,
}: {
  platformSupported: boolean;
  touchIdStatusConfigured: boolean;
}) {
  return platformSupported && touchIdStatusConfigured;
}

export function canEnrollTouchIdPassphrase({
  platformSupported,
  passphraseRequired,
  touchIdEnabled,
  touchIdAvailable,
  touchIdStale,
}: {
  platformSupported: boolean;
  passphraseRequired: boolean;
  touchIdEnabled: boolean;
  touchIdAvailable: boolean;
  touchIdStale: boolean;
}) {
  return (
    platformSupported &&
    passphraseRequired &&
    touchIdAvailable &&
    (!touchIdEnabled || touchIdStale)
  );
}

export function shouldAutoPromptTouchId({
  autoPromptRequested,
  canUseTouchId,
  appVisible,
  windowFocused,
}: {
  autoPromptRequested: boolean;
  canUseTouchId: boolean;
  appVisible: boolean;
  windowFocused: boolean;
}) {
  return autoPromptRequested && canUseTouchId && appVisible && windowFocused;
}

export function appCanStartTouchIdPrompt() {
  if (typeof document === "undefined") {
    return { appVisible: true, windowFocused: true };
  }
  const appVisible = document.visibilityState === "visible";
  const windowFocused =
    typeof document.hasFocus !== "function" || document.hasFocus();
  return { appVisible, windowFocused };
}

/**
 * The lock screen's own words for a daemon refusal the user can act on, by
 * the way they tried to unlock; null keeps the daemon's detail. A refused
 * passphrase is the everyday failure, and a refused Touch ID passphrase means
 * the saved one went stale, so neither should read as raw daemon English.
 */
export function unlockFailureKey(
  method: "passphrase" | "touch-id",
  code: string | null | undefined,
): "lock.passphraseFailed" | "lock.touchIdStale" | "lock.touchIdNotFound" | null {
  if (code === "local_auth_denied") {
    return method === "passphrase" ? "lock.passphraseFailed" : "lock.touchIdStale";
  }
  if (method === "touch-id" && code === "touch_id_passphrase_not_found") {
    return "lock.touchIdNotFound";
  }
  return null;
}
