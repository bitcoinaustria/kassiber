export type CustodyGapStatus =
  | "needs_review"
  | "conflicting"
  | "resolved"
  | "dismissed";

export interface CustodyGapDownstreamImpact {
  affected_disposals: number;
  affected_years: number[];
}

export interface CustodyGap {
  gap_id: string;
  candidate_fingerprint: string;
  status: CustodyGapStatus;
  status_reason?: string;
  asset: string;
  source_wallet_label: string;
  destination_wallet_labels: string[];
  source_total_msat: string | number;
  source_fee_msat: string | number;
  source_debit_msat: string | number;
  return_total_msat: string | number;
  retained_msat?: string | number;
  residual_msat: string | number;
  excess_msat?: string | number;
  started_at: string | null;
  ended_at: string | null;
  confidence: "strong" | "moderate" | "weak";
  promotion_eligible: boolean;
  competitor_score_margin: number | null;
  reason_codes: string[];
  correction?: {
    component_id: string;
    strategy: "create_revision_then_activate";
  };
  residual_classification?: {
    classification: CustodyResidualClassification;
    custody_state:
      | "external_confirmed"
      | "internal_reviewed"
      | "custody_suspense";
    country_tax_meaning: "not_assigned";
    amount_msat: string | number;
  };
  downstream: CustodyGapDownstreamImpact;
}

export interface CustodyGapSnapshot {
  summary: {
    total: number;
    needs_review: number;
    conflicting: number;
    resolved: number;
    dismissed: number;
    unresolved_msat: string | number;
    candidate_residual_msat?: string | number;
    candidate_residual_by_asset?: Array<{
      asset: string;
      amount_msat: string | number;
    }>;
    canonical_unresolved_msat?: string | number;
    canonical_issue_count?: number;
    canonical_unresolved_by_asset?: Array<{
      asset: string;
      amount_msat: string | number;
      issue_count: number;
    }>;
    canonical_unquantified_issue_count?: number;
    canonical_status?: string;
    canonical_status_text?: string;
    derived_state_current?: boolean;
    qualification?: string;
    search_complete?: boolean;
    search_status?: "complete" | "capacity_limited";
    search_limit_kind?: string | null;
    search_candidate_count?: number;
  };
  gaps: CustodyGap[];
  next_cursor?: string | null;
}

export type CustodyResidualClassification =
  | "external_payment"
  | "external_disposal"
  | "external_gift"
  | "external_loss"
  | "retained_custody"
  | "suspense_continuation";

export interface CustodyGapReviewHistoryEntry {
  revision: number;
  event_kind:
    | "review_decision"
    | "bridge_created"
    | "bridge_reopened"
    | "bridge_revised"
    | "residual_classified";
  status: "needs_review" | "resolved" | "dismissed";
  component_revision: number | null;
  authored_source: string | null;
  reason: string | null;
  created_at: string;
  retained_msat: string | number;
  residual_msat: string | number;
  residual_classification: CustodyResidualClassification | null;
  filed_report_impact_count: number;
}

export interface CustodyGapReviewHistory {
  gap_id: string;
  count: number;
  history: CustodyGapReviewHistoryEntry[];
}

export type CustodyGapActionMode = "create" | "reopen" | "revise" | "none";

export function custodyGapActionMode(gap: CustodyGap): CustodyGapActionMode {
  if (gap.status === "resolved") return "reopen";
  if (gap.status === "needs_review" && gap.status_reason === "bridge_reopened") {
    return "revise";
  }
  if (gap.status === "needs_review" || gap.status === "conflicting") {
    return "create";
  }
  return "none";
}

export function shouldOfferResidualClassification(gap: CustodyGap): boolean {
  if (gap.status !== "resolved" || gap.residual_classification) return false;
  try {
    return BigInt(gap.residual_msat) > 0n;
  } catch {
    return false;
  }
}

