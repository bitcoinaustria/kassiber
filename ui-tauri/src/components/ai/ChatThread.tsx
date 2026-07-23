/**
 * Auto-scrolling assistant conversation container.
 *
 * Streaming token deltas within the trailing message respect the user's
 * scroll position — if they've scrolled up by more than ~32px we don't
 * yank them back. A *new* message (count change) always snaps to the
 * bottom: the next turn is what the user expects to see, and the
 * scroll-to-latest button covers the rare case where they didn't.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";

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

const STICKY_THRESHOLD_PX = 32;

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
  const messageCountRef = React.useRef(messages.length);
  const stickyRef = React.useRef(true);
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [isAtTop, setIsAtTop] = React.useState(true);

  const scrollToBottom = React.useCallback(
    (behavior: ScrollBehavior = "auto") => {
      const node = containerRef.current;
      if (!node || !scrollable) return;
      stickyRef.current = true;
      setIsAtBottom(true);
      node.scrollTo({ top: node.scrollHeight, behavior });
    },
    [scrollable],
  );

  const scrollToTop = React.useCallback(
    (behavior: ScrollBehavior = "smooth") => {
      const node = containerRef.current;
      if (!node || !scrollable) return;
      // Leaving the bottom: stop auto-sticking so streaming doesn't yank back.
      stickyRef.current = false;
      node.scrollTo({ top: 0, behavior });
    },
    [scrollable],
  );

  const handleScroll = React.useCallback(() => {
    const node = containerRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    const atBottom = distance <= STICKY_THRESHOLD_PX;
    stickyRef.current = atBottom;
    setIsAtBottom(atBottom);
    setIsAtTop(node.scrollTop <= STICKY_THRESHOLD_PX);
  }, []);

  React.useLayoutEffect(() => {
    const node = containerRef.current;
    if (!node) return;
    if (!scrollable) return;
    if (messageCountRef.current !== messages.length) {
      stickyRef.current = true;
      messageCountRef.current = messages.length;
    }
    if (!stickyRef.current) return;
    const frame = window.requestAnimationFrame(() => scrollToBottom());
    return () => window.cancelAnimationFrame(frame);
  }, [messages, scrollToBottom, scrollable]);

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
      </ConversationContent>
      {scrollable && !isAtTop ? (
        <button
          type="button"
          className="absolute top-3 left-1/2 z-10 flex size-8 -translate-x-1/2 items-center justify-center rounded-full border border-border/60 bg-card text-muted-foreground shadow-sm outline-none transition-colors hover:border-border hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-safe:animate-in motion-safe:fade-in motion-safe:zoom-in-95 motion-safe:duration-200"
          onClick={() => scrollToTop("smooth")}
          aria-label={t("thread.scrollToTop")}
          title={t("thread.scrollToTop")}
        >
          <ArrowUp className="h-4 w-4" aria-hidden="true" />
        </button>
      ) : null}
      {scrollable && !isAtBottom ? (
        <button
          type="button"
          className="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-1.5 rounded-full border border-border/60 bg-card px-3 py-1 text-xs text-muted-foreground shadow-sm outline-none transition-colors hover:border-border hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-safe:animate-in motion-safe:fade-in motion-safe:zoom-in-95 motion-safe:duration-200"
          onClick={() => scrollToBottom("smooth")}
          aria-label={t("thread.scrollToLatest")}
          title={t("thread.scrollToLatest")}
        >
          <ArrowDown className="h-3.5 w-3.5" aria-hidden="true" />
          <span>{t("thread.scrollToLatestShort")}</span>
        </button>
      ) : null}
    </Conversation>
  );
}
