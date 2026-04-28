import { describe, expect, it } from "vitest";

import { parseTaxLongTermDays, taxLongTermDaysHint } from "./constants";

describe("onboarding tax long-term day parsing", () => {
  it("accepts positive whole days with surrounding whitespace", () => {
    expect(parseTaxLongTermDays("365")).toBe(365);
    expect(parseTaxLongTermDays("  730  ")).toBe(730);
    expect(taxLongTermDaysHint("365")).toBeNull();
  });

  it("rejects truncated or non-integer values", () => {
    for (const value of ["365.5", "1e2", "12abc", "0x10", ""]) {
      expect(parseTaxLongTermDays(value)).toBeNull();
      expect(taxLongTermDaysHint(value)).not.toBeNull();
    }
  });

  it("rejects zero, negative, and unsafe integer values", () => {
    for (const value of ["0", "-1", String(Number.MAX_SAFE_INTEGER + 1)]) {
      expect(parseTaxLongTermDays(value)).toBeNull();
      expect(taxLongTermDaysHint(value)).not.toBeNull();
    }
  });
});
