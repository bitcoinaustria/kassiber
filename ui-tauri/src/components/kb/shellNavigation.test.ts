import { describe, expect, it } from "vitest";

import {
  historyHotkeyAction,
  historyHotkeyDecision,
  resolveNavOpen,
} from "./shellNavigation";

const key = (
  overrides: Partial<Parameters<typeof historyHotkeyAction>[0]>,
): Parameters<typeof historyHotkeyAction>[0] => ({
  altKey: false,
  code: "",
  ctrlKey: false,
  key: "",
  metaKey: false,
  shiftKey: false,
  ...overrides,
});

describe("resolveNavOpen", () => {
  it("follows the saved preference in a wide window", () => {
    expect(resolveNavOpen({ narrow: false, navCollapsed: false, narrowOpen: false })).toBe(true);
    expect(resolveNavOpen({ narrow: false, navCollapsed: true, narrowOpen: true })).toBe(false);
  });

  it("folds a narrow window unless the nav was opened while narrow", () => {
    expect(resolveNavOpen({ narrow: true, navCollapsed: false, narrowOpen: false })).toBe(false);
    expect(resolveNavOpen({ narrow: true, navCollapsed: true, narrowOpen: true })).toBe(true);
  });
});

describe("historyHotkeyAction", () => {
  it("walks history with Cmd/Ctrl+[ and ]", () => {
    expect(historyHotkeyAction(key({ metaKey: true, key: "[" }))).toBe("back");
    expect(historyHotkeyAction(key({ ctrlKey: true, key: "]" }))).toBe("forward");
  });

  it("matches the bracket keys by position on layouts that need Option for them", () => {
    expect(historyHotkeyAction(key({ metaKey: true, key: "ü", code: "BracketLeft" }))).toBe("back");
    expect(historyHotkeyAction(key({ metaKey: true, key: "+", code: "BracketRight" }))).toBe("forward");
  });

  it("walks history with Alt+Left/Right", () => {
    expect(historyHotkeyAction(key({ altKey: true, key: "ArrowLeft" }))).toBe("back");
    expect(historyHotkeyAction(key({ altKey: true, key: "ArrowRight" }))).toBe("forward");
  });

  it("ignores plain arrows, repeats, and other modifier shapes", () => {
    expect(historyHotkeyAction(key({ key: "ArrowLeft" }))).toBeNull();
    expect(historyHotkeyAction(key({ metaKey: true, key: "ArrowLeft" }))).toBeNull();
    expect(historyHotkeyAction(key({ metaKey: true, shiftKey: true, key: "[" }))).toBeNull();
    expect(historyHotkeyAction(key({ metaKey: true, altKey: true, key: "[" }))).toBeNull();
    expect(historyHotkeyAction(key({ metaKey: true, key: "[", repeat: true }))).toBeNull();
  });
});

describe("historyHotkeyDecision", () => {
  const altLeft = key({ altKey: true, key: "ArrowLeft" });
  const cmdBracket = key({ metaKey: true, key: "[" });

  it("navigates when history may move", () => {
    expect(historyHotkeyDecision(altLeft, { allowed: true, mac: false })).toBe("back");
    expect(historyHotkeyDecision(cmdBracket, { allowed: true, mac: true })).toBe("back");
  });

  it("swallows Alt+arrows on Windows and Linux when history must stay put", () => {
    expect(historyHotkeyDecision(altLeft, { allowed: false, mac: false })).toBe("suppress");
  });

  it("leaves Option+arrows to text fields on macOS", () => {
    expect(historyHotkeyDecision(altLeft, { allowed: false, mac: true })).toBeNull();
  });

  it("lets Cmd/Ctrl+[ through untouched when blocked", () => {
    expect(historyHotkeyDecision(cmdBracket, { allowed: false, mac: false })).toBeNull();
  });

  it("ignores keys that are not history keys", () => {
    expect(historyHotkeyDecision(key({ key: "a" }), { allowed: true, mac: false })).toBeNull();
  });
});
