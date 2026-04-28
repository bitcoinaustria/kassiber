/**
 * `useAiChatStream` — drives one streaming `ai.chat` call against the
 * active daemon transport.
 *
 * Returns a small imperative API plus reducer state. Each delta from the
 * daemon's `ai.chat.delta` records is split into `content` / `thinking`
 * channels via `ThinkParser`, so the reasoning pane and answer pane can
 * render independently as tokens arrive.
 *
 * Stop sends a cooperative `ai.chat.cancel` daemon request for the active
 * stream request_id, while the UI still suppresses late records locally.
 */

import * as React from "react";

import type {
  DaemonEnvelope,
  DaemonStreamRecord,
} from "./transport";
import { getTransport, makeDaemonRequestId } from "./transport";
import { ThinkParser } from "@/lib/thinkParser";
import { useUiStore } from "@/store/ui";

export interface AiChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  toolCalls?: AiChatToolCall[];
  status: "pending" | "streaming" | "done" | "error" | "cancelled";
  errorCode?: string;
  errorMessage?: string;
  finishReason?: string | null;
  provider?: string;
  model?: string;
}

export type AiToolCallStatus = "pending" | "running" | "done" | "denied" | "error";

export interface AiChatToolCall {
  callId: string;
  name: string;
  arguments: Record<string, unknown>;
  kindClass: "read_only" | "mutating" | "unknown";
  needsConsent: boolean;
  status: AiToolCallStatus;
  result?: unknown;
  reason?: string;
}

export interface AiChatRequest {
  provider?: string;
  model: string;
  messages: { role: AiChatMessage["role"] | "tool"; content: string }[];
  options?: Record<string, unknown>;
  toolsEnabled?: boolean;
  toolLoopMaxIterations?: number;
  systemPromptKind?: "kassiber" | "raw" | null;
  systemPrompt?: string;
}

export interface AiChatDeltaShape {
  delta?: {
    role?: AiChatMessage["role"] | "tool";
    content?: string;
    /**
     * Structured reasoning channel emitted by OpenAI o1/o3-style models
     * and by Ollama's OpenAI-compat shim for Qwen3 / Gemma reasoning
     * builds. Distinct from inline `<think>...</think>` tags inside
     * `content`, which `ThinkParser` handles. Both flow into the same
     * UI thinking pane.
     */
    reasoning?: string;
  };
}

interface AiChatTerminalShape {
  provider?: string;
  model?: string;
  finish_reason?: string | null;
}

export interface AiChatToolCallShape {
  call_id: string;
  name: string;
  arguments?: Record<string, unknown>;
  kind_class?: "read_only" | "mutating" | "unknown";
  needs_consent?: boolean;
}

export interface AiChatToolResultShape {
  call_id: string;
  ok?: boolean;
  reason?: string;
  envelope?: unknown;
  message?: string;
}

type AiChatStreamRecordData =
  | AiChatDeltaShape
  | AiChatToolCallShape
  | AiChatToolResultShape;

export interface UseAiChatStreamResult {
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  send: (request: AiChatRequest, userMessageContent: string) => Promise<void>;
  abort: () => void;
  reset: () => void;
}

