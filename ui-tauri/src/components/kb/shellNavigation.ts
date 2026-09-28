/**
 * Shell navigation rules that do not need React: when the side nav folds to
 * its rail on its own, and which keys walk the page history.
 */

/**
 * Below this window width the side nav folds to its icon rail by itself, so a
 * laptop-sized window keeps its width for the page. The user's own
 * collapse preference is untouched and returns once the window is wide again.
 * At the 980px minimum window width an open nav would leave the page ~750px.
 */
export const NAV_AUTO_COLLAPSE_BELOW_PX = 1100;

/**
 * Whether the side nav is open.
 *
 * In a wide window the persisted preference decides. In a narrow one the nav
 * starts folded and a toggle opens it only for as long as the window stays
 * narrow (`narrowOpen`), so briefly narrowing the window never rewrites the
 * preference.
 */
export function resolveNavOpen({
  narrow,
  navCollapsed,
  narrowOpen,
}: {
  narrow: boolean;
  navCollapsed: boolean;
  narrowOpen: boolean;
}): boolean {
  return narrow ? narrowOpen : !navCollapsed;
}

export type HistoryHotkeyAction = "back" | "forward";

type HistoryHotkeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
> & {
  repeat?: boolean;
};

/**
 * Back/forward through the page history with the conventions desktop users
 * already know: Cmd/Ctrl+[ and ] (Finder, Safari, Xcode) and Alt+Left/Right
 * (Windows and Linux browsers and file managers). The brackets also match by
 * physical key, because on German layouts `[` and `]` need Option.
 */
export function historyHotkeyAction(
  event: HistoryHotkeyEvent,
): HistoryHotkeyAction | null {
  if (event.repeat || event.shiftKey) return null;
  const command = event.metaKey || event.ctrlKey;
  if (command && !event.altKey) {
    if (event.key === "[" || event.code === "BracketLeft") return "back";
    if (event.key === "]" || event.code === "BracketRight") return "forward";
    return null;
  }
  if (event.altKey && !command) {
    if (event.key === "ArrowLeft") return "back";
    if (event.key === "ArrowRight") return "forward";
  }
  return null;
}

/**
 * What a key press does to page history, given whether history may move now
 * (unlocked, not typing, no dialog open).
 *
 * `"suppress"` swallows the key without navigating. It exists for Windows,
 * where WebView2 treats Alt+Left/Right as its own back/forward accelerator:
 * returning early there would let the webview navigate anyway, behind a dialog
 * or the lock screen. macOS keeps those keys, because Option+Left/Right moves
 * the caret by word in a text field.
 */
export function historyHotkeyDecision(
  event: HistoryHotkeyEvent,
  { allowed, mac }: { allowed: boolean; mac: boolean },
): HistoryHotkeyAction | "suppress" | null {
  const action = historyHotkeyAction(event);
  if (!action) return null;
  if (allowed) return action;
  return !mac && event.altKey ? "suppress" : null;
}
