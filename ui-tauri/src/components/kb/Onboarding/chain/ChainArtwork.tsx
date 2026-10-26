import { useEffect, useMemo, useState, type CSSProperties } from "react";

import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";

import {
  BLOCK_LAYERS,
  advanceChain,
  blockCentre,
  chainBlocks,
  chainLayout,
  mempool,
  type ChainBlock,
  type ChainBox,
  type LoopState,
} from "./chainLayout";

/** One beat of the live chain: a layer lands, a block is found, or it advances. */
const TICK_MS = 900;

/**
 * Setup's Bitcoin artwork: `mined` blocks packed with transactions and the
 * next block, an outline holding `fill` of a block, with the mempool waiting
 * above it. Flat shaded blocks in the transaction graph's coin blue and
 * Bitcoin orange, drawn as SVG: no WebGL, no chain data.
 *
 * `live` runs the chain on a loop from `fill`: blocks fill, are found and move
 * back. Otherwise the drawing follows `fill`, and a new layer drops in when it
 * rises. Reduced motion holds the live chain still at `fill`.
 */
export function ChainArtwork({
  mined,
  fill,
  live = false,
  className,
}: {
  mined: number;
  fill: number;
  live?: boolean;
  /** Size of the drawing area. */
  className: string;
}) {
  const fillLayers = Math.round(Math.min(1, Math.max(0, fill)) * BLOCK_LAYERS);
  const reducedMotion = usePrefersReducedMotion();
  const looping = live && !reducedMotion;
  const [loop, setLoop] = useState<LoopState>({
    tip: 0,
    layers: fillLayers,
    found: false,
  });
  useEffect(() => {
    if (!looping) return undefined;
    const timer = window.setInterval(() => setLoop(advanceChain), TICK_MS);
    return () => window.clearInterval(timer);
  }, [looping]);

  const state = looping ? loop : { tip: 0, layers: fillLayers, found: false };
  const blocks = useMemo(
    () =>
      chainBlocks({
        tip: state.tip,
        mined,
        layers: state.layers,
        trailing: looping,
      }),
    [looping, mined, state.layers, state.tip],
  );
  const viewBox = useMemo(() => frame(mined), [mined]);
  return (
    <svg
      aria-hidden="true"
      viewBox={viewBox}
      preserveAspectRatio="xMidYMid meet"
      className={cn("block w-full", className)}
      strokeLinejoin="round"
    >
      {blocks.map((block) => (
        <Block
          key={block.id}
          block={block}
          inChain={block.offset <= mined}
          found={state.found && block.offset === 0}
        />
      ))}
      {state.layers < BLOCK_LAYERS && !state.found && <Mempool />}
    </svg>
  );
}

// Oblique projection: depth recedes up and to the left, as the chain trails
// back behind the next block, so tops and left-hand sides show.
const DEPTH_X = 0.35;
const DEPTH_Y = 0.25;
const project = (x: number, y: number, z: number) =>
  [x + z * DEPTH_X, -y + z * DEPTH_Y] as const;

/** Front, top and side face fills: the side darkest, the top lightest. */
const FACE_FILLS = {
  tx: ["#2563eb", "#3b82f6", "#1d4ed8"],
  fee: ["#f7931a", "#fbab4a", "#d97d0e"],
} as const;

/** Seam between neighbouring transactions, in the page colour. */
const SEAM = 0.018;

function corners(box: ChainBox) {
  return {
    x0: box.x - box.width / 2,
    x1: box.x + box.width / 2,
    y0: box.y - box.height / 2,
    y1: box.y + box.height / 2,
    z0: box.z - box.depth / 2,
    z1: box.z + box.depth / 2,
  };
}

const points = (list: Array<[number, number, number]>) =>
  list.map(([x, y, z]) => project(x, y, z).join(",")).join(" ");

function faces(box: ChainBox) {
  const { x0, x1, y0, y1, z0, z1 } = corners(box);
  return {
    front: points([[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]]),
    top: points([[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]]),
    side: points([[x0, y0, z1], [x0, y0, z0], [x0, y1, z0], [x0, y1, z1]]),
  };
}

/**
 * The outline's three edges meeting at its hidden back corner, dashed behind
 * the transactions, as a block that has not been found yet.
 */
