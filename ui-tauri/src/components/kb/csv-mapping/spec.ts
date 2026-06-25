/**
 * Editable draft for the custom CSV mapping workbench.
 *
 * `DraftSpec` is the UI-friendly editing shape (every field present, column
 * names default to "" = unmapped). `buildSpec` lowers it to the JSON mapping
 * object the Python engine (`kassiber.core.csv_mapping`) validates and applies —
 * the durable artifact sent as `args.mapping`. Keeping the build/validate logic
 * pure makes it unit-testable and keeps the React components thin.
 */

export type Unit = "btc" | "sat" | "msat";
export type AmountMode = "signed" | "split" | "absolute";
export type FilterOp = "equals" | "in" | "not_empty";
export type FieldTarget = "kind" | "description" | "counterparty";
export type Direction = "inbound" | "outbound";

export const UNITS: Unit[] = ["btc", "sat", "msat"];
export const AMOUNT_MODES: AmountMode[] = ["signed", "split", "absolute"];
export const FILTER_OPS: FilterOp[] = ["equals", "in", "not_empty"];
export const FIELD_TARGETS: FieldTarget[] = ["kind", "description", "counterparty"];

/** A target field is either unmapped, taken from a column, or a fixed value. */
export interface FieldDraft {
  mode: "none" | "column" | "const";
  column: string;
  const: string;
}

export interface DirectionDraft {
  mode: "const" | "column";
  const: Direction;
  column: string;
  /** Comma-separated in the UI; split into a list at build time. */
  inboundValues: string;
  outboundValues: string;
  default: "" | Direction;
}

export interface AmountDraft {
  mode: AmountMode;
  unit: Unit;
  decimalSeparator: "." | ",";
  column: string; // signed / absolute
  inboundColumn: string; // split
  outboundColumn: string; // split
  direction: DirectionDraft; // absolute
}

export interface FeeDraft {
  column: string;
  unit: Unit;
  decimalSeparator: "." | ",";
}

export interface FilterDraft {
  column: string;
  op: FilterOp;
  value: string;
}

export interface PricingDraft {
  enabled: boolean;
  currency: FieldDraft;
  rate: FieldDraft;
  value: FieldDraft;
  decimalSeparator: "." | ",";
}

export interface DraftSpec {
  name: string;
  asset: "BTC" | "LBTC";
  delimiter: string; // "" = auto-detect
  encoding: string;
  skipRows: number;
  timestampColumn: string;
  timestampFormat: string; // "" = flexible ISO
  timezone: string;
  amount: AmountDraft;
  fee: FeeDraft;
  txidColumn: string;
  fields: Record<FieldTarget, FieldDraft>;
  pricing: PricingDraft;
  filters: FilterDraft[];
}

const emptyField = (): FieldDraft => ({ mode: "none", column: "", const: "" });

export function defaultSpec(): DraftSpec {
  return {
    name: "",
    asset: "BTC",
    delimiter: "",
    encoding: "utf-8-sig",
    skipRows: 0,
    timestampColumn: "",
    timestampFormat: "",
    timezone: "UTC",
    amount: {
      mode: "signed",
      unit: "btc",
      decimalSeparator: ".",
      column: "",
      inboundColumn: "",
      outboundColumn: "",
      direction: {
        mode: "column",
        const: "inbound",
        column: "",
        inboundValues: "",
        outboundValues: "",
        default: "",
      },
    },
    fee: { column: "", unit: "btc", decimalSeparator: "." },
    txidColumn: "",
    fields: {
      kind: emptyField(),
      description: emptyField(),
      counterparty: emptyField(),
    },
    pricing: {
      enabled: false,
      currency: emptyField(),
      rate: emptyField(),
      value: emptyField(),
      decimalSeparator: ".",
    },
    filters: [],
  };
}

