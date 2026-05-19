import { describe, expect, it } from "vitest";

import { formatShortDate } from "./date";

describe("formatShortDate", () => {
  it("renders an ISO timestamp as compact 'YYYY-MM-DD HH:mm'", () => {
    expect(formatShortDate("2026-05-18T07:42:18Z")).toBe("2026-05-18 07:42");
  });

  it("returns the em-dash placeholder for empty / nullish input", () => {
    expect(formatShortDate(null)).toBe("—");
    expect(formatShortDate(undefined)).toBe("—");
    expect(formatShortDate("")).toBe("—");
  });

  it("passes through values that are already shorter than the cap", () => {
    expect(formatShortDate("2026-05-18")).toBe("2026-05-18");
  });
});
