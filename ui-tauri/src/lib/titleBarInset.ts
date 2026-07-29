// The macOS window runs with `titleBarStyle: "Overlay"`, so the traffic lights
// float over the webview's own top-left corner instead of sitting in a bar of
// their own. Whatever lands in that corner — the shell's top strip, or the
// onboarding header before the shell exists — has to leave room for them.
export const macTitleBarInset =
  typeof window !== "undefined" &&
  "__TAURI_INTERNALS__" in window &&
  navigator.userAgent.includes("Macintosh");
