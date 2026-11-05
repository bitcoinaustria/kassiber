import { describe, expect, it } from "vitest";

import { ENTERED_PRICE_REASON, exactDecimal, formatExactDecimal, priceEntryOperations } from "./priceEntry";

describe("reading a typed price", () => {
  it("keeps every digit and reads both ways of writing numbers", () => {
    expect(exactDecimal("2800", ",")).toBe("2800");
    expect(exactDecimal("2.800,50", ",")).toBe("2800.50");
    expect(exactDecimal("2,800.50", ".")).toBe("2800.50");
    expect(exactDecimal("2 800,5", ",")).toBe("2800.5");
    expect(exactDecimal("1'234'567.891234567", ".")).toBe("1234567.891234567");
    expect(exactDecimal("0,5", ".")).toBe("0.5");
    expect(exactDecimal("007", ".")).toBe("7");
  });

  it("reads a lone separator before three digits the way the language writes it", () => {
    expect(exactDecimal("2.800", ",")).toBe("2800");
    expect(exactDecimal("2.800", ".")).toBe("2.800");
    expect(exactDecimal("2,800", ".")).toBe("2800");
    expect(exactDecimal("2,800", ",")).toBe("2.800");
    expect(exactDecimal("12.5", ",")).toBe("12.5");
  });

  it("never reads a leading zero as a thousands group", () => {
    expect(exactDecimal("0.500", ",")).toBe("0.500");
    expect(exactDecimal("0,500", ".")).toBe("0.500");
    expect(exactDecimal("000.500", ",")).toBe("0.500");
    expect(exactDecimal("0.500.000", ",")).toBeNull();
  });

  it("refuses what is not one unsigned number", () => {
    for (const input of [
      "", "-5", "1e3", "5.", "1,2,3", "1.2.3,4.5", "12,34.567,8", "abc", "€ 5",
      // Grouping is by threes, with one kind of separator.
      "1 0", "10 20", "1 234.567,89", "1,234.567,89", "2 800 ,5",
    ]) {
      expect(exactDecimal(input, ".")).toBeNull();
    }
  });
});

describe("price overrides from the typed rows", () => {
  it("names each filled row in order with the unit the owner chose", () => {
    const check = priceEntryOperations(["a", "b", "c"], { a: "2.800,5", c: "" }, "value", ",");
    expect(check).toEqual({
      ok: true,
      operations: [
        { type: "price_override", transaction_id: "a", reason: ENTERED_PRICE_REASON, fiat_value: "2800.5" },
      ],
    });
    const rate = priceEntryOperations(["a"], { a: "31000" }, "rate", ".");
    expect(rate.ok && rate.operations[0]).toMatchObject({ fiat_rate: "31000" });
  });

  it("holds the set back for an unreadable row, a zero price or too many rows", () => {
    expect(priceEntryOperations(["a", "b"], { a: "5", b: "x" }, "value", ".")).toEqual({
      ok: false,
      invalid: ["b"],
      tooMany: false,
    });
    expect(priceEntryOperations(["a"], { a: "0" }, "rate", ".")).toMatchObject({ invalid: ["a"] });
    expect(priceEntryOperations(["a"], { a: "0,00" }, "value", ",")).toMatchObject({ invalid: ["a"] });
    const ids = Array.from({ length: 51 }, (_, index) => `t${index}`);
    const entries = Object.fromEntries(ids.map((id) => [id, "1"]));
    expect(priceEntryOperations(ids, entries, "value", ".")).toMatchObject({ ok: false, tooMany: true });
    expect(priceEntryOperations(["a"], {}, "value", ".").ok).toBe(false);
  });
});

describe("showing a typed price back", () => {
  it("keeps every digit in the owner's number format", () => {
    expect(formatExactDecimal("2800.50", "de-AT")).toMatch(/^2.800,50$/);
    expect(formatExactDecimal("1234567.891234567", "en-US")).toBe("1,234,567.891234567");
    expect(formatExactDecimal("12", "en-US")).toBe("12");
  });
});