export interface FiledReportImpactPreview {
  filed_report_snapshot_id: string;
  report_kind: string;
  report_state: "saved" | "filed";
  affected_period_start_year: number;
  affected_period_end_year: number;
  after_gain_summary: { status?: string };
  amendment_warning: string;
}

export interface BridgePreview {
  gap_id: string;
  candidate_fingerprint: string;
  authored_claim_fingerprint?: string;
  dry_run: true;
  activatable: boolean;
  review_mode?: "structured_candidate" | "manual_weak_hint";
  warnings?: string[];
  requires_explicit_confirmation?: boolean;
  retained_msat: string | number;
  residual_msat: string | number;
  fee_msat: string | number;
  source_count: number;
  destination_count: number;
  filed_report_impacts: FiledReportImpactPreview[];
}

export interface GuidedCorrectionPreview {
  gap_id: string;
  expected_fingerprint: string;
  dry_run: true;
  requires_explicit_confirmation: true;
  activatable?: boolean;
  resulting_status?: "needs_review";
  current_component_revision: number;
  new_component_revision?: number;
  retained_msat?: string | number;
  residual_msat?: string | number;
  filed_report_impacts: FiledReportImpactPreview[];
}

export interface ResidualClassificationPreview extends GuidedCorrectionPreview {
  classification: CustodyResidualClassification;
  custody_state:
    | "external_confirmed"
    | "internal_reviewed"
    | "custody_suspense";
  country_tax_meaning: "not_assigned";
  residual_msat: string | number;
  new_component_revision: number;
}

export function bridgePreviewArgs(gapId: string) {
  return { gap_id: gapId };
}

export function bridgeCreateArgs(preview: BridgePreview) {
  return {
    gap_id: preview.gap_id,
    expected_fingerprint:
      preview.authored_claim_fingerprint ?? preview.candidate_fingerprint,
  };
}

function reasonArg(reason: string): { reason?: string } {
  const normalized = reason.trim();
  return normalized ? { reason: normalized } : {};
}

export function reopenPreviewArgs(gapId: string, reason: string) {
  return { gap_id: gapId, ...reasonArg(reason) };
}

export function reopenConfirmArgs(
  preview: GuidedCorrectionPreview,
  reason: string,
) {
  return {
    gap_id: preview.gap_id,
    expected_fingerprint: preview.expected_fingerprint,
    ...reasonArg(reason),
  };
}

export function revisePreviewArgs(gapId: string, reason: string) {
  return { gap_id: gapId, ...reasonArg(reason) };
}

export function reviseConfirmArgs(
  preview: GuidedCorrectionPreview,
  reason: string,
) {
  return {
    gap_id: preview.gap_id,
    expected_fingerprint: preview.expected_fingerprint,
    ...reasonArg(reason),
  };
}

export function residualPreviewArgs(
  gapId: string,
  classification: CustodyResidualClassification,
  reason: string,
) {
  return {
    gap_id: gapId,
    classification,
    ...reasonArg(reason),
  };
}

export function residualConfirmArgs(
  preview: ResidualClassificationPreview,
  reason: string,
) {
  return {
    gap_id: preview.gap_id,
    classification: preview.classification,
    expected_fingerprint: preview.expected_fingerprint,
    ...reasonArg(reason),
  };
}

export interface CustodyCoverageBranch {
  branch: "receive" | "change";
  scanned_to_exclusive: number | null;
  highest_used: number | null;
  observed_at: string | null;
}

export interface CustodyCoverageSource {
  source: string;
  observer_kind: string;
  branches: CustodyCoverageBranch[];
}

export interface CustodyCoverageEpoch {
  epoch_id: string;
  status: "active" | "retired";
  chain: string;
  network: string;
  created_at: string;
  retired_at: string | null;
  sources: CustodyCoverageSource[];
}

export interface CustodyCoverageWallet {
  wallet_label: string;
  epochs: CustodyCoverageEpoch[];
}

