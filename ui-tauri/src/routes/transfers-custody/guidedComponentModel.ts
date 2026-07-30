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

/**
 * Resolved location + carried metadata for a leg loaded from an existing
 * component. When {@link GuidedLegForm.locationMode} is `"origin"`, the leg is
 * serialized from these already-resolved ids (matching the daemon's revise
 * contract) instead of the alias fields.
 */
export interface GuidedLegOrigin {
  transactionId?: string;
  anchorTransactionId?: string;
  walletId?: string;
  rail?: string;
  chain?: string;
  network?: string;
  exposure?: string;
  conservationUnit?: string;
}

export interface GuidedLegForm {
  /** Stable local id for React keys and allocation references. */
  key: string;
  /** Original persisted leg id when loaded for revise; "" for new legs. */
  originId: string;
  role: GuidedLegRole;
  /** Amount entered as a decimal BTC string; serialized as `amount_btc`. */
  amountBtc: string;
  /** "manual" = author the location via the alias fields; "origin" = keep a
   *  loaded component leg's resolved location. */
  locationMode: "manual" | "origin";
  origin: GuidedLegOrigin | null;
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
    originId: "",
    role,
    amountBtc: "",
    locationMode: "manual",
    origin: null,
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
  // Preserve the original leg id when revising so the daemon can match legs to
  // the prior revision (economic terms and per-leg evidence key on leg id).
  const spec: JsonRecord = { id: leg.originId || leg.key, role: leg.role };
  const amount = trimmed(leg.amountBtc);
  if (amount) spec.amount_btc = amount;

  if (leg.role === "suspense") {
    // A suspense leg has no wallet/transaction anchor — only its own time.
    const occurredAt = occurredAtToRfc3339(leg.occurredAt);
    if (occurredAt) spec.occurred_at = occurredAt;
  } else if (leg.locationMode === "origin" && leg.origin) {
    // Loaded from an existing component: emit the already-resolved location and
    // carried metadata directly (the daemon's revise contract), not aliases.
    // Emit BOTH transaction_id and wallet_id when present — the daemon does not
    // re-derive wallet_id from a transaction_id on revise, so dropping it would
    // silently persist wallet_id=NULL and change wallet-keyed validation.
    const origin = leg.origin;
    if (origin.transactionId) {
      spec.transaction_id = origin.transactionId;
      spec.anchor_transaction_id =
        origin.anchorTransactionId ?? origin.transactionId;
    }
    if (origin.walletId) spec.wallet_id = origin.walletId;
    if (!origin.transactionId) {
      const occurredAt = occurredAtToRfc3339(leg.occurredAt);
      if (occurredAt) spec.occurred_at = occurredAt;
    }
    if (origin.rail) spec.rail = origin.rail;
    if (origin.chain) spec.chain = origin.chain;
    if (origin.network) spec.network = origin.network;
    if (origin.exposure) spec.exposure = origin.exposure;
    if (origin.conservationUnit) spec.conservation_unit = origin.conservationUnit;
  } else {
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

    // Transaction anchors carry their own occurred_at; every other location
    // needs an explicit one for owned legs, so surface it when the user set it.
    if (leg.locationKind !== "transaction") {
      const occurredAt = occurredAtToRfc3339(leg.occurredAt);
      if (occurredAt) spec.occurred_at = occurredAt;
    }
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

const MSAT_PER_BTC = 100_000_000_000n;

/** Lossless exact-msat → BTC decimal string for a form amount input. */
export function msatToBtcInput(value: CustodyExactInteger): string {
  let msat: bigint;
  try {
    msat = BigInt(value);
  } catch {
    return "";
  }
  const negative = msat < 0n;
  const abs = negative ? -msat : msat;
  const whole = abs / MSAT_PER_BTC;
  const fraction = (abs % MSAT_PER_BTC)
    .toString()
    .padStart(11, "0")
    .replace(/0+$/, "");
  return `${negative ? "-" : ""}${whole}${fraction ? `.${fraction}` : ""}`;
}

/** RFC3339 → `datetime-local` input value (`YYYY-MM-DDTHH:mm`), local time. */
export function rfc3339ToDatetimeLocal(value: string | null | undefined): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  // Include seconds: a suspense leg must match its funding transaction's time
  // to the second, so minute truncation here would break activation on revise.
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  );
}

