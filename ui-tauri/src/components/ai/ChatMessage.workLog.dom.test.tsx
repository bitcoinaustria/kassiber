// @vitest-environment happy-dom
//
// Mounted: consecutive tool calls fold into one "Used N tools" row, but a call
// waiting on the user's consent (or one that failed) stays visible outside it.
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { AiChatMessage, AiChatToolCall } from "@/daemon/stream";

import { ChatMessage } from "./ChatMessage";

function call(
  callId: string,
  segmentId: string,
  overrides: Partial<AiChatToolCall> = {},
): AiChatToolCall {
  return {
    callId,
    name: `ui.read.${callId}`,
    arguments: {},
    kindClass: "read_only",
    needsConsent: false,
    status: "done",
    segmentId,
    ...overrides,
  };
}

function answer(toolCalls: AiChatToolCall[]): AiChatMessage {
  // One empty reasoning segment per provider round, as Copilot produces.
  const segments = [...new Set(toolCalls.map((item) => item.segmentId!))];
  return {
    id: "assistant",
    role: "assistant",
    content: "",
    status: "streaming",
    thinkingSegments: segments.map((id) => ({ id, content: "" })),
    toolCalls,
  };
}

function groups(container: HTMLElement) {
  return Array.from(
    container.querySelectorAll<HTMLElement>('[data-slot="tool-call-group"]'),
  );
}

describe("ChatMessage work log", () => {
  afterEach(cleanup);

  it("folds six one-call rounds into one expandable row", () => {
    const calls = Array.from({ length: 6 }, (_, index) =>
      call(`c${index}`, `seg-${index}`),
    );
    const { container } = render(<ChatMessage message={answer(calls)} />);

    expect(groups(container)).toHaveLength(1);
    const header = screen.getByRole("button", { name: /Used 6 tools/ });
    expect(screen.queryByText("ui.read.c3")).toBeNull();

    fireEvent.click(header);
    expect(screen.getByText("ui.read.c3")).toBeTruthy();
  });

  it("keeps a consent-gated call visible beside the folded calls", () => {
    const pending = call("write", "seg-2", {
      name: "ui.journals.process",
      kindClass: "mutating",
      needsConsent: true,
      status: "pending",
    });
    const view = render(
      <ChatMessage
        message={answer([call("a", "seg-1"), pending, call("b", "seg-3")])}
      />,
    );

    // Before the consent prompt arrives the call is already outside the group.
    expect(screen.getByText("ui.journals.process")).toBeTruthy();
    for (const group of groups(view.container)) {
      expect(within(group).queryByText("ui.journals.process")).toBeNull();
    }

    // The consent record updates the same call_id; it stays visible.
    view.rerender(
      <ChatMessage
        message={answer([
          call("a", "seg-1"),
          { ...pending, status: "awaiting_consent" },
          call("b", "seg-3"),
        ])}
      />,
    );
    expect(screen.getAllByText("ui.journals.process")).toHaveLength(1);
    expect(groups(view.container)).toHaveLength(2);

    // Once approved and running it joins the routine calls in one row.
    view.rerender(
      <ChatMessage
        message={answer([
          call("a", "seg-1"),
          { ...pending, needsConsent: false, status: "running" },
          call("b", "seg-3"),
        ])}
      />,
    );
    expect(groups(view.container)).toHaveLength(1);
    expect(screen.getByRole("button", { name: /Using 3 tools/ })).toBeTruthy();
  });

  it("does not fold a failed call away", () => {
    render(
      <ChatMessage
        message={answer([
          call("a", "seg-1"),
          call("broken", "seg-2", { status: "error", reason: "Daemon refused" }),
        ])}
      />,
    );
    expect(screen.getByText("ui.read.broken")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Used 1 tool/ })).toBeTruthy();
  });
});
