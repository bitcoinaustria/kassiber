const CUSTODY_COMPONENT_TYPES = [
  "native_transfer",
  "channel_lifecycle",
  "peg",
  "swap",
  "refund",
  "manual_bridge",
] as const;

const CUSTODY_LEG_ROLES = [
  "source",
  "destination",
  "fee",
  "external",
  "retained",
  "unresolved",
  "suspense",
] as const;

type JsonRecord = Record<string, unknown>;

/**
 * Exact custody quantities use numbers while they fit JavaScript's integer
 * range and unsigned decimal strings beyond it.  This mirrors the daemon wire
 * contract without forcing every existing consumer to handle strings.
 */
export type CustodyExactInteger = number | string;

export type CustodyPreviewIssueCode =
  | "jsonInvalid"
  | "documentRequired"
  | "componentObjectRequired"
  | "componentTypeUnsupported"
  | "conservationModeUnsupported"
  | "legsRequired"
  | "legObjectRequired"
  | "roleUnsupported"
  | "legIdDuplicate"
  | "amountInvalid"
  | "transactionlessWalletRequired"
  | "transactionlessTimeRequired"
  | "valuationPairRequired"
  | "valuationAmountInvalid"
  | "valuationTokenInvalid"
  | "conversionPolicyInvalid"
  | "conversionReviewedInvalid"
  | "conversionValuationRequired"
  | "sourceRequired"
  | "ownedDestinationRequired"
  | "anchorRequired"
  | "unresolvedValue"
  | "suspenseReviewRequired"
  | "suspenseQuantityModeRequired"
  | "suspenseLocationInvalid"
  | "suspenseTimeRequired"
  | "suspenseAllocationRequired"
  | "suspenseObservedSourceRequired"
  | "suspenseAssetMismatch"
  | "suspenseTimeMismatch"
  | "quantityUnbalanced"
  | "conversionReviewRequired"
  | "conversionValuationUnbalanced"
  | "conversionTopologyUnsupported"
  | "allocationsInvalid"
  | "allocationsRequired"
  | "allocationObjectRequired"
  | "allocationSourceInvalid"
  | "allocationSinkInvalid"
  | "allocationAmountInvalid"
  | "allocationEdgeDuplicate"
  | "allocationQuantityMismatch"
  | "allocationSourceCoverage"
  | "allocationSinkCoverage";

export interface CustodyPreviewIssue {
  code: CustodyPreviewIssueCode;
  values?: Record<string, string | number>;
}

export interface CustodyBatchPreview {
  components: JsonRecord[];
  structuralErrors: CustodyPreviewIssue[];
  activationErrors: CustodyPreviewIssue[];
  summary: {
    components: number;
    legs: number;
    sources: number;
    destinations: number;
    transactionAnchors: number;
    untrackedLegs: number;
    unresolvedLegs: number;
    suspenseLegs: number;
  };
}

const COMPONENT_TYPES = new Set<string>(CUSTODY_COMPONENT_TYPES);
const LEG_ROLES = new Set<string>(CUSTODY_LEG_ROLES);
const SINK_ROLES = new Set<string>([
  "destination",
  "fee",
  "external",
  "retained",
  "unresolved",
  "suspense",
]);
const SQLITE_MAX_INTEGER = 9_223_372_036_854_775_807n;

