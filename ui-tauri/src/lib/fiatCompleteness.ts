import type {
  FiatCompleteness,
  FiatCompletenessReason,
  FiatCompletenessState,
} from "@/mocks/seed";

const STATES: readonly FiatCompletenessState[] = [
  "complete",
  "incomplete",
  "stale",
  "unavailable",
];

const REASONS: readonly FiatCompletenessReason[] = [
  "journals_stale",
  "quarantines",
  "custody_unresolved",
  "missing_prices",
  "market_rate_missing",
];

/**
 * Safe default for payloads without a usable `fiat.completeness` block (older
 * daemons, fixtures, or malformed data): nothing is claimed complete, so
 * basis-derived figures stay hidden until the daemon says otherwise.
 */
export const UNKNOWN_FIAT_COMPLETENESS: FiatCompleteness = Object.freeze({
  state: "unavailable",
  costBasisComplete: false,
  reasons: [],
  quarantineCount: 0,
  quarantinedInboundMsat: 0,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: null,
  basisUncoveredMsat: null,
  earliestIncompleteAt: null,
  missingPriceCount: 0,
  marketRateMissing: false,
}) as FiatCompleteness;

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : 0;
}

function nullableCount(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, value)
    : null;
}

export function normalizeFiatCompleteness(value: unknown): FiatCompleteness {
  if (!isRecord(value)) return UNKNOWN_FIAT_COMPLETENESS;
  const stateIsKnown = STATES.includes(value.state as FiatCompletenessState);
  const state = stateIsKnown ? (value.state as FiatCompletenessState) : "unavailable";
  const reasons = Array.isArray(value.reasons)
    ? REASONS.filter((reason) => (value.reasons as unknown[]).includes(reason))
    : [];
  // Never let a "complete" flag outvote a stale/incomplete state.
  const costBasisComplete =
    value.costBasisComplete === true &&
    stateIsKnown &&
    state !== "stale" &&
    state !== "incomplete";
  return {
    state,
    costBasisComplete,
    reasons,
    quarantineCount: count(value.quarantineCount),
    quarantinedInboundMsat: count(value.quarantinedInboundMsat),
    quarantinedOutboundMsat: count(value.quarantinedOutboundMsat),
    basisCoveredMsat: nullableCount(value.basisCoveredMsat),
    basisUncoveredMsat: nullableCount(value.basisUncoveredMsat),
    earliestIncompleteAt:
      typeof value.earliestIncompleteAt === "string" &&
      value.earliestIncompleteAt
        ? value.earliestIncompleteAt
        : null,
    missingPriceCount: count(value.missingPriceCount),
    marketRateMissing: value.marketRateMissing === true,
  };
}

/** Completeness for any fiat block; a missing block is never "complete". */
export function fiatCompleteness(
  fiat: { completeness?: FiatCompleteness | null } | null | undefined,
): FiatCompleteness {
  return fiat?.completeness ?? UNKNOWN_FIAT_COMPLETENESS;
}

/**
 * Epoch ms from which basis-derived chart values (cost basis, avg cost,
 * unrealized) are incomplete; `null` when the basis is complete. An
 * incomplete basis without a known start date affects every point.
 */
export function basisIncompleteFromMs(completeness: FiatCompleteness): number | null {
  if (completeness.costBasisComplete) return null;
  const parsed = completeness.earliestIncompleteAt
    ? Date.parse(completeness.earliestIncompleteAt)
    : Number.NaN;
  return Number.isFinite(parsed) ? parsed : Number.NEGATIVE_INFINITY;
}

/**
 * Whether a daily chart point (`YYYY-MM-DD`, UTC) is affected. A daily point
 * values the end of its day, so the day that holds the first gap is affected.
 */
export function isBasisIncompleteOnDay(
  completeness: FiatCompleteness,
  day: string | undefined,
): boolean {
  const from = basisIncompleteFromMs(completeness);
  if (from === null) return false;
  if (from === Number.NEGATIVE_INFINITY) return true;
  const dayStart = Date.parse(`${String(day ?? "").slice(0, 10)}T00:00:00Z`);
  if (!Number.isFinite(dayStart)) return true;
  return dayStart + 86_400_000 > from;
}

/** Whether an instant (e.g. an activity event) is at/after the first gap. */
export function isBasisIncompleteAt(
  completeness: FiatCompleteness,
  timeMs: number,
): boolean {
  const from = basisIncompleteFromMs(completeness);
  if (from === null) return false;
  return !Number.isFinite(timeMs) || timeMs >= from;
}

/** Where the user resolves the gap: journals when stale, else the quarantine. */
export function completenessHref(
  completeness: FiatCompleteness,
): "/journals" | "/quarantine" {
  return completeness.state === "stale" ||
    (completeness.quarantineCount === 0 &&
      completeness.reasons.includes("custody_unresolved"))
    ? "/journals"
    : "/quarantine";
}
