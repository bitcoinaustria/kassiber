import type { DataMode } from "@/store/ui";

export function shouldUseDaemonUnlock({
  dataMode,
  hasIdentity,
  daemonAuthRequired,
}: {
  dataMode: DataMode;
  hasIdentity: boolean;
  daemonAuthRequired: boolean;
}) {
  return (dataMode === "real" && hasIdentity) || daemonAuthRequired;
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
