import { describe, expect, it } from "vitest";

import { appScaleHotkeyAction } from "./appScaleHotkeys";

describe("app scale hotkeys", () => {
  it("maps common zoom shortcuts to scale actions", () => {
    expect(
      appScaleHotkeyAction({
        key: "-",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
      }),
    ).toBe("decrease");
    expect(
      appScaleHotkeyAction({
        key: "+",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
      }),
    ).toBe("increase");
    expect(
      appScaleHotkeyAction({
        key: "=",
        metaKey: false,
        ctrlKey: true,
        altKey: false,
      }),
    ).toBe("increase");
    expect(
      appScaleHotkeyAction({
        key: "0",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
      }),
    ).toBe("reset");
  });

  it("ignores plain keys and alt-modified shortcuts", () => {
    expect(
      appScaleHotkeyAction({
        key: "-",
        metaKey: false,
        ctrlKey: false,
        altKey: false,
      }),
    ).toBeNull();
    expect(
      appScaleHotkeyAction({
        key: "+",
        metaKey: true,
        ctrlKey: false,
        altKey: true,
      }),
    ).toBeNull();
    expect(
      appScaleHotkeyAction({
        key: "x",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
      }),
    ).toBeNull();
  });
});
