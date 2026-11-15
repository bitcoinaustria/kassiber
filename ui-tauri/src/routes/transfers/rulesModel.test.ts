import { describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { ruleSentence, type Translate } from "./rulesModel";

describe("ruleSentence", () => {
  it("says what a rule matches and what the link records, in words", async () => {
    await i18n.changeLanguage("en");
    const t = i18n.getFixedT("en", "review") as unknown as Translate;
    expect(
      ruleSentence({ predicate: { out_asset: "LBTC", in_asset: "BTC", min_confidence: "exact" }, kind: "peg-out", policy: "carrying-value" }, t),
    ).toBe("Liquid → Bitcoin, exact matches only: record as Peg-out from Liquid, tax-neutral");
    expect(
      ruleSentence({ predicate: { out_wallet_kind: "phoenix", max_fee_pct: 1, min_confidence: "strong" }, kind: "submarine-swap", policy: "taxable" }, t),
    ).toBe("Phoenix → any wallet, likely or exact matches, fee up to 1 %: record as Swap to Lightning, taxable");
  });
});
