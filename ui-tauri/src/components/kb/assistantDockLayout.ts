export const ASSISTANT_DOCK_EXPANDED_MAIN_PADDING_PX = 240;
export const ASSISTANT_DOCK_COLLAPSED_MAIN_PADDING_PX = 64;

const ASSISTANT_DOCK_PADDING_DELTA_PX =
  ASSISTANT_DOCK_EXPANDED_MAIN_PADDING_PX -
  ASSISTANT_DOCK_COLLAPSED_MAIN_PADDING_PX;
const ASSISTANT_DOCK_COLLAPSE_SCROLL_TOP_PX = 96;
const ASSISTANT_DOCK_EXPAND_SCROLL_TOP_PX = 24;
const ASSISTANT_DOCK_COLLAPSE_MIN_PROGRESS = 0.04;
const ASSISTANT_DOCK_BOTTOM_GUARD_PX = 12;

export type AssistantDockScrollState = {
  collapsed: boolean;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
};

export function nextAssistantDockCollapsed({
  collapsed,
  scrollTop,
  scrollHeight,
  clientHeight,
}: AssistantDockScrollState) {
  const scrollableHeight = Math.max(0, scrollHeight - clientHeight);
  if (scrollableHeight <= 0) return false;

  if (collapsed) {
    return scrollTop > ASSISTANT_DOCK_EXPAND_SCROLL_TOP_PX;
  }

  const collapsedScrollableHeight = Math.max(
    0,
    scrollableHeight - ASSISTANT_DOCK_PADDING_DELTA_PX,
  );
  if (collapsedScrollableHeight <= ASSISTANT_DOCK_COLLAPSE_SCROLL_TOP_PX) {
    return false;
  }
  if (scrollTop >= collapsedScrollableHeight - ASSISTANT_DOCK_BOTTOM_GUARD_PX) {
    return false;
  }

  const scrolledProgress = scrollTop / Math.max(1, scrollableHeight);
  return (
    scrollTop > ASSISTANT_DOCK_COLLAPSE_SCROLL_TOP_PX &&
    scrolledProgress > ASSISTANT_DOCK_COLLAPSE_MIN_PROGRESS
  );
}
