// @vitest-environment happy-dom
//
// A change that ran without a prompt says which permission let it, and stays
// visible in the work log instead of folding into "Used N tools"; one the
// daemon can reverse says so.
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { AiChatMessage, AiChatToolCall } from "@/daemon/stream";

import { ChatMessage } from "./ChatMessage";
import { ChatToolCall } from "./ChatToolCall";

function write(overrides: Partial<AiChatToolCall> = {}): AiChatToolCall {
  return {
    callId: "w1",
    name: "ui.transactions.metadata_update",
    arguments: { transaction: "tx-1" },
    kindClass: "mutating",
    needsConsent: false,
    status: "done",
    result: { kind: "ui.transactions.metadata_update", data: { updated: true } },
    ...overrides,
  };
}

describe("approval state on tool cards", () => {
  afterEach(cleanup);

  it.each([
    ["auto", "Approved automatically (Auto)"],
    ["full_access", "Approved automatically (Full access)"],
    ["session", "Approved automatically (allowed for this chat)"],
  ] as const)("badges a call the %s approval ran", (approval, label) => {
    const { container } = render(<ChatToolCall toolCall={write({ approval })} />);
    // Visible without expanding the card.
    expect(screen.getByText(label)).toBeTruthy();
    expect(container.querySelector(`[data-tool-approval="${approval}"]`)).not.toBeNull();
  });

  it("shows no badge when the user approved it or nothing needed approval", () => {
    for (const approval of ["required", "not_needed", undefined] as const) {
      const { container, unmount } = render(<ChatToolCall toolCall={write({ approval })} />);
      expect(container.querySelector("[data-tool-approval]")).toBeNull();
      expect(screen.queryByText(/Approved automatically/)).toBeNull();
      unmount();
    }
  });

  it("says a finished change can be undone when its result names the undo", () => {
    const undoable = write({
      result: {
        kind: "ui.transactions.metadata_update",
        data: { updated: true, undo: { tool: "ui_transactions_history_revert", arguments: { edit: "e1" } } },
      },
    });
    const { rerender } = render(<ChatToolCall toolCall={undoable} />);
    expect(screen.getByText("Can be undone — ask the assistant")).toBeTruthy();
    // Nothing is offered as a button: the assistant does the undo when asked.
    expect(screen.queryByRole("button", { name: /undo/i })).toBeNull();

    rerender(<ChatToolCall toolCall={write()} />);
    expect(screen.queryByText("Can be undone — ask the assistant")).toBeNull();
  });

  it("keeps a call that ran without asking out of the folded tool row", () => {
    const read = (callId: string): AiChatToolCall => ({
      callId,
      name: `ui.read.${callId}`,
      arguments: {},
      kindClass: "read_only",
      needsConsent: false,
      status: "done",
      approval: "not_needed",
    });
    const message: AiChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "",
      status: "done",
      toolCalls: [read("a"), write({ approval: "full_access" }), read("b")],
    };
    const { container } = render(<ChatMessage message={message} />);
    const groups = Array.from(container.querySelectorAll<HTMLElement>('[data-slot="tool-call-group"]'));
    expect(groups).toHaveLength(2);
    for (const group of groups) {
      expect(within(group).queryByText("ui.transactions.metadata_update")).toBeNull();
    }
    expect(screen.getByText("Approved automatically (Full access)")).toBeTruthy();
  });
});
