import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from "@/components/ai-elements";
import type { AiChatToolCall } from "@/daemon/stream";

interface ChatToolCallProps {
  toolCall: AiChatToolCall;
}

export function ChatToolCall({ toolCall }: ChatToolCallProps) {
  const hasArguments = Object.keys(toolCall.arguments).length > 0;
  const hasResult = toolCall.result !== undefined && toolCall.result !== null;
  return (
    <Tool
      defaultOpen={
        toolCall.status === "awaiting_consent" ||
        toolCall.status === "running" ||
        toolCall.status === "error"
      }
      className={
        toolCall.status === "error"
          ? "border-destructive/35 bg-destructive/5"
          : undefined
      }
    >
      <ToolHeader name={toolCall.name} state={toolCall.status} />
      <ToolContent>
        {hasArguments ? <ToolInput input={toolCall.arguments} /> : null}
        {hasResult ? <ToolOutput output={toolCall.result} /> : null}
        {toolCall.reason ? (
          <ToolOutput output={toolCall.reason} label="Reason" />
        ) : null}
      </ToolContent>
    </Tool>
  );
}
