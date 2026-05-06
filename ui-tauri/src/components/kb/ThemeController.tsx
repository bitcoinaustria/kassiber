import * as React from "react";

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
