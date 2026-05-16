import { splitQRs } from "bbqr";
import { describe, expect, it } from "vitest";

import {
  emptyBbqrCollectorState,
  isBbqrFrame,
  processWalletMaterialQrScan,
} from "./bbqrWalletMaterial";

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
    const split = splitQRs(new TextEncoder().encode(material), "U", {
      encoding: "H",
      minSplit: 2,
      maxSplit: 2,
    });

    const first = processWalletMaterialQrScan(
      split.parts[1] ?? "",
      "bbqr",
      emptyBbqrCollectorState(),
    );
    expect(first.status).toBe("bbqr_progress");
    if (first.status !== "bbqr_progress") throw new Error("expected progress");
    expect(first.progress).toMatchObject({ received: 1, total: 2 });

    const second = processWalletMaterialQrScan(split.parts[0] ?? "", "bbqr", {
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
    const split = splitQRs(new TextEncoder().encode("not wallet material"), "P", {
      encoding: "H",
      minSplit: 1,
      maxSplit: 1,
    });

    const result = processWalletMaterialQrScan(
      split.parts[0] ?? "",
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
});
