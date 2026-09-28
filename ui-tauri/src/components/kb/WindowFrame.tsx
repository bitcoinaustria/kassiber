import type { Window as TauriWindow } from "@tauri-apps/api/window";
import * as React from "react";

import { safeTauriUnlisten } from "@/lib/tauriUnlisten";
import { useUiStore } from "@/store/ui";

import { AlphaNotice } from "./AlphaNotice";
import { WindowChromeContext } from "./windowChromeContext";
import {
  isNativeMacTitlebar,
  shouldDragInertTitlebar,
  windowChromeInsets,
} from "./windowChrome";

let appWindowPromise: Promise<TauriWindow> | null = null;

/** The Tauri window handle, loaded once and shared by the hooks below. */
function loadAppWindow(): Promise<TauriWindow> {
  appWindowPromise ??= import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow())
    .catch((error: unknown) => {
      appWindowPromise = null;
      throw error;
    });
  return appWindowPromise;
}

/**
 * Tracks native full screen, which hides the traffic lights. Tauri has no
 * full-screen event, but entering and leaving always resizes the window.
 */
function useNativeFullscreen(enabled: boolean) {
  const [fullscreen, setFullscreen] = React.useState(false);

  React.useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void loadAppWindow()
      .then((appWindow) => {
        // A live resize fires many events; keep at most one query in flight
        // and one queued behind it.
        let inFlight = false;
        let queued = false;
        const refresh = () => {
          if (inFlight) {
            queued = true;
            return;
          }
          inFlight = true;
          void appWindow
            .isFullscreen()
            .then((value) => {
              if (!disposed) setFullscreen(value);
            })
            .catch(() => {})
            .finally(() => {
              inFlight = false;
              if (queued && !disposed) {
                queued = false;
                refresh();
              }
            });
        };
        refresh();
        return appWindow.onResized(refresh);
      })
      .then((stop) => {
        if (disposed) safeTauriUnlisten(stop);
        else unlisten = stop;
      })
      .catch((error: unknown) => {
        console.warn("Could not track full-screen state", error);
      });

    return () => {
      disposed = true;
      safeTauriUnlisten(unlisten);
    };
  }, [enabled]);

  return fullscreen;
}

/**
 * Keeps the title bar draggable while a modal dialog makes the page inert.
 *
 * Tauri's `data-tauri-drag-region` handles the ordinary case. With a dialog open
 * (the full-screen chart, the command palette), `<body>` stops taking pointer
 * events, so the press never reaches a drag region — and Radix would read it as
 * a click outside and close the dialog. Portalled surfaces start below the title
 * bar, so a press in that row is a press on window chrome: move the window and
 * keep the dialog.
 */
function useInertTitlebarDrag(nativeTitlebar: boolean) {
  React.useEffect(() => {
    if (!nativeTitlebar) return;
    let disposed = false;
    let startDragging: (() => Promise<void>) | null = null;

    void loadAppWindow()
      .then((appWindow) => {
        if (disposed) return;
        startDragging = () => appWindow.startDragging();
      })
      .catch((error: unknown) => {
        console.warn("Could not prepare title bar dragging", error);
      });

    const isInertTitlebarPress = (event: MouseEvent) =>
      shouldDragInertTitlebar({
        nativeTitlebar,
        button: event.button,
        clientY: event.clientY,
        targetIsRoot: event.target === document.documentElement,
        menuOpen:
          document.querySelector(
            '[role="menu"][data-state="open"], [role="listbox"][data-state="open"]',
          ) !== null,
      });

    // `pointerdown` is what Radix's dismiss listener hears; stopping it keeps
    // the dialog open. It must not be default-prevented, or the browser skips
    // the `mousedown` that starts the drag.
    const onPointerDown = (event: PointerEvent) => {
      if (isInertTitlebarPress(event)) event.stopPropagation();
    };
    const onMouseDown = (event: MouseEvent) => {
      if (!isInertTitlebarPress(event) || event.detail > 1) return;
      event.preventDefault();
      event.stopPropagation();
      void startDragging?.().catch((error: unknown) => {
        console.warn("Could not start dragging the window", error);
      });
    };

    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("mousedown", onMouseDown, true);
    return () => {
      disposed = true;
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("mousedown", onMouseDown, true);
    };
  }, [nativeTitlebar]);
}

export function WindowFrame({ children }: { children: React.ReactNode }) {
  const preAlphaBannerVisible = useUiStore(
    (state) => state.preAlphaBannerVisible,
  );
  const nativeMac = React.useMemo(() => isNativeMacTitlebar(), []);
  const fullscreen = useNativeFullscreen(nativeMac);
  const insets = windowChromeInsets({ nativeMac, fullscreen });
  const [claims, setClaims] = React.useState(0);
  const claimed = claims > 0;

  const claimTitlebar = React.useCallback(() => {
    setClaims((count) => count + 1);
    return () => setClaims((count) => Math.max(0, count - 1));
  }, []);
  const context = React.useMemo(
    () => ({ nativeTitlebar: insets.nativeTitlebar, claimTitlebar }),
    [insets.nativeTitlebar, claimTitlebar],
  );

  useInertTitlebarDrag(insets.nativeTitlebar);

  // Radix portals mount under <body>, outside this frame. Put the shared inset
  // on the document root so routed screens and portalled surfaces agree.
  React.useLayoutEffect(() => {
    const root = document.documentElement.style;
    root.setProperty("--kb-native-titlebar-height", insets.titlebarHeight);
    root.setProperty("--kb-traffic-light-inset", insets.trafficLightInset);
    return () => {
      root.removeProperty("--kb-native-titlebar-height");
      root.removeProperty("--kb-traffic-light-inset");
    };
  }, [insets.titlebarHeight, insets.trafficLightInset]);

  return (
    <WindowChromeContext.Provider value={context}>
      <div className="relative flex h-svh flex-col overflow-hidden bg-sidebar">
        {insets.nativeTitlebar && !claimed ? (
          <div
            aria-hidden="true"
            data-tauri-drag-region
            className="h-[var(--kb-native-titlebar-height)] shrink-0 bg-[var(--kb-native-titlebar-background)]"
          />
        ) : null}
        <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
        {/* Screens without the shell's title bar still carry the warning, in
            the same place: the right end of the top row. */}
        {preAlphaBannerVisible && !claimed ? (
          <div className="pointer-events-none absolute top-0 right-3 z-40 flex h-(--kb-toolbar-height) items-center">
            <AlphaNotice className="pointer-events-auto" />
          </div>
        ) : null}
      </div>
    </WindowChromeContext.Provider>
  );
}
