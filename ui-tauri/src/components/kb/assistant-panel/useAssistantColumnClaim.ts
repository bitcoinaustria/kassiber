import * as React from "react";

import { useUiStore } from "@/store/ui";

/**
 * Lets a page hold the whole column (the expanded Transactions table) while
 * `claimed`: the assistant panel and its closed bar step out of the way, and
 * a pending approval stays on screen as a dialog. When the user asks for the
 * assistant (shortcut, title-bar toggle, Ask about this, a handoff) or a turn
 * fails, the page gives the column back through `release`.
 */
export function useAssistantColumnClaim(claimed: boolean, release: () => void) {
  const setClaimed = useUiStore((state) => state.setAssistantColumnClaimed);
  const revealRequest = useUiStore((state) => state.assistantPanelRevealRequest);
  const seenRevealRef = React.useRef(revealRequest);
  const releaseRef = React.useRef(release);
  React.useEffect(() => {
    releaseRef.current = release;
  }, [release]);

  // A layout effect, so the panel never flashes beside the claimed column.
  React.useLayoutEffect(() => {
    if (!claimed) return undefined;
    setClaimed(true);
    return () => setClaimed(false);
  }, [claimed, setClaimed]);

  React.useEffect(() => {
    if (revealRequest === seenRevealRef.current) return;
    seenRevealRef.current = revealRequest;
    if (claimed) releaseRef.current();
  }, [claimed, revealRequest]);
}
