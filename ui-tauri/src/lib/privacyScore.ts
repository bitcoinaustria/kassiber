import {
  privacySeverity,
  transactionRowSeverity,
  type PrivacyMirrorPayload,
  type PrivacySeverity,
} from "./privacyMirror";

/**
 * Client-side fallback for an evaluable owned-output population: observed
 * wallet findings reduce the base; coverage gaps and counterparty context do
 * not incur penalties. Without that population the score is unavailable. It is a
 * UI-level summary of the same local evidence the rest of the page shows — it
 * is deterministic and never fetches anything.
 */
export type PrivacyGrade = "A+" | "B" | "C" | "D" | "F";

export const SCORE_BASE = 70;

export const SEVERITY_PENALTY: Record<PrivacySeverity, number> = {
  alert: 18,
  warning: 9,
  info: 3,
};

export interface ScoreFinding {
  id: string;
  kind: string;
  severity: PrivacySeverity;
  evidenceLevel?: string;
  txid?: string | null;
  affectsScore?: boolean;
}

export const GRADE_HEX: Record<PrivacyGrade, string> = {
  "A+": "#22c55e",
  B: "#84cc16",
  C: "#f59e0b",
  D: "#f97316",
  F: "#ef4444",
};

export const SEVERITY_HEX: Record<PrivacySeverity, string> = {
  alert: "#ef4444",
  warning: "#f59e0b",
  info: "#38bdf8",
};

export const SEVERITY_ORDER: PrivacySeverity[] = ["alert", "warning", "info"];

function clampScore(value: number) {
  return Math.max(0, Math.min(100, Math.round(value)));
}

/**
 * Unify the redacted payload's signals into a single ranked findings list so
 * the score, the severity census, the waterfall, and the finding cards all
 * derive from ONE source and stay consistent. The `worst_risk` is surfaced
 * separately as the primary recommendation and is intentionally not re-added
 * here, so it is never double-counted in the score.
 */
export function deriveScoreFindings(payload: PrivacyMirrorPayload): ScoreFinding[] {
  const findings: ScoreFinding[] = [];

  for (const row of payload.transaction_view ?? []) {
    findings.push({
      id: `tx:${row.txid ?? findings.length}`,
      kind: row.tell_kinds?.[0] ?? "transaction_tell",
      severity: transactionRowSeverity(row),
      evidenceLevel: row.evidence_level,
      txid: row.txid ?? null,
      affectsScore: (row.wallet_penalty_count ?? row.tell_count ?? 0) > 0,
    });
  }

  for (const row of payload.unknowns ?? []) {
    findings.push({
      id: `unknown:${row.code ?? row.source ?? findings.length}`,
      kind: row.code ?? "unknown_coverage",
      severity: "info",
      evidenceLevel: row.evidence_level ?? "unknown",
      affectsScore: false,
    });
  }

  if (payload.coverage?.degraded) {
    findings.push({
      id: "coverage:degraded",
      kind: "coverage_degraded",
      severity: "info",
      evidenceLevel: payload.coverage.evidence_level ?? "unknown",
      affectsScore: false,
    });
  }

  const rank: Record<PrivacySeverity, number> = { alert: 0, warning: 1, info: 2 };
  return findings.sort((a, b) => rank[a.severity] - rank[b.severity]);
}

export function computeScore(findings: ScoreFinding[]): number {
  const penalty = findings.reduce(
    (sum, finding) => sum + (finding.affectsScore === false ? 0 : SEVERITY_PENALTY[finding.severity]),
    0,
  );
  return clampScore(SCORE_BASE - penalty);
}

export function gradeForScore(score: number): PrivacyGrade {
  if (score >= 90) return "A+";
  if (score >= 75) return "B";
  if (score >= 50) return "C";
  if (score >= 25) return "D";
  return "F";
}

export function severityCensus(
  findings: ScoreFinding[],
): Record<PrivacySeverity, number> {
  return findings.reduce(
    (census, finding) => {
      census[finding.severity] += 1;
      return census;
    },
    { alert: 0, warning: 0, info: 0 } as Record<PrivacySeverity, number>,
  );
}

export interface WaterfallStep {
  severity: PrivacySeverity;
  count: number;
  delta: number;
}

