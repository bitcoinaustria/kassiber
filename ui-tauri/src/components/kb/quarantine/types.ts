export interface QuarantineReason {
  reason: string;
  count: number;
}

export type QuarantineCategory =
  | "missing_wallet_history"
  | "missing_chain_evidence"
  | "missing_price"
  | "missing_acquisition_history"
  | "needs_decision"
  | "unsupported"
  | "downstream";

export type QuarantineActionKind =
  | "sync_wallet"
  | "connect_wallet"
  | "import_history"
  | "set_price"
  | "classify"
  | "pair_transfer"
  | "review_custody_gap"
  | "attach_evidence"
  | "wait_for_confirmation"
  | "resolve_root"
  | "process_journals";

export interface QuarantineAction {
  kind: QuarantineActionKind | string;
  wallet_id?: string;
  wallet_label?: string;
  gap_id?: string;
  transaction_id?: string;
}

export interface QuarantineWalletRef {
  id: string;
  label: string;
  deprecated?: boolean;
}

/** Actionable facts the daemon extracted from the stored quarantine detail. */
export interface QuarantineEvidence {
  wallet_label?: string;
  missing_source_wallets?: QuarantineWalletRef[];
  blocker_code?: string;
  blocker_codes?: string[];
  gap_id?: string;
  required_msat?: number;
  available_msat?: number;
  counterpart_transaction_ids?: string[];
  lot_state_uncertain_since?: string;
  blocked_by_reason?: string;
  row_amount_msat?: number;
  owned_receipts_msat?: number;
  owned_outputs_msat?: number;
}

export interface QuarantineRoot {
  transaction_id: string;
  reason: string | null;
  occurred_at: string | null;
  wallet: string | null;
  external_id: string;
}

export interface QuarantineItem {
  transaction_id: string;
  external_id: string;
  occurred_at: string;
  confirmed_at: string | null;
  wallet: string;
  direction: "inbound" | "outbound" | string;
  asset: string;
  amount: number;
  amount_msat: number;
  fee: number;
  fee_msat: number;
  reason: string;
  detail: Record<string, unknown>;
  created_at: string;
  category?: QuarantineCategory;
  blocks_reports?: boolean;
  is_downstream?: boolean;
  root?: QuarantineRoot | null;
  reasons?: string[];
  evidence?: QuarantineEvidence;
  actions?: QuarantineAction[];
}

export interface QuarantineGroup {
  key: string;
  category: QuarantineCategory;
  reason: string;
  root_transaction_id: string | null;
  root_occurred_at: string | null;
  root_wallet: string | null;
  root_external_id: string | null;
  root_amount_msat: number | null;
  root_direction: string | null;
  root_asset: string | null;
  count: number;
  downstream_count: number;
  blocks_reports: boolean;
  wallets: string[];
  earliest_occurred_at: string | null;
  evidence: QuarantineEvidence;
  actions: QuarantineAction[];
}

export interface QuarantineAssumptionItem {
  transaction_id: string;
  occurred_at: string | null;
  wallet: string;
  amount_msat: number;
  external_id: string;
}

export interface QuarantineAssumption {
  count: number;
  amount_msat: number;
  items: QuarantineAssumptionItem[];
}

export interface QuarantineFreshness {
  needs_processing: boolean;
  status?: string | null;
  last_processed_at: string | null;
  last_error: { code: string; message: string; at: string | null } | null;
}

export interface QuarantineSnapshot {
  summary: {
    workspace: string | null;
    profile: string | null;
    count: number;
    by_reason: QuarantineReason[];
    limit: number;
    offset?: number;
    freshness?: QuarantineFreshness | null;
    blocking_count?: number;
    reports_blocked?: boolean;
    by_category?: Array<{ category: QuarantineCategory; count: number }>;
    groups?: QuarantineGroup[];
    group_count?: number;
    assumptions?: {
      presumed_external_outbound: QuarantineAssumption;
      unclassified_inbound: QuarantineAssumption;
    } | null;
  };
  items: QuarantineItem[];
}
