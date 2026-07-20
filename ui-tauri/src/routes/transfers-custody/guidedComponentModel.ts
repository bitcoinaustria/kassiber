/**
 * Form-state model for the guided custody-component builder.
 *
 * The builder is a structured editor over the very same component-spec object
 * the daemon's `ui.transfers.components.{plan,apply}` kinds accept — the object
 * that used to be hand-written as JSON. {@link formToDocument} serializes the
 * form to that document so it can be validated locally by
 * {@link previewCustodyComponentBatch} and submitted with
 * {@link buildCustodyBulkRequest}, with no JSON typed by the user.
 */
import {
  decimalBtcToMsat,
  type CustodyExactInteger,
} from "@/lib/custodyComponentBulk";

export type GuidedLegRole =
  | "source"
  | "destination"
  | "fee"
  | "external"
  | "retained"
  | "suspense";

export type GuidedLocationKind = "transaction" | "wallet" | "untracked";

export interface GuidedLegForm {
  /** Stable local id for React keys and allocation references. */
  key: string;
  role: GuidedLegRole;
  /** Amount entered as a decimal BTC string; serialized as `amount_btc`. */
  amountBtc: string;
  locationKind: GuidedLocationKind;
  transactionRef: string;
  walletRef: string;
  untrackedWallet: string;
  /** `datetime-local` value; converted to RFC3339 UTC on serialize. */
  occurredAt: string;
  asset: string;
  /** Conversion mode only. */
  valuationUnit: string;
  valuationAmount: string;
  notes: string;
}

export interface GuidedAllocationForm {
  key: string;
  sourceKey: string;
  sinkKey: string;
  /** Allocated amount as a decimal BTC string (quantity mode: source = sink). */
  amountBtc: string;
}

export interface GuidedComponentFormState {
  componentType: "manual_bridge" | "swap";
  conservationMode: "quantity" | "conversion";
  evidenceKind: string;
  evidenceGrade: string;
  conversionPolicy: string;
  conversionReviewed: boolean;
  notes: string;
  legs: GuidedLegForm[];
  allocations: GuidedAllocationForm[];
}

export const GUIDED_LEG_ROLES: readonly GuidedLegRole[] = [
  "source",
  "destination",
  "fee",
  "retained",
  "external",
  "suspense",
];

const SINK_ROLES = new Set<GuidedLegRole>([
  "destination",
  "fee",
  "external",
  "retained",
  "suspense",
]);

export function isSinkRole(role: GuidedLegRole): boolean {
  return SINK_ROLES.has(role);
}

let keyCounter = 0;
function nextKey(prefix: string): string {
  keyCounter += 1;
  return `${prefix}-${keyCounter}`;
}

export function createGuidedLeg(role: GuidedLegRole): GuidedLegForm {
  return {
    key: nextKey("leg"),
    role,
    amountBtc: "",
    locationKind: "transaction",
    transactionRef: "",
    walletRef: "",
    untrackedWallet: "",
    occurredAt: "",
    asset: "BTC",
    valuationUnit: "",
    valuationAmount: "",
    notes: "",
  };
}

export function createGuidedAllocation(): GuidedAllocationForm {
  return { key: nextKey("alloc"), sourceKey: "", sinkKey: "", amountBtc: "" };
}

/** A sensible starting point: a one-source migration with a destination + fee. */
export function createInitialGuidedForm(): GuidedComponentFormState {
  return {
    componentType: "manual_bridge",
    conservationMode: "quantity",
    evidenceKind: "manual_migration_review",
    evidenceGrade: "reviewed",
    conversionPolicy: "",
    conversionReviewed: false,
    notes: "",
    legs: [
      createGuidedLeg("source"),
      createGuidedLeg("destination"),
      createGuidedLeg("fee"),
    ],
    allocations: [],
  };
}

/** Owned legs that carry basis; when transactionless they require occurred_at. */
export function isOwnedRole(role: GuidedLegRole): boolean {
  return role === "source" || role === "destination" || role === "retained";
}

function trimmed(value: string): string {
  return value.trim();
}