/** Structural subset of a persisted custody component used for editing. */
export interface CustodyComponentLegInput {
  id: string;
  role: string;
  rail?: string | null;
  chain?: string | null;
  network?: string | null;
  asset: string;
  exposure?: string | null;
  conservation_unit?: string | null;
  amount_msat: CustodyExactInteger;
  valuation_unit?: string | null;
  valuation_amount?: CustodyExactInteger | null;
  transaction_id: string | null;
  anchor_transaction_id?: string | null;
  wallet_id: string | null;
  occurred_at: string | null;
  notes?: string | null;
}

export interface CustodyComponentAllocationInput {
  source_leg_id: string;
  sink_leg_id: string;
  source_amount_msat: CustodyExactInteger;
  sink_amount_msat: CustodyExactInteger;
}

export interface CustodyComponentInput {
  component_type: string;
  conservation_mode: "quantity" | "conversion";
  evidence_kind?: string | null;
  evidence_grade?: string | null;
  conversion_policy?: string | null;
  conversion_reviewed?: boolean;
  notes?: string | null;
  legs: CustodyComponentLegInput[];
  allocations: CustodyComponentAllocationInput[];
}

/** The guided form authors these component types; others are system-derived. */
export function isGuidedEditableComponentType(componentType: string): boolean {
  return componentType === "manual_bridge" || componentType === "swap";
}

function guidedRole(role: string): GuidedLegRole {
  return (GUIDED_LEG_ROLES as readonly string[]).includes(role)
    ? (role as GuidedLegRole)
    : "external";
}

/** Reverse of {@link formToComponentSpec}: load an existing component to edit. */
export function componentToFormState(
  component: CustodyComponentInput,
): GuidedComponentFormState {
  const legIdToKey = new Map<string, string>();
  const legs = component.legs.map((leg) => {
    const base = createGuidedLeg(guidedRole(leg.role));
    legIdToKey.set(leg.id, base.key);
    const isSuspense = base.role === "suspense";
    const hasTransaction = Boolean(leg.transaction_id);
    const hasWallet = Boolean(leg.wallet_id);
    return {
      ...base,
      originId: leg.id,
      amountBtc: msatToBtcInput(leg.amount_msat),
      asset: leg.asset || "BTC",
      valuationUnit: leg.valuation_unit ?? "",
      valuationAmount:
        leg.valuation_amount === null || leg.valuation_amount === undefined
          ? ""
          : String(leg.valuation_amount),
      notes: leg.notes ?? "",
      occurredAt: rfc3339ToDatetimeLocal(leg.occurred_at),
      locationMode:
        !isSuspense && (hasTransaction || hasWallet)
          ? ("origin" as const)
          : ("manual" as const),
      origin:
        !isSuspense && (hasTransaction || hasWallet)
          ? {
              transactionId: leg.transaction_id ?? undefined,
              anchorTransactionId: leg.anchor_transaction_id ?? undefined,
              walletId: leg.wallet_id ?? undefined,
              rail: leg.rail ?? undefined,
              chain: leg.chain ?? undefined,
              network: leg.network ?? undefined,
              exposure: leg.exposure ?? undefined,
              conservationUnit: leg.conservation_unit ?? undefined,
            }
          : null,
    } satisfies GuidedLegForm;
  });

  const allocations = component.allocations.map((allocation) => ({
    key: nextKey("alloc"),
    sourceKey: legIdToKey.get(allocation.source_leg_id) ?? "",
    sinkKey: legIdToKey.get(allocation.sink_leg_id) ?? "",
    amountBtc: msatToBtcInput(allocation.source_amount_msat),
  }));

  return {
    componentType: component.component_type === "swap" ? "swap" : "manual_bridge",
    conservationMode: component.conservation_mode,
    evidenceKind: component.evidence_kind ?? "",
    evidenceGrade: component.evidence_grade ?? "",
    conversionPolicy: component.conversion_policy ?? "",
    conversionReviewed: component.conversion_reviewed ?? false,
    notes: component.notes ?? "",
    legs,
    allocations,
  };
}
