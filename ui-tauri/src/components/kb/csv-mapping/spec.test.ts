import { describe, expect, it } from "vitest";

import {
  buildSpec,
  defaultSpec,
  serializeSpec,
  validateSpec,
  type DraftSpec,
} from "./spec";

function signedDraft(): DraftSpec {
  const draft = defaultSpec();
  draft.timestampColumn = "Date";
  draft.amount.mode = "signed";
  draft.amount.column = "Amount";
  return draft;
}

describe("buildSpec", () => {
  it("lowers a signed draft to the engine spec shape", () => {
    const spec = buildSpec(signedDraft());
    expect(spec.version).toBe(1);
    expect(spec.delimiter).toBeNull(); // "" -> auto-detect
    expect(spec.timestamp).toEqual({ column: "Date", format: null, timezone: "UTC" });
    expect(spec.amount).toMatchObject({ mode: "signed", column: "Amount", unit: "btc" });
  });

  it("omits empty optional blocks (fee, txid, fields, pricing)", () => {
    const spec = buildSpec(signedDraft());
    expect(spec).not.toHaveProperty("fee");
    expect(spec).not.toHaveProperty("txid");
    expect(spec).not.toHaveProperty("fields");
    expect(spec).not.toHaveProperty("pricing");
  });

  it("includes mapped fee, txid, fields, and pricing", () => {
    const draft = signedDraft();
    draft.fee.column = "Fee";
    draft.txidColumn = "Ref";
    draft.fields.description = { mode: "column", column: "Note", const: "" };
    draft.fields.kind = { mode: "const", column: "", const: "buy" };
    draft.pricing = {
      enabled: true,
      currency: { mode: "const", column: "", const: "EUR" },
      rate: { mode: "column", column: "Price", const: "" },
      value: { mode: "none", column: "", const: "" },
      decimalSeparator: ".",
    };
    const spec = buildSpec(draft) as Record<string, any>;
    expect(spec.fee).toMatchObject({ column: "Fee", unit: "btc" });
    expect(spec.txid).toEqual({ column: "Ref" });
    expect(spec.fields).toEqual({ description: { column: "Note" }, kind: { const: "buy" } });
    expect(spec.pricing.fiat_currency).toEqual({ const: "EUR" });
    expect(spec.pricing.fiat_rate).toEqual({ column: "Price" });
    expect(spec.pricing).not.toHaveProperty("fiat_value");
  });

  it("builds split + absolute amount blocks", () => {
    const split = defaultSpec();
    split.amount.mode = "split";
    split.amount.inboundColumn = "In";
    split.amount.outboundColumn = "Out";
    expect(buildSpec(split).amount).toMatchObject({
      mode: "split",
      inbound_column: "In",
      outbound_column: "Out",
    });

    const abs = defaultSpec();
    abs.amount.mode = "absolute";
    abs.amount.column = "Amount";
    abs.amount.direction = {
      mode: "column",
      const: "inbound",
      column: "Type",
      inboundValues: "deposit, buy",
      outboundValues: "withdrawal",
      default: "",
    };
    const built = buildSpec(abs).amount as Record<string, any>;
    expect(built.direction).toEqual({
      column: "Type",
      inbound_values: ["deposit", "buy"],
      outbound_values: ["withdrawal"],
      default: null,
    });
  });
});

describe("validateSpec", () => {
  it("flags missing required mappings", () => {
    expect(validateSpec(defaultSpec())).toContain("needDate");
    expect(validateSpec(defaultSpec())).toContain("needAmount");
  });

  it("passes a complete signed draft", () => {
    expect(validateSpec(signedDraft())).toEqual([]);
  });

  it("requires direction values in absolute column mode", () => {
    const draft = defaultSpec();
    draft.timestampColumn = "Date";
    draft.amount.mode = "absolute";
    draft.amount.column = "Amount";
    draft.amount.direction.mode = "column";
    const issues = validateSpec(draft);
    expect(issues).toContain("needDirection");
    expect(issues).toContain("needDirectionValues");
  });
});

describe("serializeSpec", () => {
  it("is stable for identical drafts (preview cache key)", () => {
    expect(serializeSpec(signedDraft())).toBe(serializeSpec(signedDraft()));
  });

  it("changes when the draft changes", () => {
    const a = signedDraft();
    const b = signedDraft();
    b.amount.unit = "sat";
    expect(serializeSpec(a)).not.toBe(serializeSpec(b));
  });
});
