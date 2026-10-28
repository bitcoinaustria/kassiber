/**
 * Page column plus the right panel beside it: the closed bar sits under the
 * page while the assistant is out of view, and the assistant, the panel's
 * surfaces and open transactions' details are tabs of the panel.
 */

import * as React from "react";

import { useAssistantSession } from "@/components/ai/assistantSession";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { useUiStore } from "@/store/ui";

import { cn } from "@/lib/utils";
import { useRightPanelStore } from "@/store/rightPanel";

import { AssistantClosedBar } from "./AssistantClosedBar";
import { useRightPanelSheet } from "./assistantPanelLayout";
import { RightPanel } from "./RightPanel";

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
  const openPanel = useUiStore((s) => s.openAssistantPanel);
  const toggle = useUiStore((s) => s.toggleAssistantPanel);
  const setActiveTab = useRightPanelStore((s) => s.setActiveTab);
  const closedBar = useUiStore((s) => s.assistantClosedBar);
  const position = useUiStore((s) => s.assistantDockPosition);
  // A page holding the whole column (useAssistantColumnClaim) keeps both the
  // panel and the bar out until the user asks for the assistant or a turn
  // fails.
  const columnClaimed = useUiStore((s) => s.assistantColumnClaimed);
  const panelOpen = useUiStore((s) => s.rightPanelOpen);
  const maximized = useUiStore((s) => s.rightPanelMaximized);
  const hasDetail = useRightPanelStore((s) => s.details.length > 0);
  const sheet = useRightPanelSheet();
  const assistantTab = enabled && open && !columnClaimed;
  // A page holding the column (or the assistant page) still shows a detail
  // it opens, alone in the panel; the launcher and the assistant wait.
  const showPanel = panelOpen && (hasDetail || (enabled && !columnClaimed));
  const showAssistant = showPanel && assistantTab;
  const pageHidden = showPanel && maximized && !sheet;

  React.useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "i") return;
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) {
        return;
      }
      event.preventDefault();
      // Behind another tab, the shortcut brings the assistant forward
      // rather than closing it.
      const panel = useRightPanelStore.getState();
      const ui = useUiStore.getState();
      if (
        ui.rightPanelOpen &&
        ui.assistantPanelOpen &&
        !ui.assistantColumnClaimed &&
        panel.activeTab !== "assistant" &&
        (panel.details.length > 0 || panel.surfaces.length > 0)
      ) {
        setActiveTab("assistant");
        return;
      }
      toggle();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, toggle, setActiveTab]);

  // A consent prompt or a failed turn needs the conversation in view, so it
  // opens the panel once. Closing it again is the user's choice.
  const awaitingConsent = Boolean(session.pendingConsent);
  // Not a request for the column: a page holding it keeps it, and the
  // approval dialog below stays answerable over it.
  React.useEffect(() => {
    if (enabled && awaitingConsent) setOpen(true);
  }, [enabled, awaitingConsent, setOpen]);
  // The error shows only in the panel, so a failed turn asks for the column
  // the way the user's own request does: a page holding it gives it back.
  const failure = session.error;
  React.useEffect(() => {
    if (enabled && failure) openPanel();
  }, [enabled, failure, openPanel]);

  return (
    <div className="relative flex min-h-0 w-full min-w-0 flex-1">
      {/* Maximized, the panel takes the column; the page stays mounted. */}
      <div
        className={cn("flex min-h-0 min-w-0 flex-1 flex-col", pageHidden && "hidden")}
        inert={pageHidden}
      >
        {children}
        {enabled && !showAssistant && !columnClaimed && closedBar !== "hidden" ? (
          <AssistantClosedBar mode={closedBar} position={position} />
        ) : null}
      </div>
      {/* Mounted on the full assistant page too: details still open here. */}
      <RightPanel
        open={showPanel}
        assistantTab={assistantTab}
        assistantAvailable={enabled}
        sheet={sheet}
      />
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