/** Convert a `datetime-local` value to RFC3339 UTC, or "" when unset/invalid. */
export function occurredAtToRfc3339(value: string): string {
  const raw = trimmed(value);
  if (!raw) return "";
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return raw;
  return parsed.toISOString();
}

type JsonRecord = Record<string, unknown>;

function legToSpec(leg: GuidedLegForm, mode: "quantity" | "conversion"): JsonRecord {
  const spec: JsonRecord = { id: leg.key, role: leg.role };
  const amount = trimmed(leg.amountBtc);
  if (amount) spec.amount_btc = amount;

  switch (leg.locationKind) {
    case "transaction": {
      const ref = trimmed(leg.transactionRef);
      if (ref) spec.transaction = ref;
      break;
    }
    case "wallet": {
      const ref = trimmed(leg.walletRef);
      if (ref) spec.wallet = ref;
      break;
    }
    case "untracked": {
      const ref = trimmed(leg.untrackedWallet);
      if (ref) spec.untracked_wallet = ref;
      break;
    }
  }

  // Transaction anchors carry their own occurred_at; every other location needs
  // an explicit one for owned legs, so surface it whenever the user set it.
  if (leg.locationKind !== "transaction") {
    const occurredAt = occurredAtToRfc3339(leg.occurredAt);
    if (occurredAt) spec.occurred_at = occurredAt;
  }

  const asset = trimmed(leg.asset);
  if (asset && asset.toUpperCase() !== "BTC") spec.asset = asset;

  if (mode === "conversion") {
    const valuationUnit = trimmed(leg.valuationUnit);
    const valuationAmount = trimmed(leg.valuationAmount);
    if (valuationUnit) spec.valuation_unit = valuationUnit;
    if (valuationAmount) spec.valuation_amount = valuationAmount;
  }

  const notes = trimmed(leg.notes);
  if (notes) spec.notes = notes;
  return spec;
}

function allocationToSpec(
  allocation: GuidedAllocationForm,
  legs: GuidedLegForm[],
): JsonRecord | null {
  const sourceOrdinal = legs.findIndex((leg) => leg.key === allocation.sourceKey);
  const sinkOrdinal = legs.findIndex((leg) => leg.key === allocation.sinkKey);
  if (sourceOrdinal < 0 || sinkOrdinal < 0) return null;
  const msat = decimalBtcToMsat(trimmed(allocation.amountBtc));
  const amount: CustodyExactInteger = msat === null ? "" : msat.toString();
  return {
    source_ordinal: sourceOrdinal,
    sink_ordinal: sinkOrdinal,
    source_amount_msat: amount,
    sink_amount_msat: amount,
  };
}

/** Serialize the form into the exact custody-component spec object. */
export function formToComponentSpec(form: GuidedComponentFormState): JsonRecord {
  const spec: JsonRecord = { component_type: form.componentType };
  if (form.conservationMode === "conversion") {
    spec.conservation_mode = "conversion";
  }
  const evidenceKind = trimmed(form.evidenceKind);
  const evidenceGrade = trimmed(form.evidenceGrade);
  if (evidenceKind) spec.evidence_kind = evidenceKind;
  if (evidenceGrade) spec.evidence_grade = evidenceGrade;
  if (form.conservationMode === "conversion") {
    const policy = trimmed(form.conversionPolicy);
    if (policy) spec.conversion_policy = policy;
    spec.conversion_reviewed = form.conversionReviewed;
  }
  const notes = trimmed(form.notes);
  if (notes) spec.notes = notes;
  spec.legs = form.legs.map((leg) => legToSpec(leg, form.conservationMode));
  const allocations = form.allocations
    .map((allocation) => allocationToSpec(allocation, form.legs))
    .filter((value): value is JsonRecord => value !== null);
  if (allocations.length > 0) spec.allocations = allocations;
  return spec;
}

/** Serialize to the `{ components: [...] }` document the batch APIs consume. */
export function formToDocument(form: GuidedComponentFormState): string {
  return JSON.stringify({ components: [formToComponentSpec(form)] });
}
