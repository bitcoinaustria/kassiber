/**
 * Auto-scrolling assistant conversation container.
 *
 * Tracks whether the user has scrolled up away from the latest message —
 * if they have, we don't yank them back to the bottom on every delta;
 * once they scroll back to within ~32px of the bottom we resume sticking.
 */

import * as React from "react";

import { ChatMessage } from "./ChatMessage";
import { Conversation, ConversationContent } from "@/components/ai-elements";
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
  const stickyRef = React.useRef(true);

  const handleScroll = React.useCallback(() => {
    const node = containerRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    stickyRef.current = distance <= STICKY_THRESHOLD_PX;
  }, []);

  React.useEffect(() => {
    const node = containerRef.current;
    if (!node) return;
    if (!scrollable) return;
    if (!stickyRef.current) return;
    node.scrollTo({ top: node.scrollHeight, behavior: "auto" });
  }, [messages, scrollable]);

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
    </Conversation>
  );
}
