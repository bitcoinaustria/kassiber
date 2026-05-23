export interface QuarantineReason {
  reason: string;
  count: number;
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
}

export interface QuarantineSnapshot {
  summary: {
    workspace: string | null;
    profile: string | null;
    count: number;
    by_reason: QuarantineReason[];
    limit: number;
  };
  items: QuarantineItem[];
}
