/**
 * Page column plus the assistant: the closed bar sits under the page and the
 * open panel sits beside it, so the assistant never covers what the user is
 * asking about. Narrow windows get the panel as an overlay sheet instead.
 */

import * as React from "react";

import { useAssistantSession } from "@/components/ai/assistantSession";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
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
}: {
  /** The page (`<main>`), which keeps its own scroll. */
  children: React.ReactNode;
  /** False on the full assistant page, which is the assistant itself. */
  enabled: boolean;
}) {
  const session = useAssistantSession();
  const open = useUiStore((s) => s.assistantPanelOpen);
  const setOpen = useUiStore((s) => s.setAssistantPanelOpen);
  const toggle = useUiStore((s) => s.toggleAssistantPanel);
  const closedBar = useUiStore((s) => s.assistantClosedBar);
  const position = useUiStore((s) => s.assistantDockPosition);
  // A page holding the whole column (useAssistantColumnClaim) keeps both the
  // panel and the bar out until the user asks for the assistant.
  const columnClaimed = useUiStore((s) => s.assistantColumnClaimed);
  const overlay = useMediaQuery(ASSISTANT_PANEL_OVERLAY_QUERY);
  const showPanel = enabled && open && !columnClaimed;

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

  // A consent prompt or a failed turn needs the conversation in view, so it
  // opens the panel once. Closing it again is the user's choice.
  const needsAttention = Boolean(session.pendingConsent || session.error);
  // Not a request for the column: a page holding it keeps it, and the
  // approval dialog below stays answerable over it.
  React.useEffect(() => {
    if (enabled && needsAttention) setOpen(true);
  }, [enabled, needsAttention, setOpen]);

  return (
    <div className="relative flex min-h-0 w-full min-w-0 flex-1">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {children}
        {enabled && !showPanel && !columnClaimed && closedBar !== "hidden" ? (
          <AssistantClosedBar mode={closedBar} position={position} />
        ) : null}
      </div>
      {showPanel ? <AssistantSidePanel overlay={overlay} /> : null}
      {/* The approval dialog belongs to the layout, not the panel: a turn
          waiting on it must stay answerable when the panel is closed, the
          bar is hidden, or a page claims the whole column. Only the full
          assistant page, which shows its own, leaves it out. */}
      {enabled ? (
        <ToolConsentDialog
          request={session.pendingConsent}
          onDecision={session.sendConsent}
        />
      ) : null}
    </div>
  );
}
