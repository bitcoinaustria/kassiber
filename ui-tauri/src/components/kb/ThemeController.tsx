import * as React from "react";

import { computeAutoScale } from "@/lib/appAutoScale";
import { appScaleHotkeyAction } from "@/lib/appScaleHotkeys";
import { useUiStore } from "@/store/ui";

const DARK_QUERY = "(prefers-color-scheme: dark)";

export function ThemeController() {
  const theme = useUiStore((state) => state.theme);

  React.useLayoutEffect(() => {
    const media = window.matchMedia(DARK_QUERY);

    const applyTheme = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.classList.toggle("light", !dark);
      document.documentElement.style.colorScheme = dark ? "dark" : "light";
    };

    applyTheme();

    if (theme !== "system") return undefined;
    media.addEventListener("change", applyTheme);
    return () => media.removeEventListener("change", applyTheme);
  }, [theme]);

  return null;
}

export function AppScaleController() {
  const appScale = useUiStore((state) => state.appScale);
  const setAppAutoScale = useUiStore((state) => state.setAppAutoScale);
  const decreaseAppScale = useUiStore((state) => state.decreaseAppScale);
  const increaseAppScale = useUiStore((state) => state.increaseAppScale);
  const resetAppScale = useUiStore((state) => state.resetAppScale);

  // Manual nudge → CSS var. Multiplies the automatic screen-fit factor below.
  React.useLayoutEffect(() => {
    document.documentElement.style.setProperty(
      "--app-ui-scale",
      String(appScale),
    );
  }, [appScale]);

  // Automatic screen-fit: derive the base density from the window size so the
  // UI keeps a consistent information density across a laptop and a large
  // monitor, and recompute on resize (rAF-coalesced). Written to the CSS var
  // (drives the root font-size, before first paint via the layout effect) and
  // mirrored into the store so Settings can show the effective scale.
  React.useLayoutEffect(() => {
    let frame = 0;
    const apply = () => {
      frame = 0;
      const scale = computeAutoScale(window.innerWidth, window.innerHeight);
      document.documentElement.style.setProperty(
        "--app-auto-scale",
        String(scale),
      );
      setAppAutoScale(scale);
    };
    apply();
    const handleResize = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(apply);
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      if (frame) window.cancelAnimationFrame(frame);
    };
  }, [setAppAutoScale]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const action = appScaleHotkeyAction(event);
      if (!action) return;
      event.preventDefault();
      if (action === "decrease") {
        decreaseAppScale();
      } else if (action === "increase") {
        increaseAppScale();
      } else {
        resetAppScale();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [decreaseAppScale, increaseAppScale, resetAppScale]);

  return null;
}
