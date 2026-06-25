import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { mockDaemon } from "@/daemon/mock";

import { MappingControls } from "./MappingControls";
import { MappingPreview } from "./MappingPreview";
import { buildSpec, defaultSpec, specToDraft } from "./spec";
import type {
  CsvExampleResult,
  CsvPreviewResult,
  ImportMappedResult,
} from "./types";

describe("CSV mapping mock daemon", () => {
  it("csv_example returns template text + headers", async () => {
    const env = await mockDaemon.invoke<CsvExampleResult>({
      kind: "ui.wallets.csv_example",
      request_id: "1",
      args: {},
    });
    expect(env.data?.headers).toContain("Direction");
    expect(env.data?.csv).toContain("Date,Direction,Amount");
  });

  it("csv_preview auto-detects (confident + detected summary)", async () => {
    const env = await mockDaemon.invoke<CsvPreviewResult>({
      kind: "ui.wallets.csv_preview",
      request_id: "2",
      args: { source_file: "export.csv" },
    });
    expect(env.data?.confident).toBe(true);
    expect(env.data?.mapped).toBe(2);
    expect((env.data?.detected ?? []).some((d) => d.field === "date")).toBe(true);
  });

  it("import_mapped_csv persists live and previews on dry_run", async () => {
    const live = await mockDaemon.invoke<ImportMappedResult>({
      kind: "ui.wallets.import_mapped_csv",
      request_id: "3",
      args: { wallet: "W", source_file: "export.csv" },
    });
    expect(live.data?.dry_run).toBe(false);
    expect(live.data?.imported).toBe(2);
  });
});

describe("MappingPreview", () => {
  const preview: CsvPreviewResult = {
    rows_read: 3,
    mapped: 1,
    errors: 1,
    filtered: 1,
    truncated: false,
    headers: [],
    problems: [{ row: 4, kind: "error", column: "Date", reason: "bad_timestamp", detail: null }],
    preview: [
      {
        txid: "t1",
        occurred_at: "2026-01-15T00:00:00Z",
        direction: "outbound",
        asset: "BTC",
        amount: "0.5",
        fee: "0.0001",
        fiat_value: "20000",
        fiat_currency: "EUR",
        description: "Paid Bob",
      },
    ],
  };

  it("renders counts, fee/value columns, and localized problem reasons", () => {
    const html = renderToStaticMarkup(
      <MappingPreview
        preview={preview}
        loading={false}
        error={null}
        onlyProblems={false}
        setOnlyProblems={() => {}}
        lang="en"
      />,
    );
    expect(html).toContain("1 ready");
    expect(html).toContain("Paid Bob");
    expect(html).toContain("0.0001");
    expect(html).toContain("EUR");
    expect(html).toContain("Date could not be read");
  });
});

describe("MappingControls", () => {
  it("renders absolute-mode direction controls incl. the unmatched default", () => {
    const draft = defaultSpec();
    draft.amount.mode = "absolute";
    draft.amount.direction.mode = "column";
    const html = renderToStaticMarkup(
      <MappingControls draft={draft} setDraft={() => {}} headers={["Date", "Amount", "Type"]} />,
    );
    expect(html).toContain("Direction column");
    expect(html).toContain("Values meaning inbound");
    expect(html).toContain("If unmatched");
  });
});

describe("specToDraft", () => {
  it("round-trips an engine spec back into an editable draft", () => {
    const draft = defaultSpec();
    draft.timestampColumn = "When";
    draft.amount.mode = "split";
    draft.amount.inboundColumn = "In";
    draft.amount.outboundColumn = "Out";
    draft.fee.column = "Fee";
    draft.fields.description = { mode: "column", column: "Note", const: "" };
    draft.pricing = {
      enabled: true,
      currency: { mode: "const", column: "", const: "EUR" },
      rate: { mode: "column", column: "Price", const: "" },
      value: { mode: "none", column: "", const: "" },
      decimalSeparator: ".",
    };
    const restored = specToDraft(buildSpec(draft));
    expect(restored.timestampColumn).toBe("When");
    expect(restored.amount.mode).toBe("split");
    expect(restored.amount.inboundColumn).toBe("In");
    expect(restored.fee.column).toBe("Fee");
    expect(restored.fields.description).toEqual({ mode: "column", column: "Note", const: "" });
    expect(restored.pricing.enabled).toBe(true);
    expect(restored.pricing.currency).toEqual({ mode: "const", column: "", const: "EUR" });
  });
});
