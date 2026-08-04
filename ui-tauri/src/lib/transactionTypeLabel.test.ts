import { describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { transactionTypeLabel } from "./transactionTypeLabel";

const tx = () => i18n.getFixedT(null, "transactions");

describe("transactionTypeLabel", () => {
  it("translates daemon-derived type labels", async () => {
    await i18n.changeLanguage("de");
    expect(transactionTypeLabel(tx(), "Buy")).toBe("Kauf");
    expect(transactionTypeLabel(tx(), "Acquired")).toBe("Anschaffung");
    expect(transactionTypeLabel(tx(), "Hard fork")).toBe("Hard Fork");
    await i18n.changeLanguage("en");
    expect(transactionTypeLabel(tx(), "Buy")).toBe("Buy");
  });

  it("renders unknown values verbatim — they are user-authored tags", () => {
    expect(transactionTypeLabel(tx(), "Revenue")).toBe("Revenue");
    expect(transactionTypeLabel(tx(), "")).toBe("");
    expect(transactionTypeLabel(tx(), null)).toBe("");
  });
});
