import {
  compactGraphRows,
  type GraphRow,
  type TransactionGraphNode,
  type TransactionGraphPayload,
} from "./TransactionGraphModel";

/*
 * Strand widths and positions after mempool's bowtie graph
 * (frontend/src/app/components/tx-bowtie-graph: calcTotalValue, initLines and
 * linesFromWeights), shared by the 3D ribbon view and the flat fallback so both
 * tell the same story. Drawing only: an estimated width never becomes a
 * displayed amount.
 */

/** mempool's `lineLimit`: legs past it fold into one "+N more" leg. */
export const BOWTIE_LINE_LIMIT = 250;
/** mempool's `minWeight`: the thinnest strand is this wide. */
const MIN_WEIGHT = 2;
/** The least space between two strands' outer ends, as in mempool. */
const MIN_SPACING = 4;

export function isAmountless(node: TransactionGraphNode) {
  return (
    typeof node.valueSats !== "number" ||
    node.valueState === "confidential" ||
    node.valueState === "other_asset"
  );
}

/** A leg's value as mempool reads it: `null` when unknown (confidential, missing, another asset). */
export function legValue(node: TransactionGraphNode): number | null {
  return isAmountless(node) ? null : Math.max(0, node.valueSats as number);
}

/** How many transaction legs a row stands for: a folded row counts all of them. */
function legCount(row: GraphRow) {
  return row.overflow ? Math.max(1, row.overflowCount ?? 1) : 1;
}

function knownTotal(rows: GraphRow[]) {
  return rows.reduce((sum, row) => sum + (legValue(row) ?? 0), 0);
}

function unknownLegs(rows: GraphRow[]) {
  return rows.reduce((sum, row) => sum + (legValue(row) === null ? legCount(row) : 0), 0);
}

/**
 * mempool's calcTotalValue. Bitcoin: the outputs plus the fee, which sits among
 * the destination rows here. Liquid: with unknown legs on both sides the total
 * is indeterminate, so unknown legs are assumed to be as large as the average
 * known leg on their side; otherwise the larger known side is the total.
 */
export function bowtieTotal(
  inputRows: GraphRow[],
  destinationRows: GraphRow[],
  liquid: boolean,
) {
  const totalOutput = knownTotal(destinationRows);
  if (!liquid) return totalOutput;
  const totalInput = knownTotal(inputRows);
  const unknownInputs = unknownLegs(inputRows);
  const unknownOutputs = unknownLegs(destinationRows);
  if (unknownInputs && unknownOutputs) {
    const inputLegs = inputRows.reduce((sum, row) => sum + legCount(row), 0);
    const outputLegs = destinationRows.reduce((sum, row) => sum + legCount(row), 0);
    const knownInputCount = inputLegs - unknownInputs || 1;
    const knownOutputCount = outputLegs - unknownOutputs || 1;
    return Math.max(
      totalInput + (totalInput / knownInputCount) * unknownInputs,
      totalOutput + (totalOutput / knownOutputCount) * unknownOutputs,
    );
  }
  return Math.max(totalInput, totalOutput);
}

/**
 * mempool's initLines: each leg's share of the band where the legs meet. Unknown
 * legs split what the total leaves after this side's known legs; without any
 * total every leg gets the same share.
 */
export function bowtieWeights(rows: GraphRow[], total: number, combinedWeight: number) {
  if (!total) return rows.map(() => combinedWeight / Math.max(1, rows.length));
  const unknownRows = rows.filter((row) => legValue(row) === null).length;
  const unknownShare = unknownRows ? (total - knownTotal(rows)) / unknownRows : 0;
  return rows.map((row) =>
    Math.max(0, (combinedWeight * (legValue(row) ?? unknownShare)) / total),
  );
}

export type BowtieLine = {
  /** Centre of the strand's outer end. */
  outerY: number;
  /** Centre of the strand where it meets the band. */
  innerY: number;
  thickness: number;
  /** Share of the band, in the same unit as `combinedWeight`. */
  weight: number;
  /**
   * mempool's normalised curve offset: both curve ends move this far towards
   * the outer edge, which keeps neighbouring strands apart.
   */
  offset: number;
  /** mempool's `pad + maxOffset`: where the side's curves start before any offset. */
  curveBase: number;
  /** A known amount of zero: drawn as a stub that never reaches the band. */
  zeroValue: boolean;
  /** No known amount: the width is an estimate. */
  estimated: boolean;
};

/**
 * mempool's linesFromWeights: strand thickness, the outer ends spread over the
 * same span on both sides, and the inner ends stacked into the band.
 */
