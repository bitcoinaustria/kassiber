import type { AiChatRound, AiChatToolCall } from "@/daemon/stream";

import { evidenceRequest } from "./evidenceRequest";

/**
 * One assistant answer as an ordered work log: reasoning panes, runs of
 * routine tool calls folded into one "Used N tools" row, and the calls the
 * user must see or act on, which are never folded away.
 */
export type WorkLogItem =
  | { kind: "thinking"; id: string; content: string; roundIndex: number }
  | { kind: "tools"; id: string; calls: AiChatToolCall[] }
  | { kind: "call"; id: string; call: AiChatToolCall };

/**
 * A call stays visible on its own when it waits on the user's consent, failed,
 * was denied, or is an evidence request card. Consent cards are updates keyed
 * by `call_id`: the same call is "pending" before the consent prompt arrives
 * and "running" after approval, so it moves into a group only once the user
 * has decided.
 */
export function toolCallNeedsAttention(call: AiChatToolCall): boolean {
  return (
    call.status === "awaiting_consent" ||
    (call.status === "pending" && call.needsConsent) ||
    call.status === "error" ||
    call.status === "denied" ||
    evidenceRequest(call) !== null
  );
}

export function toolCallIsActive(call: AiChatToolCall): boolean {
  return call.status === "running" || call.status === "pending";
}

/**
 * Consecutive routine calls merge across provider rounds as long as no
 * reasoning text separates them, so six one-call rounds read as one row.
 * Group ids come from their first call, which keeps a group's open state
 * stable while later calls stream in.
 */
export function workLogItems(rounds: readonly AiChatRound[]): WorkLogItem[] {
  const items: WorkLogItem[] = [];
  rounds.forEach((round, roundIndex) => {
    if (round.thinking) {
      items.push({
        kind: "thinking",
        id: round.id,
        content: round.thinking,
        roundIndex,
      });
    }
    for (const call of round.toolCalls) {
      if (toolCallNeedsAttention(call)) {
        items.push({ kind: "call", id: call.callId, call });
        continue;
      }
      const last = items.at(-1);
      if (last?.kind === "tools") {
        last.calls.push(call);
      } else {
        items.push({ kind: "tools", id: `tools:${call.callId}`, calls: [call] });
      }
    }
  });
  return items;
}
