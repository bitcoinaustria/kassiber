import {
  BOWTIE_LINE_LIMIT,
  bowtieLines,
  bowtieTotal,
  graphIsLiquid,
  graphLayoutRows,
  type BowtieLine,
} from "../TransactionGraphGeometry";
import type { GraphRow, TransactionGraphPayload } from "../TransactionGraphModel";

/**
 * mempool's bowtie in glass, after the Bitcoin Austria artwork lab: one ribbon
 * per input and per output, as thick as mempool draws that strand, with a block
 * for the coin at its outer end. The ribbons meet in one glass collar, because
 * a transaction spends its inputs together; their order does not say which
 * input paid which output. Nothing here is accounting truth.
 *
 * Positions come from mempool's default 1200 × 600 canvas with a band of up to
 * 100 px; one scene unit is 100 px. x runs from inputs to outputs, y is up.
 */

const CANVAS_WIDTH = 1200;
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

function toScene(x: number, y: number): [number, number] {
  return [(x - CANVAS_WIDTH / 2) * UNIT, (CANVAS_HEIGHT / 2 - y) * UNIT];
}

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
  const offset = Math.min(line.offset, Math.max(0, end - start - 44));
  const curveStart = Math.min(Math.max(start + 5, OUTER_EDGE + offset), end - 28);
  const curveEnd = Math.min(Math.max(end - offset - 10, curveStart + 18), end - 4);
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
  graph: TransactionGraphPayload,
  hideSensitive: boolean,
  maxRows = BOWTIE_LINE_LIMIT,
): RibbonLayout {
  const { layoutInputRows, layoutDestinationRows } = graphLayoutRows(
    graph,
    hideSensitive,
    maxRows,
  );
  const total = bowtieTotal(layoutInputRows, layoutDestinationRows, graphIsLiquid(graph));
  // Everything stays on screen: the 3D view has no scrolling, so the outer ends
  // of every leg share the full height on both sides.
  const options = {
    height: CANVAS_HEIGHT,
    combinedWeight: COMBINED_WEIGHT,
    curveWidth: CANVAS_WIDTH / 2 - OUTER_EDGE - 12,
    outerTop: 0,
    outerSpan: CANVAS_HEIGHT,
    zeroThickness: ZERO_THICKNESS,
  };
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
