import * as React from "react";

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
  const decreaseAppScale = useUiStore((state) => state.decreaseAppScale);
  const increaseAppScale = useUiStore((state) => state.increaseAppScale);
  const resetAppScale = useUiStore((state) => state.resetAppScale);

  React.useLayoutEffect(() => {
    document.documentElement.style.setProperty(
      "--app-ui-scale",
      String(appScale),
    );
  }, [appScale]);

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
