import { describe, expect, it } from "vitest";

import { appWorkflowHotkeyAction } from "./appWorkflowHotkeys";

describe("app workflow hotkeys", () => {
  it("maps the add-wallet shortcut", () => {
    expect(
      appWorkflowHotkeyAction({
        key: "a",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        shiftKey: true,
      }),
    ).toBe("add-wallet");
  });

  it("maps common refresh shortcuts to book refresh actions", () => {
    expect(
      appWorkflowHotkeyAction({
        key: "r",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
      }),
    ).toBe("refresh-book");
    expect(
      appWorkflowHotkeyAction({
        key: "R",
        metaKey: false,
        ctrlKey: true,
        altKey: false,
        shiftKey: true,
      }),
    ).toBe("rescan-book");
    expect(
      appWorkflowHotkeyAction({
        key: "s",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        shiftKey: true,
      }),
    ).toBe("refresh-book");
  });

  it("maps the journal-processing shortcut", () => {
    expect(
      appWorkflowHotkeyAction({
        key: "j",
        metaKey: false,
        ctrlKey: true,
        altKey: false,
        shiftKey: true,
      }),
    ).toBe("process-journals");
  });

  it("ignores plain, alt-modified, and repeated keys", () => {
    expect(
      appWorkflowHotkeyAction({
        key: "r",
        metaKey: false,
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
      }),
    ).toBeNull();
    expect(
      appWorkflowHotkeyAction({
        key: "r",
        metaKey: true,
        ctrlKey: false,
        altKey: true,
        shiftKey: false,
      }),
    ).toBeNull();
    expect(
      appWorkflowHotkeyAction({
        key: "r",
        metaKey: true,
        ctrlKey: false,
        altKey: false,
        shiftKey: false,
        repeat: true,
      }),
    ).toBeNull();
  });
});
