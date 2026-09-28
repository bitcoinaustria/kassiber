import { describe, expect, it } from "vitest";

import { formatShortcut, isMacPlatform } from "./shortcutLabel";

describe("formatShortcut", () => {
  it("writes macOS shortcuts as glyphs", () => {
    expect(formatShortcut(["mod", "shift", "a"], true)).toBe("⌘⇧A");
    expect(formatShortcut(["mod", "k"], true)).toBe("⌘K");
    expect(formatShortcut(["mod", ","], true)).toBe("⌘,");
  });

  it("spells shortcuts out on Windows and Linux", () => {
    expect(formatShortcut(["mod", "shift", "a"], false)).toBe("Ctrl+Shift+A");
    expect(formatShortcut(["mod", "1"], false)).toBe("Ctrl+1");
    expect(formatShortcut(["alt", "left"], false)).toBe("Alt+Left");
  });
});

describe("isMacPlatform", () => {
  it("recognises macOS user agents only", () => {
    expect(isMacPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe(true);
    expect(isMacPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(false);
    expect(isMacPlatform("Mozilla/5.0 (X11; Linux x86_64)")).toBe(false);
  });
});
