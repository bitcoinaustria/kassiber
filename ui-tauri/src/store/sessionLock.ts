const encoder = new TextEncoder();

let sessionPassphraseDigest: string | null = null;
let failedAttempts = 0;
let lockedUntilMs = 0;

async function digestPassphrase(passphrase: string): Promise<string> {
  const digest = await globalThis.crypto.subtle.digest(
    "SHA-256",
    encoder.encode(passphrase),
  );
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export async function setSessionUnlockPassphrase(
  passphrase: string | null,
): Promise<void> {
  sessionPassphraseDigest = passphrase ? await digestPassphrase(passphrase) : null;
  failedAttempts = 0;
  lockedUntilMs = 0;
}

export function clearSessionUnlockPassphrase(): void {
  sessionPassphraseDigest = null;
  failedAttempts = 0;
  lockedUntilMs = 0;
}

export function hasSessionUnlockPassphrase(): boolean {
  return sessionPassphraseDigest !== null;
}

function timingSafeEqualHex(left: string, right: string): boolean {
  const maxLength = Math.max(left.length, right.length);
  let diff = left.length ^ right.length;
  for (let index = 0; index < maxLength; index += 1) {
    diff |= (left.charCodeAt(index) || 0) ^ (right.charCodeAt(index) || 0);
  }
  return diff === 0;
}

export function getSessionUnlockBackoffMs(): number {
  return Math.max(0, lockedUntilMs - Date.now());
}

export async function verifySessionUnlockPassphrase(
  passphrase: string,
): Promise<boolean> {
  if (!sessionPassphraseDigest) return false;
  if (getSessionUnlockBackoffMs() > 0) return false;

  const ok = timingSafeEqualHex(
    await digestPassphrase(passphrase),
    sessionPassphraseDigest,
  );
  if (ok) {
    failedAttempts = 0;
    lockedUntilMs = 0;
    return true;
  }

  failedAttempts += 1;
  if (failedAttempts >= 3) {
    lockedUntilMs =
      Date.now() + Math.min(30_000, 1_000 * 2 ** (failedAttempts - 3));
  }
  return false;
}
