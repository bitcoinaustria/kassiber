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
  originalOccurredAt?: string;
  originalOccurredAtInput?: string;
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
  /** Source allocation in BTC units. Quantity-mode edits mirror the sink. */
  amountBtc: string;
  sinkAmountBtc: string;
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
  return { key: nextKey("alloc"), sourceKey: "", sinkKey: "", amountBtc: "", sinkAmountBtc: "" };
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

function legOccurredAt(leg: GuidedLegForm): string {
  return leg.originalOccurredAt && leg.occurredAt === leg.originalOccurredAtInput
    ? leg.originalOccurredAt
    : occurredAtToRfc3339(leg.occurredAt);
}

function legToSpec(leg: GuidedLegForm, mode: "quantity" | "conversion"): JsonRecord {
  // Preserve the original leg id when revising so the daemon can match legs to
  // the prior revision (economic terms and per-leg evidence key on leg id).
  const spec: JsonRecord = { id: leg.originId || leg.key, role: leg.role };
  const amount = trimmed(leg.amountBtc);
  if (amount) spec.amount_btc = amount;

  if (leg.role === "suspense") {
    // A suspense leg has no wallet/transaction anchor — only its own time.
    const occurredAt = legOccurredAt(leg);
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
    if (origin.anchorTransactionId) spec.anchor_transaction_id = origin.anchorTransactionId;
    if (origin.walletId) spec.wallet_id = origin.walletId;
    const occurredAt = legOccurredAt(leg);
    if (occurredAt) spec.occurred_at = occurredAt;
  } else {
    // Changing a persisted location must not revive its hidden old location_ref.
    if (leg.originId) spec.location_ref = null;
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
      const occurredAt = legOccurredAt(leg);
      if (occurredAt) spec.occurred_at = occurredAt;
    }
  }

  if (leg.locationMode === "origin" && leg.origin) {
    const origin = leg.origin;
    if (origin.rail) spec.rail = origin.rail;
    if (origin.chain) spec.chain = origin.chain;
    if (origin.network) spec.network = origin.network;
    if (origin.exposure) spec.exposure = origin.exposure;
    if (origin.conservationUnit) spec.conservation_unit = origin.conservationUnit;
  }

  const asset = trimmed(leg.asset);
  if (asset && (leg.locationMode === "origin" || asset.toUpperCase() !== "BTC")) spec.asset = asset;

  if (mode === "conversion" || leg.locationMode === "origin") {
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
  mode: "quantity" | "conversion",
): JsonRecord {
  const sourceOrdinal = legs.findIndex((leg) => leg.key === allocation.sourceKey);
  const sinkOrdinal = legs.findIndex((leg) => leg.key === allocation.sinkKey);
  const msat = decimalBtcToMsat(trimmed(allocation.amountBtc));
  const amount: CustodyExactInteger = msat === null ? "" : msat.toString();
  return {
    source_ordinal: sourceOrdinal,
    sink_ordinal: sinkOrdinal,
    source_amount_msat: amount,
    sink_amount_msat: mode === "conversion" || trimmed(allocation.sinkAmountBtc) !== ""
      ? (decimalBtcToMsat(trimmed(allocation.sinkAmountBtc))?.toString() ?? "")
      : amount,
  };
}

/** Serialize the form into the exact custody-component spec object. */
export function formToComponentSpec(form: GuidedComponentFormState): JsonRecord {
  const spec: JsonRecord = { component_type: form.componentType, conservation_mode: form.conservationMode };
  const evidenceKind = trimmed(form.evidenceKind);
  const evidenceGrade = trimmed(form.evidenceGrade);
  if (evidenceKind) spec.evidence_kind = evidenceKind;
  if (evidenceGrade) spec.evidence_grade = evidenceGrade;
  spec.conversion_policy = trimmed(form.conversionPolicy) || null;
  spec.conversion_reviewed = form.conversionReviewed;
  const notes = trimmed(form.notes);
  spec.notes = notes;
  spec.legs = form.legs.map((leg) => legToSpec(leg, form.conservationMode));
  const allocations = form.allocations
    .map((allocation) => allocationToSpec(allocation, form.legs, form.conservationMode));
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

/** RFC3339 → `datetime-local` input value (`YYYY-MM-DDTHH:mm:ss`), local time. */
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
  if (!(GUIDED_LEG_ROLES as readonly string[]).includes(role)) {
    throw new Error("guided_component_shape_unsupported");
  }
  return role as GuidedLegRole;
}

