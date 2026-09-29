import {
  BufferGeometry,
  CatmullRomCurve3,
  Float32BufferAttribute,
  Mesh,
  TubeGeometry,
  Vector3,
  type Material,
} from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";
import { mergeGeometries } from "three/addons/utils/BufferGeometryUtils.js";

import {
  BLOCK_DEPTH,
  BLOCK_WIDTH,
  RIBBON_DEPTH,
  type RibbonLayout,
} from "./ribbonLayout";

// This scene and its shared three.js dependencies load only when the 3D view opens.
import { coinMaterial, glass, satin, LIGHT_TONES, DARK_TONES } from "../../kb/glass3d/materials";
import { createGlassStage, type GlassScene, type GlassSceneLook } from "../../kb/glass3d/stage";
export type { GlassScene, GlassSceneLook } from "../../kb/glass3d/stage";

const EDGE = 0.014;
const PROFILE_STEPS = 4;
/** Ribbons thinner than this get no edge lines: the lines would swallow them. */
const EDGED_THICKNESS = 0.08;

/** A stadium cross-section: flat faces towards the viewer, round edges. */
function profile(height: number, depth: number) {
  const radius = Math.min(depth / 2, height / 2);
  const reach = height / 2 - radius;
  const points: Array<[number, number]> = [];
  for (let step = 0; step <= PROFILE_STEPS; step += 1) {
    const angle = -Math.PI / 2 + (Math.PI * step) / PROFILE_STEPS;
    points.push([reach + radius * Math.cos(angle), radius * Math.sin(angle)]);
  }
  for (let step = 0; step <= PROFILE_STEPS; step += 1) {
    const angle = Math.PI / 2 + (Math.PI * step) / PROFILE_STEPS;
    points.push([-reach + radius * Math.cos(angle), radius * Math.sin(angle)]);
  }
  return points;
}

/**
 * A flat ribbon swept along a centreline in the xy plane, capped at both ends.
 * `height` is its width across the path; `offset` shifts it sideways, for the
 * edge lines beside a ribbon.
 */
