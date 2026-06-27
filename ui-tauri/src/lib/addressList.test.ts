import { describe, expect, it } from "vitest";

import {
  classifyKeyMaterial,
  looksLikeMainnetAddress,
  parseAddressList,
  stripKeyMaterial,
} from "./addressList";

// Real mainnet addresses (genesis P2PKH, a known P2SH, BIP173 bech32 example).
const P2PKH = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
const P2SH = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy";
const BECH32 = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
const BAD_BASE58_CHECKSUM = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb";
const BAD_BECH32_CHECKSUM = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080";

// Key material that must never be accepted (BIP32 test vectors + a test WIF +
// the secp256k1 generator point as a compressed pubkey).
const WIF = "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ";
const TESTNET_WIF = "cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN87JcbXMTcA";
const XPRV =
  "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";
const XPUB =
  "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet";
const HEX_PUBKEY =
  "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

describe("looksLikeMainnetAddress", () => {
  it("accepts legacy, P2SH, and bech32 mainnet addresses", () => {
    expect(looksLikeMainnetAddress(P2PKH)).toBe(true);
    expect(looksLikeMainnetAddress(P2SH)).toBe(true);
    expect(looksLikeMainnetAddress(BECH32)).toBe(true);
    expect(looksLikeMainnetAddress(BECH32.toUpperCase())).toBe(true);
  });

  it("rejects testnet, bad checksums, junk, mixed-case bech32, and key material", () => {
    expect(looksLikeMainnetAddress("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx")).toBe(false);
    expect(looksLikeMainnetAddress(BAD_BASE58_CHECKSUM)).toBe(false);
    expect(looksLikeMainnetAddress(BAD_BECH32_CHECKSUM)).toBe(false);
    expect(looksLikeMainnetAddress("not-an-address")).toBe(false);
    expect(looksLikeMainnetAddress("")).toBe(false);
    expect(looksLikeMainnetAddress("Bc1QW508D6")).toBe(false);
    expect(looksLikeMainnetAddress(WIF)).toBe(false);
    expect(looksLikeMainnetAddress(XPUB)).toBe(false);
  });
});

describe("classifyKeyMaterial", () => {
  it("flags private keys (WIF, xprv)", () => {
    expect(classifyKeyMaterial(WIF)).toBe("private");
    expect(classifyKeyMaterial(TESTNET_WIF)).toBe("private");
    expect(classifyKeyMaterial(XPRV)).toBe("private");
  });

  it("flags extended and raw public keys", () => {
    expect(classifyKeyMaterial(XPUB)).toBe("public");
    expect(classifyKeyMaterial(HEX_PUBKEY)).toBe("public");
  });

  it("does not flag real addresses as keys", () => {
    expect(classifyKeyMaterial(P2PKH)).toBeNull();
    expect(classifyKeyMaterial(P2SH)).toBeNull();
    expect(classifyKeyMaterial(BECH32)).toBeNull();
  });
});

describe("parseAddressList", () => {
  it("splits on newlines, commas, semicolons, and spaces", () => {
    const result = parseAddressList(`${P2PKH}\n${P2SH}, ${BECH32};`);
    expect(result.valid).toEqual([P2PKH, P2SH, BECH32]);
    expect(result.invalid).toEqual([]);
    expect(result.duplicates).toBe(0);
  });

  it("de-duplicates valid addresses and counts the collapses", () => {
    const result = parseAddressList(`${P2PKH}\n${P2PKH}\n${P2SH}\n${P2PKH}`);
    expect(result.valid).toEqual([P2PKH, P2SH]);
    expect(result.duplicates).toBe(2);
  });

  it("separates junk into invalid without dropping good addresses", () => {
    const result = parseAddressList(`address,balance\n${P2PKH},0.5\ngarbage`);
    expect(result.valid).toEqual([P2PKH]);
    expect(result.invalid).toContain("address");
    expect(result.invalid).toContain("balance");
    expect(result.invalid).toContain("garbage");
  });

  it("never lets key material into valid/invalid, and counts it", () => {
    const result = parseAddressList(
      `${P2PKH}\n${WIF}\n${XPRV}\n${XPUB}\n${HEX_PUBKEY}\n${P2SH}`,
    );
    expect(result.valid).toEqual([P2PKH, P2SH]);
    expect(result.privateKeys).toBe(2); // WIF + xprv
    expect(result.publicKeys).toBe(2); // xpub + hex pubkey
    // keys must not leak into any displayed set
    const surfaced = [...result.valid, ...result.invalid, ...result.entries];
    expect(surfaced).not.toContain(WIF);
    expect(surfaced).not.toContain(XPRV);
    expect(surfaced).not.toContain(XPUB);
    expect(surfaced).not.toContain(HEX_PUBKEY);
  });

  it("does not surface testnet WIF private keys as invalid samples", () => {
    const result = parseAddressList(`${P2PKH}\n${TESTNET_WIF}\nlabel`);
    expect(result.valid).toEqual([P2PKH]);
    expect(result.privateKeys).toBe(1);
    expect(result.invalid).toEqual(["label"]);
    expect([...result.invalid, ...result.entries]).not.toContain(TESTNET_WIF);
  });

  it("returns empty sets for blank input", () => {
    const result = parseAddressList("   \n\n  ");
    expect(result.entries).toEqual([]);
    expect(result.valid).toEqual([]);
    expect(result.invalid).toEqual([]);
  });
});

describe("stripKeyMaterial", () => {
  it("removes keys while keeping addresses, and reports counts", () => {
    const { text, privateKeys, publicKeys } = stripKeyMaterial(
      `${P2PKH}, ${WIF}\n${P2SH}\n${XPUB}`,
    );
    expect(text).toBe(`${P2PKH}\n${P2SH}`);
    expect(privateKeys).toBe(1);
    expect(publicKeys).toBe(1);
  });

  it("leaves a key-free blob's tokens intact (one per line)", () => {
    const { text, privateKeys, publicKeys } = stripKeyMaterial(
      `${P2PKH} ${P2SH}`,
    );
    expect(text).toBe(`${P2PKH}\n${P2SH}`);
    expect(privateKeys).toBe(0);
    expect(publicKeys).toBe(0);
  });
});
