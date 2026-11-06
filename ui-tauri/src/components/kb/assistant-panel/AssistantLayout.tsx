/**
 * Page column plus the right panel beside it: the closed bar sits under the
 * page while the assistant is out of view, and the assistant, the panel's
 * surfaces and open transactions' details are tabs of the panel.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { Shield } from "lucide-react";

import { useAssistantSession } from "@/components/ai/assistantSession";
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
  const openPanel = useUiStore((s) => s.openAssistantPanel);
  const toggle = useUiStore((s) => s.toggleAssistantPanel);
  const setActiveTab = useRightPanelStore((s) => s.setActiveTab);
  const activeTab = useRightPanelStore((s) => s.activeTab);
  const closedBar = useUiStore((s) => s.assistantClosedBar);
  const position = useUiStore((s) => s.assistantDockPosition);
  // A page holding the whole column (useAssistantColumnClaim) keeps both the
  // panel and the bar out until the user asks for the assistant, a turn
  // fails, or a tool waits for approval.
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

  // An approval is answered in its row in the conversation, and a failed
  // turn shows only in the panel, so each asks for the column once, the way
  // the user's own request does: a page holding it gives it back. Closing the
  // panel again is the user's choice; a waiting approval then keeps the
  // notice below the page.
  const pendingConsents = session.pendingConsents;
  const newestApproval = pendingConsents.at(-1)?.callId ?? null;
  React.useEffect(() => {
    if (enabled && newestApproval) openPanel();
  }, [enabled, newestApproval, openPanel]);
  const failure = session.error;
  React.useEffect(() => {
    if (enabled && failure) openPanel();
  }, [enabled, failure, openPanel]);

  // The row that answers it is in view only while the assistant is the
  // panel's front tab.
  const assistantInView = showAssistant && activeTab === "assistant";
  const approvalNotice =
    enabled && !assistantInView && pendingConsents.length > 0 ? (
      <PendingApprovalNotice
        summary={pendingConsents[0]!.summary}
        count={pendingConsents.length}
        onReview={() => {
          openPanel();
          setActiveTab("assistant");
        }}
      />
    ) : null;

  return (
    <div className="relative flex min-h-0 w-full min-w-0 flex-1">
      {/* Maximized, the panel takes the column; the page stays mounted. */}
      <div
        className={cn("flex min-h-0 min-w-0 flex-1 flex-col", pageHidden && "hidden")}
        inert={pageHidden}
      >
        {children}
        {approvalNotice}
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
      {/* Announced wherever the conversation is shown, the full assistant
          page included, and whether or not focus moves to the row. */}
      <PendingApprovalAnnouncer />
    </div>
  );
}

/**
 * Below the page while an approval waits and the panel is not shown (closed,
 * or the column is a page's): the turn is blocked on the user, so it stays
 * one click from the row that answers it, whatever the closed-bar setting.
 */
function PendingApprovalNotice({
  summary,
  count,
  onReview,
}: {
  summary: string;
  count: number;
  onReview: () => void;
}) {
  const { t } = useTranslation("assistant");
  return (
    <section
      aria-label={t("consent.notice.label")}
      data-pending-approval-notice
      className="flex shrink-0 items-center gap-2 border-t border-amber-500/30 bg-amber-500/5 px-3 py-1.5 text-xs sm:px-4"
    >
      <Shield
        className="size-3.5 shrink-0 text-amber-700 dark:text-amber-400"
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate">
        <span className="font-medium text-amber-800 dark:text-amber-300">
          {t("consent.notice.waiting", { count })}
        </span>{" "}
        <span className="text-muted-foreground">{summary}</span>
      </span>
      <button
        type="button"
        className="kb-hit-floor shrink-0 rounded-sm font-medium text-foreground underline-offset-2 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onReview}
      >
        {t("consent.notice.review")}
      </button>
    </section>
  );
}

/**
 * A polite live region that names each new approval as it arrives, so it is
 * heard even when focus stays in the composer the user is typing in.
 */
function PendingApprovalAnnouncer() {
  const { t } = useTranslation("assistant");
  const { pendingConsents } = useAssistantSession();
  const [message, setMessage] = React.useState("");
  const announcedRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    const fresh = pendingConsents.filter(
      (request) => !announcedRef.current.has(request.callId),
    );
    for (const request of fresh) announcedRef.current.add(request.callId);
    if (fresh.length > 0) {
      setMessage(t("consent.announce", { summary: fresh.at(-1)!.summary }));
    } else if (pendingConsents.length === 0) {
      setMessage("");
    }
  }, [pendingConsents, t]);
  return (
    <div role="status" aria-live="polite" className="sr-only">
      {message}
    </div>
  );
}
