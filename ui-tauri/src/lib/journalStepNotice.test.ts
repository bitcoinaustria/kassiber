import { beforeEach, describe, expect, it } from "vitest";

import { useFeedbackStore } from "@/lib/feedback";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

import { notifyJournalStepAfterImport, recordJournalStep } from "./journalStepNotice";

const BOOK = "book-a";

beforeEach(() => {
  resetNotificationStore();
  useFeedbackStore.setState({ items: [] });
  useUiStore.setState({ identity: null });
});

describe("recordJournalStep", () => {
  it("turns a failed journal step into a condition that processes journals again", () => {
    recordJournalStep(
      { status: "failed", error: { code: "boom", message: "writer busy" } },
      BOOK,
    );
    const condition =
      useNotificationStore.getState().conditions[
        conditionKey("journals.process-failed", { book: BOOK })
      ];
    expect(condition).toMatchObject({
      severity: "error",
      action: {
        type: "operation",
        operation: { kind: "process-journals", autoPair: true },
        review: "/journals",
      },
    });
    expect(condition.details?.[0]).toMatchObject({
      message: "writer busy",
      phraseKey: "failed",
    });
  });

  it("marks journals stale when the step was deferred, like the Review badges do", () => {
    recordJournalStep({ status: "deferred" }, BOOK);
    expect(
      useNotificationStore.getState().conditions[
        conditionKey("journals.stale", { book: BOOK })
      ],
    ).toMatchObject({ priority: "attention" });
  });

  it("only toasts a skipped pairing: the journals are current", () => {
    recordJournalStep(
      { status: "processed", auto_pair_skipped: true, error: { message: "locked" } },
      BOOK,
    );
    expect(useNotificationStore.getState().conditions).toEqual({});
    expect(useFeedbackStore.getState().items).toHaveLength(1);
  });

  it("leaves a finished step alone", () => {
    recordJournalStep({ status: "processed", quarantines: 0 }, BOOK);
    expect(useNotificationStore.getState().conditions).toEqual({});
    expect(useFeedbackStore.getState().items).toEqual([]);
  });

  it("reads the step from import payloads only", () => {
    notifyJournalStepAfterImport("ui.wallets.sync", { journals: { status: "failed" } });
    expect(useNotificationStore.getState().conditions).toEqual({});
    notifyJournalStepAfterImport("ui.wallets.import_file", {
      journals: { status: "failed" },
    });
    expect(
      Object.keys(useNotificationStore.getState().conditions),
    ).toEqual([conditionKey("journals.process-failed", { book: null })]);
  });
});
