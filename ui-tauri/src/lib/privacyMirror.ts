import { formatSats, formatUiNumber } from "@/lib/localeFormat";

export type EvidenceLevel = "exact" | "derived" | "unknown" | string;

export interface PrivacyScoreFactor {
  key?: string;
  linked?: number;
  leaking?: number;
  total?: number;
  weight?: number;
  points?: number;
}

export interface PrivacyScoreSummary {
  value?: number;
  base?: number;
  evidence_level?: EvidenceLevel;
  coverage_ratio?: number;
  factors?: PrivacyScoreFactor[];
}

export interface PrivacyMirrorPayload {
  local_only?: boolean;
  read_only?: boolean;
  advisory_only?: boolean;
  summary?: {
    evidence_level?: EvidenceLevel;
    privacy_score?: PrivacyScoreSummary;
    linkage_score?: number;
    linkable_cluster_count?: number;
    adversary_view_count?: number;
    wallet_count?: number;
    transaction_tell_count?: number;
    utxo_count?: number;
    unknown_count?: number;
    finding_count?: number;
    worst_risk?: WorstRisk;
  };
  exposure_summary?: {
    evidence_level?: EvidenceLevel;
    linkage?: Record<string, unknown>;
    hygiene?: Record<string, unknown>;
  };
  adversary_cards?: AdversaryCard[];
  wallet_view?: WalletPrivacyRow[];
  transaction_view?: TransactionPrivacyRow[];
  utxo_view?: UtxoPrivacyRow[];
  timeline?: TimelineEvent[];
  psbt_what_if_panel?: Record<string, unknown>;
  coverage?: {
    evidence_level?: EvidenceLevel;
    source_proximity_known_coin_count?: number;
    source_proximity_unknown_coin_count?: number;
    unknown_coverage_count?: number;
    degraded?: boolean;
  };
  unknowns?: UnknownRow[];
  evidence_drilldowns?: EvidenceDrilldown[];
  limitations?: UnknownRow[];
}

export interface WorstRisk {
  kind?: string | null;
  severity?: string | null;
  title?: string | null;
  answer?: string | null;
  evidence_level?: EvidenceLevel;
  source?: string | null;
  finding_id?: string | null;
}

export interface AdversaryCard {
  tier?: string;
  label?: string;
  evidence_level?: EvidenceLevel;
  summary?: {
    exposed_cluster_count?: number;
    wallet_count?: number;
    observer_entity_count?: number;
    unknown_coverage?: {
      status?: string;
      node_count?: number;
      wallet_count?: number;
      evidence_level?: EvidenceLevel;
    };
  };
  model_assumptions?: Array<{
    code?: string;
    statement?: string;
    evidence_level?: EvidenceLevel;
  }>;
}

export interface WalletPrivacyRow {
  wallet_id?: string;
  coin_count?: number;
  amount_msat?: number;
  linkage_edge_count?: number;
  cluster_count?: number;
  unknown_role_coin_count?: number;
  evidence_level?: EvidenceLevel;
}

export interface TransactionPrivacyRow {
  txid?: string;
  tell_count?: number;
  tell_kinds?: string[];
  wallet_penalty_count?: number;
  evidence_level?: EvidenceLevel;
}

export interface UtxoPrivacyRow {
  coin_id?: string;
  wallet_id?: string;
  amount_msat?: number;
  branch_role?: string;
  source_proximity?: string;
  evidence_level?: EvidenceLevel;
}

export interface TimelineEvent {
  id?: string;
  category?: string;
  kind?: string;
  txid?: string | null;
  evidence_level?: EvidenceLevel;
  detail?: string | null;
  new_linkage?: boolean;
}

export interface UnknownRow {
  source?: string;
  code?: string;
  title?: string;
  message?: string;
  evidence_level?: EvidenceLevel;
}

export interface EvidenceDrilldown {
  section?: string;
  id?: string;
  kind?: string;
  evidence_level?: EvidenceLevel;
  evidence?: Record<string, unknown>;
}

