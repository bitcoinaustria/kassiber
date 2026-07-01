import { describe, expect, it } from "vitest";

import {
  detectWalletMaterial,
  scriptTypesFromDetectionPayload,
} from "./walletMaterialFormat";

describe("detectWalletMaterial", () => {
  it("recognizes empty input", () => {
    expect(detectWalletMaterial("   ").kind).toBe("empty");
  });

  it("recognizes JSON descriptor exports by leading brace or bracket", () => {
    expect(detectWalletMaterial('{"descriptors":[]}').kind).toBe(
      "descriptor-json",
    );
    expect(detectWalletMaterial("[]").kind).toBe("descriptor-json");
  });

  it("recognizes output descriptors by prefix", () => {
    expect(detectWalletMaterial("wpkh(xpub.../0/*)").kind).toBe("descriptor");
    expect(detectWalletMaterial("sh(wpkh(xpub.../0/*))").kind).toBe(
      "descriptor",
    );
    expect(detectWalletMaterial("ct(slip77(...),elwpkh(...))").kind).toBe(
      "descriptor",
    );
  });

  it("recognizes BSMS descriptor records", () => {
    expect(
      detectWalletMaterial(
        "BSMS 1.0\nwsh(sortedmulti(2,xpubA/**,xpubB/**))\n/0/*,/1/*\nbc1q...",
      ).kind,
    ).toBe("bsms");
  });

  it("recognizes SLIP132 prefixes", () => {
    expect(detectWalletMaterial("zpub6r...").kind).toBe("slip132");
    expect(detectWalletMaterial("ypub6W...").kind).toBe("slip132");
    expect(detectWalletMaterial("vpub5T...").kind).toBe("slip132");
    expect(detectWalletMaterial("upub5E...").kind).toBe("slip132");
  });

  it("flags bare xpub/tpub as ambiguous", () => {
    expect(detectWalletMaterial("xpub6C...").kind).toBe("bare-xpub");
    expect(detectWalletMaterial("tpub5...").kind).toBe("bare-xpub");
    expect(detectWalletMaterial("xpub6C...").hint).toBeDefined();
  });

  it("returns unknown for unrelated text", () => {
    expect(detectWalletMaterial("hello world").kind).toBe("unknown");
  });
});

describe("scriptTypesFromDetectionPayload", () => {
  it("blocks failed auto-detection instead of silently defaulting", () => {
    expect(
      scriptTypesFromDetectionPayload({
        probed: false,
        active: ["p2wpkh"],
        reason: "backend unavailable",
      }),
    ).toEqual({ ok: false, reason: "backend unavailable" });
  });

  it("falls back to native segwit only after a real empty probe", () => {
    expect(
      scriptTypesFromDetectionPayload({
        probed: true,
        active: [],
      }),
    ).toEqual({ ok: true, scriptTypes: ["p2wpkh"] });
  });

  it("keeps only supported script type names from daemon payloads", () => {
    expect(
      scriptTypesFromDetectionPayload({
        probed: true,
        active: ["p2tr", "p2wsh", 1, "p2wpkh"],
      }),
    ).toEqual({ ok: true, scriptTypes: ["p2tr", "p2wpkh"] });
  });
});
