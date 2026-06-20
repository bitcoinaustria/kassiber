import { describe, expect, it } from "vitest";

import {
  journalProcessOutcome,
  warningSummary,
  type JournalProcessWarning,
} from "./useJournalProcessingAction";

const dupLabel: JournalProcessWarning = {
  code: "duplicate_wallet_label",
  label: "Hot",
  wallet_ids: ["w1", "w2"],
  message: "2 wallets share the label 'Hot'. Rename them to be unique.",
};

describe("journalProcessOutcome", () => {
  it("reports a clean success with no warnings or quarantines", () => {
    const { body, tone } = journalProcessOutcome({
      processed_transactions: 3,
      entries_created: 5,
    });
    expect(tone).toBe("success");
    expect(body).toBe("3 transactions, 5 entries");
  });

  it("keeps warning tone for quarantines", () => {
    const { tone } = journalProcessOutcome({ quarantined: 2 });
    expect(tone).toBe("warning");
  });

  it("surfaces warnings with warning tone even when nothing is quarantined", () => {
    const { body, tone } = journalProcessOutcome({
      processed_transactions: 1,
      entries_created: 1,
      warnings: [dupLabel],
    });
    expect(tone).toBe("warning");
    expect(body).toContain("share the label 'Hot'");
  });

  it("summarizes multiple warnings", () => {
    expect(
      warningSummary([dupLabel, { code: "ownership_index", message: "x" }]),
    ).toMatch(/^2 warnings — /);
  });
});
