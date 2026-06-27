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

const BASE58_ALPHABET =
  "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE58_INDEX = new Map(
  [...BASE58_ALPHABET].map((char, index) => [char, BigInt(index)]),
);
const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_INDEX = new Map(
  [...BECH32_CHARSET].map((char, index) => [char, index]),
);
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b,
  0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
  0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7,
  0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
  0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152,
  0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
  0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
  0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
  0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
  0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

// Key material — never valid input for a watch-only address list. These patterns
// are intentionally self-contained and anchored/length-bounded (stricter than the
// startsWith-only prefix helpers in walletMaterialFormat.ts / samouraiSourceSet.ts)
// so this security guard stays auditable in one place; do not replace them with
// those looser, conversion-oriented checks.
const WIF = /^[59KLc][1-9A-HJ-NP-Za-km-z]{50,51}$/; // mainnet/testnet WIF
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

function rotateRight(value: number, bits: number): number {
  return (value >>> bits) | (value << (32 - bits));
}

function sha256(bytes: number[]): number[] {
  const message = bytes.slice();
  const bitLength = message.length * 8;
  message.push(0x80);
  while (message.length % 64 !== 56) {
    message.push(0);
  }
  const high = Math.floor(bitLength / 0x100000000);
  const low = bitLength >>> 0;
  for (const value of [high, low]) {
    message.push(
      (value >>> 24) & 0xff,
      (value >>> 16) & 0xff,
      (value >>> 8) & 0xff,
      value & 0xff,
    );
  }

  let h0 = 0x6a09e667;
  let h1 = 0xbb67ae85;
  let h2 = 0x3c6ef372;
  let h3 = 0xa54ff53a;
  let h4 = 0x510e527f;
  let h5 = 0x9b05688c;
  let h6 = 0x1f83d9ab;
  let h7 = 0x5be0cd19;
  const words = new Uint32Array(64);

  for (let offset = 0; offset < message.length; offset += 64) {
    for (let index = 0; index < 16; index += 1) {
      const base = offset + index * 4;
      words[index] =
        ((message[base] ?? 0) << 24) |
        ((message[base + 1] ?? 0) << 16) |
        ((message[base + 2] ?? 0) << 8) |
        (message[base + 3] ?? 0);
    }
    for (let index = 16; index < 64; index += 1) {
      const s0 =
        rotateRight(words[index - 15] ?? 0, 7) ^
        rotateRight(words[index - 15] ?? 0, 18) ^
        ((words[index - 15] ?? 0) >>> 3);
      const s1 =
        rotateRight(words[index - 2] ?? 0, 17) ^
        rotateRight(words[index - 2] ?? 0, 19) ^
        ((words[index - 2] ?? 0) >>> 10);
      words[index] =
        ((words[index - 16] ?? 0) +
          s0 +
          (words[index - 7] ?? 0) +
          s1) >>>
        0;
    }

    let a = h0;
    let b = h1;
    let c = h2;
    let d = h3;
    let e = h4;
    let f = h5;
    let g = h6;
    let h = h7;
    for (let index = 0; index < 64; index += 1) {
      const s1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 =
        (h + s1 + ch + (SHA256_K[index] ?? 0) + (words[index] ?? 0)) >>> 0;
      const s0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (s0 + maj) >>> 0;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }

    h0 = (h0 + a) >>> 0;
    h1 = (h1 + b) >>> 0;
    h2 = (h2 + c) >>> 0;
    h3 = (h3 + d) >>> 0;
    h4 = (h4 + e) >>> 0;
    h5 = (h5 + f) >>> 0;
    h6 = (h6 + g) >>> 0;
    h7 = (h7 + h) >>> 0;
  }

  return [h0, h1, h2, h3, h4, h5, h6, h7].flatMap((value) => [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ]);
}

