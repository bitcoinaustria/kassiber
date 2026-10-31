// @vitest-environment happy-dom
//
// The approval row in the chat work log: it answers its own call, offers
// "Allow for this chat" only where the daemon allows it, never approves a
// review or plan that is not current, and takes focus only when the user is
// not typing.
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type { ComponentProps } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiChatMessage, AiToolConsentRequest } from "@/daemon/stream";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to: string }) => <a href={to}>{children}</a>,
}));

import { AssistantSessionContext, type AssistantSessionContextValue } from "./assistantSession";
import { ChatMessage } from "./ChatMessage";
import { PendingApprovalRow } from "./PendingApprovalRow";

const session = {
  sendConsent: vi.fn(async () => undefined),
  abort: vi.fn(),
  pendingConsents: [] as AiToolConsentRequest[],
};

function consent(overrides: Partial<AiToolConsentRequest> = {}): AiToolConsentRequest {
  return {
    targetRequestId: "chat-1",
    callId: "call-1",
    name: "ui.transactions.metadata_update",
    summary: "Add the note “rent” to tx-1",
    argumentsPreview: { transaction: "tx-1", note: "rent" },
    reversible: true,
    egresses: false,
    canAllowSession: true,
    ...overrides,
  };
}

function mount(node: React.ReactNode) {
  return render(
    <AssistantSessionContext.Provider value={session as unknown as AssistantSessionContextValue}>
      {node}
    </AssistantSessionContext.Provider>,
  );
}

function openMore() {
  fireEvent.pointerDown(screen.getByRole("button", { name: "More approval options" }), {
    button: 0,
    pointerType: "mouse",
  });
}

describe("PendingApprovalRow", () => {
  beforeEach(() => {
    session.sendConsent.mockClear();
    session.abort.mockClear();
    session.pendingConsents = [];
  });
  afterEach(cleanup);

  it("shows what will run and answers its own call", async () => {
    mount(<PendingApprovalRow request={consent()} />);
    const row = screen.getByRole("group", { name: /Add the note/ });
    expect(row.textContent).toContain("ui.transactions.metadata_update");
    expect(within(row).getByLabelText("Arguments").textContent).toContain('"note": "rent"');
    expect(row.textContent).toContain("Waiting for approval");
    // Reversible and local: no risk note.
    expect(row.textContent).not.toContain("Contacts the network");
    expect(row.textContent).not.toContain("Can't be undone");

    await act(async () => {
      fireEvent.click(within(row).getByRole("button", { name: "Approve" }));
    });
    expect(session.sendConsent).toHaveBeenCalledWith("call-1", "allow_once");
  });

  it("declines, and names the risks the daemon reports", async () => {
    mount(<PendingApprovalRow request={consent({ egresses: true, reversible: false })} />);
    const row = screen.getByRole("group", { name: /Add the note/ });
    expect(row.textContent).toContain("Contacts the network");
    expect(row.textContent).toContain("Can't be undone");
    expect(row.getAttribute("aria-describedby")).toBeTruthy();
    await act(async () => {
      fireEvent.click(within(row).getByRole("button", { name: "Decline" }));
    });
    expect(session.sendConsent).toHaveBeenCalledWith("call-1", "deny");
  });

  it("allows the tool for this chat from More, and can stop the answer", async () => {
    mount(<PendingApprovalRow request={consent()} />);
    openMore();
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Allow for this chat" }));
    });
    expect(session.sendConsent).toHaveBeenCalledWith("call-1", "allow_session");

    openMore();
    fireEvent.click(screen.getByRole("menuitem", { name: "Stop the answer" }));
    expect(session.abort).toHaveBeenCalledOnce();
  });

  it("hides Allow for this chat when the daemon does not offer it", () => {
    mount(<PendingApprovalRow request={consent({ canAllowSession: false })} />);
    openMore();
    expect(screen.queryByRole("menuitem", { name: "Allow for this chat" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "Stop the answer" })).toBeTruthy();
  });

  it("never approves a review whose server preview is unavailable", () => {
    const review = consent({
      callId: "apply-1",
      name: "ui.review.apply",
      summary: "Apply 1 review operation",
      argumentsPreview: { artifact: { forged: true } },
      reviewPreview: { status: "unavailable", code: "review_plan_stale" },
      canAllowSession: true,
    });
    mount(<PendingApprovalRow request={review} />);
    const apply = screen.getByRole("button", { name: "Apply once" }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("could not verify this plan");
    fireEvent.click(apply);
    openMore();
    // Once-only, and nothing to approve: only stopping is offered.
    expect(screen.queryByRole("menuitem", { name: "Allow for this chat" })).toBeNull();
    expect(session.sendConsent).not.toHaveBeenCalled();
  });

  it("takes focus when it appears, but not from someone typing", () => {
    const first = mount(<PendingApprovalRow request={consent()} />);
    expect(document.activeElement).toBe(screen.getByRole("group", { name: /Add the note/ }));
    first.unmount();

    const field = document.createElement("textarea");
    document.body.append(field);
    field.value = "and also";
    field.focus();
    mount(<PendingApprovalRow request={consent({ callId: "call-2" })} />);
    expect(document.activeElement).toBe(field);
    field.remove();
  });

  it("hands focus to the composer once it is answered", async () => {
    mount(
      <div data-assistant-surface="panel">
        <PendingApprovalRow request={consent()} />
        <textarea aria-label="Composer" />
      </div>,
    );
    const approve = screen.getByRole("button", { name: "Approve" });
    approve.focus();
    await act(async () => {
      fireEvent.click(approve);
    });
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Composer" }));
  });

  it("replaces its call's card in the work log while it waits", () => {
    const pending = consent();
    session.pendingConsents = [pending];
    const message: AiChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "",
      status: "streaming",
      activityLabel: "Generating",
      toolCalls: [
        {
          callId: "call-1",
          name: "ui.transactions.metadata_update",
          arguments: pending.argumentsPreview,
          kindClass: "mutating",
          needsConsent: true,
          status: "awaiting_consent",
          approval: "required",
        },
      ],
    };
    mount(<ChatMessage message={message} />);
    expect(screen.getByRole("group", { name: /Add the note/ })).toBeTruthy();
    // Nothing generates while the call waits on the user.
    expect(screen.queryByText("Generating")).toBeNull();
  });
});
