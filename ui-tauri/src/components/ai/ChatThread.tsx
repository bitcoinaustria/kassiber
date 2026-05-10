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
import { ArrowDown } from "lucide-react";

import { ChatMessage } from "./ChatMessage";
import { Conversation, ConversationContent } from "@/components/ai-elements";
import { Button } from "@/components/ui/button";
import type { AiChatMessage } from "@/daemon/stream";

interface ChatThreadProps {
  messages: AiChatMessage[];
  className?: string;
  scrollable?: boolean;
}

const STICKY_THRESHOLD_PX = 32;

export function ChatThread({
  messages,
  className,
  scrollable = true,
}: ChatThreadProps) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const messageCountRef = React.useRef(messages.length);
  const stickyRef = React.useRef(true);
  const [isAtBottom, setIsAtBottom] = React.useState(true);

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

  const handleScroll = React.useCallback(() => {
    const node = containerRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    const atBottom = distance <= STICKY_THRESHOLD_PX;
    stickyRef.current = atBottom;
    setIsAtBottom(atBottom);
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
        onScroll={handleScroll}
        scrollable={scrollable}
      >
        {messages.map((message) => (
          <ChatMessage key={message.id} message={message} />
        ))}
      </ConversationContent>
      {scrollable && !isAtBottom ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-lg"
          className="absolute bottom-3 left-1/2 z-10 -translate-x-1/2 rounded-full border border-border/70 bg-background/90 text-foreground shadow-[0_12px_30px_rgba(15,23,42,0.18)] backdrop-blur hover:bg-muted dark:bg-zinc-950/80"
          onClick={() => scrollToBottom("smooth")}
          aria-label="Scroll to latest message"
          title="Scroll to latest message"
        >
          <ArrowDown className="h-5 w-5" aria-hidden="true" />
        </Button>
      ) : null}
    </Conversation>
  );
}
