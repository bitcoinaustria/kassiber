import {
  fallbackVisualSats,
  geometryScale,
  graphLayoutRows,
  legWeights,
  uniformStrandWeight,
} from "../TransactionGraphGeometry";
import type { GraphRow, TransactionGraphPayload } from "../TransactionGraphModel";

/**
 * The ribbon piece from the Bitcoin Austria artwork lab, laid out for one
 * transaction. A coin is a block and its value a bundle of ribbons. Every
 * input's ribbons run into one transaction block and new ribbons leave it for
 * the outputs: a transaction spends its inputs together, so no ribbon links a
 * particular input to a particular output. Widths come from the same geometry
 * as the 2D graph; nothing here is accounting truth.
 *
 * Units are scene units. x runs from inputs to outputs, y is up, z is depth.
 */

export const RIBBON_PITCH = 0.3;
export const RIBBON_HEIGHT = 0.2;
export const RIBBON_DEPTH = 0.07;
export const BLOCK_WIDTH = 0.5;
export const BLOCK_DEPTH = 0.62;
export const CENTER_WIDTH = 0.8;
const LEG_GAP = 0.36;
const ZERO_BLOCK_HEIGHT = RIBBON_PITCH * 0.6;
/** A leg never gets fewer ribbons than this, so dust stays visible. */
const MIN_RIBBONS = 1;
/** Hard ceiling per side, whatever the weights say. */
const MAX_RIBBONS_PER_SIDE = 600;

export type RibbonLeg = {
  id: string;
  side: "input" | "output";
  row: GraphRow;
  ribbons: number;
  /** No known amount: its ribbons are drawn frosted. */
  estimated: boolean;
  owned: boolean;
  zero: boolean;
  /** Block centre and extent. */
  x: number;
  top: number;
  bottom: number;
};

export type RibbonPath = {
  legId: string;
  side: "input" | "output";
  estimated: boolean;
  from: [number, number];
  to: [number, number];
};

export type RibbonLayout = {
  legs: RibbonLeg[];
  ribbons: RibbonPath[];
  /** The network fee: a single thin filament out of the transaction block. */
  fee: { from: [number, number]; to: [number, number]; estimated: boolean } | null;
  center: { halfHeight: number };
  span: number;
  /** True when no leg carries a known amount, so every bundle is the same size. */
  uniform: boolean;
};

/** Ribbons per side: enough to show proportions, few enough to read. */
function ribbonBudget(legCount: number) {
  return Math.max(legCount, Math.min(40, Math.max(12, legCount * 3)));
}

function ribbonCounts(weights: number[], budget: number) {
  // A side's weights can add up to more than the band (a graph with no inputs
  // gives every output the whole band); scale them back so the budget holds.
  const total = weights.reduce((sum, weight) => sum + Math.max(0, weight), 0);
  const scale = total > 1 ? 1 / total : 1;
  const counts = weights.map((weight) =>
    Math.max(MIN_RIBBONS, Math.round(Math.max(0, weight) * scale * budget)),
  );
  const sum = counts.reduce((acc, count) => acc + count, 0);
  if (sum <= MAX_RIBBONS_PER_SIDE) return counts;
  return counts.map((count) =>
    Math.max(MIN_RIBBONS, Math.floor((count * MAX_RIBBONS_PER_SIDE) / sum)),
  );
}

function stackLegs(
  rows: GraphRow[],
  counts: number[],
  flags: Array<{ estimated: boolean; zero: boolean }>,
  side: "input" | "output",
  x: number,
): RibbonLeg[] {
  const heights = rows.map((_, index) =>
    flags[index].zero ? ZERO_BLOCK_HEIGHT : counts[index] * RIBBON_PITCH,
  );
  const total = heights.reduce((sum, height) => sum + height, 0) + LEG_GAP * (rows.length - 1);
  let cursor = total / 2;
  return rows.map((row, index) => {
    const top = cursor;
    const bottom = cursor - heights[index];
    cursor = bottom - LEG_GAP;
    return {
      id: `${side}:${row.id}`,
      side,
      row,
      ribbons: flags[index].zero ? 0 : counts[index],
      estimated: flags[index].estimated,
      owned: row.ownership === "owned",
      zero: flags[index].zero,
      x,
      top,
      bottom,
    };
  });
}

