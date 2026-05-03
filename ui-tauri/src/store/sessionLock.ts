let unlockedDaemonSession = false;

export async function setSessionUnlockPassphrase(
  passphrase: string | null,
): Promise<void> {
  // Keep only an ephemeral "the daemon accepted a passphrase this session"
  // marker. The renderer must not keep a passphrase-derived verifier that can
  // be copied into an offline cracking workflow.
  unlockedDaemonSession = Boolean(passphrase);
}

export function clearSessionUnlockPassphrase(): void {
  unlockedDaemonSession = false;
}

export function hasSessionUnlockPassphrase(): boolean {
  return unlockedDaemonSession;
}

export function getSessionUnlockBackoffMs(): number {
  return 0;
}

export async function verifySessionUnlockPassphrase(
  passphrase: string,
): Promise<boolean> {
  void passphrase;
  return false;
}
