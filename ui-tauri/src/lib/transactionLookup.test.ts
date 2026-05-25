import { describe, expect, it } from "vitest";

import { isTransactionLookupQuery } from "./transactionLookup";

describe("isTransactionLookupQuery", () => {
  it("accepts Kassiber UUID transaction ids", () => {
    expect(
      isTransactionLookupQuery(" 550e8400-e29b-41d4-a716-446655440000 "),
    ).toBe(true);
  });

  it("accepts external transaction id shapes", () => {
    expect(isTransactionLookupQuery("a".repeat(64))).toBe(true);
    expect(isTransactionLookupQuery("tx_mock-123")).toBe(true);
  });

  it("ignores ordinary route search text", () => {
    expect(isTransactionLookupQuery("transactions")).toBe(false);
    expect(isTransactionLookupQuery("reports")).toBe(false);
  });
});