export interface PsbtPrivacyResult {
  summary?: {
    cluster_merge_delta?: number;
    unknown_input_count?: number;
    blast_radius_score?: number;
    evidence_level?: EvidenceLevel;
  };
  findings?: Array<{
    id?: string;
    kind?: string;
    severity?: string;
    title?: string;
    detail?: string;
    evidence_level?: EvidenceLevel;
  }>;
  adversary_deltas?: Array<{
    tier?: string;
    cluster_merge_delta?: number;
    newly_exposed_component_count?: number;
    evidence_level?: EvidenceLevel;
  }>;
  what_if?: Array<{
    scenario?: string;
    cluster_merge_delta?: number;
    support_status?: string;
    evidence_level?: EvidenceLevel;
  }>;
  unknowns?: Record<string, unknown>;
}

export function formatPrivacyInt(value: unknown) {
  return typeof value === "number" && Number.isFinite(value)
    ? formatUiNumber(value)
    : "0";
}

export function formatPrivacyMsat(value: unknown) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "0 sats";
  return formatSats(Math.round(value / 1000));
}

export function shortPrivacyId(value: unknown) {
  const text = String(value || "");
  if (text.length <= 24) return text || "unknown";
  return `${text.slice(0, 12)}...${text.slice(-8)}`;
}

export function privacyEvidenceTone(level: EvidenceLevel | undefined) {
  if (level === "exact") {
    return "border-foreground/20 text-foreground";
  }
  if (level === "derived") {
    return "border-sky-500/30 text-sky-700 dark:text-sky-300";
  }
  return "border-amber-500/30 text-amber-800 dark:text-amber-300";
}

export type PrivacySeverity = "info" | "warning" | "alert";

/**
 * Severity is the "how bad" axis (info < warning < alert), orthogonal to the
 * evidence "how sure" axis. It uses a SEPARATE visual channel (a left stripe +
 * a mono chip) so the two never collide (both would otherwise reach for amber).
 */
export function privacySeverity(value: unknown): PrivacySeverity {
  return value === "alert" || value === "warning" ? value : "info";
}

export function privacySeverityTone(severity: PrivacySeverity) {
  if (severity === "alert") {
    return {
      text: "text-destructive",
      dot: "bg-destructive",
      bg: "bg-destructive/10",
      stripe: "border-l-destructive",
    };
  }
  if (severity === "warning") {
    return {
      text: "text-amber-700 dark:text-amber-300",
      dot: "bg-amber-500",
      bg: "bg-amber-500/10",
      stripe: "border-l-amber-500",
    };
  }
  return {
    text: "text-sky-700 dark:text-sky-300",
    dot: "bg-sky-500",
    bg: "bg-sky-500/10",
    stripe: "border-l-sky-500",
  };
}

/**
 * Count -> severity policy for rows that carry no explicit `severity` field
 * (wallet/transaction/timeline rows). Deliberately conservative: any positive
 * leak signal escalates to `warning` so a real finding is never under-stated;
 * a clean row stays `info`. Kept in one place so the threshold is testable.
 */
export function transactionRowSeverity(row: {
  tell_count?: number;
  wallet_penalty_count?: number;
}): PrivacySeverity {
  if ((row.wallet_penalty_count ?? 0) > 0) return "warning";
  if ((row.tell_count ?? 0) > 0) return "warning";
  return "info";
}

function normalizedRef(value: unknown) {
  return String(value || "").trim().toLowerCase();
}

export function findPrivacyWalletRow(
  payload: PrivacyMirrorPayload | undefined,
  refs: Array<string | null | undefined>,
) {
  const candidates = new Set(refs.map(normalizedRef).filter(Boolean));
  if (!candidates.size) return undefined;
  return (payload?.wallet_view ?? []).find((row) =>
    candidates.has(normalizedRef(row.wallet_id)),
  );
}

export function findPrivacyTransactionRow(
  payload: PrivacyMirrorPayload | undefined,
  refs: Array<string | null | undefined>,
) {
  const candidates = new Set(refs.map(normalizedRef).filter(Boolean));
  if (!candidates.size) return undefined;
  return (payload?.transaction_view ?? []).find((row) =>
    candidates.has(normalizedRef(row.txid)),
  );
}