function hiddenEdges(box: ChainBox) {
  const { x0, x1, y0, y1, z0, z1 } = corners(box);
  const at = (x: number, y: number, z: number) => project(x, y, z).join(",");
  const corner = at(x1, y0, z0);
  return [at(x0, y0, z0), at(x1, y1, z0), at(x1, y0, z1)]
    .map((end) => `M${corner} L${end}`)
    .join(" ");
}

/** Back to front: lower first, and a box hides the left side of its right neighbour. */
const paintOrder = (a: ChainBox, b: ChainBox) => a.z - b.z || a.y - b.y || b.x - a.x;

/** A key that survives the block filling, so only newly arrived boxes animate in. */
const boxKey = (box: ChainBox) =>
  `${box.kind}:${box.x.toFixed(3)}:${box.y.toFixed(3)}:${box.z.toFixed(3)}`;

/**
 * The drawing's fixed frame: every slot of the chain full, with the mempool
 * above the next block, so the chain moving or a block filling never rescales
 * the picture. The block leaving the chain slides out past its edge.
 */
function frame(mined: number) {
  const xs: number[] = [];
  const ys: number[] = [];
  for (const box of chainLayout({ mined, fill: 1 }).boxes.concat(mempool())) {
    const { x0, x1, y0, y1, z0, z1 } = corners(box);
    for (const x of [x0, x1]) {
      for (const y of [y0, y1]) {
        for (const z of [z0, z1]) {
          const [sx, sy] = project(x, y, z);
          xs.push(sx);
          ys.push(sy);
        }
      }
    }
  }
  const pad = 0.25;
  const minX = Math.min(...xs) - pad;
  const minY = Math.min(...ys) - pad;
  return `${minX} ${minY} ${Math.max(...xs) + pad - minX} ${Math.max(...ys) + pad - minY}`;
}

/**
 * One block, placed at its slot. Moving slots is a transform transition on
 * the group, so the whole block glides back as the chain advances; leaving
 * the chain fades it out.
 */
function Block({
  block,
  inChain,
  found,
}: {
  block: ChainBlock;
  inChain: boolean;
  found: boolean;
}) {
  const { x, z } = blockCentre(block.offset);
  const [dx, dy] = project(x, 0, z);
  const transactions = useMemo(
    () => block.boxes.filter((box) => box.kind !== "shell").sort(paintOrder),
    [block.boxes],
  );
  const shell = block.boxes.find((box) => box.kind === "shell");
  return (
    <g
      className="kb-chain-block"
      style={{ transform: `translate(${dx}px, ${dy}px)`, opacity: inChain ? 1 : 0 }}
    >
      {transactions.map((box, index) => {
        const { front, top, side } = faces(box);
        const [frontFill, topFill, sideFill] = FACE_FILLS[box.kind === "fee" ? "fee" : "tx"];
        return (
          <g
            key={boxKey(box)}
            className="kb-chain-tx stroke-background"
            strokeWidth={SEAM}
            style={{ "--kb-chain-i": index } as CSSProperties}
          >
            <polygon points={side} fill={sideFill} />
            <polygon points={top} fill={topFill} />
            <polygon points={front} fill={frontFill} />
          </g>
        );
      })}
      {shell && (
        <g
          fill="none"
          className={cn(
            "kb-chain-shell stroke-ink/35",
            block.offset !== 0 && "opacity-0",
            found && "kb-chain-found",
          )}
          strokeWidth={0.022}
        >
          <path d={hiddenEdges(shell)} strokeDasharray="0.07 0.06" />
          <polygon points={faces(shell).side} className="fill-ink/[0.03]" />
          <polygon points={faces(shell).top} className="fill-ink/[0.05]" />
          <polygon points={faces(shell).front} className="fill-ink/[0.02]" />
        </g>
      )}
    </g>
  );
}

/** Unconfirmed transactions drifting above the next block. */
function Mempool() {
  const boxes = useMemo(() => [...mempool()].sort(paintOrder), []);
  return (
    <g className="kb-chain-mempool">
      {boxes.map((box, index) => {
        const { front, top, side } = faces(box);
        return (
          <g
            key={boxKey(box)}
            className="kb-chain-pending fill-paper stroke-ink/45"
            strokeWidth={0.02}
            style={{ "--kb-chain-i": index } as CSSProperties}
          >
            <polygon points={side} className="fill-paper-2" />
            <polygon points={top} />
            <polygon points={front} />
          </g>
        );
      })}
    </g>
  );
}
