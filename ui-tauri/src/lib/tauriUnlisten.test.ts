import { describe, expect, it, vi } from "vitest";

import { safeTauriUnlisten } from "./tauriUnlisten";

describe("safeTauriUnlisten", () => {
  it("does not throw when Tauri rejects stale listener cleanup synchronously", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    expect(() =>
      safeTauriUnlisten(() => {
        throw new Error("listener missing");
      }),
    ).not.toThrow();

    expect(warn).toHaveBeenCalledWith(
      "Could not unregister Tauri event listener",
      expect.any(Error),
    );
    warn.mockRestore();
  });

  it("does not leave rejected async cleanup promises unhandled", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});

    safeTauriUnlisten(() => Promise.reject(new Error("listener missing")));
    await Promise.resolve();

    expect(warn).toHaveBeenCalledWith(
      "Could not unregister Tauri event listener",
      expect.any(Error),
    );
    warn.mockRestore();
  });
});
