/**
 * Page column plus the assistant: the closed bar sits under the page and the
 * open panel sits beside it, so the assistant never covers what the user is
 * asking about. Narrow windows get the panel as an overlay sheet instead.
 */

import * as React from "react";

import { useAssistantSession } from "@/components/ai/assistantSession";
import { useUiStore } from "@/store/ui";

import { AssistantClosedBar } from "./AssistantClosedBar";
import { AssistantSidePanel } from "./AssistantSidePanel";
import {
  ASSISTANT_PANEL_OVERLAY_QUERY,
  useMediaQuery,
} from "./assistantPanelLayout";

export function AssistantLayout({
  children,
  enabled,
  barSuppressed = false,
}: {
  /** The page (`<main>`), which keeps its own scroll. */
  children: React.ReactNode;
  /** False on the full assistant page, which is the assistant itself. */
  enabled: boolean;
  /** A page may claim the whole column (e.g. the expanded table). */
  barSuppressed?: boolean;
}) {
  const session = useAssistantSession();
  const open = useUiStore((s) => s.assistantPanelOpen);
  const openPanel = useUiStore((s) => s.openAssistantPanel);
  const toggle = useUiStore((s) => s.toggleAssistantPanel);
  const closedBar = useUiStore((s) => s.assistantClosedBar);
  const position = useUiStore((s) => s.assistantDockPosition);
  const overlay = useMediaQuery(ASSISTANT_PANEL_OVERLAY_QUERY);
  const showPanel = enabled && open;

  React.useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "i") return;
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) {
        return;
      }
      event.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, toggle]);

  // A consent prompt or a failed turn needs the conversation in view; the
  // panel owns the consent dialog.
  const needsAttention = Boolean(session.pendingConsent || session.error);
  React.useEffect(() => {
    if (enabled && needsAttention) openPanel();
  }, [enabled, needsAttention, openPanel]);

  return (
    <div className="relative flex min-h-0 w-full min-w-0 flex-1">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {children}
        {enabled && !showPanel && !barSuppressed && closedBar !== "hidden" ? (
          <AssistantClosedBar mode={closedBar} position={position} />
        ) : null}
      </div>
      {showPanel ? <AssistantSidePanel overlay={overlay} /> : null}
    </div>
  );
}
