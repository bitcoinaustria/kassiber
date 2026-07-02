import { describe, expect, it } from "vitest";

import {
  dataModeForActiveBackend,
  dataModeFromSourceSwitch,
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

  it("maps the sidebar switch between the daemon-backed side and preview", () => {
    expect(dataModeFromSourceSwitch(true, true)).toBe("regtest");
    expect(dataModeFromSourceSwitch(true, false)).toBe("real");
    expect(dataModeFromSourceSwitch(false, true)).toBe("mock");
    expect(dataModeFromSourceSwitch(false, false)).toBe("mock");
  });
});
