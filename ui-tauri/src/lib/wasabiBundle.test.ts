import { describe, expect, it } from "vitest";

import { buildWasabiBundle } from "./wasabiBundle";

describe("buildWasabiBundle", () => {
  it("builds a Wasabi bundle from pasted RPC responses", () => {
    const { bundle, errors } = buildWasabiBundle({
      history: '{"result":[{"tx":"aa"}]}',
      coins: '{"result":[{"txid":"aa","index":0}]}',
      walletInfo: '{"result":{"walletName":"Wasabi"}}',
      additional: '{"listpaymentsincoinjoin":{"result":[]}}',
    });

    expect(errors).toEqual({});
    expect(bundle).toEqual({
      gethistory: { result: [{ tx: "aa" }] },
      listcoins: { result: [{ txid: "aa", index: 0 }] },
      getwalletinfo: { result: { walletName: "Wasabi" } },
      listpaymentsincoinjoin: { result: [] },
    });
  });

  it("requires gethistory JSON", () => {
    const { errors } = buildWasabiBundle({
      history: "",
      coins: "",
      walletInfo: "",
      additional: "",
    });

    expect(errors.history).toContain("gethistory");
  });

  it("rejects invalid JSON and non-object additional sections", () => {
    const invalidJson = buildWasabiBundle({
      history: "{",
      coins: "",
      walletInfo: "",
      additional: "",
    });
    expect(invalidJson.errors.history).toContain("valid JSON");

    const nonObjectAdditional = buildWasabiBundle({
      history: "[]",
      coins: "",
      walletInfo: "",
      additional: "[]",
    });
    expect(nonObjectAdditional.errors.additional).toContain("JSON object");
  });
});
