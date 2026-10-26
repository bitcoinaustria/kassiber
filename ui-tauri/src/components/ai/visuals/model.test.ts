import { describe, expect, it } from "vitest";

import type { AiChatToolCall } from "@/daemon/stream";
import fixtures from "./__fixtures__/daemon-visuals.json";
import { visualFormat } from "./format";
import {
  chatVisualFromToolCall,
  chatVisualsForMessage,
  pendingVisualSources,
  type ChatVisual,
} from "./model";
import { tableCsv, visualTable } from "./table";

type FixtureName = keyof typeof fixtures;

function call(result: unknown, overrides: Partial<AiChatToolCall> = {}): AiChatToolCall {
  return {
    callId: "call-visual",
    name: "ui.visuals.render",
    arguments: {},
    kindClass: "read_only",
    needsConsent: false,
    status: "done",
    result,
    ...overrides,
  };
}

function parse(name: FixtureName): ChatVisual {
  const visual = chatVisualFromToolCall(call(structuredClone(fixtures[name])));
  if (!visual) throw new Error(`${name} did not parse`);
  return visual;
}

const t = ((key: string) => key) as never;

describe("daemon visual payloads", () => {
  // The fixture is real `ui.visuals.render` output from the daemon, so this
  // pins the Python projection and the desktop parser to one contract.
  it.each(Object.keys(fixtures) as FixtureName[])("parses %s", (name) => {
    const visual = parse(name);
    expect(visual.source).toBe(name);
    expect(visual.fiatCurrency).toBe("EUR");
  });

  it("reads each source's own figures", () => {
    const overview = parse("overview");
    expect(overview.source === "overview" && overview.stats.btcBalance).toBeCloseTo(0.48998, 5);
    const flow = parse("cash_flow");
    expect(flow.source === "cash_flow" && flow.totals.incoming.count).toBe(4);
    const transactions = parse("transactions");
    expect(transactions.source === "transactions" && transactions.rows[0].amountSat).toBe(25_000_000);
    const tax = parse("tax_summary");
    expect(tax.source === "tax_summary" && tax.years[0].gainLoss).toBeCloseTo(898.2);
    const single = parse("transaction_flow");
    expect(single.source === "transaction_flow" && single.transaction?.direction).toBe("out");
    expect(single.source === "transaction_flow" && single.localTransaction).toBe("tx-demo-out-1");
    const readiness = parse("readiness");
    expect(readiness.source === "readiness" && readiness.summary.passed).toBe(6);
  });

  it("builds a table and CSV for every source", () => {
    const fmt = visualFormat("en-GB", "EUR");
    for (const name of Object.keys(fixtures) as FixtureName[]) {
      const table = visualTable(parse(name), fmt, t);
      expect(table.columns.length).toBeGreaterThan(1);
      for (const row of table.rows) expect(row).toHaveLength(table.columns.length);
      expect(tableCsv(table).split("\n")).toHaveLength(table.rows.length + 1);
    }
  });
});

describe("chatVisualFromToolCall", () => {
  it("draws nothing for running, other or malformed results", () => {
    const good = fixtures.balance_history;
    expect(chatVisualFromToolCall(call(good, { status: "running" }))).toBeNull();
    expect(chatVisualFromToolCall(call(good, { name: "ui.reports.balance_history" }))).toBeNull();
    const broken = (patch: (data: Record<string, unknown>) => void) => {
      const copy = structuredClone(good) as { data: Record<string, unknown> };
      patch(copy.data);
      return chatVisualFromToolCall(call(copy));
    };
    expect(broken((data) => ((data.visual as Record<string, unknown>).version = 2))).toBeNull();
    expect(broken((data) => ((data.visual as Record<string, unknown>).source = "html"))).toBeNull();
    expect(broken((data) => ((data.visual as Record<string, unknown>).unit = { kind: "fiat", code: "<b>" }))).toBeNull();
    // A value the chart cannot place breaks the contract; it is not skipped.
    expect(broken((data) => (data.rows = [{ period: "2026-01-01", asset: "BTC", quantity: "1" }]))).toBeNull();
    expect(broken((data) => (data.rows = [{ asset: "BTC", quantity: 1 }]))).toBeNull();
    const readiness = structuredClone(fixtures.readiness) as { data: { rows: Array<Record<string, unknown>> } };
    readiness.data.rows[0].action = "rm -rf";
    expect(chatVisualFromToolCall(call(readiness))).toBeNull();
  });

  it("shows a visual the router and the model both drew once, with the model's title", () => {
    const routed = structuredClone(fixtures.portfolio);
    const fromModel = structuredClone(fixtures.portfolio) as { data: { visual: Record<string, unknown> } };
    fromModel.data.visual.title = "Holdings by wallet";
    fromModel.data.visual.as_of = "2026-10-26T10:00:05Z";
    const visuals = chatVisualsForMessage([
      call(routed, { callId: "auto_read_1" }),
      call(fixtures.tax_summary, { callId: "tax" }),
      call(fromModel, { callId: "call_model" }),
    ]);
    expect(visuals.map((visual) => visual.callId)).toEqual(["tax", "call_model"]);
    expect(visuals[1].title).toBe("Holdings by wallet");
  });

  it("lists visuals still being drawn", () => {
    expect(
      pendingVisualSources([
        call(null, { status: "running", arguments: { source: "cash_flow" }, callId: "a" }),
        call(fixtures.overview, { callId: "b" }),
      ]),
    ).toEqual([{ callId: "a", source: "cash_flow" }]);
  });
});

describe("csv", () => {
  it("keeps numbers raw and neutralises formula text", () => {
    const csv = tableCsv({
      columns: ["Label", "Value"],
      rows: [[{ display: "=cmd", raw: "=cmd" }, { display: "−€ 5", raw: -5, numeric: true }]],
    });
    expect(csv).toBe("Label,Value\n'=cmd,-5");
  });
});
