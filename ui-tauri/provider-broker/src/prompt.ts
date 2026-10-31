import { messageImages, type BrokerImage, type ChatMessage, type ChatRequest } from "./protocol.js";

function promptMessages(messages: ChatMessage[], resumed: boolean): ChatMessage[] {
  // Resuming replays only the newest user turn; the provider still holds the rest.
  const lastUser = messages.findLast((message) => message.role === "user");
  return resumed ? (lastUser ? [lastUser] : messages.slice(-1)) : messages;
}

export function promptFromMessages(messages: ChatMessage[], resumed: boolean): string {
  return promptMessages(messages, resumed)
    .filter((message) => message.content.trim())
    .map((message) => `${message.role.toUpperCase()}:\n${message.content}`)
    .join("\n\n");
}

/** The attached images the prompt carries, from the messages it replays. */
export function promptImages(messages: ChatMessage[], resumed: boolean): BrokerImage[] {
  return messageImages(promptMessages(messages, resumed));
}

/**
 * Said in the prompt when a provider cannot take the attached images, so the
 * model does not answer as if it had seen them.
 */
export function imagesNotShownNote(count: number): string {
  return (
    `\n\n[Kassiber: ${count} attached image${count === 1 ? " was" : "s were"} ` +
    "not shown to you because this agent cannot receive images. Tell the user, " +
    "and do not describe the image.]"
  );
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