export function ribbonGeometry(
  path: ReadonlyArray<readonly [number, number]>,
  height: number,
  depth = RIBBON_DEPTH,
  offset = 0,
) {
  const ring = profile(height, depth);
  const positions: number[] = [];
  const indices: number[] = [];
  path.forEach(([x, y], index) => {
    const [ax, ay] = path[Math.max(0, index - 1)];
    const [bx, by] = path[Math.min(path.length - 1, index + 1)];
    const length = Math.hypot(bx - ax, by - ay) || 1;
    const nx = -(by - ay) / length;
    const ny = (bx - ax) / length;
    for (const [across, deep] of ring) {
      positions.push(x + nx * (across + offset), y + ny * (across + offset), deep);
    }
  });
  const size = ring.length;
  for (let index = 0; index < path.length - 1; index += 1) {
    for (let corner = 0; corner < size; corner += 1) {
      const a = index * size + corner;
      const b = index * size + ((corner + 1) % size);
      const c = a + size;
      const d = b + size;
      // The profile runs counterclockwise around the path, so this order
      // turns the side faces outward, like the end caps.
      indices.push(a, b, c, b, d, c);
    }
  }
  for (const [ringIndex, flip] of [
    [0, true],
    [path.length - 1, false],
  ] as const) {
    const centre = positions.length / 3;
    const [ex, ey] = [0, 1].map((axis) => {
      let sum = 0;
      for (let corner = 0; corner < size; corner += 1) {
        sum += positions[(ringIndex * size + corner) * 3 + axis];
      }
      return sum / size;
    });
    positions.push(ex, ey, 0);
    for (let corner = 0; corner < size; corner += 1) {
      const a = ringIndex * size + corner;
      const b = ringIndex * size + ((corner + 1) % size);
      indices.push(...(flip ? [centre, b, a] : [centre, a, b]));
    }
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new Float32BufferAttribute(positions, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

/**
 * The collar where every input's ribbons end and the outputs' begin: a
 * rounded-rectangle glass loop around the waist, in the yz plane at x = 0.
 */
function collarGeometry(halfHeight: number, halfDepth: number, radius: number) {
  const corner = Math.min(halfDepth, halfHeight) * 0.9;
  const points: Vector3[] = [];
  const corners: Array<[number, number, number]> = [
    [halfHeight - corner, halfDepth - corner, 0],
    [-(halfHeight - corner), halfDepth - corner, Math.PI / 2],
    [-(halfHeight - corner), -(halfDepth - corner), Math.PI],
    [halfHeight - corner, -(halfDepth - corner), (3 * Math.PI) / 2],
  ];
  for (const [cy, cz, start] of corners) {
    for (let step = 0; step <= 6; step += 1) {
      const angle = start + ((Math.PI / 2) * step) / 6;
      points.push(new Vector3(0, cy + corner * Math.cos(angle), cz + corner * Math.sin(angle)));
    }
  }
  return new TubeGeometry(new CatmullRomCurve3(points, true), 96, radius, 12, true);
}

export function createGlassScene(
  canvas: HTMLCanvasElement,
  layout: RibbonLayout,
  look: GlassSceneLook,
): GlassScene {
  return createGlassStage(canvas, look, (content, own) => {
    const tones = look.dark ? DARK_TONES : LIGHT_TONES;
    const materials = {
      known: own(glass(tones.known)),
      estimated: own(glass(tones.estimated)),
      fee: own(glass(tones.fee)),
      feeEstimated: own(glass(tones.feeEstimated)),
      center: own(glass(tones.center, 1.4)),
      // The dark line along each ribbon edge that gives the lab pieces their drawing.
      edge: own(satin(look.dark ? "#0b1220" : "#3f4a5c")),
      owned: own(coinMaterial(true, look.dark)),
      external: own(coinMaterial(false, look.dark)),
    } satisfies Record<string, Material>;

    const ribbonGroups = {
      known: [] as BufferGeometry[],
      estimated: [] as BufferGeometry[],
      fee: [] as BufferGeometry[],
      feeEstimated: [] as BufferGeometry[],
      edge: [] as BufferGeometry[],
    };
    for (const ribbon of layout.ribbons) {
      const kind = ribbon.fee
        ? ribbon.estimated
          ? "feeEstimated"
          : "fee"
        : ribbon.estimated
          ? "estimated"
          : "known";
      ribbonGroups[kind].push(ribbonGeometry(ribbon.points, ribbon.thickness));
      // Only along the edges: a full backing would darken the glass it shows through.
      if (ribbon.fee || ribbon.thickness < EDGED_THICKNESS) continue;
      for (const side of [1, -1]) {
        ribbonGroups.edge.push(
          ribbonGeometry(
            ribbon.points,
            EDGE * 2,
            RIBBON_DEPTH * 0.7,
            side * (ribbon.thickness / 2 + EDGE * 0.4),
          ),
        );
      }
    }
    for (const [kind, geometries] of Object.entries(ribbonGroups) as Array<
      [keyof typeof ribbonGroups, BufferGeometry[]]
    >) {
      if (!geometries.length) continue;
      const merged = mergeGeometries(geometries);
      geometries.forEach((geometry) => geometry.dispose());
      if (merged) content.add(new Mesh(merged, materials[kind]));
    }
    for (const leg of layout.legs) {
      const block = new Mesh(
        new RoundedBoxGeometry(
          BLOCK_WIDTH,
          leg.height,
          // Thin coins are shallow too: seen from above, a deep block would
          // cover the gap to its neighbour and a fan-in would read as one wall.
          Math.min(BLOCK_DEPTH, Math.max(0.08, leg.height * 3)),
          3,
          Math.min(0.05, leg.height / 2.5),
        ),
        leg.owned ? materials.owned : materials.external,
      );
      block.position.set(leg.x, leg.y, 0);
      content.add(block);
    }
    content.add(
      new Mesh(
        collarGeometry(layout.center.halfHeight, RIBBON_DEPTH / 2 + 0.07, 0.05),
        materials.center,
      ),
    );

  });
}
