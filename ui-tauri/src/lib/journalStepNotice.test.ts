import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it } from "vitest";

import { daemonQueryKey } from "@/daemon/client";

import { useFeedbackStore } from "@/lib/feedback";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

import { notifyJournalStepAfterImport, recordJournalStep } from "./journalStepNotice";

const BOOK = "book-a";
const SCOPE = { book: BOOK, expected: { workspace_id: "ws", profile_id: "pf-a" } };

beforeEach(() => {
  resetNotificationStore();
  useFeedbackStore.setState({ items: [] });
  useUiStore.setState({ identity: null });
});

describe("recordJournalStep", () => {
  it("turns a failed journal step into a condition that processes journals again", () => {
    recordJournalStep(
      { status: "failed", error: { code: "boom", message: "writer busy" } },
      SCOPE,
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
    recordJournalStep({ status: "deferred" }, SCOPE);
    expect(
      useNotificationStore.getState().conditions[
        conditionKey("journals.stale", { book: BOOK })
      ],
    ).toMatchObject({ priority: "attention" });
  });

  it("only toasts a skipped pairing: the journals are current", () => {
    recordJournalStep(
      { status: "processed", auto_pair_skipped: true, error: { message: "locked" } },
      SCOPE,
    );
    expect(useNotificationStore.getState().conditions).toEqual({});
    expect(useFeedbackStore.getState().items).toHaveLength(1);
  });

  it("leaves a finished step alone", () => {
    recordJournalStep({ status: "processed", quarantines: 0 }, SCOPE);
    expect(useNotificationStore.getState().conditions).toEqual({});
    expect(useFeedbackStore.getState().items).toEqual([]);
  });

  it("files nothing that could run without the book's daemon ids", () => {
    recordJournalStep({ status: "failed" }, { book: BOOK });
    recordJournalStep({ status: "failed" }, null);
    expect(useNotificationStore.getState().conditions).toEqual({});
  });

  it("reads the step from import payloads only, for the book open in the same session", () => {
    const queryClient = new QueryClient();
    const { dataMode, daemonSession } = useUiStore.getState();
    queryClient.setQueryData(daemonQueryKey(dataMode, daemonSession, "ui.profiles.snapshot"), {
      data: { activeWorkspaceId: "ws", activeProfileId: "pf-a" },
    });
    notifyJournalStepAfterImport(queryClient, daemonSession, "ui.wallets.sync", {
      journals: { status: "failed" },
    });
    expect(useNotificationStore.getState().conditions).toEqual({});
    // A session that changed since the import started proves nothing.
    notifyJournalStepAfterImport(queryClient, daemonSession - 1, "ui.wallets.import_file", {
      journals: { status: "failed" },
    });
    expect(useNotificationStore.getState().conditions).toEqual({});
    notifyJournalStepAfterImport(queryClient, daemonSession, "ui.wallets.import_file", {
      journals: { status: "failed" },
    });
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition).toMatchObject({
      code: "journals.process-failed",
      scope: { expected: { workspace_id: "ws", profile_id: "pf-a" } },
    });
  });
});
