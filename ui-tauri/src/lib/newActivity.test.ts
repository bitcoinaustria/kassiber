import { beforeEach, describe, expect, it } from "vitest";

import { projectResult } from "@/components/kb/notificationProjection";
import i18n from "@/i18n";
import { parseDaemonFreshnessEvent } from "@/lib/daemonFreshnessEvent";
import { newActivityFromRun, recordNewActivity } from "@/lib/newActivity";
import { applyFreshnessEvent } from "@/lib/notificationConditions";
import type { FreshnessRunData } from "@/lib/syncResults";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";

const t = i18n.getFixedT("en", "chrome");
const BOOK = "db:/books/a.sqlite3";
const SCOPE = { book: BOOK, expected: { workspace_id: "ws", profile_id: "pf-a" } };

function record(id: string, direction: string, wallet = "Treasury") {
  return {
    transaction_id: id,
    direction,
    wallet,
    asset: "BTC",
    amount_msat: 100_000_000,
  };
}

function job(label: string, result: Record<string, unknown>, status = "done") {
  return {
    id: `job-${label}`,
    job_type: "onchain_wallet",
    source_key: `wallet:${label}`,
    source_label: label,
    status,
    result,
  };
}

function activity() {
  return useNotificationStore
    .getState()
    .results.filter((entry) => entry.code === "activity.new");
}

beforeEach(() => resetNotificationStore());

describe("new transactions from a run", () => {
  it("counts each listed transaction once, by direction and wallet", () => {
    const data: FreshnessRunData = {
      results: [
        {
          wallet: "Treasury",
          status: "synced",
          inserted_records: [record("t1", "inbound"), record("t2", "outbound")],
        } as never,
      ],
      completed: [
        job("Treasury", { inserted_records: [record("t2", "outbound")] }),
        job("Spending", {
          inserted_records: [record("t3", "inbound", "Spending")],
        }),
        job("Broken", { inserted_records: [record("t9", "inbound")] }, "error"),
      ],
    };
    const found = newActivityFromRun(data)!;
    expect(found.records.map((item) => [item.id, item.direction, item.wallet])).toEqual([
      ["t1", "incoming", "Treasury"],
      ["t2", "outgoing", "Treasury"],
      ["t3", "incoming", "Spending"],
    ]);
  });

  it("falls back to the imported count when a source lists no records", () => {
    const found = newActivityFromRun({ completed: [job("Exchange", { imported: 4 })] });
    expect(found).toEqual({ records: [], unlisted: [{ wallet: "Exchange", count: 4 }] });
  });

  it("is nothing when a run imported nothing", () => {
    expect(
      newActivityFromRun({ completed: [job("A", { imported: 0, inserted_records: [] })] }),
    ).toBeNull();
    expect(newActivityFromRun(undefined)).toBeNull();
  });
});

describe("the new transactions entry", () => {
  it("names the one wallet, splits by direction, and opens Transactions", () => {
    recordNewActivity(
      newActivityFromRun({
        completed: [
          job("Treasury", {
            inserted_records: [
              record("t1", "inbound"),
              record("t2", "inbound"),
              record("t3", "outbound"),
            ],
          }),
        ],
      }),
      SCOPE,
    );
    const [entry] = activity();
    expect(entry.action).toEqual({ type: "navigate", to: "/transactions" });
    expect(projectResult(entry, "normal", t)).toMatchObject({
      title: "3 new transactions in Treasury",
      body: "2 incoming, 1 outgoing",
      details: [],
    });
    // Counts stay; the wallet name does not.
    expect(projectResult(entry, "hideAll", t)).toMatchObject({
      title: "3 new transactions",
      body: "2 incoming, 1 outgoing",
    });
  });

  it("opens the one transaction when only one arrived", () => {
    recordNewActivity(
      newActivityFromRun({ completed: [job("Treasury", { inserted_records: [record("t1", "inbound")] })] }),
      SCOPE,
    );
    expect(activity()[0].action).toEqual({ type: "open-transaction", transactionId: "t1" });
  });

  it("grows one unseen entry across runs and starts a new one once seen", () => {
    const run = (ids: string[], wallet: string) =>
      newActivityFromRun({
        completed: [
          job(wallet, { inserted_records: ids.map((id) => record(id, "inbound", wallet)) }),
        ],
      });
    recordNewActivity(run(["t1", "t2"], "Treasury"), SCOPE);
    recordNewActivity(run(["t2", "t3"], "Spending"), SCOPE);
    let entries = activity();
    expect(entries).toHaveLength(1);
    expect(entries[0].params).toMatchObject({ transactions: 3, incoming: 3, outgoing: 0 });
    expect(projectResult(entries[0], "normal", t).details).toEqual([
      "Treasury: 2 new",
      "Spending: 1 new",
    ]);

    useNotificationStore.getState().markSeen();
    recordNewActivity(run(["t4"], "Treasury"), SCOPE);
    entries = activity();
    // The seen entry is replaced by the newest run's entry.
    expect(entries).toHaveLength(1);
    expect(entries[0].params.transactions).toBe(1);
    expect(entries[0].seenAt).toBeUndefined();
  });

  it("is filed by a background event for the open book only", () => {
    const event = parseDaemonFreshnessEvent({
      event: true,
      kind: "ui.freshness.background",
      schema_version: 1,
      data: {
        profile: { id: "pf-a" },
        completed: [job("Treasury", { inserted_records: [record("t1", "inbound")] })],
        sources: [],
      },
    })!;
    applyFreshnessEvent(event, {
      key: "db:/books/b.sqlite3",
      expected: { workspace_id: "ws", profile_id: "pf-b" },
    });
    expect(activity()).toHaveLength(0);
    applyFreshnessEvent(event, { key: BOOK, expected: SCOPE.expected });
    expect(activity()).toHaveLength(1);
    expect(activity()[0].scope.book).toBe(BOOK);
  });
});
