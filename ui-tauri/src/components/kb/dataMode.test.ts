import { describe, expect, it } from "vitest";

import { dataModeForActiveBackend } from "./dataMode";

describe("sidebar data mode model", () => {
  it("coerces regtest books onto the regtest daemon-backed mode", () => {
    expect(dataModeForActiveBackend("real", true)).toBe("regtest");
  });

  it("coerces stale regtest mode back to live data outside regtest books", () => {
    expect(dataModeForActiveBackend("regtest", false)).toBe("real");
  });
});
