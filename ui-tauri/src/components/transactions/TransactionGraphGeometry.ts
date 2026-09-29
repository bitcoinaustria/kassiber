import {
  compactGraphRows,
  type GraphRow,
  type TransactionGraphNode,
  type TransactionGraphPayload,
} from "./TransactionGraphModel";

/**
 * How wide each leg of a transaction is drawn. Shared by the 2D bowtie and the
 * 3D ribbon view so both tell the same story. Drawing only: confidential and
 * missing values still render as such, whatever width their leg gets.
 */

/** Drawing-only view of a leg's amount. Nothing here is accounting truth. */
type GeometryValue = {
  /** No usable amount: missing, or confidential on Liquid. */
  amountless: boolean;
  /** A known amount of zero — drawn as a stub, not a hairline. */
  zero: boolean;
  /** Known visual sats, or null when amountless. */
  known: number | null;
  /** Always-positive sats, so an unknown leg still gets a visible strand. */
  visual: number;
};

export function isAmountless(node: TransactionGraphNode) {
  return (
    typeof node.valueSats !== "number" ||
    node.valueState === "confidential" ||
    node.valueState === "other_asset"
  );
}

function positiveKnownSats(node: TransactionGraphNode) {
  return !isAmountless(node) && (node.valueSats as number) > 0
    ? (node.valueSats as number)
    : 0;
}

export type GeometryScale =
  /** Widths follow a visual total in sats. */
  | { kind: "value"; totalSats: number }
  /**
   * No leg but the fee has a known amount (a confidential Liquid row, a
   * reference-only record, or hidden values): every unknown leg gets the same
   * modest width instead of each claiming the full band.
   */
  | { kind: "uniform" };

/** A leg drawn at uniform width takes at most this share of the full band. */
const UNIFORM_STRAND_BAND_SHARE = 1 / 6;

function amountlessLegCount(rows: GraphRow[]) {
  return rows.filter((node) => node.side !== "fee" && isAmountless(node)).length;
}

export function geometryScale(inputRows: GraphRow[], destinationRows: GraphRow[]): GeometryScale {
  const inputKnownTotal = inputRows.reduce((sum, node) => sum + positiveKnownSats(node), 0);
  const outputKnownTotal = destinationRows.reduce((sum, node) => sum + positiveKnownSats(node), 0);
  const inputUnknown = amountlessLegCount(inputRows);
  const outputUnknown = amountlessLegCount(destinationRows);
  if (!inputUnknown || !outputUnknown) {
    // One complete side fixes the total; the other side's unknown legs share
    // whatever it leaves unaccounted for.
    return { kind: "value", totalSats: Math.max(inputKnownTotal, outputKnownTotal, 1) };
  }
  // Unknown legs on both sides leave the total open. As mempool's Liquid graph
  // does, estimate it by assuming each unknown leg is as large as the average
  // known one; both sides must still meet in the middle, so each side's unknown
  // legs then share what that side leaves of the larger estimate. The fee is
  // left out of the average: it would make every unknown leg look like dust.
  const knownLegs = [...inputRows, ...destinationRows]
    .filter((node) => node.side !== "fee")
    .map(positiveKnownSats)
    .filter((sats) => sats > 0);
  if (!knownLegs.length) return { kind: "uniform" };
  const average = knownLegs.reduce((sum, sats) => sum + sats, 0) / knownLegs.length;
  return {
    kind: "value",
    totalSats: Math.max(
      inputKnownTotal + average * inputUnknown,
      outputKnownTotal + average * outputUnknown,
      1,
    ),
  };
}

export function fallbackVisualSats(scale: GeometryScale, rowCount: number) {
  if (scale.kind === "uniform") return 1;
  return Math.max(1, scale.totalSats / Math.max(1, rowCount));
}

/**
 * Where no leg but the fee has a known amount, both sides share one band, as
 * mempool's graph does, so a 72-input consolidation into two outputs uses the
 * same room on each side. The band is only as wide as the busier side needs at
 * a modest width per leg, so a small all-confidential row stays thin.
 */
export function uniformBandWeight(
  combinedWeight: number,
  inputRows: GraphRow[],
  destinationRows: GraphRow[],
) {
  const legs = Math.max(amountlessLegCount(inputRows), amountlessLegCount(destinationRows), 1);
  return Math.min(combinedWeight, combinedWeight * UNIFORM_STRAND_BAND_SHARE * legs);
}

