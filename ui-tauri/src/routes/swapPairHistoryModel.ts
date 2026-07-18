export type PairHistoryExactInteger = number | string;

export interface PairHistoryAllocation {
  id: string;
  component_id?: string | null;
  component?: {
    id: string;
    source_count: number;
    sink_count: number;
  } | null;
  out: { amount_msat: PairHistoryExactInteger };
  in: { amount_msat: PairHistoryExactInteger };
}

export interface PairedComponentGroup<T extends PairHistoryAllocation> {
  id: string;
  sourceCount: number;
  sinkCount: number;
  pairs: T[];
  sourceTotalMsat: bigint;
  sinkTotalMsat: bigint;
}

export function pairHistoryExactInteger(
  value: PairHistoryExactInteger,
): bigint {
  return BigInt(value);
}

export function formatPairHistoryMsatAsBtc(value: bigint): string {
  const negative = value < 0n;
  const absolute = negative ? -value : value;
  const roundedSats = (absolute + 500n) / 1000n;
  const whole = roundedSats / 100_000_000n;
  const fraction = (roundedSats % 100_000_000n).toString().padStart(8, "0");
  return `${negative ? "−" : ""}₿${whole.toString()}.${fraction}`;
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
        sourceTotalMsat: 0n,
        sinkTotalMsat: 0n,
      };
      grouped.set(id, group);
    }
    group.pairs.push(pair);
    group.sourceTotalMsat += pairHistoryExactInteger(pair.out.amount_msat);
    group.sinkTotalMsat += pairHistoryExactInteger(pair.in.amount_msat);
  }
  return [...grouped.values()];
}
