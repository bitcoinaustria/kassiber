import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";
import {
  applyAiChatStreamRecordToMessage,
  type AiChatMessage,
} from "@/daemon/stream";
import { ThinkParser } from "@/lib/thinkParser";
import { ChatMessage } from "./ChatMessage";

const streaming: AiChatMessage = {
  id: "assistant-1",
  role: "assistant",
  content: "",
  status: "streaming",
  activityLabel: "Thinking",
};

const notice = {
  kind: "ai.chat.status",
  schema_version: 1,
  request_id: "chat-1",
  // The daemon's English label is not what the desktop shows.
  data: {
    phase: "fast_mode_unavailable",
    label: "Fast mode unavailable; answering at standard speed",
  },
} as const;

describe("fast mode unavailable notice", () => {
  afterEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("is kept on the answer and does not replace the progress label", () => {
    const once = applyAiChatStreamRecordToMessage(streaming, notice, new ThinkParser(), false);
    expect(once.notices).toEqual(["fast_mode_unavailable"]);
    expect(once.activityLabel).toBe("Thinking");
    // Repeats do not stack.
    const twice = applyAiChatStreamRecordToMessage(once, notice, new ThinkParser(), false);
    expect(twice.notices).toEqual(["fast_mode_unavailable"]);
  });

  it("renders translated copy under the answer", async () => {
    const message = applyAiChatStreamRecordToMessage(
      { ...streaming, content: "ok", status: "done" },
      notice,
      new ThinkParser(),
      false,
    );
    const english = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(english).toContain(
      "Fast mode was unavailable for this answer; it ran at standard speed.",
    );
    expect(english).not.toContain("answering at standard speed");

    await i18n.changeLanguage("de");
    const german = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(german).toContain(
      "Der Schnellmodus war für diese Antwort nicht verfügbar; sie lief mit Standardtempo.",
    );
  });
});
