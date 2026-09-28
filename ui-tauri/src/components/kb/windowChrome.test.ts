import { describe, expect, it } from "vitest";

import {
  isNativeMacTitlebar,
  shouldDragInertTitlebar,
  TRAFFIC_LIGHT_INSET_PX,
  WINDOW_TITLEBAR_HEIGHT_PX,
  windowChromeInsets,
} from "./windowChrome";

const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)";
const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64)";

describe("isNativeMacTitlebar", () => {
  it("is true only inside Tauri on macOS", () => {
    const tauri = { __TAURI_INTERNALS__: {} };
    expect(isNativeMacTitlebar({ window: tauri, userAgent: MAC })).toBe(true);
    expect(isNativeMacTitlebar({ window: {}, userAgent: MAC })).toBe(false);
    expect(isNativeMacTitlebar({ window: tauri, userAgent: WINDOWS })).toBe(false);
    expect(isNativeMacTitlebar({ window: tauri, userAgent: LINUX })).toBe(false);
  });
});

describe("windowChromeInsets", () => {
  it("reserves the title bar and traffic lights on macOS", () => {
    expect(windowChromeInsets({ nativeMac: true, fullscreen: false })).toEqual({
      nativeTitlebar: true,
      titlebarHeight: `${WINDOW_TITLEBAR_HEIGHT_PX}px`,
      trafficLightInset: `${TRAFFIC_LIGHT_INSET_PX}px`,
    });
  });

  it("drops both in full screen, where the traffic lights are hidden", () => {
    expect(windowChromeInsets({ nativeMac: true, fullscreen: true })).toEqual({
      nativeTitlebar: false,
      titlebarHeight: "0px",
      trafficLightInset: "0px",
    });
  });

  it("reserves nothing under a native Windows or Linux frame", () => {
    expect(windowChromeInsets({ nativeMac: false, fullscreen: false })).toEqual({
      nativeTitlebar: false,
      titlebarHeight: "0px",
      trafficLightInset: "0px",
    });
  });
});

describe("shouldDragInertTitlebar", () => {
  const press = {
    nativeTitlebar: true,
    button: 0,
    clientY: 20,
    targetIsRoot: true,
    menuOpen: false,
  };

  it("drags from the title bar while a modal makes the page inert", () => {
    expect(shouldDragInertTitlebar(press)).toBe(true);
  });

  it("leaves presses below the title bar to the dialog", () => {
    expect(
      shouldDragInertTitlebar({ ...press, clientY: WINDOW_TITLEBAR_HEIGHT_PX }),
    ).toBe(false);
  });

  it("leaves ordinary presses to Tauri's drag regions", () => {
    expect(shouldDragInertTitlebar({ ...press, targetIsRoot: false })).toBe(false);
  });

  it("lets a press outside an open menu close it", () => {
    expect(shouldDragInertTitlebar({ ...press, menuOpen: true })).toBe(false);
  });

  it("ignores secondary buttons and platforms without an overlay title bar", () => {
    expect(shouldDragInertTitlebar({ ...press, button: 2 })).toBe(false);
    expect(shouldDragInertTitlebar({ ...press, nativeTitlebar: false })).toBe(false);
  });
});