/** Slot centres of a contiguous bundle of `count` ribbons centred on y = 0. */
function centerSlots(count: number) {
  const top = (count * RIBBON_PITCH) / 2;
  return Array.from({ length: count }, (_, index) => top - RIBBON_PITCH * (index + 0.5));
}

export function ribbonLayout(
  graph: TransactionGraphPayload,
  hideSensitive: boolean,
  maxRows: number,
): RibbonLayout {
  const { layoutInputRows, layoutDestinationRows } = graphLayoutRows(graph, hideSensitive, maxRows);
  const scale = geometryScale(layoutInputRows, layoutDestinationRows);
  const rowCount = Math.max(layoutInputRows.length, layoutDestinationRows.length, 2);
  const options = {
    combinedWeight: 1,
    fallbackSats: fallbackVisualSats(scale, rowCount),
    uniformWeight: uniformStrandWeight(1, rowCount),
    hairlineWeight: 0,
  };
  const inputWeights = legWeights(layoutInputRows, scale, options);
  const destinationWeights = legWeights(layoutDestinationRows, scale, options);
  const feeIndex = layoutDestinationRows.findIndex((row) => row.side === "fee");
  const outputRows = layoutDestinationRows.filter((row) => row.side !== "fee");
  const outputWeights = destinationWeights.filter((_, index) => index !== feeIndex);

  const budget = ribbonBudget(Math.max(layoutInputRows.length, outputRows.length));
  const inputCounts = ribbonCounts(
    inputWeights.map((leg) => leg.weight),
    budget,
  );
  const outputCounts = ribbonCounts(
    outputWeights.map((leg) => leg.weight),
    budget,
  );
  const inputStackHeight =
    inputCounts.reduce((sum, count) => sum + count, 0) * RIBBON_PITCH +
    LEG_GAP * Math.max(0, layoutInputRows.length - 1);
  const outputStackHeight =
    outputCounts.reduce((sum, count) => sum + count, 0) * RIBBON_PITCH +
    LEG_GAP * Math.max(0, outputRows.length - 1);
  // Longer runs keep tall fan-ins from folding into steep S-curves.
  const span = Math.max(3.4, Math.max(inputStackHeight, outputStackHeight) * 0.42);

  const inputs = stackLegs(layoutInputRows, inputCounts, inputWeights, "input", -span);
  const outputs = stackLegs(outputRows, outputCounts, outputWeights, "output", span);

  const ribbons: RibbonPath[] = [];
  const inputSlots = centerSlots(inputs.reduce((sum, leg) => sum + leg.ribbons, 0));
  const outputSlots = centerSlots(outputs.reduce((sum, leg) => sum + leg.ribbons, 0));
  const centerInner = CENTER_WIDTH / 2 - 0.06;
  let slot = 0;
  for (const leg of inputs) {
    for (let index = 0; index < leg.ribbons; index += 1) {
      ribbons.push({
        legId: leg.id,
        side: "input",
        estimated: leg.estimated,
        from: [leg.x + BLOCK_WIDTH / 2 - 0.04, leg.top - RIBBON_PITCH * (index + 0.5)],
        to: [-centerInner, inputSlots[slot]],
      });
      slot += 1;
    }
  }
  slot = 0;
  for (const leg of outputs) {
    for (let index = 0; index < leg.ribbons; index += 1) {
      ribbons.push({
        legId: leg.id,
        side: "output",
        estimated: leg.estimated,
        from: [centerInner, outputSlots[slot]],
        to: [leg.x - BLOCK_WIDTH / 2 + 0.04, leg.top - RIBBON_PITCH * (index + 0.5)],
      });
      slot += 1;
    }
  }

  const halfHeight =
    (Math.max(inputSlots.length, outputSlots.length, 1) * RIBBON_PITCH) / 2 + RIBBON_PITCH * 0.5;
  const feeRow = feeIndex >= 0 ? layoutDestinationRows[feeIndex] : null;
  const outputTop = outputs.length ? outputs[0].top : halfHeight;
  const fee = feeRow
    ? {
        from: [centerInner, halfHeight - RIBBON_PITCH * 0.35] as [number, number],
        to: [span * 0.72, Math.max(outputTop, halfHeight) + RIBBON_PITCH * 2.2] as [number, number],
        estimated: destinationWeights[feeIndex].estimated,
      }
    : null;

  return {
    legs: [...inputs, ...outputs],
    ribbons,
    fee,
    center: { halfHeight },
    span,
    uniform: scale.kind === "uniform",
  };
}
