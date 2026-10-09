import {
  BOWTIE_LINE_LIMIT,
  bowtieLines,
  bowtieMinimumSpan,
  bowtieTotal,
  graphIsLiquid,
  graphLayoutRows,
  type BowtieLine,
  type DrawableGraph,
} from "../TransactionGraphGeometry";
import type { GraphRow } from "../TransactionGraphModel";

/**
 * mempool's bowtie in glass, after the Bitcoin Austria artwork lab: one ribbon
 * per input and per output, as thick as mempool draws that strand, with a block
 * for the coin at its outer end. The ribbons meet in one glass collar, because
 * a transaction spends its inputs together; their order does not say which
 * input paid which output. Nothing here is accounting truth.
 *
 * Positions come from a 1500 × 600 canvas with mempool's band of up to 100 px;
 * one scene unit is 100 px. x runs from inputs to outputs, y is up. mempool
 * sizes its canvas to the page; 2.5 : 1 matches the sheet's graph panel, so the
 * piece fills its width instead of leaving it half empty.
 */

const CANVAS_WIDTH = 1500;
const CANVAS_HEIGHT = 600;
const COMBINED_WEIGHT = 100;
/** Where a ribbon leaves its block: the block sits just outside. */
const OUTER_EDGE = 70;
const UNIT = 1 / 100;
/** mempool draws a zero-value output as a short stub of this length and width. */
const ZERO_STUB_LENGTH = 60;
const ZERO_THICKNESS = 4;
const CURVE_SAMPLES = 40;

export const RIBBON_DEPTH = 0.07;
export const BLOCK_WIDTH = 0.42;
export const BLOCK_DEPTH = 0.5;
/** A coin never shrinks below this height, so dust stays a visible block. */
const MIN_BLOCK_HEIGHT = 0.03;

export type RibbonLeg = {
  id: string;
  side: "input" | "output";
  row: GraphRow;
  /** No known amount: its ribbon is drawn frosted. */
  estimated: boolean;
  owned: boolean;
  zeroValue: boolean;
  /** Block centre and height, in scene units. */
  x: number;
  y: number;
  height: number;
};

export type RibbonPath = {
  legId: string;
  side: "input" | "output";
  estimated: boolean;
  fee: boolean;
  /** Ribbon width across its path, in scene units. */
  thickness: number;
  /** Centreline, outer end first. */
  points: Array<[number, number]>;
};

export type RibbonLayout = {
  /** One block per input and output; the fee has a ribbon and no coin. */
  legs: RibbonLeg[];
  ribbons: RibbonPath[];
  /** The collar around the band where both sides meet. */
  center: { halfHeight: number };
};

function bezier(p0: number, p1: number, p2: number, p3: number, t: number) {
  const u = 1 - t;
  return u * u * u * p0 + 3 * u * u * t * p1 + 3 * u * t * t * p2 + t * t * t * p3;
}

/**
 * mempool's makePath for the left side: straight out of the coin, one cubic
 * curve whose start and end shift by the line's offset, straight into the band.
 */
function strandPoints(line: BowtieLine): Array<[number, number]> {
  const start = OUTER_EDGE;
  const end = CANVAS_WIDTH / 2;
  // mempool's makePath: both curve ends move out by the offset, from a start the
  // side's widest strand and largest offset set.
  const curveStart = Math.max(start + 5, OUTER_EDGE + line.curveBase - line.offset);
  const curveEnd = Math.max(curveStart + 18, end - line.offset - 10);
  const midpoint = (curveStart + curveEnd) / 2;
  const points: Array<[number, number]> = [[start, line.outerY]];
  for (let step = 0; step <= CURVE_SAMPLES; step += 1) {
    const t = step / CURVE_SAMPLES;
    points.push([
      bezier(curveStart, midpoint, midpoint, curveEnd, t),
      bezier(line.outerY, line.outerY, line.innerY, line.innerY, t),
    ]);
  }
  points.push([end, line.innerY]);
  return points;
}