function makeId(): string {
  if (
    typeof globalThis.crypto !== "undefined" &&
    typeof globalThis.crypto.randomUUID === "function"
  ) {
    return globalThis.crypto.randomUUID();
  }
  return `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export function applyAiChatDeltaToMessage(
  current: AiChatMessage,
  record: DaemonStreamRecord<AiChatDeltaShape>,
  parser: ThinkParser,
  aborted: boolean,
): AiChatMessage {
  if (aborted || record.kind !== "ai.chat.delta") return current;
  const content = record.data?.delta?.content;
  const reasoning = record.data?.delta?.reasoning;
  if (!content && !reasoning) return current;

  // `content` may carry inline `<think>...</think>` chunks (DeepSeek-R1,
  // older Qwen builds). `reasoning` is the structured channel
  // (OpenAI o1/o3, Ollama's OpenAI-compat for Qwen3 / Gemma reasoning
  // builds). Merge both into the thinking pane; visible answer comes
  // from the parsed-content channel only.
  let visibleAdd = "";
  let thinkingAdd = "";
  if (content) {
    const split = parser.feed(content);
    visibleAdd = split.content;
    thinkingAdd = split.thinking;
  }
  if (reasoning) {
    thinkingAdd += reasoning;
  }
  if (!visibleAdd && !thinkingAdd) return current;

  return {
    ...current,
    status: "streaming",
    content: current.content + visibleAdd,
    thinking: (current.thinking ?? "") + thinkingAdd,
  };
}

function toolResultStatus(data: AiChatToolResultShape): AiToolCallStatus {
  if (data.ok) return "done";
  if (data.reason === "tool_not_allowed" || data.reason === "user_denied") {
    return "denied";
  }
  return "error";
}

export function applyAiChatStreamRecordToMessage(
  current: AiChatMessage,
  record: DaemonStreamRecord<AiChatStreamRecordData>,
  parser: ThinkParser,
  aborted: boolean,
): AiChatMessage {
  if (aborted) return current;
  if (record.kind === "ai.chat.delta") {
    return applyAiChatDeltaToMessage(
      current,
      record as DaemonStreamRecord<AiChatDeltaShape>,
      parser,
      false,
    );
  }
  if (record.kind === "ai.chat.tool_call") {
    const data = record.data as AiChatToolCallShape | undefined;
    if (!data?.call_id || !data.name) return current;
    const nextToolCall: AiChatToolCall = {
      callId: data.call_id,
      name: data.name,
      arguments: data.arguments ?? {},
      kindClass: data.kind_class ?? "unknown",
      needsConsent: Boolean(data.needs_consent),
      status: data.needs_consent ? "pending" : "running",
    };
    const existing = current.toolCalls ?? [];
    const found = existing.some((toolCall) => toolCall.callId === data.call_id);
    return {
      ...current,
      status: "streaming",
      toolCalls: found
        ? existing.map((toolCall) =>
            toolCall.callId === data.call_id
              ? { ...toolCall, ...nextToolCall }
              : toolCall,
          )
        : [...existing, nextToolCall],
    };
  }
  if (record.kind === "ai.chat.tool_result") {
    const data = record.data as AiChatToolResultShape | undefined;
    if (!data?.call_id) return current;
    const existing = current.toolCalls ?? [];
    const status = toolResultStatus(data);
    const found = existing.some((toolCall) => toolCall.callId === data.call_id);
    const applyResult = (toolCall: AiChatToolCall): AiChatToolCall => ({
      ...toolCall,
      status,
      result: data.envelope ?? data.message ?? null,
      reason: data.reason,
    });
    return {
      ...current,
      status: "streaming",
      toolCalls: found
        ? existing.map((toolCall) =>
            toolCall.callId === data.call_id ? applyResult(toolCall) : toolCall,
          )
        : [
            ...existing,
            applyResult({
              callId: data.call_id,
              name: "Tool",
              arguments: {},
              kindClass: "unknown",
              needsConsent: false,
              status,
            }),
          ],
    };
  }
  return current;
}

export function terminalAiChatStatus(
  finishReason: string | null | undefined,
  aborted: boolean,
): AiChatMessage["status"] {
  return aborted || finishReason === "cancelled" ? "cancelled" : "done";
}

/** React hook driving one assistant thread. */
export function useAiChatStream(): UseAiChatStreamResult {
  const dataMode = useUiStore((state) => state.dataMode);
  const [messages, setMessages] = React.useState<AiChatMessage[]>([]);
  const [isStreaming, setIsStreaming] = React.useState(false);
  const [error, setError] = React.useState<UseAiChatStreamResult["error"]>(
    null,
  );
  const abortRef = React.useRef<AbortController | null>(null);
  const parserRef = React.useRef<ThinkParser | null>(null);
  const assistantIdRef = React.useRef<string | null>(null);
  const requestIdRef = React.useRef<string | null>(null);

  const updateAssistant = React.useCallback(
    (
      mutator: (current: AiChatMessage) => AiChatMessage,
    ) => {
      const id = assistantIdRef.current;
      if (!id) return;
      setMessages((prev) =>
        prev.map((message) => (message.id === id ? mutator(message) : message)),
      );
    },
    [],
  );

  const onRecord = React.useCallback(
    (record: DaemonStreamRecord<AiChatStreamRecordData>) => {
      const parser = parserRef.current ?? new ThinkParser();
      parserRef.current = parser;
      updateAssistant((current) =>
        applyAiChatStreamRecordToMessage(
          current,
          record,
          parser,
          abortRef.current?.signal.aborted ?? false,
        ),
      );
    },
    [updateAssistant],
  );

  const send = React.useCallback(
    async (request: AiChatRequest, userMessageContent: string) => {
      if (isStreaming) return;
      setError(null);
      setIsStreaming(true);
      const userId = makeId();
      const assistantId = makeId();
      assistantIdRef.current = assistantId;
      parserRef.current = new ThinkParser();
      setMessages((prev) => [
        ...prev,
        {
          id: userId,
          role: "user",
          content: userMessageContent,
          status: "done",
        },
        {
          id: assistantId,
          role: "assistant",
          content: "",
          thinking: "",
          status: "pending",
        },
      ]);

      const controller = new AbortController();
      abortRef.current = controller;
      const requestId = makeDaemonRequestId();
      requestIdRef.current = requestId;
      try {
        const transport = getTransport(dataMode);
        const envelope = (await transport.stream<
          AiChatTerminalShape,
          AiChatStreamRecordData
        >(
          {
            kind: "ai.chat",
            request_id: requestId,
            args: {
              provider: request.provider,
              model: request.model,
              messages: request.messages,
              options: request.options,
              tools_enabled: request.toolsEnabled,
              tool_loop_max_iterations: request.toolLoopMaxIterations,
              system_prompt_kind: request.systemPromptKind,
              system_prompt: request.systemPrompt,
            },
          },
          { onRecord, signal: controller.signal },
        )) as DaemonEnvelope<AiChatTerminalShape>;

        if (envelope.kind === "error" || envelope.error) {
          const code = envelope.error?.code ?? "unknown_error";
          const message = envelope.error?.message ?? "AI chat failed";
          setError({ code, message });
          updateAssistant((current) => ({
            ...current,
            status: "error",
            errorCode: code,
            errorMessage: message,
          }));
          return;
        }

        const finishReason = envelope.data?.finish_reason ?? null;
        // Flush any pending tag-prefix bytes still in the parser.
        const parser = parserRef.current;
        if (parser && !controller.signal.aborted) {
          const tail = parser.flush();
          if (tail.content || tail.thinking) {
            updateAssistant((current) => ({
              ...current,
              content: current.content + tail.content,
              thinking: (current.thinking ?? "") + tail.thinking,
            }));
          }
        }

        updateAssistant((current) => ({
          ...current,
          status: terminalAiChatStatus(finishReason, controller.signal.aborted),
          finishReason,
          provider: envelope.data?.provider ?? request.provider,
          model: envelope.data?.model ?? request.model,
        }));
      } catch (caught) {
        const message =
          caught instanceof Error ? caught.message : String(caught);
        setError({ code: "stream_failed", message });
        updateAssistant((current) => ({
          ...current,
          status: "error",
          errorCode: "stream_failed",
          errorMessage: message,
        }));
      } finally {
        setIsStreaming(false);
        abortRef.current = null;
        parserRef.current = null;
        assistantIdRef.current = null;
        requestIdRef.current = null;
      }
    },
    [dataMode, isStreaming, onRecord, updateAssistant],
  );

  const abort = React.useCallback(() => {
    const requestId = requestIdRef.current;
    abortRef.current?.abort();
    if (requestId) {
      void getTransport(dataMode).invoke({
        kind: "ai.chat.cancel",
        request_id: makeDaemonRequestId(),
        args: { target_request_id: requestId },
      }).catch(() => undefined);
    }
    updateAssistant((current) => ({
      ...current,
      status: "cancelled",
    }));
  }, [dataMode, updateAssistant]);

  const reset = React.useCallback(() => {
    setMessages([]);
    setError(null);
  }, []);

  return { messages, isStreaming, error, send, abort, reset };
}
