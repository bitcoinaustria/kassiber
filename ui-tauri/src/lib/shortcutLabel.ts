/**
 * Keyboard shortcut labels in each platform's own notation.
 *
 * Shortcuts are written once, platform-neutrally — `["mod", "shift", "a"]` —
 * and rendered the way the OS writes them: `⌘⇧A` on macOS, `Ctrl+Shift+A` on
 * Windows and Linux. `mod` is Cmd on macOS and Ctrl elsewhere, matching the
 * `metaKey || ctrlKey` checks the handlers use.
 */

const MAC_GLYPHS: Record<string, string> = {
  mod: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
  enter: "↩",
  escape: "⎋",
  left: "←",
  right: "→",
};

const OTHER_NAMES: Record<string, string> = {
  mod: "Ctrl",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
  enter: "Enter",
  escape: "Esc",
  left: "Left",
  right: "Right",
};

export function isMacPlatform(
  userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent,
): boolean {
  return /Mac|iPhone|iPad/.test(userAgent);
}

export function formatShortcut(
  keys: readonly string[],
  mac = isMacPlatform(),
): string {
  const labels = keys.map((key) => {
    const name = key.toLowerCase();
    const named = mac ? MAC_GLYPHS[name] : OTHER_NAMES[name];
    return named ?? (key.length === 1 ? key.toUpperCase() : key);
  });
  return mac ? labels.join("") : labels.join("+");
}
