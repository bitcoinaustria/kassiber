import {
  privacySeverity,
  transactionRowSeverity,
  type PrivacyMirrorPayload,
  type PrivacySeverity,
} from "./privacyMirror";

/**
 * Client-side privacy score, modelled on am-i-exposed: start from a neutral
 * base and let findings pull it DOWN by severity (there are no positive signals
 * in the redacted local payload, so the base is the ceiling). The score is a
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
    });
  }

  for (const row of payload.unknowns ?? []) {
    findings.push({
      id: `unknown:${row.code ?? row.source ?? findings.length}`,
      kind: row.code ?? "unknown_coverage",
      severity: "info",
      evidenceLevel: row.evidence_level ?? "unknown",
    });
  }

  if (payload.coverage?.degraded) {
    findings.push({
      id: "coverage:degraded",
      kind: "coverage_degraded",
      severity: "info",
      evidenceLevel: payload.coverage.evidence_level ?? "unknown",
    });
  }

  const rank: Record<PrivacySeverity, number> = { alert: 0, warning: 1, info: 2 };
  return findings.sort((a, b) => rank[a.severity] - rank[b.severity]);
}

export function computeScore(findings: ScoreFinding[]): number {
  const penalty = findings.reduce(
    (sum, finding) => sum + SEVERITY_PENALTY[finding.severity],
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
  const census = severityCensus(findings);
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
  points: number;
}

export interface PrivacyScoreModel {
  score: number;
  grade: PrivacyGrade;
  base: number;
  findings: ScoreFinding[];
  census: Record<PrivacySeverity, number>;
  factors: ScoreFactor[];
  coverageRatio?: number;
  /** true when the score comes from the daemon, false for the UI fallback. */
  grounded: boolean;
  worstSeverity: PrivacySeverity;
}

// The am-i-exposed heuristic catalog mirrored with Kassiber's honest local
// coverage (verified against privacy_linkage.py / privacy_hygiene.py). Status =
// whether the local engine computes an equivalent signal, with no chain fetch,
// entity DB, or entropy engine. Names are proper/technical terms, kept in
// English (like the raw tell kinds); only the wrapper strings are localized.
export type HeuristicStatus = "computed" | "partial" | "not_local";

export const HEURISTIC_STATUS_HEX: Record<HeuristicStatus, string> = {
  computed: "#22c55e",
  partial: "#f59e0b",
  not_local: "#6b7280",
};

export const AIE_HEURISTIC_COVERAGE: Array<{ id: string; name: string; status: HeuristicStatus }> = [
  { id: "h3", name: "Common input ownership", status: "computed" },
  { id: "h8", name: "Address reuse", status: "computed" },
  { id: "h2", name: "Change detection", status: "computed" },
  { id: "h1", name: "Round amounts", status: "computed" },
  { id: "h6", name: "Fee fingerprinting", status: "computed" },
  { id: "h7", name: "OP_RETURN metadata", status: "computed" },
  { id: "h11", name: "Wallet fingerprinting", status: "computed" },
  { id: "script", name: "Script type analysis", status: "computed" },
  { id: "witness", name: "Witness data", status: "computed" },
  { id: "dust", name: "Dust output detection", status: "computed" },
  { id: "unnecessary", name: "Unnecessary inputs", status: "computed" },
  { id: "h9", name: "UTXO analysis", status: "computed" },
  { id: "h10", name: "Address type", status: "computed" },
  { id: "h4", name: "CoinJoin detection", status: "computed" },
  { id: "consolidation", name: "Consolidation patterns", status: "partial" },
  { id: "utxo-age", name: "UTXO age spread", status: "partial" },
  { id: "bip69", name: "BIP69 ordering", status: "partial" },
  { id: "coinsel", name: "Coin selection", status: "partial" },
  { id: "dust-spend", name: "Dust spending", status: "partial" },
  { id: "h17", name: "Multisig / escrow", status: "partial" },
  { id: "coinbase", name: "Coinbase", status: "partial" },
  { id: "spending", name: "Spending patterns", status: "partial" },
  { id: "recurring", name: "Recurring payment", status: "partial" },
  { id: "highactivity", name: "High activity", status: "partial" },
  { id: "h5", name: "Transaction entropy", status: "not_local" },
  { id: "anon", name: "Anonymity sets", status: "not_local" },
  { id: "peel", name: "Peel chain", status: "not_local" },
  { id: "tx0", name: "CoinJoin premix", status: "not_local" },
  { id: "postmix", name: "Post-mix consolidation", status: "not_local" },
  { id: "ricochet", name: "Ricochet", status: "not_local" },
  { id: "entity", name: "Known entity", status: "not_local" },
  { id: "exchange", name: "Exchange pattern", status: "not_local" },
  { id: "bip47", name: "BIP47 notification", status: "not_local" },
  { id: "timing", name: "Timing analysis", status: "not_local" },
];

export function heuristicComputedCount() {
  return AIE_HEURISTIC_COVERAGE.filter((h) => h.status === "computed").length;
}

export function privacyScoreModel(payload: PrivacyMirrorPayload): PrivacyScoreModel {
  const findings = deriveScoreFindings(payload);
  const census = severityCensus(findings);
  const worstSeverity = privacySeverity(payload.summary?.worst_risk?.severity);
  const daemon = payload.summary?.privacy_score;

  if (daemon && typeof daemon.value === "number") {
    const factors: ScoreFactor[] = (daemon.factors ?? []).map((factor) => ({
      key: String(factor.key ?? "factor"),
      linked: factor.linked,
      leaking: factor.leaking,
      total: factor.total,
      points: typeof factor.points === "number" ? factor.points : 0,
    }));
    return {
      score: daemon.value,
      grade: gradeForScore(daemon.value),
      base: typeof daemon.base === "number" ? daemon.base : 100,
      findings,
      census,
      factors,
      coverageRatio:
        typeof daemon.coverage_ratio === "number" ? daemon.coverage_ratio : undefined,
      grounded: true,
      worstSeverity,
    };
  }

  // Fallback for payloads without a daemon score: the legacy client-side model.
  const score = computeScore(findings);
  const factors: ScoreFactor[] = scoreWaterfall(findings).map((step) => ({
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
    factors,
    coverageRatio: undefined,
    grounded: false,
    worstSeverity,
  };
}
