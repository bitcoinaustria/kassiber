import * as React from "react";

type WindowChromeContextValue = {
  /** The title bar row is window chrome: traffic lights on it, drags move the window. */
  nativeTitlebar: boolean;
  claimTitlebar: () => () => void;
};

/** Provided by `WindowFrame`; the defaults describe a plain browser window. */
export const WindowChromeContext = React.createContext<WindowChromeContextValue>({
  nativeTitlebar: false,
  claimTitlebar: () => () => {},
});

export function useWindowChrome() {
  return React.useContext(WindowChromeContext);
}

/**
 * Declare that this screen draws the window's title bar row itself.
 *
 * The app shell does: its first row carries the traffic lights, navigation,
 * and shell actions. Screens that do not (setup, the error boundary, a route
 * that is still loading) get the frame's plain drag strip instead. A layout
 * effect, so the strip is gone before the claiming screen first paints.
 */
export function useClaimWindowTitlebar(active = true) {
  const { claimTitlebar } = useWindowChrome();
  React.useLayoutEffect(() => {
    if (!active) return;
    return claimTitlebar();
  }, [active, claimTitlebar]);
}