function geometryValues(rows: GraphRow[], fallbackSats: number) {
  const hasAmountlessNonFeeRows = rows.some(
    (node) => node.side !== "fee" && isAmountless(node),
  );
  const values: GeometryValue[] = rows.map((node) => {
    if (isAmountless(node)) {
      return {
        amountless: true,
        zero: false,
        known: null,
        visual: Math.max(1, fallbackSats),
      };
    }
    const sats = Math.max(0, node.valueSats as number);
    // A known fee sitting next to amountless legs would otherwise dominate the
    // band, so it contributes a single sat of visual weight.
    const known = node.side === "fee" && hasAmountlessNonFeeRows && sats > 0 ? 1 : sats;
    return { amountless: false, zero: sats <= 0, known, visual: known };
  });
  return { values, hasAmountlessNonFeeRows };
}

export function redactRowsForGeometry(rows: GraphRow[]): GraphRow[] {
  return rows.map((node) => ({
    ...node,
    valueSats: null,
    valueBtc: null,
    valueState:
      node.valueState === "confidential" || node.valueState === "other_asset"
        ? node.valueState
        : "missing",
  }));
}

export type LegWeight = {
  /** Width of the leg where it meets the transaction, in band units. */
  weight: number;
  /** A fee next to unknown legs: drawn as a hairline, whatever its weight. */
  hairline: boolean;
  /** The leg has no known amount, so its width is an estimate. */
  estimated: boolean;
  /** A known amount of zero. */
  zero: boolean;
  visualSats: number;
};

/**
 * One side's leg widths. `combinedWeight` is the width of the band where all
 * legs meet; `uniformBand` and `hairlineWeight` are in the same unit.
 */
export function legWeights(
  rows: GraphRow[],
  scale: GeometryScale,
  {
    combinedWeight,
    fallbackSats,
    uniformBand,
    hairlineWeight,
  }: {
    combinedWeight: number;
    fallbackSats: number;
    uniformBand: number;
    hairlineWeight: number;
  },
): LegWeight[] {
  const { values, hasAmountlessNonFeeRows } = geometryValues(rows, fallbackSats);
  const unknownCount = values.filter((value) => value.amountless).length;
  const knownTotal = values.reduce((sum, value) => sum + (value.known ?? 0), 0);
  const totalSats = scale.kind === "value" ? scale.totalSats : 0;
  // Unknown legs share whatever the opposite side says is unaccounted for.
  const unknownShare = unknownCount
    ? Math.max(1, (Math.max(totalSats, knownTotal) - knownTotal) / unknownCount)
    : 0;
  // Each side splits the shared band evenly; the fee never takes a share.
  const uniformWeight = uniformBand / Math.max(1, amountlessLegCount(rows));
  return rows.map((node, index) => {
    const value = values[index];
    const weight =
      scale.kind === "uniform"
        ? value.zero
          ? 0
          : value.amountless && node.side !== "fee"
            ? uniformWeight
            : hairlineWeight
        : (combinedWeight * (value.known ?? unknownShare)) / Math.max(1, totalSats);
    return {
      weight,
      hairline: node.side === "fee" && hasAmountlessNonFeeRows && weight > 0,
      estimated: value.amountless,
      zero: value.zero,
      visualSats: value.visual,
    };
  });
}

export type GraphLayoutRows = {
  inputRows: GraphRow[];
  destinationRows: GraphRow[];
  /** The rows geometry is computed from: amounts removed when values are hidden. */
  layoutInputRows: GraphRow[];
  layoutDestinationRows: GraphRow[];
};

/** The legs a drawing shows: compacted inputs, and the fee ahead of the outputs. */
export function graphLayoutRows(
  graph: TransactionGraphPayload,
  hideSensitive: boolean,
  maxRows: number,
): GraphLayoutRows {
  const inputRows = compactGraphRows(graph.inputs, "input", maxRows);
  const outputRows = compactGraphRows(graph.outputs, "output", maxRows);
  const feeRow: GraphRow | null = graph.fee ? { ...graph.fee, side: "fee" } : null;
  const destinationRows = feeRow ? [feeRow, ...outputRows] : outputRows;
  return {
    inputRows,
    destinationRows,
    layoutInputRows: hideSensitive ? redactRowsForGeometry(inputRows) : inputRows,
    layoutDestinationRows: hideSensitive
      ? redactRowsForGeometry(destinationRows)
      : destinationRows,
  };
}
