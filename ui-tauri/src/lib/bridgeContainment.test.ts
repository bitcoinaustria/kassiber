import { describe, expect, it } from "vitest";

import {
  isAllowedBridgeOrigin,
  isLoopbackHost,
  redactBridgeText,
} from "../../vite.config";

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

  it("redacts bridge stderr text before it can become a dev error payload", () => {
    const redacted = redactBridgeText(
      "api_key=sk-test-secret Bearer bridge-token token=btcpay-secret passphrase_secret=correct raw xpub661MyMwAqRbcF12345678901234567890 wpkh(xpub661MyMwAqRbcF09876543210987654321)",
    );
    expect(redacted).not.toContain("sk-test-secret");
    expect(redacted).not.toContain("bridge-token");
    expect(redacted).not.toContain("btcpay-secret");
    expect(redacted).not.toContain("correct");
    expect(redacted).not.toContain("xpub661MyMwAqRbcF12345678901234567890");
    expect(redacted).not.toContain("xpub661MyMwAqRbcF09876543210987654321");
    expect(redacted).toContain("[redacted]");
  });
});
