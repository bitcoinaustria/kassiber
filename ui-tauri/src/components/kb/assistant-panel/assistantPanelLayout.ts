import * as React from "react";

import {
  ASSISTANT_PANEL_DEFAULT_WIDTH,
  ASSISTANT_PANEL_MAX_WIDTH,
  ASSISTANT_PANEL_MIN_WIDTH,
} from "@/store/ui";

/** Toggle shortcut; `mod` is Cmd on macOS and Ctrl elsewhere. */
export const ASSISTANT_PANEL_SHORTCUT = ["mod", "i"] as const;

/**
 * Below this window width the panel overlays the page as a sheet: pushing
 * would leave the page too narrow to read beside a ~400px panel.
 */
export const ASSISTANT_PANEL_OVERLAY_QUERY = "(max-width: 1023px)";

/** Room the page keeps beside the panel when the user drags it wider. */
export const ASSISTANT_PANEL_MIN_PAGE_WIDTH = 480;

/** Keyboard resize step for the separator, in CSS px. */
export const ASSISTANT_PANEL_KEY_STEP = 16;

export function assistantPanelMaxWidth(containerWidth: number | null): number {
  if (containerWidth === null || !Number.isFinite(containerWidth)) {
    return ASSISTANT_PANEL_MAX_WIDTH;
  }
  return Math.max(
    ASSISTANT_PANEL_MIN_WIDTH,
    Math.min(
      ASSISTANT_PANEL_MAX_WIDTH,
      containerWidth - ASSISTANT_PANEL_MIN_PAGE_WIDTH,
    ),
  );
}

export function clampAssistantPanelWidth(
  width: number,
  containerWidth: number | null,
): number {
  return Math.round(
    Math.min(
      assistantPanelMaxWidth(containerWidth),
      Math.max(ASSISTANT_PANEL_MIN_WIDTH, width),
    ),
  );
}

/** The handle sits on the panel's left edge, so dragging left widens it. */
export function draggedAssistantPanelWidth({
  startWidth,
  startX,
  clientX,
  containerWidth,
}: {
  startWidth: number;
  startX: number;
  clientX: number;
  containerWidth: number | null;
}): number {
  return clampAssistantPanelWidth(startWidth + (startX - clientX), containerWidth);
}

export function keyedAssistantPanelWidth(
  key: string,
  width: number,
  containerWidth: number | null,
): number | null {
  switch (key) {
    case "ArrowLeft":
      return clampAssistantPanelWidth(width + ASSISTANT_PANEL_KEY_STEP, containerWidth);
    case "ArrowRight":
      return clampAssistantPanelWidth(width - ASSISTANT_PANEL_KEY_STEP, containerWidth);
    case "Home":
      return ASSISTANT_PANEL_MIN_WIDTH;
    case "End":
      return assistantPanelMaxWidth(containerWidth);
    case "Enter":
      return clampAssistantPanelWidth(ASSISTANT_PANEL_DEFAULT_WIDTH, containerWidth);
    default:
      return null;
  }
}

export function useMediaQuery(query: string): boolean {
  const subscribe = React.useCallback(
    (onChange: () => void) => {
      if (typeof window === "undefined" || !window.matchMedia) return () => {};
      const list = window.matchMedia(query);
      list.addEventListener("change", onChange);
      return () => list.removeEventListener("change", onChange);
    },
    [query],
  );
  return React.useSyncExternalStore(
    subscribe,
    () =>
      typeof window !== "undefined" && window.matchMedia
        ? window.matchMedia(query).matches
        : false,
    () => false,
  );
}
