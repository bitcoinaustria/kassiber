/**
 * Parse a free-form blob of Bitcoin addresses pasted or loaded from a file into
 * a clean, de-duplicated list ready to create an "address" wallet.
 *
 * This powers the pre-HD "Address list" connection (a flat Bitcoin Core keypool
 * has no xpub/descriptor, so every address is scanned individually — there is no
 * gap limit). Address validation here is a lightweight mainnet sanity check, not
 * a checksum verification: it keeps obvious junk (labels, headers, testnet
 * addresses, blank lines) out of the submitted set. The authoritative check
 * happens in the daemon when the wallet syncs.
 *
 * Kassiber is watch-only: key material must NEVER be accepted here. Anything that
 * looks like a private key (WIF, BIP38, xprv-family), an extended public key
 * (xpub-family) or a raw public key is detected, counted, and purged — it is
 * never added to the submitted set and never echoed back in any sample.
 */

// Base58Check P2PKH ("1…") / P2SH ("3…"). Base58 alphabet excludes 0 O I l.
const MAINNET_BASE58 = /^[13][1-9A-HJ-NP-Za-km-z]{25,39}$/;
// Bech32 / Bech32m (segwit v0–v1, "bc1…"). Charset is case-uniform.
const MAINNET_BECH32_LOWER = /^bc1[qpzry9x8gf2tvdw0s3jn54khce6mua7l]{6,87}$/;
const MAINNET_BECH32_UPPER = /^BC1[QPZRY9X8GF2TVDW0S3JN54KHCE6MUA7L]{6,87}$/;

// Key material — never valid input for a watch-only address list. These patterns
// are intentionally self-contained and anchored/length-bounded (stricter than the
// startsWith-only prefix helpers in walletMaterialFormat.ts / samouraiSourceSet.ts)
// so this security guard stays auditable in one place; do not replace them with
// those looser, conversion-oriented checks.
const WIF = /^[5KL][1-9A-HJ-NP-Za-km-z]{50,51}$/; // 5… uncompressed, K…/L… compressed
const BIP38 = /^6P[1-9A-HJ-NP-Za-km-z]{56}$/; // encrypted private key
const EXT_PRIV =
  /^(?:xprv|yprv|zprv|tprv|uprv|vprv|Yprv|Zprv|Uprv|Vprv)[1-9A-HJ-NP-Za-km-z]{70,120}$/;
const EXT_PUB =
  /^(?:xpub|ypub|zpub|tpub|upub|vpub|Ypub|Zpub|Upub|Vpub)[1-9A-HJ-NP-Za-km-z]{70,120}$/;
const HEX_PUBKEY = /^(?:0[23][0-9a-fA-F]{64}|04[0-9a-fA-F]{128})$/;

export type KeyKind = "private" | "public";

export interface AddressListParseResult {
  /** Every non-empty, non-key token found, in order, including duplicates. */
  entries: string[];
  /** Unique mainnet-looking addresses, order-preserving — the set to submit. */
  valid: string[];
  /** Unique tokens that do not look like mainnet Bitcoin addresses. */
  invalid: string[];
  /** Count of duplicate valid addresses that were collapsed. */
  duplicates: number;
  /** Private keys (WIF/BIP38/xprv-family) seen — counted, never accepted. */
  privateKeys: number;
  /** Extended or raw public keys seen — counted, never accepted. */
  publicKeys: number;
}

/**
 * Classify a token as key material. Returns null for anything that is not an
 * obvious key (addresses, junk). Used both to keep keys out of the submit set
 * and to scrub them from the input field.
 */
export function classifyKeyMaterial(value: string): KeyKind | null {
  const token = value.trim();
  if (WIF.test(token) || BIP38.test(token) || EXT_PRIV.test(token)) {
    return "private";
  }
  if (EXT_PUB.test(token) || HEX_PUBKEY.test(token)) {
    return "public";
  }
  return null;
}

/**
 * Heuristic mainnet-address check. Deliberately conservative: it accepts the
 * three mainnet address shapes and rejects everything else (including testnet
 * `tb1…`/`m…`/`n…`/`2…`). It does NOT verify the checksum.
 */
export function looksLikeMainnetAddress(value: string): boolean {
  const token = value.trim();
  return (
    MAINNET_BASE58.test(token) ||
    MAINNET_BECH32_LOWER.test(token) ||
    MAINNET_BECH32_UPPER.test(token)
  );
}

/** Split on any whitespace, comma, or semicolon so paste and CSV both work. */
export function parseAddressList(input: string): AddressListParseResult {
  const valid: string[] = [];
  const invalid: string[] = [];
  const entries: string[] = [];
  const seenValid = new Set<string>();
  const seenInvalid = new Set<string>();
  let duplicates = 0;
  let privateKeys = 0;
  let publicKeys = 0;

  for (const raw of input.split(/[\s,;]+/)) {
    const token = raw.trim();
    if (!token) continue;

    // Key material is never an address and never accepted — count and drop it
    // before it can reach the valid/invalid sets or any displayed sample.
    const keyKind = classifyKeyMaterial(token);
    if (keyKind === "private") {
      privateKeys += 1;
      continue;
    }
    if (keyKind === "public") {
      publicKeys += 1;
      continue;
    }

    entries.push(token);
    if (looksLikeMainnetAddress(token)) {
      if (seenValid.has(token)) {
        duplicates += 1;
        continue;
      }
      seenValid.add(token);
      valid.push(token);
    } else if (!seenInvalid.has(token)) {
      seenInvalid.add(token);
      invalid.push(token);
    }
  }

  return { entries, valid, invalid, duplicates, privateKeys, publicKeys };
}

/**
 * Remove key material from a raw blob, returning the surviving tokens (one per
 * line) plus how many keys were stripped. Callers use this to purge keys from
 * the input field the moment they are pasted. The reflow to one-per-line is only
 * meaningful when something was removed; callers should keep the original text
 * verbatim when no keys are found so normal typing is not disturbed.
 */
export function stripKeyMaterial(input: string): {
  text: string;
  privateKeys: number;
  publicKeys: number;
} {
  const survivors: string[] = [];
  let privateKeys = 0;
  let publicKeys = 0;

  for (const raw of input.split(/[\s,;]+/)) {
    const token = raw.trim();
    if (!token) continue;
    const keyKind = classifyKeyMaterial(token);
    if (keyKind === "private") {
      privateKeys += 1;
      continue;
    }
    if (keyKind === "public") {
      publicKeys += 1;
      continue;
    }
    survivors.push(token);
  }

  return { text: survivors.join("\n"), privateKeys, publicKeys };
}