export interface CustodyCoverageSnapshot {
  schema_version: 1;
  scope: "imported_policy_technical_coverage";
  ownership_universe_known: false;
  coverage_can_clear_custody_gaps: false;
  summary: {
    wallet_count: number;
    epoch_count: number;
    active_epoch_count: number;
    retired_epoch_count: number;
    source_count: number;
    covered_branch_count: number;
  };
  wallets: CustodyCoverageWallet[];
}

export type CustodyLineageState =
  | "internal_verified"
  | "internal_reviewed";

export type CustodyLineageBasisState =
  | "eligible"
  | "blocked_by_prior_custody_basis";

export interface CustodyLineageItem {
  out_transaction_id: string;
  in_transaction_id: string;
  occurred_at: string | null;
  asset: string;
  amount_msat: string | number;
  from_wallet_id: string;
  from_wallet_label: string;
  to_wallet_id: string;
  to_wallet_label: string;
  custody_state: CustodyLineageState;
  basis_state: CustodyLineageBasisState;
  basis_barrier_at: string | null;
  evidence_reason: string;
  network: string;
  rail: string;
  atomic_bundle_id?: string | null;
  component_id?: string | null;
}

export interface CustodyLineageSnapshot {
  next_cursor?: string | null;
  summary: {
    total_count: number;
    returned_count?: number;
    truncated: boolean;
    internal_verified?: number;
    internal_reviewed?: number;
    basis_eligible?: number;
    basis_blocked?: number;
    qualification?: string;
  };
  items: CustodyLineageItem[];
}

export function collectCustodyLineagePages(
  pages: readonly CustodyLineageSnapshot[],
): CustodyLineageSnapshot | undefined {
  const first = pages[0];
  if (!first) return undefined;
  const items = pages.flatMap((page) => page.items);
  const last = pages.at(-1);
  return {
    ...first,
    items,
    next_cursor: last?.next_cursor ?? null,
    summary: {
      ...first.summary,
      returned_count: items.length,
      truncated: Boolean(last?.next_cursor),
      internal_verified: pages.reduce(
        (total, page) => total + (page.summary.internal_verified ?? 0),
        0,
      ),
      internal_reviewed: pages.reduce(
        (total, page) => total + (page.summary.internal_reviewed ?? 0),
        0,
      ),
      basis_eligible: pages.reduce(
        (total, page) => total + (page.summary.basis_eligible ?? 0),
        0,
      ),
      basis_blocked: pages.reduce(
        (total, page) => total + (page.summary.basis_blocked ?? 0),
        0,
      ),
    },
  };
}

export function collectCustodyGapPages(
  pages: readonly CustodyGapSnapshot[],
): CustodyGap[] {
  const seen = new Set<string>();
  const gaps: CustodyGap[] = [];
  for (const page of pages) {
    for (const gap of page.gaps) {
      if (seen.has(gap.gap_id)) continue;
      seen.add(gap.gap_id);
      gaps.push(gap);
    }
  }
  return gaps;
}

const BTC_MSAT = 100_000_000_000n;

export function formatCustodyMsat(value: string | number, asset: string): string {
  let msat: bigint;
  try {
    msat = BigInt(value);
  } catch {
    return `— ${asset}`;
  }
  const sign = msat < 0n ? "−" : "";
  const absolute = msat < 0n ? -msat : msat;
  const whole = absolute / BTC_MSAT;
  const fraction = (absolute % BTC_MSAT)
    .toString()
    .padStart(11, "0")
    .replace(/0+$/, "");
  return `${sign}${whole}${fraction ? `.${fraction}` : ""} ${asset}`;
}

export function canShowNoKnownCustodyGaps(
  snapshot: CustodyGapSnapshot,
  reviewGapCount: number,
): boolean {
  return (
    reviewGapCount === 0 &&
    snapshot.summary.needs_review === 0 &&
    snapshot.summary.conflicting === 0 &&
    snapshot.summary.derived_state_current === true &&
    snapshot.summary.search_complete !== false &&
    (snapshot.summary.canonical_issue_count ?? 0) === 0
  );
}
