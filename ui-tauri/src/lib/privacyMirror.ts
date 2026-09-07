import type { AnalysisQuery } from "./chainAnalysis";

export interface PrivacyInvestigation {
  query: AnalysisQuery;
  snapshot_id: string;
}
export interface PrivacyFinding {
  id: string;
  code: string;
  category: "linkage" | "structure" | "pattern" | "attribution";
  severity: "warning" | "info";
  authority: string;
  relevance: "own_spend" | "owned_output" | "received_context" | "nearby_context";
  title: string;
  detail: string;
  assumptions: string[];
  limitations: string[];
  affected_output_count: number;
  investigation: PrivacyInvestigation;
}
export interface PrivacyCheck {
  code: string;
  status: "complete" | "partial" | "unavailable" | "not_applicable" | "bounded";
  evaluated: number;
  eligible: number;
  reason?: string;
}
export interface PrivacyMirrorPayload {
  payload_schema_version: 2;
  local_only: true;
  read_only: true;
  advisory_only: true;
  observer: "public";
  investigation: PrivacyInvestigation;
  summary: {
    status: "findings" | "no_observed_exposure" | "unavailable";
    finding_count: number;
    attention_count: number;
    owned_output_count: number;
    analyzed_transaction_count: number;
    local_transaction_count: number;
    domain_count: number;
  };
  findings: PrivacyFinding[];
  coverage: {
    status: "complete" | "partial" | "unavailable";
    examined_transactions: number;
    available_transactions: number;
    missing_nodes: number;
    stale_nodes: number;
    conflicting_nodes: number;
    truncated: boolean;
    stopped_reasons: string[];
    checks: PrivacyCheck[];
  };
  entropy: {
    evaluated: number;
    eligible: number;
    omitted: number;
    results: Array<{
      subject: string;
      status: string;
      reason?: string | null;
      interpretation_count?: string | null;
      interpretation_count_lower_bound?: string | null;
      entropy_bits?: number | null;
      conditional_on_model?: boolean;
      investigation: PrivacyInvestigation;
    }>;
  };
  assumptions: string[];
}

export type PrivacyHeadline =
  | "personal_findings"
  | "context_only"
  | "no_local_evidence"
  | "no_owned_outputs"
  | "nothing_found"
  | "unavailable";

export const PERSONAL_RELEVANCE: ReadonlySet<PrivacyFinding["relevance"]> = new Set(["own_spend", "owned_output"]);

/**
 * Pick the one-line answer for the report. Personal relevance comes from the
 * findings themselves (attention_count is a severity count); the backend summary
 * stays authoritative for population and status, and partial coverage never turns
 * "nothing found" into a clean result; the coverage badge carries that.
 */
export function privacyHeadline(
  summary: PrivacyMirrorPayload["summary"],
  findings: ReadonlyArray<Pick<PrivacyFinding, "relevance">>,
): PrivacyHeadline {
  if (findings.some(finding => PERSONAL_RELEVANCE.has(finding.relevance))) return "personal_findings";
  // Surrounding findings never upgrade an unassessed snapshot: the backend's
  // unavailable status and empty populations stay unknown, not "nothing personal".
  if (summary.local_transaction_count === 0) return "no_local_evidence";
  if (summary.status === "unavailable") {
    return summary.owned_output_count === 0 ? "no_owned_outputs" : "unavailable";
  }
  if (summary.owned_output_count === 0) return "no_owned_outputs";
  return summary.finding_count > 0 ? "context_only" : "nothing_found";
}

/** Group repeated presentation rows without combining evidence, counts or assumptions. */
export function groupPrivacyFindings(findings: PrivacyFinding[]): PrivacyFinding[][] {
  const groups = new Map<string, PrivacyFinding[]>();
  for (const finding of findings) {
    const key = JSON.stringify([finding.code, finding.relevance, finding.authority]);
    const group = groups.get(key);
    if (group) group.push(finding);
    else groups.set(key, [finding]);
  }
  return [...groups.values()];
}
