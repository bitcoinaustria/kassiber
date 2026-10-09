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
  | "review_pair"
  | "process_journals";

export interface QuarantineAction {
  kind: QuarantineActionKind | string;
  wallet_id?: string;
  wallet_label?: string;
  gap_id?: string;
  transaction_id?: string;
  pair_id?: string;
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
  /** A suspense-holding pair this transaction is one leg of. */
  pair_id?: string;
  pair_counterpart_transaction_id?: string;
  /** The legs carry different same-asset txids: likely not one movement. */
  pair_txids_differ?: boolean;
  /** The receipt is older than the spend it is paired with. */
  pair_receipt_before_spend?: boolean;
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
  /** The cause this row belongs to: a `QuarantineGroup.key`. */
  group_key?: string;
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
  /** Up to 25 of the cause's root transactions, roots that block reports first. */
  root_transaction_ids: string[];
  root_count: number;
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

/**
 * Which rows a page lists: what needs the user (root causes, and rows whose
 * cause cannot be named), what only waits on a named cause, or everything.
 */
export type QuarantineScope = "attention" | "waiting" | "all";

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
    /** Whole-book counts; absent from daemons that predate the scopes. */
    attention_count?: number;
    waiting_count?: number;
    scope?: QuarantineScope;
    /** Rows in the listed scope, for paging. */
    scope_count?: number;
  };
  items: QuarantineItem[];
}
