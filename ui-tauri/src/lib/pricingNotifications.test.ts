import { beforeEach, describe, expect, it } from "vitest";

import { resolveEntryAction } from "@/components/kb/notificationRouting";
import type { ActiveBook } from "@/lib/activeBook";
import { useFeedbackStore } from "@/lib/feedback";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";

import {
  pricingRebuildJob,
  recordPricingRebuilt,
  recordPricingRebuildFailed,
} from "./pricingNotifications";
import { recordReportExported, reportExportJob } from "./reportNotifications";

const BOOK = "book-a";
const ACTIVE: ActiveBook = {
  key: BOOK,
  expected: { workspace_id: "ws", profile_id: "pf-a" },
};
const SCOPE = { book: BOOK, expected: ACTIVE.expected };

beforeEach(() => {
  resetNotificationStore();
  useFeedbackStore.setState({ items: [] });
});

describe("price rebuilds", () => {
  it("is a job, and a failure opens the rebuild row of Market data", () => {
    const job = pricingRebuildJob(SCOPE);
    job.start();
    expect(Object.values(useNotificationStore.getState().jobs)).toMatchObject([
      { code: "pricing-rebuild", scope: { book: BOOK } },
    ]);
    recordPricingRebuildFailed(job.scope, "provider unreachable");
    job.end();
    const condition =
      useNotificationStore.getState().conditions[
        conditionKey("pricing.rebuild-failed", { book: BOOK })
      ];
    expect(resolveEntryAction(condition.action, condition.scope, ACTIVE, false)).toEqual({
      kind: "navigate",
      to: "/settings/market",
      stale: false,
      anchor: "rebuildPricing",
    });
    expect(useNotificationStore.getState().jobs).toEqual({});
  });

  it("resolves the failure on success and hands a journal blocker to journal processing", () => {
    const { scope } = pricingRebuildJob(SCOPE);
    recordPricingRebuildFailed(scope, "boom");
    recordPricingRebuilt(scope, { summary: "Cleared 3 prices.", journalBlocker: "writer busy" });
    const { conditions, results } = useNotificationStore.getState();
    expect(Object.keys(conditions)).toEqual([
      conditionKey("journals.process-failed", { book: BOOK }),
    ]);
    expect(conditions[conditionKey("journals.process-failed", { book: BOOK })].action).toEqual({
      type: "operation",
      operation: { kind: "process-journals", autoPair: true },
      review: "/journals",
    });
    expect(results).toMatchObject([{ code: "pricing.rebuilt", severity: "warning" }]);
  });
});

describe("report exports", () => {
  it("land in Recent opening Reports, with the file name kept as free text", () => {
    const job = reportExportJob(SCOPE);
    job.start();
    recordReportExported(job.scope, {
      flagged: false,
      warnings: 0,
      file: "kassiber-2025.pdf",
      detail: "4 pages",
    });
    job.end();
    const { results, jobs } = useNotificationStore.getState();
    expect(jobs).toEqual({});
    expect(results).toMatchObject([
      {
        code: "report.exported",
        action: { type: "navigate", to: "/reports" },
        details: [{ label: "kassiber-2025.pdf", message: "4 pages" }],
      },
    ]);
    expect(useFeedbackStore.getState().items[0]).toMatchObject({
      tone: "success",
      description: "kassiber-2025.pdf",
      descriptionPrivacy: "free-text",
    });
  });
});
