// Pure logic for the split transfer/swap resolution card. Kept separate from
// the .tsx component so it can be unit-tested without a DOM/daemon and so the
// component file only exports components (react-refresh friendly).

export const PAYOUT_ASSETS = ["BTC", "LBTC", "LNBTC"] as const;

export type SplitPayoutPolicy = "carrying-value" | "taxable";

export function parseBtc(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

export interface SplitPayoutFields {
  outAmount: string;
  payoutAmount: string;
  outboundBtc: number;
}

export interface SplitPayoutValidation {
  outError: string | null;
  payoutError: string | null;
  ready: boolean;
}

export function splitPayoutValidation({
  outAmount,
  payoutAmount,
  outboundBtc,
}: SplitPayoutFields): SplitPayoutValidation {
  const outValue = parseBtc(outAmount);
  const payoutValue = parseBtc(payoutAmount);
  const outError =
    outAmount.trim() === ""
      ? null
      : outValue === null || outValue <= 0
        ? "Enter a positive BTC amount"
        : outboundBtc > 0 && outValue > outboundBtc
          ? `Cannot exceed the outbound total (${outboundBtc})`
          : null;
  const payoutError =
    payoutAmount.trim() === ""
      ? null
      : payoutValue === null || payoutValue <= 0
        ? "Enter a positive BTC amount"
        : null;
  const ready =
    outValue !== null &&
    outValue > 0 &&
    (outboundBtc <= 0 || outValue <= outboundBtc) &&
    payoutValue !== null &&
    payoutValue > 0;
  return { outError, payoutError, ready };
}

export interface SplitPayoutArgsInput {
  transactionId: string;
  outAmount: string;
  payoutAsset: string;
  payoutAmount: string;
  policy: SplitPayoutPolicy;
  counterparty: string;
  notes: string;
}

/** Builds the `ui.transfers.payouts.create` daemon args from form state. */
export function buildSplitPayoutArgs(input: SplitPayoutArgsInput) {
  return {
    tx_out: input.transactionId,
    out_amount: input.outAmount.trim(),
    payout_asset: input.payoutAsset,
    payout_amount: input.payoutAmount.trim(),
    policy: input.policy,
    counterparty: input.counterparty.trim() || undefined,
    notes: input.notes.trim() || undefined,
  };
}

export function defaultPayoutAsset(sourceAsset: string): string {
  return (PAYOUT_ASSETS as readonly string[]).includes(sourceAsset)
    ? sourceAsset
    : "BTC";
}
