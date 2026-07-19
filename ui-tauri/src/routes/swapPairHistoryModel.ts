export type PairHistoryExactInteger = number | string;

export interface PairHistoryAllocation {
  id: string;
  component_id?: string | null;
  component?: {
    id: string;
    source_count: number;
    sink_count: number;
  } | null;
  out: { asset: string; amount_msat: PairHistoryExactInteger };
  in: { asset: string; amount_msat: PairHistoryExactInteger };
}

export interface PairHistoryAssetTotal {
  asset: string;
  amountMsat: bigint;
}

export interface PairedComponentGroup<T extends PairHistoryAllocation> {
  id: string;
  sourceCount: number;
  sinkCount: number;
  pairs: T[];
  sourceTotals: PairHistoryAssetTotal[];
  sinkTotals: PairHistoryAssetTotal[];
}

export function pairHistoryExactInteger(
  value: PairHistoryExactInteger,
): bigint {
  return BigInt(value);
}

export function pairHistoryFeePercent(
  feeMsat: PairHistoryExactInteger,
  outAmountMsat: PairHistoryExactInteger,
): number {
  const rawFee = pairHistoryExactInteger(feeMsat);
  const rawOut = pairHistoryExactInteger(outAmountMsat);
  if (rawOut === 0n) return 0;
  const absoluteFee = rawFee < 0n ? -rawFee : rawFee;
  const absoluteOut = rawOut < 0n ? -rawOut : rawOut;
  // Retain six decimal places of percentage precision without routing unsafe
  // custody integers through Number first. UI callers render two decimals.
  const scaledPercent = (absoluteFee * 100_000_000n) / absoluteOut;
  return Number(scaledPercent) / 1_000_000;
}

function formatPairHistoryMsatDecimal(value: bigint): string {
  const negative = value < 0n;
  const absolute = negative ? -value : value;
  const roundedSats = (absolute + 500n) / 1000n;
  const whole = roundedSats / 100_000_000n;
  const fraction = (roundedSats % 100_000_000n).toString().padStart(8, "0");
  return `${negative ? "−" : ""}${whole.toString()}.${fraction}`;
}

export function formatPairHistoryMsatAsBtc(value: bigint): string {
  const decimal = formatPairHistoryMsatDecimal(value);
  return decimal.startsWith("−") ? `−₿${decimal.slice(1)}` : `₿${decimal}`;
}

export function formatPairHistoryAssetTotals(
  totals: PairHistoryAssetTotal[],
): string {
  return totals
    .map((total) => `${formatPairHistoryMsatDecimal(total.amountMsat)} ${total.asset}`)
    .join(" + ");
}

function addAssetTotal(
  totals: PairHistoryAssetTotal[],
  asset: string,
  amountMsat: PairHistoryExactInteger,
) {
  const normalizedAsset = asset.trim().toUpperCase() || "UNKNOWN";
  const current = totals.find((total) => total.asset === normalizedAsset);
  if (current) {
    current.amountMsat += pairHistoryExactInteger(amountMsat);
    return;
  }
  totals.push({
    asset: normalizedAsset,
    amountMsat: pairHistoryExactInteger(amountMsat),
  });
}

export function groupPairedComponents<T extends PairHistoryAllocation>(
  pairs: T[],
): PairedComponentGroup<T>[] {
  const grouped = new Map<string, PairedComponentGroup<T>>();
  for (const pair of pairs) {
    const id = pair.component?.id || pair.component_id || pair.id;
    let group = grouped.get(id);
    if (!group) {
      group = {
        id,
        sourceCount: pair.component?.source_count ?? 1,
        sinkCount: pair.component?.sink_count ?? 1,
        pairs: [],
        sourceTotals: [],
        sinkTotals: [],
      };
      grouped.set(id, group);
    }
    group.pairs.push(pair);
    addAssetTotal(group.sourceTotals, pair.out.asset, pair.out.amount_msat);
    addAssetTotal(group.sinkTotals, pair.in.asset, pair.in.amount_msat);
  }
  return [...grouped.values()];
}