function mirrored(points: Array<[number, number]>): Array<[number, number]> {
  return points.map(([x, y]) => [CANVAS_WIDTH - x, y]);
}

export function ribbonLayout(
  graph: DrawableGraph,
  hideSensitive: boolean,
  maxRows = BOWTIE_LINE_LIMIT,
): RibbonLayout {
  // mempool keeps `lineLimit` legs and folds the rest into one more-leg.
  const { layoutInputRows, layoutDestinationRows, totalInputRows, totalDestinationRows } =
    graphLayoutRows(graph, hideSensitive, maxRows + 1);
  const total = bowtieTotal(totalInputRows, totalDestinationRows, graphIsLiquid(graph));
  // The 3D view has no scrolling: both sides share one span, tall enough for
  // the fuller side at minimum spacing, so their outer ends fill the same height.
  const span = Math.max(
    CANVAS_HEIGHT,
    bowtieMinimumSpan(layoutInputRows, total, COMBINED_WEIGHT, ZERO_THICKNESS),
    bowtieMinimumSpan(layoutDestinationRows, total, COMBINED_WEIGHT, ZERO_THICKNESS),
  );
  const options = {
    height: span,
    combinedWeight: COMBINED_WEIGHT,
    curveWidth: CANVAS_WIDTH / 2 - OUTER_EDGE - 12,
    outerTop: 0,
    outerSpan: span,
    zeroThickness: ZERO_THICKNESS,
  };
  const toScene = (x: number, y: number): [number, number] => [
    (x - CANVAS_WIDTH / 2) * UNIT,
    (span / 2 - y) * UNIT,
  ];
  const legs: RibbonLeg[] = [];
  const ribbons: RibbonPath[] = [];
  const place = (rows: GraphRow[], side: "input" | "output") => {
    bowtieLines(rows, total, options).forEach((line, index) => {
      const row = rows[index];
      const id = `${side}:${row.id}`;
      const fee = row.side === "fee";
      const outer: Array<[number, number]> = line.zeroValue
        ? [
            [OUTER_EDGE, line.outerY],
            [OUTER_EDGE + ZERO_STUB_LENGTH, line.outerY],
          ]
        : strandPoints(line);
      const points = (side === "input" ? outer : mirrored(outer)).map(([x, y]) => toScene(x, y));
      ribbons.push({
        legId: id,
        side,
        estimated: line.estimated,
        fee,
        thickness: line.thickness * UNIT,
        points,
      });
      if (fee) return;
      const blockCentre = OUTER_EDGE - BLOCK_WIDTH / UNIT / 2;
      const [x, y] = toScene(
        side === "input" ? blockCentre : CANVAS_WIDTH - blockCentre,
        line.outerY,
      );
      legs.push({
        id,
        side,
        row,
        estimated: line.estimated,
        owned: row.ownership === "owned",
        zeroValue: line.zeroValue,
        x,
        y,
        height: Math.max(MIN_BLOCK_HEIGHT, line.thickness * UNIT),
      });
    });
  };
  place(layoutInputRows, "input");
  place(layoutDestinationRows, "output");
  return {
    legs,
    ribbons,
    center: { halfHeight: ((COMBINED_WEIGHT + 0.5) / 2) * UNIT },
  };
}

export type LegendKey = "known" | "unknown" | "own" | "other" | "fee";

/** The swatches a drawing uses; a legend entry for nothing on screen is noise. */
export function legendKeys(layout: RibbonLayout): Set<LegendKey> {
  const keys = new Set<LegendKey>();
  for (const ribbon of layout.ribbons) {
    if (ribbon.fee) keys.add("fee");
    else keys.add(ribbon.estimated ? "unknown" : "known");
  }
  for (const leg of layout.legs) keys.add(leg.owned ? "own" : "other");
  return keys;
}
