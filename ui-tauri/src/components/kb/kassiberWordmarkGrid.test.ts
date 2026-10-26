import { describe, expect, it } from "vitest";

import {
  DECRYPT_MS,
  WORDMARK_COLUMNS,
  WORDMARK_GRID,
  WORDMARK_K_COLUMNS,
  WORDMARK_PATH,
  decryptFrame,
} from "./kassiberWordmarkGrid";

describe("kassiber wordmark grid", () => {
  it("keeps every row the same width, drawn in cells and gaps only", () => {
    for (const row of WORDMARK_GRID) {
      expect(row).toHaveLength(WORDMARK_COLUMNS);
      expect(row).toMatch(/^[#.]+$/);
    }
  });

  it("leaves the column after the K empty, so folding to the K clips no letter", () => {
    for (const row of WORDMARK_GRID) {
      expect(row[WORDMARK_K_COLUMNS]).toBe(".");
    }
  });

  it("starts the decrypt fully scrambled and ends on the plain wordmark", () => {
    const first = decryptFrame(0);
    expect(first.solid).toBe("");
    expect(first.noise).not.toBe("");

    const last = decryptFrame(DECRYPT_MS);
    expect(last.solid).toBe(WORDMARK_PATH);
    expect(last.noise).toBe("");
  });
});