function isRecord(value: unknown): value is JsonRecord {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function exactNonnegativeIntegerAsBigInt(value: unknown): bigint | null {
  if (Number.isSafeInteger(value) && (value as number) >= 0) {
    return BigInt(value as number);
  }
  if (typeof value !== "string" || !/^[0-9]+$/.test(value)) return null;
  const normalized = value.replace(/^0+(?=\d)/, "");
  // SQLite's signed integer maximum has 19 digits.  Bound the string before
  // BigInt parsing so arbitrarily large pasted values cannot become expensive.
  if (normalized.length > 19) return null;
  const parsed = BigInt(normalized);
  return parsed <= SQLITE_MAX_INTEGER ? parsed : null;
}

function exactNonnegativeInteger(value: unknown): value is CustodyExactInteger {
  return exactNonnegativeIntegerAsBigInt(value) !== null;
}

function issue(
  code: CustodyPreviewIssueCode,
  values?: Record<string, string | number>,
): CustodyPreviewIssue {
  return values ? { code, values } : { code };
}

function pow10(exponent: number): bigint {
  return 10n ** BigInt(exponent);
}

/** Match the daemon's Decimal + ROUND_HALF_UP BTC-to-msat boundary exactly. */
function decimalBtcToMsat(value: string): bigint | null {
  if (value.length > 128) return null;
  const match = value
    .trim()
    .match(/^\+?(?:(\d+)(?:\.(\d*))?|\.(\d+))(?:[eE]([+-]?\d+))?$/);
  if (!match) return null;
  const whole = match[1] ?? "0";
  const fraction = match[1] === undefined ? (match[3] ?? "") : (match[2] ?? "");
  const exponent = Number.parseInt(match[4] ?? "0", 10);
  if (!Number.isSafeInteger(exponent) || Math.abs(exponent) > 1000) return null;
  const coefficientDigits = `${whole}${fraction}`.replace(/^0+(?=\d)/, "");
  // A valid SQLite msat result never needs an unbounded decimal coefficient.
  // Apply the same 19-digit ceiling before BigInt construction that direct
  // amount_msat strings use.
  if (coefficientDigits.length > 19) return null;
  const coefficient = BigInt(coefficientDigits);
  const scale = exponent + 11 - fraction.length;
  if (scale >= 0) return coefficient * pow10(scale);
  const divisor = pow10(-scale);
  const quotient = coefficient / divisor;
  const remainder = coefficient % divisor;
  return quotient + (remainder * 2n >= divisor ? 1n : 0n);
}

function amountIsStructurallyValid(leg: JsonRecord): boolean {
  const hasMsat = Object.prototype.hasOwnProperty.call(leg, "amount_msat");
  const hasBtc = Object.prototype.hasOwnProperty.call(leg, "amount_btc");
  if (hasMsat === hasBtc) return false;
  if (hasMsat) return exactNonnegativeInteger(leg.amount_msat);
  if (typeof leg.amount_btc === "number") {
    if (!Number.isFinite(leg.amount_btc) || leg.amount_btc < 0) return false;
    const amount = decimalBtcToMsat(String(leg.amount_btc));
    return amount !== null && amount <= SQLITE_MAX_INTEGER;
  }
  if (typeof leg.amount_btc !== "string") return false;
  const amount = decimalBtcToMsat(leg.amount_btc);
  return amount !== null && amount <= SQLITE_MAX_INTEGER;
}

function amountAsMsat(leg: JsonRecord): bigint | null {
  const exactMsat = exactNonnegativeIntegerAsBigInt(leg.amount_msat);
  if (exactMsat !== null) return exactMsat;
  if (typeof leg.amount_btc === "string") {
    return decimalBtcToMsat(leg.amount_btc);
  }
  if (typeof leg.amount_btc === "number" && Number.isFinite(leg.amount_btc)) {
    return decimalBtcToMsat(String(leg.amount_btc));
  }
  return null;
}

function normalizedAsset(leg: JsonRecord) {
  return hasText(leg.asset) ? leg.asset.trim().toUpperCase() : "BTC";
}

function normalizedExposure(leg: JsonRecord) {
  if (hasText(leg.exposure)) return leg.exposure.trim();
  const asset = normalizedAsset(leg);
  return asset === "BTC" || asset === "LBTC" ? "bitcoin" : asset.toLowerCase();
}

function normalizedUnit(leg: JsonRecord) {
  if (hasText(leg.conservation_unit)) return leg.conservation_unit.trim();
  const asset = normalizedAsset(leg);
  return asset === "BTC" || asset === "LBTC" ? "msat" : "asset-quantum";
}

function legHasTransactionAnchor(leg: JsonRecord) {
  return [
    leg.transaction,
    leg.transaction_ref,
    leg.transaction_id,
    leg.anchor_transaction_id,
  ].some(hasText);
}

function legHasWallet(leg: JsonRecord) {
  return [leg.wallet, leg.wallet_ref, leg.wallet_id, leg.untracked_wallet].some(hasText);
}

function allocationOrdinal(
  allocation: JsonRecord,
  kind: "source" | "sink",
  legs: JsonRecord[],
): number | null {
  const rawOrdinal = allocation[`${kind}_ordinal`];
  if (Number.isInteger(rawOrdinal) && (rawOrdinal as number) >= 0) {
    return rawOrdinal as number;
  }
  const rawId = allocation[`${kind}_leg_id`];
  if (!hasText(rawId)) return null;
  const index = legs.findIndex((leg) => leg.id === rawId);
  return index >= 0 ? index : null;
}

/**
 * Parse and locally validate a bulk custody-component document.
 *
 * Structural errors would make even a draft invalid at the daemon boundary.
 * Activation errors are intentionally separate: a draft may preserve an
 * incomplete migration chain while the user finds the missing wallet or fee.
 */
export function previewCustodyComponentBatch(text: string): CustodyBatchPreview {
  const structuralErrors: CustodyPreviewIssue[] = [];
  const activationErrors: CustodyPreviewIssue[] = [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    const message = error instanceof Error ? error.message : "Invalid JSON";
    return {
      components: [],
      structuralErrors: [issue("jsonInvalid", { detail: message })],
      activationErrors: [],
      summary: {
        components: 0,
        legs: 0,
        sources: 0,
        destinations: 0,
        transactionAnchors: 0,
        untrackedLegs: 0,
        unresolvedLegs: 0,
        suspenseLegs: 0,
      },
    };
  }

  const rawComponents = Array.isArray(parsed)
    ? parsed
    : isRecord(parsed) && Array.isArray(parsed.components)
      ? parsed.components
      : null;
  if (!rawComponents || rawComponents.length === 0) {
    return {
      components: [],
      structuralErrors: [issue("documentRequired")],
      activationErrors: [],
      summary: {
        components: 0,
        legs: 0,
        sources: 0,
        destinations: 0,
        transactionAnchors: 0,
        untrackedLegs: 0,
        unresolvedLegs: 0,
        suspenseLegs: 0,
      },
    };
  }

  const components = rawComponents.filter(isRecord);
  if (components.length !== rawComponents.length) {
    rawComponents.forEach((component, index) => {
      if (!isRecord(component)) {
        structuralErrors.push(issue("componentObjectRequired", { component: index + 1 }));
      }
    });
  }

  let legCount = 0;
  let sourceCount = 0;
  let destinationCount = 0;
  let transactionAnchors = 0;
  let untrackedLegs = 0;
  let unresolvedLegs = 0;
  let suspenseLegs = 0;

  rawComponents.forEach((rawComponent, componentIndex) => {
    if (!isRecord(rawComponent)) return;
    const componentType = rawComponent.component_type ?? "manual_bridge";
    if (!hasText(componentType) || !COMPONENT_TYPES.has(componentType)) {
      structuralErrors.push(
        issue("componentTypeUnsupported", { component: componentIndex + 1 }),
      );
    }
    const mode = rawComponent.conservation_mode ?? "quantity";
    if (mode !== "quantity" && mode !== "conversion") {
      structuralErrors.push(
        issue("conservationModeUnsupported", { component: componentIndex + 1 }),
      );
    }
    if (
      rawComponent.conversion_policy !== undefined &&
      rawComponent.conversion_policy !== null &&
      (typeof rawComponent.conversion_policy !== "string" ||
        (rawComponent.conversion_policy.trim().length > 0 &&
          !/^[a-z][a-z0-9_.:-]*$/.test(rawComponent.conversion_policy.trim())))
    ) {
      structuralErrors.push(
        issue("conversionPolicyInvalid", { component: componentIndex + 1 }),
      );
    }
    if (
      rawComponent.conversion_reviewed !== undefined &&
      typeof rawComponent.conversion_reviewed !== "boolean"
    ) {
      structuralErrors.push(
        issue("conversionReviewedInvalid", { component: componentIndex + 1 }),
      );
    }
    if (!Array.isArray(rawComponent.legs) || rawComponent.legs.length < 2) {
      structuralErrors.push(issue("legsRequired", { component: componentIndex + 1 }));
      return;
    }

    const legs = rawComponent.legs.filter(isRecord);
    legCount += rawComponent.legs.length;
    if (legs.length !== rawComponent.legs.length) {
      rawComponent.legs.forEach((leg, legIndex) => {
        if (!isRecord(leg)) {
          structuralErrors.push(
            issue("legObjectRequired", {
              component: componentIndex + 1,
              leg: legIndex + 1,
            }),
          );
        }
      });
    }

    const ids = new Set<string>();
    const balance = new Map<string, { source: bigint; sink: bigint }>();
    const valuationBalance = new Map<string, { source: bigint; sink: bigint }>();
    let componentSources = 0;
    let componentOwnedDestinations = 0;
    let componentAnchors = 0;
    let componentUnresolved = 0;
    let componentSuspense = 0;

    legs.forEach((leg, legIndex) => {
      const values = { component: componentIndex + 1, leg: legIndex + 1 };
      if (!hasText(leg.role) || !LEG_ROLES.has(leg.role)) {
        structuralErrors.push(issue("roleUnsupported", values));
        return;
      }
      if (hasText(leg.id)) {
        if (ids.has(leg.id)) {
          structuralErrors.push(issue("legIdDuplicate", { ...values, id: leg.id }));
        }
        ids.add(leg.id);
      }
      if (!amountIsStructurallyValid(leg)) {
        structuralErrors.push(issue("amountInvalid", values));
      }
      const amount = amountAsMsat(leg);
      const isMaterialOwnedLeg =
        amount !== null &&
        amount > 0n &&
        ["source", "destination", "retained"].includes(leg.role);
      if (isMaterialOwnedLeg && !legHasTransactionAnchor(leg)) {
        if (!legHasWallet(leg)) {
          structuralErrors.push(issue("transactionlessWalletRequired", values));
        }
        if (!hasText(leg.occurred_at)) {
          structuralErrors.push(issue("transactionlessTimeRequired", values));
        }
        untrackedLegs += 1;
      }
      if (legHasTransactionAnchor(leg)) {
        transactionAnchors += 1;
        componentAnchors += 1;
      }
      if (leg.role === "source" && amount !== null && amount > 0n) {
        sourceCount += 1;
        componentSources += 1;
      }
      if (
        ["destination", "retained"].includes(leg.role) &&
        amount !== null &&
        amount > 0n
      ) {
        destinationCount += 1;
        componentOwnedDestinations += 1;
      }
      if (leg.role === "unresolved" && amount !== null && amount > 0n) {
        unresolvedLegs += 1;
        componentUnresolved += 1;
      }
      if (leg.role === "suspense" && amount !== null && amount > 0n) {
        suspenseLegs += 1;
        componentSuspense += 1;
        if (legHasTransactionAnchor(leg) || legHasWallet(leg)) {
          activationErrors.push(issue("suspenseLocationInvalid", values));
        }
        if (!hasText(leg.occurred_at)) {
          activationErrors.push(issue("suspenseTimeRequired", values));
        }
      }
      if (amount !== null) {
        const key = `${normalizedExposure(leg)}\u0000${normalizedUnit(leg)}`;
        const row = balance.get(key) ?? { source: 0n, sink: 0n };
        if (leg.role === "source") row.source += amount;
        else if (SINK_ROLES.has(leg.role)) row.sink += amount;
        balance.set(key, row);
      }

      const valuationUnit = hasText(leg.valuation_unit)
        ? leg.valuation_unit.trim()
        : null;
      const valuationUnitPresent = valuationUnit !== null;
      const valuationAmountProvided =
        leg.valuation_amount !== undefined && leg.valuation_amount !== null;
      const valuationAmount = exactNonnegativeIntegerAsBigInt(
        leg.valuation_amount,
      );
      const valuationAmountValid = valuationAmount !== null;
      if (valuationUnitPresent !== valuationAmountProvided) {
        structuralErrors.push(issue("valuationPairRequired", values));
      }
      if (valuationAmountProvided && !valuationAmountValid) {
        structuralErrors.push(issue("valuationAmountInvalid", values));
      }
      if (
        valuationUnitPresent &&
        !/^[a-z][a-z0-9_.:-]*$/.test(valuationUnit)
      ) {
        structuralErrors.push(issue("valuationTokenInvalid", values));
      }
      if (mode === "conversion") {
        const material =
          (amount !== null && amount > 0n) ||
          (valuationAmount !== null && valuationAmount > 0n);
        if (material && (!valuationUnitPresent || !valuationAmountValid)) {
          activationErrors.push(issue("conversionValuationRequired", values));
        }
        if (valuationUnit !== null && valuationAmount !== null) {
          const row = valuationBalance.get(valuationUnit) ?? {
            source: 0n,
            sink: 0n,
          };
          if (leg.role === "source") row.source += valuationAmount;
          else if (SINK_ROLES.has(leg.role)) row.sink += valuationAmount;
          valuationBalance.set(valuationUnit, row);
        }
      }
    });

    if (componentSources === 0) {
      activationErrors.push(issue("sourceRequired", { component: componentIndex + 1 }));
    }
    if (componentOwnedDestinations === 0) {
      activationErrors.push(
        issue("ownedDestinationRequired", { component: componentIndex + 1 }),
      );
    }
    if (componentAnchors === 0) {
      activationErrors.push(
        issue("anchorRequired", { component: componentIndex + 1 }),
      );
    }
    if (componentUnresolved > 0) {
      activationErrors.push(
        issue("unresolvedValue", { component: componentIndex + 1 }),
      );
    }
    if (componentSuspense > 0) {
      if (
        componentType !== "manual_bridge" ||
        !hasText(rawComponent.evidence_grade) ||
        rawComponent.evidence_grade.trim().toLowerCase() !== "reviewed"
      ) {
        activationErrors.push(
          issue("suspenseReviewRequired", { component: componentIndex + 1 }),
        );
      }
      if (mode !== "quantity") {
        activationErrors.push(
          issue("suspenseQuantityModeRequired", {
            component: componentIndex + 1,
          }),
        );
      }
    }

    if (mode === "quantity") {
      for (const [key, amounts] of balance) {
        if (amounts.source !== amounts.sink) {
          const [exposure, unit] = key.split("\u0000");
          activationErrors.push(
            issue("quantityUnbalanced", {
              component: componentIndex + 1,
              exposure,
              unit,
              sources: amounts.source.toString(),
              sinks: amounts.sink.toString(),
            }),
          );
        }
      }
    } else {
      if (
        !hasText(rawComponent.conversion_policy) ||
        rawComponent.conversion_reviewed !== true
      ) {
        activationErrors.push(
          issue("conversionReviewRequired", { component: componentIndex + 1 }),
        );
      }
      if (valuationBalance.size === 0) {
        activationErrors.push(
          issue("conversionValuationRequired", {
            component: componentIndex + 1,
            leg: 0,
          }),
        );
      }
      for (const [unit, amounts] of valuationBalance) {
        if (amounts.source !== amounts.sink) {
          activationErrors.push(
            issue("conversionValuationUnbalanced", {
              component: componentIndex + 1,
              unit,
              sources: amounts.source.toString(),
              sinks: amounts.sink.toString(),
            }),
          );
        }
      }
    }

    const positiveSources = legs.filter(
      (leg) => leg.role === "source" && (amountAsMsat(leg) ?? 0n) > 0n,
    );
    const positiveSinks = legs.filter(
      (leg) =>
        SINK_ROLES.has(String(leg.role)) && (amountAsMsat(leg) ?? 0n) > 0n,
    );
    const attributedSinks = positiveSinks.filter((leg) =>
      ["fee", "external", "unresolved", "suspense"].includes(String(leg.role)),
    );
    const ownedSinks = positiveSinks.filter((leg) =>
      ["destination", "retained"].includes(String(leg.role)),
    );
    const needsAllocations =
      componentSuspense > 0
        ? true
        : mode === "conversion"
        ? positiveSources.length !== 1 || positiveSinks.length !== 1
        : positiveSources.length > 1 &&
          !(ownedSinks.length === 1 && attributedSinks.length === 0);
    if (
      mode === "conversion" &&
      (positiveSources.length !== 1 || ownedSinks.length !== 1)
    ) {
      activationErrors.push(
        issue("conversionTopologyUnsupported", { component: componentIndex + 1 }),
      );
    }
    if (
      rawComponent.allocations !== undefined &&
      rawComponent.allocations !== null &&
      !Array.isArray(rawComponent.allocations)
    ) {
      structuralErrors.push(
        issue("allocationsInvalid", { component: componentIndex + 1 }),
      );
    }
    const allocations = Array.isArray(rawComponent.allocations)
      ? rawComponent.allocations
      : [];
    if (needsAllocations && allocations.length === 0) {
      activationErrors.push(
        issue(
          componentSuspense > 0
            ? "suspenseAllocationRequired"
            : "allocationsRequired",
          { component: componentIndex + 1 },
        ),
      );
    }
    const sourceCoverage = new Map<number, bigint>();
    const sinkCoverage = new Map<number, bigint>();
    const allocationEdges = new Set<string>();
    allocations.forEach((rawAllocation, allocationIndex) => {
      if (!isRecord(rawAllocation)) {
        structuralErrors.push(
          issue("allocationObjectRequired", {
            component: componentIndex + 1,
            allocation: allocationIndex + 1,
          }),
        );
        return;
      }
      const sourceOrdinal = allocationOrdinal(rawAllocation, "source", legs);
      const sinkOrdinal = allocationOrdinal(rawAllocation, "sink", legs);
      if (sourceOrdinal === null || legs[sourceOrdinal]?.role !== "source") {
        structuralErrors.push(
          issue("allocationSourceInvalid", {
            component: componentIndex + 1,
            allocation: allocationIndex + 1,
          }),
        );
      }
      if (sinkOrdinal === null || !SINK_ROLES.has(String(legs[sinkOrdinal]?.role))) {
        structuralErrors.push(
          issue("allocationSinkInvalid", {
            component: componentIndex + 1,
            allocation: allocationIndex + 1,
          }),
        );
      }
      for (const field of ["source_amount_msat", "sink_amount_msat"] as const) {
        if (!exactNonnegativeInteger(rawAllocation[field])) {
          structuralErrors.push(
            issue("allocationAmountInvalid", {
              component: componentIndex + 1,
              allocation: allocationIndex + 1,
              field,
            }),
          );
        }
      }
      const sourceLeg = sourceOrdinal === null ? undefined : legs[sourceOrdinal];
      const sinkLeg = sinkOrdinal === null ? undefined : legs[sinkOrdinal];
      if (
        sourceOrdinal !== null &&
        sinkOrdinal !== null &&
        sourceLeg?.role === "source" &&
        SINK_ROLES.has(String(sinkLeg?.role))
      ) {
        const edge = `${sourceOrdinal}:${sinkOrdinal}`;
        if (allocationEdges.has(edge)) {
          structuralErrors.push(
            issue("allocationEdgeDuplicate", {
              component: componentIndex + 1,
              allocation: allocationIndex + 1,
              edge,
            }),
          );
        }
        allocationEdges.add(edge);
        const sourceAmount = exactNonnegativeIntegerAsBigInt(
          rawAllocation.source_amount_msat,
        );
        const sinkAmount = exactNonnegativeIntegerAsBigInt(
          rawAllocation.sink_amount_msat,
        );
        if (sourceAmount !== null) {
          sourceCoverage.set(
            sourceOrdinal,
            (sourceCoverage.get(sourceOrdinal) ?? 0n) + sourceAmount,
          );
        }
        if (sinkAmount !== null) {
          sinkCoverage.set(
            sinkOrdinal,
            (sinkCoverage.get(sinkOrdinal) ?? 0n) + sinkAmount,
          );
        }
        if (
          mode === "quantity" &&
          sourceAmount !== null &&
          sinkAmount !== null &&
          (sourceAmount !== sinkAmount ||
            normalizedExposure(sourceLeg) !== normalizedExposure(sinkLeg as JsonRecord) ||
            normalizedUnit(sourceLeg) !== normalizedUnit(sinkLeg as JsonRecord))
        ) {
          activationErrors.push(
            issue("allocationQuantityMismatch", {
              component: componentIndex + 1,
              allocation: allocationIndex + 1,
            }),
          );
        }
        if (sinkLeg?.role === "suspense") {
          if (!legHasTransactionAnchor(sourceLeg)) {
            activationErrors.push(
              issue("suspenseObservedSourceRequired", {
                component: componentIndex + 1,
                allocation: allocationIndex + 1,
              }),
            );
          }
          if (normalizedAsset(sourceLeg) !== normalizedAsset(sinkLeg)) {
            activationErrors.push(
              issue("suspenseAssetMismatch", {
                component: componentIndex + 1,
                allocation: allocationIndex + 1,
              }),
            );
          }
          if (
            hasText(sourceLeg.occurred_at) &&
            hasText(sinkLeg.occurred_at) &&
            Number.isFinite(Date.parse(sourceLeg.occurred_at)) &&
            Number.isFinite(Date.parse(sinkLeg.occurred_at)) &&
            Date.parse(sourceLeg.occurred_at) !== Date.parse(sinkLeg.occurred_at)
          ) {
            activationErrors.push(
              issue("suspenseTimeMismatch", {
                component: componentIndex + 1,
                allocation: allocationIndex + 1,
              }),
            );
          }
        }
      }
    });
    if (allocations.length > 0) {
      positiveSources.forEach((leg) => {
        const ordinal = legs.indexOf(leg);
        const expected = amountAsMsat(leg);
        if (expected !== null && sourceCoverage.get(ordinal) !== expected) {
          activationErrors.push(
            issue("allocationSourceCoverage", {
              component: componentIndex + 1,
              leg: ordinal + 1,
              covered: (sourceCoverage.get(ordinal) ?? 0n).toString(),
              expected: expected.toString(),
            }),
          );
        }
      });
      positiveSinks.forEach((leg) => {
        const ordinal = legs.indexOf(leg);
        const expected = amountAsMsat(leg);
        if (expected !== null && sinkCoverage.get(ordinal) !== expected) {
          activationErrors.push(
            issue("allocationSinkCoverage", {
              component: componentIndex + 1,
              leg: ordinal + 1,
              covered: (sinkCoverage.get(ordinal) ?? 0n).toString(),
              expected: expected.toString(),
            }),
          );
        }
      });
    }
  });

  return {
    components,
    structuralErrors,
    activationErrors,
    summary: {
      components: rawComponents.length,
      legs: legCount,
      sources: sourceCount,
      destinations: destinationCount,
      transactionAnchors,
      untrackedLegs,
      unresolvedLegs,
      suspenseLegs,
    },
  };
}

export interface CustodyBulkRequest {
  [key: string]: unknown;
  components: JsonRecord[];
  activate: boolean;
  dry_run?: true;
}

interface CustodyRevisionLegInput {
  id: string;
  role: string;
  rail: string;
  chain?: string | null;
  network?: string | null;
  asset: string;
  exposure?: string | null;
  conservation_unit?: string | null;
  amount_msat: CustodyExactInteger;
  valuation_unit?: string | null;
  valuation_amount?: CustodyExactInteger | null;
  occurred_at: string | null;
  transaction_id: string | null;
  anchor_transaction_id?: string | null;
  wallet_id: string | null;
  notes?: string | null;
}

interface CustodyRevisionAllocationInput {
  source_leg_id: string;
  sink_leg_id: string;
  source_amount_msat: CustodyExactInteger;
  sink_amount_msat: CustodyExactInteger;
}

interface CustodyRevisionComponentInput {
  component_type: string;
  conservation_mode: "quantity" | "conversion";
  evidence_kind: string | null;
  evidence_grade: string | null;
  conversion_policy: string | null;
  conversion_reviewed: boolean;
  notes: string | null;
  legs: CustodyRevisionLegInput[];
  allocations: CustodyRevisionAllocationInput[];
}

function compactDefinedRecord(values: JsonRecord): JsonRecord {
  return Object.fromEntries(
    Object.entries(values).filter(
      ([, value]) => value !== null && value !== undefined,
    ),
  );
}

/** Serialize a visible component into the exact document used for revision. */
export function buildCustodyRevisionDocument(
  component: CustodyRevisionComponentInput,
): string {
  const legs = component.legs.map((leg) =>
    compactDefinedRecord({
      id: leg.id,
      role: leg.role,
      rail: leg.rail,
      chain: leg.chain,
      network: leg.network,
      asset: leg.asset,
      exposure: leg.exposure,
      conservation_unit: leg.conservation_unit,
      amount_msat: leg.amount_msat,
      valuation_unit: leg.valuation_unit,
      valuation_amount: leg.valuation_amount,
      occurred_at: leg.occurred_at,
      transaction_id: leg.transaction_id,
      anchor_transaction_id: leg.anchor_transaction_id,
      wallet_id: leg.wallet_id,
      notes: leg.notes,
    }),
  );
  const allocations = component.allocations.map((allocation) => ({
    source_leg_id: allocation.source_leg_id,
    sink_leg_id: allocation.sink_leg_id,
    source_amount_msat: allocation.source_amount_msat,
    sink_amount_msat: allocation.sink_amount_msat,
  }));
  const spec = compactDefinedRecord({
    component_type: component.component_type,
    conservation_mode: component.conservation_mode,
    evidence_kind: component.evidence_kind,
    evidence_grade: component.evidence_grade,
    conversion_policy: component.conversion_policy,
    conversion_reviewed: component.conversion_reviewed,
    notes: component.notes,
    legs,
    allocations,
  });
  return JSON.stringify({ components: [spec] }, null, 2);
}

/** Format a lossless custody integer without first coercing it to Number. */
export function formatCustodyExactInteger(
  value: CustodyExactInteger,
  locale: string,
): string {
  const exact = exactNonnegativeIntegerAsBigInt(value);
  if (exact === null) return String(value);
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }).format(exact);
}

