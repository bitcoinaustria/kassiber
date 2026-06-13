import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  clearAppLogRecords,
  getAppLogRecords,
  subscribeAppLogRecords,
} from "./appLogs";
import {
  installGlobalErrorCapture,
  uninstallGlobalErrorCapture,
} from "./globalErrorCapture";

const originalConsoleError = console.error;
const originalConsoleWarn = console.warn;

describe("global error capture", () => {
  let passthroughError: ReturnType<typeof vi.fn>;
  let passthroughWarn: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    clearAppLogRecords();
    vi.stubGlobal("window", new EventTarget());
    passthroughError = vi.fn();
    passthroughWarn = vi.fn();
    console.error = passthroughError;
    console.warn = passthroughWarn;
  });

  afterEach(() => {
    uninstallGlobalErrorCapture();
    console.error = originalConsoleError;
    console.warn = originalConsoleWarn;
    vi.unstubAllGlobals();
  });

  it("calls through to the original console and captures records", () => {
    installGlobalErrorCapture();

    console.error("broadcast failed for txid", "deadbeef");
    console.warn("mempool fee estimate slow");

    expect(passthroughError).toHaveBeenCalledTimes(1);
    expect(passthroughError).toHaveBeenCalledWith(
      "broadcast failed for txid",
      "deadbeef",
    );
    expect(passthroughWarn).toHaveBeenCalledTimes(1);
    expect(passthroughWarn).toHaveBeenCalledWith("mempool fee estimate slow");

    const records = getAppLogRecords();
    expect(records).toHaveLength(2);
    expect(records[0].level).toBe("error");
    expect(records[0].module).toBe("console");
    expect(records[0].msg).toBe("broadcast failed for txid deadbeef");
    expect(records[1].level).toBe("warning");
    expect(records[1].module).toBe("console");
  });

  it("stores console secrets redacted while the passthrough stays raw", () => {
    installGlobalErrorCapture();
    const xprv =
      "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi";

    console.error(`wallet restore failed for ${xprv}`);

    expect(passthroughError).toHaveBeenCalledWith(
      `wallet restore failed for ${xprv}`,
    );
    const records = getAppLogRecords();
    expect(records).toHaveLength(1);
    expect(records[0].msg).toBe(
      "wallet restore failed for [redacted-private-key]",
    );
    expect(records[0].msg).not.toContain(xprv);
  });

  it("does not recurse when a log subscriber logs during notification", () => {
    installGlobalErrorCapture();
    let notifications = 0;
    const unsubscribe = subscribeAppLogRecords(() => {
      notifications += 1;
      console.error("subscriber noticed a failure");
    });

    console.error("first failure");
    unsubscribe();

    expect(notifications).toBe(1);
    const records = getAppLogRecords();
    expect(records).toHaveLength(1);
    expect(records[0].msg).toBe("first failure");
    expect(passthroughError).toHaveBeenCalledTimes(2);
  });

  it("throttles duplicates to five per window and emits one summary", () => {
    let at = 0;
    installGlobalErrorCapture({ now: () => at });

    for (let index = 0; index < 6; index += 1) {
      console.error("rate fetch failed for BTC-JPY");
      at += 100;
    }
    expect(getAppLogRecords()).toHaveLength(5);
    expect(passthroughError).toHaveBeenCalledTimes(6);

    at = 20_000;
    console.error("rate fetch failed for BTC-JPY");

    const records = getAppLogRecords();
    expect(records).toHaveLength(7);
    expect(records[5].level).toBe("error");
    expect(records[5].msg).toBe("suppressed 1 duplicate records");
    expect(records[6].msg).toBe("rate fetch failed for BTC-JPY");
  });

  it("flushes the duplicate summary when a different message arrives", () => {
    let at = 0;
    installGlobalErrorCapture({ now: () => at });

    for (let index = 0; index < 8; index += 1) {
      console.error("connection refused by electrum backend");
    }
    console.error("descriptor import finished with warnings");

    const records = getAppLogRecords();
    expect(records).toHaveLength(7);
    expect(records[5].msg).toBe("suppressed 3 duplicate records");
    expect(records[6].msg).toBe("descriptor import finished with warnings");
  });

  it("captures window error events with basename, lineno, and stack head", () => {
    installGlobalErrorCapture();
    const error = new Error("descriptor parse failed");
    error.stack = [
      "Error: descriptor parse failed",
      ...Array.from(
        { length: 20 },
        (_, index) => `    at frame${index} (Wallets.tsx:${index})`,
      ),
    ].join("\n");
    const event = new Event("error") as Event & {
      message?: string;
      filename?: string;
      lineno?: number;
      error?: unknown;
    };
    event.message = "Uncaught Error: descriptor parse failed";
    event.filename = "http://localhost:5173/src/routes/Wallets.tsx?t=1718000000";
    event.lineno = 88;
    event.error = error;

    window.dispatchEvent(event);

    const records = getAppLogRecords();
    expect(records).toHaveLength(1);
    expect(records[0].level).toBe("error");
    expect(records[0].module).toBe("window");
    expect(records[0].file).toBe("Wallets.tsx");
    expect(records[0].line).toBe(88);
    expect(records[0].msg).toBe("Uncaught Error: descriptor parse failed");
    expect(String(records[0].fields.stack.value).split("\n")).toHaveLength(10);
  });

  it("captures unhandled promise rejections with the reason message", () => {
    installGlobalErrorCapture();
    const reason = new Error(
      "broadcast rejected: bad-txns-inputs-missingorspent",
    );
    const event = new Event("unhandledrejection") as Event & {
      reason?: unknown;
    };
    event.reason = reason;

    window.dispatchEvent(event);

    const records = getAppLogRecords();
    expect(records).toHaveLength(1);
    expect(records[0].level).toBe("error");
    expect(records[0].module).toBe("window");
    expect(records[0].msg).toBe(
      "Unhandled promise rejection: broadcast rejected: bad-txns-inputs-missingorspent",
    );
    expect(String(records[0].fields.stack.value)).toContain(
      "broadcast rejected",
    );
  });

  it("treats a second install as a no-op", () => {
    installGlobalErrorCapture();
    installGlobalErrorCapture();

    console.error("only once");

    expect(passthroughError).toHaveBeenCalledTimes(1);
    expect(getAppLogRecords()).toHaveLength(1);
  });
});
