// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ChatToolCall } from "./ChatToolCall";

describe("ChatToolCall", () => {
  afterEach(cleanup);

  it("summarizes a load_tools call by the areas it added", () => {
    render(
      <ChatToolCall
        toolCall={{
          callId: "call-load",
          name: "load_tools",
          arguments: { areas: ["reports"] },
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: {
            kind: "load_tools",
            data: {
              areas: ["reports"],
              added_tools: ["ui_reports_capital_gains", "ui_reports_tax_summary"],
              already_offered: 0,
            },
          },
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /load_tools/ }));
    expect(screen.getByText("Loaded 2 tools for reports")).toBeTruthy();
  });

  it("summarizes withheld journal figures as unavailable, not zero", () => {
    render(
      <ChatToolCall
        toolCall={{
          callId: "call-transfers",
          name: "ui.journals.transfers.list",
          arguments: {},
          kindClass: "read_only",
          needsConsent: false,
          status: "done",
          result: {
            kind: "ui.journals.transfers.list",
            data: {
              status: "unavailable",
              reason: "journals_stale",
              summary: { manual_pairs: null, journal_transfer_entries: null },
              pairs: null,
            },
          },
        }}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /ui.journals.transfers.list/ }));
    expect(
      screen.getByText("Figures withheld: journals need processing first."),
    ).toBeTruthy();
    expect(screen.queryByText(/0 pairs/)).toBeNull();
  });
});
