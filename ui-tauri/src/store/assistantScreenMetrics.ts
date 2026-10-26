import * as React from "react";
import { create } from "zustand";

import type {
  AssistantMetricsScope,
  AssistantReturnPath,
  AssistantScreenMetrics,
} from "@/components/ai/assistantSession";

/**
 * What the visible page is showing right now, for the assistant's typed
 * screen context: a few counts plus the page's period. Pages publish while
 * mounted and clear on unmount; it is ephemeral and never persisted.
 */
export interface AssistantScreenSnapshot {
  route: AssistantReturnPath;
  metrics: AssistantScreenMetrics;
  /** Book state the counts were computed on; counts without it are not sent. */
  scope: AssistantMetricsScope;
  /** Resolved period key of the page's date filter (e.g. "1year"). */
  period?: string;
}

interface AssistantScreenMetricsState {
  snapshot: AssistantScreenSnapshot | null;
  publish: (snapshot: AssistantScreenSnapshot) => void;
  clear: (route: AssistantReturnPath) => void;
}

export const useAssistantScreenMetricsStore = create<AssistantScreenMetricsState>()(
  (set) => ({
    snapshot: null,
    publish: (snapshot) => set({ snapshot }),
    clear: (route) =>
      set((state) =>
        state.snapshot?.route === route ? { snapshot: null } : state,
      ),
  }),
);

const PERIOD_KEY = /^[a-z0-9]{1,16}$/;

/**
 * Publish the page's visible counts while mounted. Pass `null` while the data
 * is still loading so the assistant never sees a half-loaded page.
 */
export function usePublishAssistantScreenMetrics(
  snapshot: AssistantScreenSnapshot | null,
) {
  const key = snapshot ? JSON.stringify(snapshot) : null;
  const route = snapshot?.route ?? null;
  React.useEffect(() => {
    if (!key) return;
    const next = JSON.parse(key) as AssistantScreenSnapshot;
    useAssistantScreenMetricsStore.getState().publish({
      route: next.route,
      metrics: next.metrics,
      scope: next.scope,
      ...(next.period && PERIOD_KEY.test(next.period)
        ? { period: next.period }
        : {}),
    });
  }, [key]);
  React.useEffect(() => {
    if (!route) return;
    return () => useAssistantScreenMetricsStore.getState().clear(route);
  }, [route]);
}
