import type { ChatRequest } from "./protocol.js";

export function promptFromMessages(
  messages: Array<{ role: string; content: string }>,
  resumed: boolean,
): string {
  // Resuming replays only the newest user turn; the provider still holds the rest.
  const lastUser = messages.findLast((message) => message.role === "user");
  const source = resumed ? (lastUser ? [lastUser] : messages.slice(-1)) : messages;
  return source
    .filter((message) => message.content.trim())
    .map((message) => `${message.role.toUpperCase()}:\n${message.content}`)
    .join("\n\n");
}

export const CHAT_ONLY_INSTRUCTIONS =
  "You are the chat model inside Kassiber. Answer the user's message directly. " +
  "Do not use provider-native tools, run commands, browse, read or write files, inspect the " +
  "working directory, load provider skills, delegate, or edit anything. Use only the typed " +
  "Kassiber tools explicitly supplied for this turn, and never invent their results.";

export function systemInstructions(request: ChatRequest): string {
  return [CHAT_ONLY_INSTRUCTIONS, request.instructions]
    .filter((value): value is string => typeof value === "string" && Boolean(value.trim()))
    .join("\n\n");
}
