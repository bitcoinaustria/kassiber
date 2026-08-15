import { useLayoutEffect, type ReactNode } from "react";

import { useUiStore } from "@/store/ui";

import { PreAlphaBanner } from "./PreAlphaBanner";

// Keep in sync with TITLEBAR_HEIGHT in src-tauri/src/lib.rs.
const NATIVE_TITLEBAR_HEIGHT = "28px";

export function WindowFrame({ children }: { children: ReactNode }) {
  const preAlphaBannerVisible = useUiStore(
    (state) => state.preAlphaBannerVisible,
  );
  const nativeMacTitlebar =
    typeof window !== "undefined" &&
    "__TAURI_INTERNALS__" in window &&
    (navigator.userAgent.includes("Macintosh") ||
      navigator.userAgent.includes("Mac OS X"));
  const nativeTitlebarHeight = nativeMacTitlebar
    ? NATIVE_TITLEBAR_HEIGHT
    : "0px";
  const warningBarHeight = preAlphaBannerVisible ? "28px" : "0px";

  // Radix portals mount under <body>, outside this frame. Put the shared inset
  // on the document root so routed screens and portalled surfaces agree.
  useLayoutEffect(() => {
    document.documentElement.style.setProperty(
      "--kb-native-titlebar-height",
      nativeTitlebarHeight,
    );
    document.documentElement.style.setProperty(
      "--kb-warning-bar-height",
      warningBarHeight,
    );
    return () => {
      document.documentElement.style.removeProperty(
        "--kb-native-titlebar-height",
      );
      document.documentElement.style.removeProperty("--kb-warning-bar-height");
    };
  }, [nativeTitlebarHeight, warningBarHeight]);

  return (
    <div className="flex h-svh flex-col overflow-hidden bg-sidebar">
      {nativeMacTitlebar ? (
        <div
          aria-hidden="true"
          className="h-[var(--kb-native-titlebar-height)] shrink-0 bg-[var(--kb-native-titlebar-background)]"
        />
      ) : null}
      {preAlphaBannerVisible ? (
        <PreAlphaBanner className="relative z-[60] shrink-0" />
      ) : null}
      <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
    </div>
  );
}
