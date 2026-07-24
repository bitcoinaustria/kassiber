/**
 * Scroll-mode rules for the assistant conversation ({@link ChatThread}).
 *
 * Kept as pure functions in their own module so the opt-out and affordance
 * rules are unit-testable without a DOM (the vitest env is `node`).
 */

export type ScrollMode = "following" | "anchoring" | "free";

/**
 * Resolve the scroll mode after a scroll event.
 *
 * `wheel`/`touchmove` are hooked directly, but they are not the only way to
 * leave the live edge: the scrollbar, PageUp/Home and a screen reader's caret
 * all move the scroller without either event. Those have to opt out of
 * auto-follow too, or streaming deltas yank the reader back to the bottom and
 * the jump pill never appears. Only *backwards* movement counts, so our own
 * programmatic scrolls (always toward the end) and content growth don't trip
 * it.
 */
export function resolveScrollMode(
  mode: ScrollMode,
  { atBottom, scrolledUp }: { atBottom: boolean; scrolledUp: boolean },
): ScrollMode {
  // Reaching the live edge re-engages following.
  if (atBottom) return mode === "free" ? "following" : mode;
  if (scrolledUp) return "free";
  return mode;
}

/** Which scroll affordances the given mode/position should surface. */
export function resolveScrollAffordances(
  mode: ScrollMode,
  { atBottom, atTop }: { atBottom: boolean; atTop: boolean },
): { jumpToLatest: boolean; jumpToTop: boolean } {
  return {
    // Only offer "jump to latest" when we're not already chasing the tail.
    jumpToLatest: mode === "free" && !atBottom,
    // Jumping to the top doesn't fight the stream, so it stays available in
    // any settled position — hidden only mid-anchor, where the view is being
    // held deliberately and the pill would sit on the anchored turn.
    jumpToTop: mode !== "anchoring" && !atTop,
  };
}
