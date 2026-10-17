import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";
import {
  aiChatNotices,
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
  // An HTTP endpoint refused the effort; the daemon retried without it.
  data: {
    phase: "reasoning_effort_unavailable",
    label: "Reasoning effort unavailable; answering without it",
  },
} as const;

describe("reasoning effort unavailable notice", () => {
  afterEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("is a recognised answer notice, kept once", () => {
    const once = applyAiChatStreamRecordToMessage(streaming, notice, new ThinkParser(), false);
    expect(once.notices).toEqual(["reasoning_effort_unavailable"]);
    expect(once.activityLabel).toBe("Thinking");
    const twice = applyAiChatStreamRecordToMessage(once, notice, new ThinkParser(), false);
    expect(twice.notices).toEqual(["reasoning_effort_unavailable"]);
    expect(
      aiChatNotices(["reasoning_effort_unavailable", "provider_says_anything"]),
    ).toEqual(["reasoning_effort_unavailable"]);
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
      "The model did not accept the reasoning effort; it answered without it.",
    );
    expect(english).not.toContain("answering without it");

    await i18n.changeLanguage("de");
    const german = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(german).toContain(
      "Das Modell hat den Denkaufwand nicht angenommen; es hat ohne ihn geantwortet.",
    );
  });
});
