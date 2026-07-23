/**
 * Auto-scrolling assistant conversation container.
 *
 * Scroll behaviour is ported from T3Code. When a new user turn is sent, that
 * message becomes the *anchor*: the list reserves blank space at the end so the
 * message can pin ~16px from the TOP of the viewport, and the assistant reply
 * streams in below it (rather than the message sitting at the bottom and being
 * shoved upward). The reserved space shrinks as the reply grows; once the turn
 * outgrows the viewport it follows the streaming tail. A manual wheel/touch
 * gesture opts out into free-scrolling and surfaces the "scroll to latest" pill.
 * On mount an actively-streaming turn anchors; a resumed (settled) thread just
 * opens pinned to the newest message.
 *
 * T3Code drives this through LegendList's `anchoredEndSpace`; here the same
 * behaviour is reproduced on a plain scroll container with an imperative
 * bottom spacer (kept out of React state so streaming deltas don't thrash).
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronUp } from "lucide-react";

import { ChatMessage } from "./ChatMessage";
import { Conversation, ConversationContent } from "@/components/ai-elements";
import type { AiChatMessage } from "@/daemon/stream";

interface ChatThreadProps {
  messages: AiChatMessage[];
  className?: string;
  /** Centers message content while the scroll container spans full width. */
  contentClassName?: string;
  scrollable?: boolean;
  /**
   * When provided, assistant messages expose a "Branch in new chat" action.
   */
  onBranchMessage?: (messageId: string) => void;
  /**
   * When provided, user messages expose an inline "Edit" action. Confirming
   * calls this with the edited text, which regenerates the conversation from
   * that prompt onward. Only wired on the full assistant page.
   */
  onEditMessage?: (messageId: string, nextContent?: string) => void;
}

// Distance from an edge (px) still counted as "at" that edge.
const EDGE_THRESHOLD_PX = 32;
// Where the anchored user turn sits below the top of the viewport (T3Code: 16).
const ANCHOR_OFFSET_PX = 16;

type ScrollMode = "following" | "anchoring" | "free";

function findLastUserId(messages: AiChatMessage[]): string | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") return messages[index].id;
  }
  return null;
}

