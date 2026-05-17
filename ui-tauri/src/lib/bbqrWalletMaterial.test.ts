import { deflateRaw } from "pako";
import { describe, expect, it } from "vitest";

import {
  emptyBbqrCollectorState,
  isBbqrFrame,
  processWalletMaterialQrScan,
} from "./bbqrWalletMaterial";

const BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function toHex(value: string) {
  return Array.from(new TextEncoder().encode(value))
    .map((byte) => byte.toString(16).toUpperCase().padStart(2, "0"))
    .join("");
}

function toBase32(bytes: Uint8Array) {
  let output = "";
  let buffer = 0;
  let bits = 0;
  for (const byte of bytes) {
    buffer = (buffer << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      output += BASE32_ALPHABET[(buffer >> bits) & 31];
    }
  }
  if (bits > 0) {
    output += BASE32_ALPHABET[(buffer << (5 - bits)) & 31];
  }
  return output;
}

describe("wallet material QR scan processing", () => {
  it("accepts normal QR text as wallet material in auto mode", () => {
    const result = processWalletMaterialQrScan(
      "wpkh([abcd1234/84h/0h/0h]xpub.../0/*)",
      "auto",
      emptyBbqrCollectorState(),
    );

    expect(result).toEqual({
      status: "single",
      material: "wpkh([abcd1234/84h/0h/0h]xpub.../0/*)",
    });
  });

  it("collects BBQR frames out of order and returns decoded text", () => {
    const material = `wpkh([abcd1234/84h/0h/0h]${"x".repeat(200)}/0/*)`;
    const encoded = toHex(material);
    const splitPoint = Math.ceil(encoded.length / 4) * 2;
    const split = [
      `B$HU0200${encoded.slice(0, splitPoint)}`,
      `B$HU0201${encoded.slice(splitPoint)}`,
    ];

    const first = processWalletMaterialQrScan(
      split[1] ?? "",
      "bbqr",
      emptyBbqrCollectorState(),
    );
    expect(first.status).toBe("bbqr_progress");
    if (first.status !== "bbqr_progress") throw new Error("expected progress");
    expect(first.progress).toMatchObject({ received: 1, total: 2 });

    const second = processWalletMaterialQrScan(split[0] ?? "", "bbqr", {
      ...first.state,
    });
    expect(second.status).toBe("bbqr_complete");
    if (second.status !== "bbqr_complete") throw new Error("expected complete");
    expect(second.material).toBe(material);
    expect(second.progress).toMatchObject({ received: 2, total: 2 });
  });

  it("ignores normal QR text in BBQR-only mode", () => {
    const result = processWalletMaterialQrScan(
      "not-a-bbqr-frame",
      "bbqr",
      emptyBbqrCollectorState(),
    );

    expect(result).toEqual({
      status: "ignored",
      message: "Waiting for a BBQR frame.",
    });
  });

  it("does not accept PSBT BBQR payloads as wallet material", () => {
    const frame = `B$HP0100${toHex("not wallet material")}`;

    const result = processWalletMaterialQrScan(
      frame,
      "auto",
      emptyBbqrCollectorState(),
    );

    expect(result).toEqual({
      status: "error",
      message: "BBQR type P is not a wallet export or descriptor.",
    });
  });

  it("recognizes BBQR-looking frames", () => {
    expect(isBbqrFrame("B$HU0100AAAA")).toBe(true);
    expect(isBbqrFrame("wpkh(xpub...)")).toBe(false);
  });

  it("decodes compressed Base32 BBQR text without the bbqr package", () => {
    const material = `wpkh([abcd1234/84h/0h/0h]${"x".repeat(100)}/0/*)`;
    const compressed = deflateRaw(new TextEncoder().encode(material), {
      windowBits: 10,
    });
    const result = processWalletMaterialQrScan(
      `B$ZU0100${toBase32(compressed)}`,
      "bbqr",
      emptyBbqrCollectorState(),
    );

    expect(result.status).toBe("bbqr_complete");
    if (result.status !== "bbqr_complete") {
      throw new Error("expected complete");
    }
    expect(result.material).toBe(material);
  });
});
