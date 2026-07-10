import { isDaemonDataMode, type DataMode } from "@/store/ui";

export function shouldUseDaemonUnlock({
  dataMode,
  hasIdentity,
  daemonAuthRequired,
}: {
  dataMode: DataMode;
  hasIdentity: boolean;
  daemonAuthRequired: boolean;
}) {
  return (isDaemonDataMode(dataMode) && hasIdentity) || daemonAuthRequired;
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

export function lockScreenConfig({
  daemonAuthRequired,
  encryptedWorkspace,
}: {
  daemonAuthRequired: boolean;
  encryptedWorkspace: boolean;
}) {
  return {
    reason: daemonAuthRequired
      ? "The daemon needs the database passphrase before it can return live books data."
      : encryptedWorkspace
        ? undefined
        : "Open the local daemon session to continue.",
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