/** Grouped score contributions: base -> (-alerts) -> (-warnings) -> (-info). */
export function scoreWaterfall(findings: ScoreFinding[]): WaterfallStep[] {
  const census = severityCensus(findings.filter(finding => finding.affectsScore !== false));
  return SEVERITY_ORDER.map((severity) => ({
    severity,
    count: census[severity],
    delta: -census[severity] * SEVERITY_PENALTY[severity],
  })).filter((step) => step.count > 0);
}

export interface ScoreFactor {
  key: string;
  linked?: number;
  leaking?: number;
  total?: number;
  points: number | null;
}

export interface PrivacyScoreModel {
  score: number | null;
  grade: PrivacyGrade | null;
  base: number;
  findings: ScoreFinding[];
  census: Record<PrivacySeverity, number>;
  factors: ScoreFactor[];
  coverageRatio?: number;
  /** true when the score comes from the daemon, false for the UI fallback. */
  grounded: boolean;
  worstSeverity: PrivacySeverity;
}

// Capability inventory, not an execution report. Mirror and transaction detail
// currently have separate analysis surfaces. Availability does not mean a check
// ran for this book, and it is not a measured privacy guarantee. Names are proper/technical terms, kept in
// English (like the raw tell kinds); only the wrapper strings are localized.
export type HeuristicStatus = "mirror" | "transaction_detail" | "chain_analysis_workspace" | "not_implemented";

export const AIE_HEURISTIC_COVERAGE: Array<{ id: string; name: string; status: HeuristicStatus }> = [
  { id: "h3", name: "Common input ownership", status: "mirror" },
  { id: "h8", name: "Address reuse", status: "mirror" },
  { id: "h2", name: "Private wallet change evidence", status: "mirror" },
  { id: "h1", name: "Round amounts", status: "transaction_detail" },
  { id: "h6", name: "Rounded fee-rate pattern", status: "mirror" },
  { id: "h7", name: "OP_RETURN metadata", status: "mirror" },
  { id: "h11", name: "Wallet fingerprinting", status: "transaction_detail" },
  { id: "script", name: "Script type analysis", status: "transaction_detail" },
  { id: "witness", name: "Witness data", status: "transaction_detail" },
  { id: "dust", name: "Dust output detection", status: "transaction_detail" },
  { id: "unnecessary", name: "Unnecessary inputs", status: "transaction_detail" },
  { id: "h9", name: "UTXO analysis", status: "mirror" },
  { id: "h10", name: "Address type", status: "transaction_detail" },
  { id: "h4", name: "CoinJoin boundary patterns", status: "mirror" },
  { id: "consolidation", name: "Consolidation patterns", status: "chain_analysis_workspace" },
  { id: "utxo-age", name: "UTXO age spread", status: "not_implemented" },
  { id: "bip69", name: "BIP69 ordering", status: "chain_analysis_workspace" },
  { id: "coinsel", name: "Coin selection", status: "not_implemented" },
  { id: "dust-spend", name: "Dust spending", status: "not_implemented" },
  { id: "h17", name: "Multisig / escrow", status: "not_implemented" },
  { id: "coinbase", name: "Coinbase", status: "not_implemented" },
  { id: "spending", name: "Spending patterns", status: "not_implemented" },
  { id: "recurring", name: "Recurring payment", status: "not_implemented" },
  { id: "highactivity", name: "High activity", status: "not_implemented" },
  { id: "h5", name: "Conditional transaction entropy", status: "chain_analysis_workspace" },
  { id: "anon", name: "Anonymity sets", status: "not_implemented" },
  { id: "peel", name: "Peel chain", status: "chain_analysis_workspace" },
  { id: "tx0", name: "CoinJoin premix", status: "not_implemented" },
  { id: "postmix", name: "Post-mix consolidation", status: "chain_analysis_workspace" },
  { id: "ricochet", name: "Ricochet", status: "not_implemented" },
  { id: "entity", name: "Sourced attribution claims", status: "chain_analysis_workspace" },
  { id: "exchange", name: "Exchange pattern", status: "not_implemented" },
  { id: "bip47", name: "BIP47 notification", status: "not_implemented" },
  { id: "timing", name: "Timing analysis", status: "not_implemented" },
];

