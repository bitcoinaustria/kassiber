const encoder = new TextEncoder();

let sessionPassphraseDigest: string | null = null;

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
}

export function clearSessionUnlockPassphrase(): void {
  sessionPassphraseDigest = null;
}

export function hasSessionUnlockPassphrase(): boolean {
  return sessionPassphraseDigest !== null;
}

export async function verifySessionUnlockPassphrase(
  passphrase: string,
): Promise<boolean> {
  if (!sessionPassphraseDigest) return false;
  return (await digestPassphrase(passphrase)) === sessionPassphraseDigest;
}