function splitList(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

function fieldRef(field: FieldDraft): Record<string, string> | null {
  if (field.mode === "column" && field.column) return { column: field.column };
  if (field.mode === "const" && field.const !== "") return { const: field.const };
  return null;
}

/** Lower the editable draft to the JSON mapping spec the engine validates. */
export function buildSpec(draft: DraftSpec): Record<string, unknown> {
  const amount: Record<string, unknown> = {
    mode: draft.amount.mode,
    unit: draft.amount.unit,
    decimal_separator: draft.amount.decimalSeparator,
  };
  if (draft.amount.mode === "signed" || draft.amount.mode === "absolute") {
    amount.column = draft.amount.column;
  }
  if (draft.amount.mode === "split") {
    amount.inbound_column = draft.amount.inboundColumn;
    amount.outbound_column = draft.amount.outboundColumn;
  }
  if (draft.amount.mode === "absolute") {
    const dir = draft.amount.direction;
    amount.direction =
      dir.mode === "const"
        ? { const: dir.const }
        : {
            column: dir.column,
            inbound_values: splitList(dir.inboundValues),
            outbound_values: splitList(dir.outboundValues),
            default: dir.default || null,
          };
  }

  const spec: Record<string, unknown> = {
    version: 1,
    name: draft.name || "",
    asset: draft.asset,
    delimiter: draft.delimiter || null,
    encoding: draft.encoding || "utf-8-sig",
    skip_rows: draft.skipRows || 0,
    timestamp: {
      column: draft.timestampColumn,
      format: draft.timestampFormat || null,
      timezone: draft.timezone || "UTC",
    },
    amount,
    filters: draft.filters
      .filter((f) => f.column)
      .map((f) =>
        f.op === "not_empty"
          ? { column: f.column, op: f.op }
          : { column: f.column, op: f.op, value: f.value },
      ),
  };

  if (draft.fee.column) {
    spec.fee = {
      column: draft.fee.column,
      unit: draft.fee.unit,
      decimal_separator: draft.fee.decimalSeparator,
    };
  }
  if (draft.txidColumn) {
    spec.txid = { column: draft.txidColumn };
  }

  const fields: Record<string, unknown> = {};
  for (const target of FIELD_TARGETS) {
    const ref = fieldRef(draft.fields[target]);
    if (ref) fields[target] = ref;
  }
  if (Object.keys(fields).length > 0) {
    spec.fields = fields;
  }

  if (draft.pricing.enabled) {
    const pricing: Record<string, unknown> = {
      decimal_separator: draft.pricing.decimalSeparator,
    };
    const currency = fieldRef(draft.pricing.currency);
    const rate = fieldRef(draft.pricing.rate);
    const value = fieldRef(draft.pricing.value);
    if (currency) pricing.fiat_currency = currency;
    if (rate) pricing.fiat_rate = rate;
    if (value) pricing.fiat_value = value;
    spec.pricing = pricing;
  }

  return spec;
}

/** A stable string for the preview query key (identical drafts hit the cache). */
export function serializeSpec(draft: DraftSpec): string {
  return JSON.stringify(buildSpec(draft));
}

function refToField(ref: unknown): FieldDraft {
  if (ref && typeof ref === "object") {
    const obj = ref as Record<string, unknown>;
    if (typeof obj.column === "string") return { mode: "column", column: obj.column, const: "" };
    if ("const" in obj) return { mode: "const", column: "", const: String(obj.const ?? "") };
  }
  return emptyField();
}

/**
 * Best-effort inverse of {@link buildSpec}: turn an engine spec (e.g. the
 * auto-detected mapping returned by the daemon) into an editable draft so the
 * Advanced editor starts from the guess rather than a blank slate. Tolerant of
 * partial/unknown shapes — anything missing falls back to defaults.
 */
export function specToDraft(spec: Record<string, unknown> | null | undefined): DraftSpec {
  const draft = defaultSpec();
  if (!spec || typeof spec !== "object") return draft;
  const s = spec as Record<string, any>;
  if (typeof s.name === "string") draft.name = s.name;
  if (s.asset === "LBTC" || s.asset === "BTC") draft.asset = s.asset;
  draft.delimiter = typeof s.delimiter === "string" ? s.delimiter : "";
  if (typeof s.encoding === "string") draft.encoding = s.encoding;
  if (typeof s.skip_rows === "number") draft.skipRows = s.skip_rows;

  const ts = s.timestamp ?? {};
  draft.timestampColumn = typeof ts.column === "string" ? ts.column : "";
  draft.timestampFormat = typeof ts.format === "string" ? ts.format : "";
  draft.timezone = typeof ts.timezone === "string" ? ts.timezone : "UTC";

  const amount = s.amount ?? {};
  if (amount.mode === "signed" || amount.mode === "split" || amount.mode === "absolute") {
    draft.amount.mode = amount.mode;
  }
  if (amount.unit === "btc" || amount.unit === "sat" || amount.unit === "msat") draft.amount.unit = amount.unit;
  if (amount.decimal_separator === "," || amount.decimal_separator === ".") {
    draft.amount.decimalSeparator = amount.decimal_separator;
  }
  if (typeof amount.column === "string") draft.amount.column = amount.column;
  if (typeof amount.inbound_column === "string") draft.amount.inboundColumn = amount.inbound_column;
  if (typeof amount.outbound_column === "string") draft.amount.outboundColumn = amount.outbound_column;
  const dir = amount.direction;
  if (dir && typeof dir === "object") {
    if ("const" in dir) {
      draft.amount.direction.mode = "const";
      draft.amount.direction.const = dir.const === "outbound" ? "outbound" : "inbound";
    } else {
      draft.amount.direction.mode = "column";
      draft.amount.direction.column = typeof dir.column === "string" ? dir.column : "";
      draft.amount.direction.inboundValues = Array.isArray(dir.inbound_values) ? dir.inbound_values.join(", ") : "";
      draft.amount.direction.outboundValues = Array.isArray(dir.outbound_values) ? dir.outbound_values.join(", ") : "";
      draft.amount.direction.default = dir.default === "inbound" || dir.default === "outbound" ? dir.default : "";
    }
  }

  const fee = s.fee;
  if (fee && typeof fee === "object" && typeof fee.column === "string") {
    draft.fee.column = fee.column;
    if (fee.unit === "btc" || fee.unit === "sat" || fee.unit === "msat") draft.fee.unit = fee.unit;
    if (fee.decimal_separator === "," || fee.decimal_separator === ".") draft.fee.decimalSeparator = fee.decimal_separator;
  }
  if (s.txid && typeof s.txid === "object" && typeof s.txid.column === "string") {
    draft.txidColumn = s.txid.column;
  }

  const fields = s.fields ?? {};
  for (const target of FIELD_TARGETS) {
    if (fields[target]) draft.fields[target] = refToField(fields[target]);
  }

  const pricing = s.pricing;
  if (pricing && typeof pricing === "object") {
    draft.pricing.enabled = true;
    if (pricing.decimal_separator === "," || pricing.decimal_separator === ".") {
      draft.pricing.decimalSeparator = pricing.decimal_separator;
    }
    if (pricing.fiat_currency) draft.pricing.currency = refToField(pricing.fiat_currency);
    if (pricing.fiat_rate) draft.pricing.rate = refToField(pricing.fiat_rate);
    if (pricing.fiat_value) draft.pricing.value = refToField(pricing.fiat_value);
  }

  if (Array.isArray(s.filters)) {
    draft.filters = s.filters
      .filter((f: any) => f && typeof f === "object" && typeof f.column === "string")
      .map((f: any) => ({
        column: f.column,
        op: f.op === "in" || f.op === "not_empty" ? f.op : "equals",
        value: f.value == null ? "" : String(f.value),
      }));
  }
  return draft;
}

/**
 * Lightweight client-side gating: which required mappings are still missing.
 * Returns stable issue codes the UI localizes; the daemon is the source of
 * truth and re-validates fully on preview/import.
 */
export function validateSpec(draft: DraftSpec): string[] {
  const issues: string[] = [];
  if (!draft.timestampColumn) issues.push("needDate");
  if (draft.amount.mode === "signed" || draft.amount.mode === "absolute") {
    if (!draft.amount.column) issues.push("needAmount");
  }
  if (draft.amount.mode === "split") {
    if (!draft.amount.inboundColumn && !draft.amount.outboundColumn) {
      issues.push("needAmount");
    }
  }
  if (draft.amount.mode === "absolute" && draft.amount.direction.mode === "column") {
    if (!draft.amount.direction.column) issues.push("needDirection");
    if (
      !splitList(draft.amount.direction.inboundValues).length &&
      !splitList(draft.amount.direction.outboundValues).length &&
      !draft.amount.direction.default
    ) {
      issues.push("needDirectionValues");
    }
  }
  return issues;
}
