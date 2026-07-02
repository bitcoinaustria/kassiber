import { describe, expect, it } from "vitest";

import {
  dataModeForActiveBackend,
  dataModeLabelKey,
} from "./dataMode";

describe("sidebar data mode model", () => {
  it("coerces regtest books onto the regtest daemon-backed mode", () => {
    expect(dataModeForActiveBackend("real", true)).toBe("regtest");
    expect(dataModeLabelKey(dataModeForActiveBackend("real", true))).toBe(
      "regtest",
    );
  });

  it("coerces stale regtest mode back to live data outside regtest books", () => {
    expect(dataModeForActiveBackend("regtest", false)).toBe("real");
  });

  it("coerces persisted mock mode to the daemon-backed side", () => {
    expect(dataModeForActiveBackend("mock", true)).toBe("regtest");
    expect(dataModeForActiveBackend("mock", false)).toBe("real");
    expect(dataModeLabelKey(dataModeForActiveBackend("mock", true))).toBe(
      "regtest",
    );
    expect(dataModeLabelKey(dataModeForActiveBackend("mock", false))).toBe(
      "real",
    );
  });
});
