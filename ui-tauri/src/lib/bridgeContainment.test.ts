import { describe, expect, it } from "vitest";

import { isAllowedBridgeOrigin, isLoopbackHost } from "../../vite.config";

describe("daemon bridge containment", () => {
  it("recognizes loopback hosts accepted by the dev bridge", () => {
    expect(isLoopbackHost("127.0.0.1:5173")).toBe(true);
    expect(isLoopbackHost("localhost:5173")).toBe(true);
    expect(isLoopbackHost("[::1]:5173")).toBe(true);
    expect(isLoopbackHost("example.test:5173")).toBe(false);
  });

  it("requires same-origin loopback browser requests", () => {
    expect(
      isAllowedBridgeOrigin("http://127.0.0.1:5173", "127.0.0.1:5173"),
    ).toBe(true);
    expect(
      isAllowedBridgeOrigin("http://localhost:5173", "localhost:5173"),
    ).toBe(true);
    expect(isAllowedBridgeOrigin(undefined, "127.0.0.1:5173")).toBe(false);
    expect(
      isAllowedBridgeOrigin("https://example.test", "127.0.0.1:5173"),
    ).toBe(false);
    expect(
      isAllowedBridgeOrigin("http://127.0.0.1:5174", "127.0.0.1:5173"),
    ).toBe(false);
  });
});
