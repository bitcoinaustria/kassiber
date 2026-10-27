import * as React from "react";

import {
  ASSISTANT_PANEL_DEFAULT_WIDTH,
  ASSISTANT_PANEL_MAX_WIDTH,
  ASSISTANT_PANEL_MIN_WIDTH,
  DETAIL_PANEL_DEFAULT_WIDTH,
  DETAIL_PANEL_MAX_WIDTH,
  DETAIL_PANEL_MIN_WIDTH,
} from "@/store/ui";

/** Toggle shortcut; `mod` is Cmd on macOS and Ctrl elsewhere. */
export const ASSISTANT_PANEL_SHORTCUT = ["mod", "i"] as const;

/** A panel tab's remembered width range. */
export interface PanelWidthBounds {
  min: number;
  max: number;
  default: number;
}

export const ASSISTANT_PANEL_BOUNDS: PanelWidthBounds = {
  min: ASSISTANT_PANEL_MIN_WIDTH,
  max: ASSISTANT_PANEL_MAX_WIDTH,
  default: ASSISTANT_PANEL_DEFAULT_WIDTH,
};

export const DETAIL_PANEL_BOUNDS: PanelWidthBounds = {
  min: DETAIL_PANEL_MIN_WIDTH,
  max: DETAIL_PANEL_MAX_WIDTH,
  default: DETAIL_PANEL_DEFAULT_WIDTH,
};

/**
 * Below this window width the right panel is a sheet over the page, after
 * T3 Code: docked, it would leave the page too narrow to read.
 */
export const RIGHT_PANEL_SHEET_QUERY = "(max-width: 1023px)";

/** Room the page keeps beside the docked panel when the user drags it wider. */
export const RIGHT_PANEL_MIN_PAGE_WIDTH = 480;

/**
 * Window strip the sheet always leaves uncovered on its left, after
 * T3 Code's sheet (`calc(100% - spacing(12))`), so the page stays in view.
 */
export const RIGHT_PANEL_SHEET_MARGIN = 48;

/** Keyboard resize step for the separator, in CSS px. */
export const ASSISTANT_PANEL_KEY_STEP = 16;

/**
 * How wide a tab may get in a container `containerWidth` wide that keeps
 * `reserve` px for the page.
 */
export interface PanelWidthLimits {
  bounds: PanelWidthBounds;
  containerWidth: number | null;
  reserve: number;
}

export function panelMaxWidth({ bounds, containerWidth, reserve }: PanelWidthLimits): number {
  if (containerWidth === null || !Number.isFinite(containerWidth)) {
    return bounds.max;
  }
  return Math.max(bounds.min, Math.min(bounds.max, containerWidth - reserve));
}

export function clampPanelWidth(width: number, limits: PanelWidthLimits): number {
  return Math.round(
    Math.min(panelMaxWidth(limits), Math.max(limits.bounds.min, width)),
  );
}

/** The handle sits on the panel's left edge, so dragging left widens it. */
export function draggedPanelWidth(
  { startWidth, startX, clientX }: { startWidth: number; startX: number; clientX: number },
  limits: PanelWidthLimits,
): number {
  return clampPanelWidth(startWidth + (startX - clientX), limits);
}

export function keyedPanelWidth(
  key: string,
  width: number,
  limits: PanelWidthLimits,
): number | null {
  switch (key) {
    case "ArrowLeft":
      return clampPanelWidth(width + ASSISTANT_PANEL_KEY_STEP, limits);
    case "ArrowRight":
      return clampPanelWidth(width - ASSISTANT_PANEL_KEY_STEP, limits);
    case "Home":
      return limits.bounds.min;
    case "End":
      return panelMaxWidth(limits);
    case "Enter":
      return clampPanelWidth(limits.bounds.default, limits);
    default:
      return null;
  }
}

/** Moves the caret into the panel composer, at the end of its draft. */
export function focusAssistantComposer(container: HTMLElement | null): boolean {
  const field = container?.querySelector<HTMLTextAreaElement>("textarea");
  if (!field) return false;
  field.focus();
  const end = field.value.length;
  field.setSelectionRange(end, end);
  return true;
}

function subscribeToSheetQuery(onChange: () => void) {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const query = window.matchMedia(RIGHT_PANEL_SHEET_QUERY);
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}

const sheetQueryMatches = () =>
  typeof window !== "undefined" &&
  !!window.matchMedia &&
  window.matchMedia(RIGHT_PANEL_SHEET_QUERY).matches;

/** The window is narrow enough that the right panel is a sheet over the page. */
export function useRightPanelSheet(): boolean {
  return React.useSyncExternalStore(subscribeToSheetQuery, sheetQueryMatches, () => false);
}