/**
 * Build the exact daemon payload used by preview and commit.
 *
 * Activation is an operation-level decision. Stripping an embedded value keeps
 * pasted JSON from silently changing the meaning of either button and makes a
 * successful dry run representative of the following write.
 */
export function buildCustodyBulkRequest(
  preview: CustodyBatchPreview,
  options: { activate: boolean; dryRun?: boolean },
): CustodyBulkRequest {
  const components = preview.components.map((raw) => {
    const component = { ...raw };
    delete component.activate;
    return component;
  });
  return {
    components,
    activate: options.activate,
    ...(options.dryRun ? { dry_run: true as const } : {}),
  };
}

export const CUSTODY_COMPONENT_EXAMPLE = JSON.stringify(
  {
    components: [
      {
        component_type: "manual_bridge",
        conservation_mode: "quantity",
        evidence_kind: "manual_migration_review",
        evidence_grade: "reviewed",
        notes: "Old wallet consolidated into two current wallets; the middle wallet is not imported.",
        legs: [
          {
            id: "old-wallet-out",
            role: "source",
            transaction: "outgoing transaction id or external id",
            amount_msat: 10_000_000,
          },
          {
            id: "missing-middle",
            role: "retained",
            untracked_wallet: "Untracked migration wallet",
            occurred_at: "2024-01-15T12:00:00Z",
            amount_msat: 10_000_000,
          },
        ],
      },
      {
        component_type: "manual_bridge",
        conservation_mode: "quantity",
        evidence_kind: "manual_migration_review",
        evidence_grade: "reviewed",
        legs: [
          {
            id: "middle-source",
            role: "source",
            untracked_wallet: "Untracked migration wallet",
            occurred_at: "2024-01-20T09:00:00Z",
            amount_msat: 10_000_000,
          },
          {
            id: "new-wallet-a",
            role: "destination",
            transaction: "first incoming transaction id or external id",
            amount_msat: 6_000_000,
          },
          {
            id: "new-wallet-b",
            role: "destination",
            transaction: "second incoming transaction id or external id",
            amount_msat: 3_900_000,
          },
          {
            id: "network-fee",
            role: "fee",
            rail: "bitcoin",
            asset: "BTC",
            amount_msat: 100_000,
          },
        ],
      },
    ],
  },
  null,
  2,
);