export function bowtieLines(
  rows: GraphRow[],
  total: number,
  {
    height,
    combinedWeight,
    curveWidth,
    outerTop,
    outerSpan,
    zeroThickness,
  }: {
    height: number;
    combinedWeight: number;
    /** Horizontal run of a strand's curve, for the overlap correction. */
    curveWidth: number;
    /** Where the first outer end starts, and the span the outer ends fill. */
    outerTop: number;
    outerSpan: number;
    zeroThickness: number;
  },
): BowtieLine[] {
  if (!rows.length) return [];
  const weights = bowtieWeights(rows, total, combinedWeight);
  const lines: BowtieLine[] = rows.map((row, index) => {
    const value = legValue(row);
    return {
      outerY: height / 2,
      innerY: height / 2,
      thickness:
        value === 0
          ? zeroThickness
          : Math.min(combinedWeight + 0.5, Math.max(MIN_WEIGHT - 1, weights[index]) + 1),
      weight: weights[index],
      offset: 0,
      curveBase: 0,
      zeroValue: value === 0,
      estimated: value === null,
    };
  });
  const visibleWeight = lines.reduce((sum, line) => sum + line.thickness, 0);
  const spacing =
    lines.length <= 1
      ? 0
      : Math.max(MIN_SPACING, (outerSpan - visibleWeight) / (lines.length - 1));
  const innerTop = height / 2 - combinedWeight / 2;
  const innerBottom = innerTop + combinedWeight + 0.5;
  let lastOuter = outerTop;
  let lastInner = innerTop;
  let offset = 0;
  let minOffset = 0;
  let maxOffset = 0;
  let lastWeight = 0;
  let pad = 0;
  lines.forEach((line) => {
    if (line.zeroValue) {
      line.outerY = lines.length === 1 ? height / 2 : lastOuter + line.thickness / 2;
      lastOuter += line.thickness + spacing;
      return;
    }
    line.outerY = lines.length === 1 ? height / 2 : lastOuter + line.thickness / 2;
    line.innerY = Math.min(
      innerBottom - line.thickness / 2,
      Math.max(innerTop + line.thickness / 2, lastInner + line.weight / 2),
    );
    lastOuter += line.thickness + spacing;
    lastInner += line.weight;

    // Parallel curves must stay >= t apart at their inflection point.
    const t = (lastWeight + line.weight) / 2;
    const dx = Math.max(1, 0.75 * curveWidth);
    const dy = 1.5 * (line.innerY - line.outerY);
    const angle = Math.atan2(dy, dx);
    if (Math.sin(angle) !== 0) {
      offset += Math.max(Math.min((t * (1 - Math.cos(angle))) / Math.sin(angle), t), -t);
    }
    line.offset = offset;
    minOffset = Math.min(minOffset, offset);
    maxOffset = Math.max(maxOffset, offset);
    pad = Math.max(pad, line.thickness / 2);
    lastWeight = line.weight;
  });
  return lines.map((line) => ({
    ...line,
    offset: line.offset - minOffset,
    curveBase: pad + (maxOffset - minOffset),
  }));
}

/**
 * The least span that fits `rows` at minimum spacing: sides share the larger of
 * this and the drawing height, so a 250-leg side is not taller than its peer.
 */
export function bowtieMinimumSpan(
  rows: GraphRow[],
  total: number,
  combinedWeight: number,
  zeroThickness: number,
) {
  const weights = bowtieWeights(rows, total, combinedWeight);
  const thickness = rows.reduce((sum, row, index) => {
    const value = legValue(row);
    return (
      sum +
      (value === 0
        ? zeroThickness
        : Math.min(combinedWeight + 0.5, Math.max(MIN_WEIGHT - 1, weights[index]) + 1))
    );
  }, 0);
  return thickness + MIN_SPACING * Math.max(0, rows.length - 1);
}

function redactRowsForGeometry(rows: GraphRow[]): GraphRow[] {
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

/** The parts of a graph payload its drawings read. */
export type DrawableGraph = Pick<TransactionGraphPayload, "inputs" | "outputs" | "fee"> & {
  transaction?: { chain?: string | null } | null;
};

export type GraphLayoutRows = {
  inputRows: GraphRow[];
  destinationRows: GraphRow[];
  /** The rows geometry is computed from: amounts removed when values are hidden. */
  layoutInputRows: GraphRow[];
  layoutDestinationRows: GraphRow[];
  /** Every leg before folding, for the total: mempool sums before it truncates. */
  totalInputRows: GraphRow[];
  totalDestinationRows: GraphRow[];
};

/**
 * The legs a drawing shows. On Bitcoin the fee leads the outputs, as mempool
 * puts it first; on Liquid it is an output of its own and comes last.
 */
export function graphLayoutRows(
  graph: DrawableGraph,
  hideSensitive: boolean,
  maxRows: number,
): GraphLayoutRows {
  const feeRow: GraphRow | null = graph.fee ? { ...graph.fee, side: "fee" } : null;
  const liquid = graph.transaction?.chain === "liquid";
  const withFee = (outputRows: GraphRow[]) =>
    feeRow ? (liquid ? [...outputRows, feeRow] : [feeRow, ...outputRows]) : outputRows;
  const redact = (rows: GraphRow[]) => (hideSensitive ? redactRowsForGeometry(rows) : rows);
  const inputRows = compactGraphRows(graph.inputs, "input", maxRows);
  // The fee counts towards the destination side's limit, as mempool truncates
  // its outputs with the fee already among them; it always stays visible.
  const destinationRows = withFee(
    compactGraphRows(graph.outputs, "output", feeRow ? Math.max(1, maxRows - 1) : maxRows),
  );
  return {
    inputRows,
    destinationRows,
    layoutInputRows: redact(inputRows),
    layoutDestinationRows: redact(destinationRows),
    totalInputRows: redact(graph.inputs.map((node) => ({ ...node, side: "input" as const }))),
    totalDestinationRows: redact(
      withFee(graph.outputs.map((node) => ({ ...node, side: "output" as const }))),
    ),
  };
}

export function graphIsLiquid(graph: DrawableGraph) {
  return graph.transaction?.chain === "liquid";
}
