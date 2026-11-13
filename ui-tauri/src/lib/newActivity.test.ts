import { beforeEach, describe, expect, it } from "vitest";

import { projectResult } from "@/components/kb/notificationProjection";
import i18n from "@/i18n";
import { parseDaemonFreshnessEvent } from "@/lib/daemonFreshnessEvent";
import {
  newActivityFromRun,
  recordNewActivity,
  resetAnnouncedActivity,
} from "@/lib/newActivity";
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

let jobSequence = 0;
function job(
  label: string,
  result: Record<string, unknown>,
  status = "done",
  id = `job-${label}-${++jobSequence}`,
) {
  return {
    id,
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

beforeEach(() => {
  resetNotificationStore();
  resetAnnouncedActivity();
});

describe("new transactions from a run", () => {
  it("counts each listed transaction once, by direction and wallet", () => {
    const data: FreshnessRunData = {
      completed: [
        job("Treasury", {
          inserted_records: [record("t1", "inbound"), record("t2", "outbound")],
        }),
        job("Spending", {
          inserted_records: [record("t2", "outbound"), record("t3", "inbound", "Spending")],
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

  it("reads one copy when a run mirrors its jobs into results", () => {
    // The daemon builds `results` from the completed jobs' results.
    const outcome = { wallet: "Exchange", status: "synced", imported: 4 };
    const found = newActivityFromRun({
      results: [outcome as never],
      completed: [job("Exchange", outcome, "done", "job-Exchange")],
    });
    expect(found).toEqual({
      records: [],
      unlisted: [{ wallet: "Exchange", count: 4, job: "job-Exchange" }],
    });
  });

  it("reads results when no job completed with a result", () => {
    const found = newActivityFromRun({
      results: [
        { wallet: "Treasury", status: "synced", inserted_records: [record("t1", "inbound")] } as never,
      ],
    });
    expect(found?.records.map((item) => item.id)).toEqual(["t1"]);
  });

  it("falls back to the imported count when a source lists no records", () => {
    const found = newActivityFromRun({
      completed: [job("Exchange", { imported: 4 }, "done", "job-Exchange")],
    });
    expect(found).toEqual({
      records: [],
      unlisted: [{ wallet: "Exchange", count: 4, job: "job-Exchange" }],
    });
  });

  it("is nothing when a run imported nothing", () => {
    expect(
      newActivityFromRun({ completed: [job("A", { imported: 0, inserted_records: [] })] }),
    ).toBeNull();
    expect(newActivityFromRun(undefined)).toBeNull();
  });
});

describe("new transactions from a direct sync", () => {
  it("keeps a direct wallet outcome next to a completed journal job", () => {
    // The daemon returns direct outcomes in `results`; `completed` may hold
    // only the journal step (planning.rs).
    const found = newActivityFromRun({
      results: [
        {
          wallet: "Mapped",
          status: "synced",
          inserted_records: [record("t1", "inbound", "Mapped")],
        } as never,
      ],
      completed: [
        { id: "j1", job_type: "journal_refresh", status: "done", result: { rebuilt: true } },
      ],
    });
    expect(found?.records.map((item) => item.id)).toEqual(["t1"]);
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
    // Replayed after it was seen: nothing new to announce.
    useNotificationStore.getState().markSeen();
    recordNewActivity(run(["t1", "t2", "t3"], "Treasury"), SCOPE);
    expect(activity()[0].params.transactions).toBe(3);
    expect(activity()[0].seenAt).toBeDefined();

    useNotificationStore.getState().markSeen();
    recordNewActivity(run(["t4"], "Treasury"), SCOPE);
    entries = activity();
    // The seen entry is replaced by the newest run's entry.
    expect(entries).toHaveLength(1);
    expect(entries[0].params.transactions).toBe(1);
    expect(entries[0].seenAt).toBeUndefined();
  });

  it("keeps every wallet's count past the detail cap and shortens only the display", () => {
    const wallets = Array.from({ length: 13 }, (_, index) => `W${index}`);
    recordNewActivity(
      newActivityFromRun({
        completed: wallets.map((wallet, index) =>
          job(wallet, { inserted_records: [record(`a${index}`, "inbound", wallet)] }),
        ),
      }),
      SCOPE,
    );
    recordNewActivity(
      newActivityFromRun({
        completed: [job("W12", { inserted_records: [record("b12", "inbound", "W12")] })],
      }),
      SCOPE,
    );
    const [entry] = activity();
    expect(entry.params.transactions).toBe(14);
    expect(entry.params.walletCounts?.find((item) => item.label === "W12")?.count).toBe(2);
    expect(projectResult(entry, "normal", t).details).toEqual([
      "W0: 1 new",
      "W1: 1 new",
      "W2: 1 new",
      "W3: 1 new",
      "+9 more wallets",
    ]);
  });

  it("does not announce a replayed count-only job, or a re-inserted txid, twice", () => {
    const countOnly = () =>
      newActivityFromRun({
        completed: [job("Exchange", { imported: 4 }, "done", "job-replayed")],
      });
    recordNewActivity(countOnly(), SCOPE);
    useNotificationStore.getState().markSeen();
    recordNewActivity(countOnly(), SCOPE);
    expect(activity()[0].params.transactions).toBe(4);
    expect(activity()[0].seenAt).toBeDefined();

    // A rescan re-inserts the same txid under a new row id.
    const withTxid = (rowId: string) =>
      newActivityFromRun({
        completed: [
          job("Treasury", {
            inserted_records: [{ ...record(rowId, "inbound"), external_id: "txid#aa" }],
          }),
        ],
      });
    recordNewActivity(withTxid("row-1"), SCOPE);
    useNotificationStore.getState().markSeen();
    recordNewActivity(withTxid("row-2"), SCOPE);
    expect(activity()[0].params.transactionIds).toEqual(["row-1"]);
  });

  it("announces both sides of a transfer between own wallets", () => {
    const transfer = newActivityFromRun({
      completed: [
        job("Hot", {
          inserted_records: [{ ...record("r1", "outbound", "Hot"), external_id: "txid#bb" }],
        }),
        job("Cold", {
          inserted_records: [{ ...record("r2", "inbound", "Cold"), external_id: "txid#bb" }],
        }),
      ],
    });
    recordNewActivity(transfer, SCOPE);
    expect(activity()[0].params).toMatchObject({ transactions: 2, incoming: 1, outgoing: 1 });
  });

  it("does not let a large replayed job through after per-transaction keys roll over", () => {
    const big = () =>
      newActivityFromRun({
        completed: [
          job(
            "Treasury",
            {
              inserted_records: Array.from({ length: 5_001 }, (_, index) =>
                record(`big-${index}`, "inbound"),
              ),
            },
            "done",
            "job-big",
          ),
        ],
      });
    recordNewActivity(big(), SCOPE);
    recordNewActivity(big(), SCOPE);
    expect(activity()[0].params.transactions).toBe(5_001);
  });

  it("keeps what each book announced apart", () => {
    const run = () =>
      newActivityFromRun({ completed: [job("Treasury", { inserted_records: [record("t1", "inbound")] })] });
    recordNewActivity(run(), SCOPE);
    recordNewActivity(run(), { book: "db:/books/b.sqlite3" });
    const books = activity().map((entry) => entry.scope.book).sort();
    expect(books).toEqual([BOOK, "db:/books/b.sqlite3"].sort());
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
