/**
 * One message in the assistant thread.
 *
 * Distinguishes user vs assistant visually but keeps things minimal —
 * matching the cosmetic style of the existing Ai02 input bar. Streams the
 * assistant content live with a typing indicator until any visible-answer
 * text lands.
 */

import { AlertTriangle, Sparkles, User } from "lucide-react";

import { ChatLoader } from "./ChatLoader";
import { ChatMarkdown } from "./ChatMarkdown";
import { ChatReasoning } from "./ChatReasoning";
import { ChatToolCall } from "./ChatToolCall";
import type { AiChatMessage } from "@/daemon/stream";
import { cn } from "@/lib/utils";

interface ChatMessageProps {
  message: AiChatMessage;
}

export function ChatMessage({ message }: ChatMessageProps) {
  if (message.role === "user") {
    return (
      <div className="flex justify-end">
        <div className="flex max-w-[85%] items-start gap-2">
          <div className="rounded-2xl rounded-tr-sm bg-primary px-3 py-2 text-sm text-primary-foreground shadow-sm">
            <p className="whitespace-pre-wrap break-words">{message.content}</p>
          </div>
          <div className="mt-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary">
            <User className="h-3.5 w-3.5" aria-hidden="true" />
          </div>
        </div>
      </div>
    );
  }

  const isStreaming = message.status === "streaming" || message.status === "pending";
  const hasAnswer = Boolean(message.content);
  const hasToolCalls = Boolean(message.toolCalls?.length);
  const showLoader = !hasAnswer && (message.status === "pending" || message.status === "streaming");

  return (
    <div className="flex justify-start">
      <div className="flex max-w-[90%] items-start gap-2">
        <div
          className={cn(
            "mt-1 flex h-6 w-6 shrink-0 items-center justify-center rounded-full",
            message.status === "error"
              ? "bg-destructive/15 text-destructive"
              : "bg-muted text-muted-foreground",
          )}
        >
          {message.status === "error" ? (
            <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" aria-hidden="true" />
          )}
        </div>
        <div className="flex-1 rounded-2xl rounded-tl-sm border border-border/60 bg-background px-3 py-2 text-sm shadow-sm">
          {message.thinking ? (
            <ChatReasoning
              thinking={message.thinking}
              isStreaming={isStreaming}
              hasAnswer={hasAnswer}
            />
          ) : null}
          {hasToolCalls ? (
            <div className={message.thinking ? "mt-2" : undefined}>
              {message.toolCalls?.map((toolCall) => (
                <ChatToolCall key={toolCall.callId} toolCall={toolCall} />
              ))}
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
    </div>
  );
}
