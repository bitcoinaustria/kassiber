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
  "Do not call tools, run commands, browse, read or write files, inspect the working directory, " +
  "use MCP, delegate, or edit anything. Kassiber exposes accounting data only through its own " +
  "separately governed typed-tool boundary; no such tools are available in this provider session.";