export function heuristicAvailableCount() {
  return AIE_HEURISTIC_COVERAGE.filter((h) => h.status !== "not_implemented").length;
}

function validCount(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function hasOwnedPopulation(payload: PrivacyMirrorPayload): boolean {
  if ([...(payload.unknowns ?? []), ...(payload.limitations ?? [])].some(row => row.code === "no_owned_bitcoin_outputs")) return false;
  // A declared zero cannot be overridden by stale detail rows. Older payloads
  // without the summary can still establish ownership with actual coin rows.
  if (validCount(payload.summary?.utxo_count)) return payload.summary.utxo_count > 0;
  if (payload.utxo_view?.length) return true;
  if (payload.wallet_view?.some(row => validCount(row.coin_count) && row.coin_count > 0)) return true;
  const known = payload.coverage?.source_proximity_known_coin_count;
  const unknown = payload.coverage?.source_proximity_unknown_coin_count;
  return validCount(known) && validCount(unknown) && known + unknown > 0;
}

function sourceCoverageRatio(payload: PrivacyMirrorPayload): number | undefined {
  const known = payload.coverage?.source_proximity_known_coin_count;
  const unknown = payload.coverage?.source_proximity_unknown_coin_count;
  const ratio = payload.summary?.privacy_score?.coverage_ratio;
  // In particular, the old daemon's empty-population ratio of 1 is not coverage.
  return validCount(known) && validCount(unknown) && known + unknown > 0 &&
    typeof ratio === "number" && Number.isFinite(ratio) && ratio >= 0 && ratio <= 1
    ? ratio : undefined;
}

function hasDaemonScorePopulation(payload: PrivacyMirrorPayload): boolean {
  const factors = payload.summary?.privacy_score?.factors ?? [];
  const known = payload.coverage?.source_proximity_known_coin_count;
  const unknown = payload.coverage?.source_proximity_unknown_coin_count;
  return validCount(known) && validCount(unknown) && known + unknown > 0 &&
    ["wallet_linkage", "transaction_leaks"].every(key => {
      const total = factors.find(factor => factor.key === key)?.total;
      return validCount(total) && total > 0;
    });
}

export function privacyScoreModel(payload: PrivacyMirrorPayload): PrivacyScoreModel {
  const findings = deriveScoreFindings(payload);
  const census = severityCensus(findings);
  const worstSeverity = privacySeverity(payload.summary?.worst_risk?.severity);
  const daemon = payload.summary?.privacy_score;
  const evaluable = hasOwnedPopulation(payload) && daemon?.evaluation_status !== "unavailable" && (!daemon || hasDaemonScorePopulation(payload));
  const factors: ScoreFactor[] = (daemon?.factors ?? []).map((factor) => ({
    key: String(factor.key ?? "factor"),
    linked: factor.linked,
    leaking: factor.leaking,
    total: factor.total,
    points: typeof factor.points === "number" && Number.isFinite(factor.points) ? factor.points : null,
  }));

  if (!evaluable || (daemon && (daemon.value == null || !Number.isFinite(daemon.value)))) {
    return {
      score: null,
      grade: null,
      base: daemon?.base ?? SCORE_BASE,
      findings,
      census,
      factors,
      coverageRatio: hasOwnedPopulation(payload) ? sourceCoverageRatio(payload) : undefined,
      grounded: !!daemon,
      worstSeverity,
    };
  }

  if (daemon && typeof daemon.value === "number") {
    return {
      score: daemon.value,
      grade: gradeForScore(daemon.value),
      base: typeof daemon.base === "number" ? daemon.base : 100,
      findings,
      census,
      factors,
      coverageRatio: sourceCoverageRatio(payload),
      grounded: true,
      worstSeverity,
    };
  }

  // Fallback for payloads without a daemon score: the legacy client-side model.
  const score = computeScore(findings);
  const fallbackFactors: ScoreFactor[] = scoreWaterfall(findings).map((step) => ({
    key: step.severity,
    total: step.count,
    points: step.delta,
  }));
  return {
    score,
    grade: gradeForScore(score),
    base: SCORE_BASE,
    findings,
    census,
    factors: fallbackFactors,
    coverageRatio: undefined,
    grounded: false,
    worstSeverity,
  };
}
