/**
 * One message in the assistant thread.
 *
 * Distinguishes user vs assistant visually but keeps things minimal —
 * matching the cosmetic style of the existing Ai02 input bar. Streams the
 * assistant content live with a typing indicator until any visible-answer
 * text lands.
 */

import { Wrench } from "lucide-react";

import { ChatLoader } from "./ChatLoader";
import { ChatMarkdown } from "./ChatMarkdown";
import { ChatReasoning } from "./ChatReasoning";
import { ChatToolCall } from "./ChatToolCall";
import {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
} from "@/components/ai-elements";
import type { AiChatMessage } from "@/daemon/stream";
import { cn } from "@/lib/utils";

interface ChatMessageProps {
  message: AiChatMessage;
}

export function ChatMessage({ message }: ChatMessageProps) {
  if (message.role === "user") {
    return (
      <div className="flex w-full justify-end">
        <div className="max-w-[82%] rounded-2xl rounded-tr-sm bg-primary px-3 py-2 text-sm text-primary-foreground shadow-sm sm:max-w-[72%]">
          <p className="whitespace-pre-wrap break-words">{message.content}</p>
        </div>
      </div>
    );
  }

  const isStreaming =
    message.status === "streaming" || message.status === "pending";
  const hasAnswer = Boolean(message.content);
  const hasToolCalls = Boolean(message.toolCalls?.length);
  const showLoader =
    !hasAnswer &&
    (message.status === "pending" || message.status === "streaming");

  return (
    <div className="flex w-full justify-start">
      <div className="w-full min-w-0 px-1 py-1 text-sm">
        {message.thinking ? (
          <ChatReasoning
            thinking={message.thinking}
            isStreaming={isStreaming}
            hasAnswer={hasAnswer}
          />
        ) : null}
        {hasToolCalls ? (
          <div
            className={cn(
              message.thinking ? "mt-3" : undefined,
              "mb-4 w-full min-w-0",
            )}
          >
            <ChainOfThought defaultOpen>
              <ChainOfThoughtHeader icon={Wrench}>
                Tool usage
              </ChainOfThoughtHeader>
              <ChainOfThoughtContent>
                <div className="mt-2 space-y-3 border-l border-border/70 py-1 pl-4">
                  {message.toolCalls?.map((toolCall) => (
                    <ChatToolCall key={toolCall.callId} toolCall={toolCall} />
                  ))}
                </div>
              </ChainOfThoughtContent>
            </ChainOfThought>
          </div>
        ) : null}
        {hasAnswer ? <ChatMarkdown content={message.content} /> : null}
        {showLoader ? <ChatLoader className="mt-1" /> : null}
        {message.status === "error" ? (
          <p className="text-sm text-destructive">
            {message.errorMessage ?? "Chat failed"}
            {message.errorCode ? (
              <span className="ml-2 rounded-md bg-destructive/10 px-1.5 py-0.5 font-mono text-[10px] uppercase">
                {message.errorCode}
              </span>
            ) : null}
          </p>
        ) : null}
        {message.status === "cancelled" ? (
          <p className="mt-1 text-xs italic text-muted-foreground">
            Stopped by user.
          </p>
        ) : null}
      </div>
    </div>
  );
}