/** Reverse of {@link formToComponentSpec}: load an existing component to edit. */
export function componentToFormState(
  component: CustodyComponentInput,
): GuidedComponentFormState {
  if (!isGuidedEditableComponentType(component.component_type) ||
      !["quantity", "conversion"].includes(component.conservation_mode) ||
      component.legs.some((leg) =>
        (leg.conservation_unit && leg.conservation_unit !== "msat") ||
        (typeof leg.amount_msat === "number" && !Number.isSafeInteger(leg.amount_msat)) ||
        (leg.role === "suspense" && (leg.transaction_id || leg.wallet_id || leg.anchor_transaction_id))) ||
      new Set(component.legs.map((leg) => leg.id)).size !== component.legs.length ||
      component.allocations.some((allocation) =>
        !component.legs.some((leg) => leg.id === allocation.source_leg_id) ||
        !component.legs.some((leg) => leg.id === allocation.sink_leg_id))) {
    throw new Error("guided_component_shape_unsupported");
  }
  const legIdToKey = new Map<string, string>();
  const legs = component.legs.map((leg) => {
    const base = createGuidedLeg(guidedRole(leg.role));
    legIdToKey.set(leg.id, base.key);
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
      originalOccurredAt: leg.occurred_at ?? undefined,
      originalOccurredAtInput: rfc3339ToDatetimeLocal(leg.occurred_at),
      locationMode: "origin" as const,
      origin: {
        transactionId: leg.transaction_id ?? undefined,
        anchorTransactionId: leg.anchor_transaction_id ?? undefined,
        walletId: leg.wallet_id ?? undefined,
        rail: leg.rail ?? undefined,
        chain: leg.chain ?? undefined,
        network: leg.network ?? undefined,
        exposure: leg.exposure ?? undefined,
        conservationUnit: leg.conservation_unit ?? undefined,
      },
    } satisfies GuidedLegForm;
  });

  const allocations = component.allocations.map((allocation) => ({
    key: nextKey("alloc"),
    sourceKey: legIdToKey.get(allocation.source_leg_id) ?? "",
    sinkKey: legIdToKey.get(allocation.sink_leg_id) ?? "",
    amountBtc: msatToBtcInput(allocation.source_amount_msat),
    sinkAmountBtc: msatToBtcInput(allocation.sink_amount_msat),
  }));

  return {
    componentType: component.component_type as "manual_bridge" | "swap",
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

/** Bind an explicit confirmation to the exact reviewed form and active book. */
export interface GuidedPlanCommitment {
  document: string;
  scope: string;
  inputVersion: number;
}

export function canConfirmGuidedPlan(
  commitment: GuidedPlanCommitment | null,
  document: string,
  scope: string,
): boolean {
  return commitment !== null && Number.isSafeInteger(commitment.inputVersion) &&
    commitment.inputVersion >= 0 && commitment.document === document && commitment.scope === scope;
}

/** Keep the untouched builder quiet; validate entered movements and loaded revisions. */
export function hasGuidedFormInput(form: GuidedComponentFormState): boolean {
  return Boolean(form.notes.trim()) || form.legs.some((leg) =>
    [leg.amountBtc, leg.transactionRef, leg.walletRef, leg.untrackedWallet,
      leg.occurredAt, leg.valuationUnit, leg.valuationAmount, leg.notes].some((value) => value.trim() !== ""),
  ) || form.allocations.some((allocation) =>
    [allocation.sourceKey, allocation.sinkKey, allocation.amountBtc, allocation.sinkAmountBtc]
      .some((value) => value.trim() !== ""),
  );
}

/** Apply to the daemon scope actually reviewed, never a persisted display identity. */
export function bindGuidedPlanScope(
  args: Record<string, unknown>,
  plan: { workspace_id?: string; profile_id?: string },
): Record<string, unknown> | null {
  if (typeof plan.workspace_id !== "string" || !plan.workspace_id.trim() ||
      typeof plan.profile_id !== "string" || !plan.profile_id.trim()) return null;
  return { ...args, workspace: plan.workspace_id, profile: plan.profile_id };
}
