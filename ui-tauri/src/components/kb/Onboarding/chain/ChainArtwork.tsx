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
 * above it. Each 3D face is rasterized into square cells, with monochrome
 * shading and Bitcoin orange. Local SVG only: no WebGL or live chain data.
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
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const update = () => setVisible(document.visibilityState !== "hidden");
    update();
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  const looping = live && !reducedMotion;
  const [loop, setLoop] = useState<LoopState>({
    tip: 0,
    layers: fillLayers,
    found: false,
  });
  useEffect(() => {
    if (!looping || !visible) return undefined;
    const timer = window.setInterval(() => setLoop(advanceChain), TICK_MS);
    return () => window.clearInterval(timer);
  }, [looping, visible]);

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
      className={cn("kb-chain-artwork block w-full text-foreground", className)}
      shapeRendering="crispEdges"
      data-paused={!visible ? "" : undefined}
      data-still={reducedMotion ? "" : undefined}
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
const PIXELS = 24;
const project = (x: number, y: number, z: number): [number, number] =>
  [Math.round((x + z * DEPTH_X) * PIXELS), Math.round((-y + z * DEPTH_Y) * PIXELS)];

/** Front, top and side fills; opacity keeps their depth readable in either theme. */
const FACE_FILLS = {
  tx: ["currentColor", "currentColor", "currentColor"],
  fee: ["#f7931a", "#fbab4a", "#d97d0e"],
} as const;

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

type Point = [number, number];
const points = (list: Array<[number, number, number]>) =>
  list.map(([x, y, z]) => project(x, y, z));

/** Scan each face into whole-cell horizontal runs, preserving its 3D projection. */
function pixelFace(vertices: Point[]) {
  let path = "";
  const ys = vertices.map((p) => p[1]);
  for (let y = Math.min(...ys); y < Math.max(...ys); y++) {
    const scan = y + 0.5;
    const crossings: number[] = [];
    vertices.forEach(([x0, y0], index) => {
      const [x1, y1] = vertices[(index + 1) % vertices.length];
      if ((y0 > scan) !== (y1 > scan)) {
        crossings.push(x0 + ((scan - y0) * (x1 - x0)) / (y1 - y0));
      }
    });
    crossings.sort((a, b) => a - b);
    if (crossings.length < 2) continue;
    const start = Math.ceil(crossings[0] - 0.5);
    const end = Math.ceil(crossings.at(-1)! - 0.5);
    if (end > start) path += `M${start} ${y}h${end - start}v1h${start - end}z`;
  }
  return path;
}

/** Bresenham edges keep outlines on the same coarse pixel grid as the faces. */
function pixelEdge([x0, y0]: Point, [x1, y1]: Point, dashed = false) {
  const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0);
  const sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
  let error = dx + dy, path = "", tick = 0;
  for (;;) {
    if (!dashed || Math.floor(tick / 2) % 2 === 0) path += `M${x0} ${y0}h1v1h-1z`;
    if (x0 === x1 && y0 === y1) break;
    const twice = 2 * error;
    if (twice >= dy) { error += dy; x0 += sx; }
    if (twice <= dx) { error += dx; y0 += sy; }
    tick++;
  }
  return path;
}

function pixelOutline(vertices: Point[]) {
  return vertices.map((p, i) => pixelEdge(p, vertices[(i + 1) % vertices.length])).join("");
}

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
  const corner = project(x1, y0, z0);
  return [project(x0, y0, z0), project(x1, y1, z0), project(x1, y0, z1)]
    .map((end) => pixelEdge(corner, end, true)).join("");
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
  const pad = 0.25 * PIXELS;
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
  const [dx, dy] = project(x, 0, z).map((v) => Math.round(v / 4) * 4);
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
            className="kb-chain-tx"
            style={{ "--kb-chain-i": index } as CSSProperties}
          >
            <path d={pixelFace(side)} fill={sideFill} opacity="0.4" />
            <path d={pixelFace(top)} fill={topFill} opacity="0.7" />
            <path d={pixelFace(front)} fill={frontFill} />
            <path d={[side, top, front].map(pixelOutline).join("")} className="fill-background" />
          </g>
        );
      })}
      {shell && (
        <g
          fill="currentColor"
          className={cn(
            "kb-chain-shell text-foreground/45",
            block.offset !== 0 && "opacity-0",
            found && "kb-chain-found",
          )}
        >
          <path d={hiddenEdges(shell)} opacity="0.5" />
          <path d={Object.values(faces(shell)).map(pixelOutline).join("")} />
          <path d={pixelFace(faces(shell).side)} className="fill-ink/[0.03]" />
          <path d={pixelFace(faces(shell).top)} className="fill-ink/[0.05]" />
          <path d={pixelFace(faces(shell).front)} className="fill-ink/[0.02]" />
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
            className="kb-chain-pending fill-current"
            style={{ "--kb-chain-i": index } as CSSProperties}
          >
            <path d={pixelOutline(side)} opacity="0.5" />
            <path d={pixelOutline(top)} />
            <path d={pixelOutline(front)} />
          </g>
        );
      })}
    </g>
  );
}