function escapeId(id: string): string {
  return typeof CSS !== "undefined" && typeof CSS.escape === "function"
    ? CSS.escape(id)
    : id.replace(/"/g, '\\"');
}

export function ChatThread({
  messages,
  className,
  contentClassName,
  scrollable = true,
  onBranchMessage,
  onEditMessage,
}: ChatThreadProps) {
  const { t } = useTranslation("assistant");
  const containerRef = React.useRef<HTMLDivElement>(null);
  const spacerRef = React.useRef<HTMLDivElement>(null);
  const frameRef = React.useRef<number | null>(null);

  const modeRef = React.useRef<ScrollMode>("following");
  const anchorIdRef = React.useRef<string | null>(null);
  const lastUserIdRef = React.useRef<string | null>(null);
  const messageCountRef = React.useRef(0);
  const initializedRef = React.useRef(false);

  const [showJumpPill, setShowJumpPill] = React.useState(false);
  const [showScrollTop, setShowScrollTop] = React.useState(false);

  const setSpacer = React.useCallback((px: number) => {
    const spacer = spacerRef.current;
    if (spacer) spacer.style.height = `${Math.max(0, Math.round(px))}px`;
  }, []);

  // Position the list for the current mode. Runs in rAF so freshly-rendered
  // (or still-streaming) rows have settled before we measure them.
  const applyScroll = React.useCallback(() => {
    if (frameRef.current !== null) window.cancelAnimationFrame(frameRef.current);
    frameRef.current = window.requestAnimationFrame(() => {
      frameRef.current = null;
      const node = containerRef.current;
      const spacer = spacerRef.current;
      if (!node || !spacer) return;

      if (modeRef.current === "anchoring" && anchorIdRef.current) {
        const anchorEl = node.querySelector<HTMLElement>(
          `[data-message-id="${escapeId(anchorIdRef.current)}"]`,
        );
        if (!anchorEl) {
          setSpacer(0);
          node.scrollTo({ top: node.scrollHeight });
          return;
        }
        const containerTop = node.getBoundingClientRect().top;
        const anchorTop =
          anchorEl.getBoundingClientRect().top - containerTop + node.scrollTop;
        const realContentBottom = node.scrollHeight - spacer.offsetHeight;
        const usableViewport = node.clientHeight - ANCHOR_OFFSET_PX;
        const turnHeight = realContentBottom - anchorTop;
        if (turnHeight >= usableViewport) {
          // Turn now fills the viewport — hand off to following the tail.
          modeRef.current = "following";
          anchorIdRef.current = null;
          setSpacer(0);
          node.scrollTo({ top: node.scrollHeight });
          return;
        }
        // Reserve exactly enough blank space to lift the anchor to the top.
        setSpacer(usableViewport - turnHeight);
        node.scrollTo({ top: node.scrollHeight - node.clientHeight });
        return;
      }

      if (modeRef.current === "following") {
        setSpacer(0);
        node.scrollTo({ top: node.scrollHeight });
      }
    });
  }, [setSpacer]);

  const scrollToLatest = React.useCallback(
    (behavior: ScrollBehavior = "smooth") => {
      const node = containerRef.current;
      if (!node || !scrollable) return;
      modeRef.current = "following";
      anchorIdRef.current = null;
      setSpacer(0);
      setShowJumpPill(false);
      node.scrollTo({ top: node.scrollHeight, behavior });
    },
    [scrollable, setSpacer],
  );

  const scrollToTop = React.useCallback(
    (behavior: ScrollBehavior = "smooth") => {
      const node = containerRef.current;
      if (!node || !scrollable) return;
      // Leaving the live edge on purpose: stop auto-follow so streaming deltas
      // don't yank the view back down.
      modeRef.current = "free";
      anchorIdRef.current = null;
      setSpacer(0);
      node.scrollTo({ top: 0, behavior });
    },
    [scrollable, setSpacer],
  );

  const handleScroll = React.useCallback(() => {
    const node = containerRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    const atBottom = distance <= EDGE_THRESHOLD_PX;
    const atTop = node.scrollTop <= EDGE_THRESHOLD_PX;
    // Scrolling back to the live edge re-engages following.
    if (atBottom && modeRef.current === "free") modeRef.current = "following";
    const free = modeRef.current === "free";
    // The scroll affordances only appear while the user is browsing away from
    // the live edge — never mid-anchor, where we hold the view deliberately.
    setShowJumpPill(free && !atBottom);
    setShowScrollTop(free && !atTop);
  }, []);

  // A real wheel/touch gesture (never our own programmatic scrollTo) drops out
  // of auto-follow, mirroring T3Code's manual-navigation opt-out.
  React.useEffect(() => {
    const node = containerRef.current;
    if (!node || !scrollable) return;
    const onManualNavigation = () => {
      if (modeRef.current === "free") return;
      modeRef.current = "free";
      anchorIdRef.current = null;
      setSpacer(0);
    };
    node.addEventListener("wheel", onManualNavigation, { passive: true });
    node.addEventListener("touchmove", onManualNavigation, { passive: true });
    return () => {
      node.removeEventListener("wheel", onManualNavigation);
      node.removeEventListener("touchmove", onManualNavigation);
    };
  }, [scrollable, setSpacer]);

  React.useLayoutEffect(() => {
    const node = containerRef.current;
    if (!node || !scrollable) {
      messageCountRef.current = messages.length;
      return;
    }

    if (messages.length === 0) {
      initializedRef.current = false;
      lastUserIdRef.current = null;
      anchorIdRef.current = null;
      modeRef.current = "following";
      messageCountRef.current = 0;
      return;
    }

    const lastUserId = findLastUserId(messages);

    if (!initializedRef.current) {
      // First paint: anchor only if we're opening onto a live, streaming turn;
      // a resumed thread just pins to the newest message.
      initializedRef.current = true;
      lastUserIdRef.current = lastUserId;
      messageCountRef.current = messages.length;
      const last = messages[messages.length - 1];
      const streamingTurn =
        last?.role === "assistant" &&
        (last.status === "streaming" || last.status === "pending");
      if (streamingTurn && lastUserId) {
        modeRef.current = "anchoring";
        anchorIdRef.current = lastUserId;
      } else {
        modeRef.current = "following";
      }
    } else {
      const grew = messages.length > messageCountRef.current;
      messageCountRef.current = messages.length;
      if (grew && lastUserId && lastUserId !== lastUserIdRef.current) {
        // A fresh user turn re-engages anchoring even from free-scrolling.
        lastUserIdRef.current = lastUserId;
        modeRef.current = "anchoring";
        anchorIdRef.current = lastUserId;
      } else {
        lastUserIdRef.current = lastUserId;
      }
    }

    if (modeRef.current === "free") return;
    applyScroll();
    return () => {
      if (frameRef.current !== null) {
        window.cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [messages, scrollable, applyScroll]);

  if (messages.length === 0) return null;

  return (
    <Conversation className={className}>
      <ConversationContent
        ref={containerRef}
        className="conversation-scrollbar"
        onScroll={handleScroll}
        scrollable={scrollable}
        contentClassName={contentClassName}
      >
        {messages.map((message) => (
          <ChatMessage
            key={message.id}
            message={message}
            onBranch={onBranchMessage}
            onEdit={onEditMessage}
          />
        ))}
        {/* Reserved end-space that lets a fresh turn lift to the top. */}
        <div ref={spacerRef} aria-hidden="true" style={{ height: 0 }} />
      </ConversationContent>
      {scrollable && showScrollTop ? (
        <button
          type="button"
          className="absolute top-3 left-1/2 z-10 flex size-8 -translate-x-1/2 items-center justify-center rounded-full border border-border/60 bg-card text-muted-foreground shadow-sm outline-none transition-colors hover:border-border hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-safe:animate-in motion-safe:fade-in motion-safe:zoom-in-95 motion-safe:duration-200"
          onClick={() => scrollToTop("smooth")}
          aria-label={t("thread.scrollToTop")}
          title={t("thread.scrollToTop")}
        >
          <ChevronUp className="h-4 w-4" aria-hidden="true" />
        </button>
      ) : null}
      {scrollable && showJumpPill ? (
        <button
          type="button"
          className="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-border/60 bg-card px-3 py-1 text-xs text-muted-foreground shadow-sm outline-none transition-colors hover:border-border hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-safe:animate-in motion-safe:fade-in motion-safe:zoom-in-95 motion-safe:duration-200"
          onClick={() => scrollToLatest("smooth")}
          aria-label={t("thread.scrollToLatest")}
          title={t("thread.scrollToLatest")}
        >
          <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
          <span>{t("thread.scrollToLatestShort")}</span>
        </button>
      ) : null}
    </Conversation>
  );
}