function base58CheckPayload(value: string): number[] | null {
  let number = 0n;
  for (const char of value) {
    const digit = BASE58_INDEX.get(char);
    if (digit === undefined) return null;
    number = number * 58n + digit;
  }

  const bytes: number[] = [];
  while (number > 0n) {
    bytes.unshift(Number(number & 0xffn));
    number >>= 8n;
  }
  const leadingZeros = value.length - value.replace(/^1+/, "").length;
  const payload = [...Array<number>(leadingZeros).fill(0), ...bytes];
  if (payload.length < 5) return null;

  const body = payload.slice(0, -4);
  const checksum = payload.slice(-4);
  const expected = sha256(sha256(body)).slice(0, 4);
  return checksum.every((byte, index) => byte === expected[index])
    ? body
    : null;
}

function bech32Polymod(values: number[]): number {
  const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let checksum = 1;
  for (const value of values) {
    const top = checksum >>> 25;
    checksum = ((checksum & 0x1ffffff) << 5) ^ value;
    for (let index = 0; index < generators.length; index += 1) {
      if ((top >>> index) & 1) {
        checksum ^= generators[index] ?? 0;
      }
    }
  }
  return checksum;
}

function bech32HrpExpand(hrp: string): number[] {
  return [
    ...[...hrp].map((char) => char.charCodeAt(0) >>> 5),
    0,
    ...[...hrp].map((char) => char.charCodeAt(0) & 31),
  ];
}

function convertBits(
  data: number[],
  fromBits: number,
  toBits: number,
  pad: boolean,
): number[] | null {
  let accumulator = 0;
  let bits = 0;
  const output: number[] = [];
  const maxValue = (1 << toBits) - 1;
  const maxAccumulator = (1 << (fromBits + toBits - 1)) - 1;

  for (const value of data) {
    if (value < 0 || value >>> fromBits) return null;
    accumulator = ((accumulator << fromBits) | value) & maxAccumulator;
    bits += fromBits;
    while (bits >= toBits) {
      bits -= toBits;
      output.push((accumulator >>> bits) & maxValue);
    }
  }
  if (pad) {
    if (bits > 0) output.push((accumulator << (toBits - bits)) & maxValue);
  } else if (bits >= fromBits || ((accumulator << (toBits - bits)) & maxValue)) {
    return null;
  }
  return output;
}

function bech32Decode(value: string):
  | { hrp: string; data: number[]; spec: "bech32" | "bech32m" }
  | null {
  if (value !== value.toLowerCase() && value !== value.toUpperCase()) return null;
  const normalized = value.toLowerCase();
  const separator = normalized.lastIndexOf("1");
  if (separator < 1 || separator + 7 > normalized.length) return null;
  const hrp = normalized.slice(0, separator);
  const data: number[] = [];
  for (const char of normalized.slice(separator + 1)) {
    const value = BECH32_INDEX.get(char);
    if (value === undefined) return null;
    data.push(value);
  }
  const polymod = bech32Polymod([...bech32HrpExpand(hrp), ...data]);
  const spec = polymod === 1 ? "bech32" : polymod === 0x2bc830a3 ? "bech32m" : null;
  if (spec === null) return null;
  return { hrp, data: data.slice(0, -6), spec };
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
 * Conservative mainnet-address check. It accepts the three mainnet address
 * shapes only after checksum/witness validation and rejects everything else
 * (including testnet `tb1…`/`m…`/`n…`/`2…`).
 */
export function looksLikeMainnetAddress(value: string): boolean {
  const token = value.trim();
  if (!token) return false;

  const base58Payload = base58CheckPayload(token);
  if (base58Payload) {
    return (
      base58Payload.length === 21 &&
      (base58Payload[0] === 0x00 || base58Payload[0] === 0x05)
    );
  }

  const bech32 = bech32Decode(token);
  if (!bech32 || bech32.hrp !== "bc" || bech32.data.length === 0) return false;
  const version = bech32.data[0];
  if (version === undefined || version > 16) return false;
  const program = convertBits(bech32.data.slice(1), 5, 8, false);
  if (!program || program.length < 2 || program.length > 40) return false;
  if (version === 0) {
    return (
      bech32.spec === "bech32" &&
      (program.length === 20 || program.length === 32)
    );
  }
  return bech32.spec === "bech32m";
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
