/**
 * Window-chrome geometry shared by the frame, the app shell's title bar, and
 * portalled full-window surfaces.
 *
 * On macOS the Tauri window uses an overlay title bar: the traffic lights sit
 * on top of the webview, so the app draws the title bar itself — one row that
 * carries the window controls, navigation, and shell actions, the way native
 * Mac apps unify title bar and toolbar. Windows and Linux keep their native
 * decorated frame, so the same row is an ordinary in-app toolbar there and
 * reserves nothing for window controls.
 */

/**
 * Height of the shell's title bar row, in CSS px rather than rem: the native
 * traffic lights are positioned in points and do not follow the UI scale, so
 * the row they are centred in must not either.
 *
 * Keep in sync with `trafficLightPosition` in src-tauri/tauri.conf.json.
 */
export const WINDOW_TITLEBAR_HEIGHT_PX = 40;

/**
 * Space the macOS traffic lights occupy from the window's left edge, plus the
 * gap before the first app control.
 */
export const TRAFFIC_LIGHT_INSET_PX = 80;

type NativeTitlebarEnvironment = {
  window?: object;
  userAgent?: string;
};

/** True inside the Tauri shell on macOS, where the title bar is ours to draw. */
export function isNativeMacTitlebar({
  window: win = typeof window === "undefined" ? undefined : window,
  userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent,
}: NativeTitlebarEnvironment = {}): boolean {
  return (
    win !== undefined &&
    "__TAURI_INTERNALS__" in win &&
    (userAgent.includes("Macintosh") || userAgent.includes("Mac OS X"))
  );
}

export type WindowChromeInsets = {
  /** True while the traffic lights are on screen and the row is window chrome. */
  nativeTitlebar: boolean;
  /** Height of window chrome above the page; portalled surfaces start below it. */
  titlebarHeight: string;
  /** Leading space the traffic lights need inside the title bar row. */
  trafficLightInset: string;
};

/**
 * Full screen hides the traffic lights and the title bar with them, so the row
 * stops being window chrome: nothing to dodge, nothing to drag.
 */
export function windowChromeInsets({
  nativeMac,
  fullscreen,
}: {
  nativeMac: boolean;
  fullscreen: boolean;
}): WindowChromeInsets {
  const nativeTitlebar = nativeMac && !fullscreen;
  return {
    nativeTitlebar,
    titlebarHeight: nativeTitlebar ? `${WINDOW_TITLEBAR_HEIGHT_PX}px` : "0px",
    trafficLightInset: nativeTitlebar ? `${TRAFFIC_LIGHT_INSET_PX}px` : "0px",
  };
}

/**
 * Whether a press should move the window even though the page is inert.
 *
 * A modal dialog (the full-screen chart, the command palette, any Radix dialog)
 * sets `pointer-events: none` on `<body>`, so a press on the title bar lands on
 * `<html>` and Tauri's drag regions never see it. The title bar is still on
 * screen above the dialog, so treat that press as a title-bar drag, as AppKit
 * does for a window with a sheet open. An open menu is the exception: a press
 * outside a menu closes it, and should keep doing so.
 */
export function shouldDragInertTitlebar({
  nativeTitlebar,
  button,
  clientY,
  targetIsRoot,
  menuOpen,
}: {
  nativeTitlebar: boolean;
  button: number;
  clientY: number;
  targetIsRoot: boolean;
  menuOpen: boolean;
}): boolean {
  return (
    nativeTitlebar &&
    button === 0 &&
    targetIsRoot &&
    !menuOpen &&
    clientY >= 0 &&
    clientY < WINDOW_TITLEBAR_HEIGHT_PX
  );
}
