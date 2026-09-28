import {
  BufferGeometry,
  CatmullRomCurve3,
  Color,
  DirectionalLight,
  Float32BufferAttribute,
  Group,
  HemisphereLight,
  Mesh,
  MeshPhysicalMaterial,
  NeutralToneMapping,
  OrthographicCamera,
  PMREMGenerator,
  Scene,
  SRGBColorSpace,
  Sphere,
  Box3,
  TubeGeometry,
  Vector3,
  WebGLRenderer,
  type Material,
} from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";
import { mergeGeometries } from "three/addons/utils/BufferGeometryUtils.js";

import {
  BLOCK_DEPTH,
  BLOCK_WIDTH,
  RIBBON_DEPTH,
  RIBBON_HEIGHT,
  type RibbonLayout,
} from "./ribbonLayout";

/**
 * The only module that imports three.js; it is loaded when the 3D view opens.
 * Everything is generated here: no textures, models or network requests.
 */

export type GlassSceneLook = { background: string; dark: boolean };

export type GlassScene = {
  resize: (width: number, height: number) => void;
  setView: (yaw: number, pitch: number) => void;
  render: () => void;
  dispose: () => void;
};

const SEGMENTS = 56;
const EDGE = 0.014;
/** The smallest frame, so a two-leg transaction is not blown up to fill the view. */
const MIN_HALF_WIDTH = 4.4;
const MIN_HALF_HEIGHT = 2.5;
const PROFILE_STEPS = 4;
// The straight lead-in and lead-out of each ribbon, as in the lab's ribbon().
const CURVE_START = 0.14;
const CURVE_END = 0.86;

function smootherstep(value: number) {
  const t = Math.min(1, Math.max(0, value));
  return t * t * t * (t * (t * 6 - 15) + 10);
}

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

function centreline(from: [number, number], to: [number, number]) {
  return Array.from({ length: SEGMENTS + 1 }, (_, index) => {
    const t = index / SEGMENTS;
    const bend = smootherstep((t - CURVE_START) / (CURVE_END - CURVE_START));
    return [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * bend] as const;
  });
}

/**
 * A flat ribbon swept along an S-curve in the xy plane, capped at both ends.
 * `offset` shifts it across the curve, for the edge lines beside a ribbon.
 */
function ribbonGeometry(
  from: [number, number],
  to: [number, number],
  height = RIBBON_HEIGHT,
  depth = RIBBON_DEPTH,
  offset = 0,
) {
  const path = centreline(from, to);
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
      indices.push(a, c, b, b, c, d);
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

type GlassTone = {
  color: string;
  attenuation: string;
  roughness: number;
  distance: number;
  /** Below 1 the base colour shows; a dark surface leaves full glass black. */
  transmission: number;
  glow: string;
  glowIntensity: number;
  rim: string;
};

function glass(tone: GlassTone, thickness = 0.6) {
  return new MeshPhysicalMaterial({
    color: new Color(tone.color),
    metalness: 0,
    roughness: tone.roughness,
    transmission: tone.transmission,
    thickness,
    ior: 1.5,
    clearcoat: 1,
    clearcoatRoughness: 0.04,
    attenuationColor: new Color(tone.attenuation),
    attenuationDistance: tone.distance,
    specularIntensity: 1,
    emissive: new Color(tone.glow),
    emissiveIntensity: tone.glowIntensity,
    // A light rim on grazing edges reads as cut acrylic.
    sheen: 1,
    sheenColor: new Color(tone.rim),
    sheenRoughness: 0.25,
  });
}

const LIGHT_TONES = {
  known: { color: "#d6e6ff", attenuation: "#2f7cf6", roughness: 0.04, distance: 1.1, transmission: 0.92, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
  estimated: { color: "#f5f7fa", attenuation: "#b8c2d0", roughness: 0.5, distance: 3, transmission: 0.85, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
  fee: { color: "#ffe9a8", attenuation: "#f59e0b", roughness: 0.06, distance: 0.8, transmission: 0.9, glow: "#000000", glowIntensity: 0, rim: "#fff7d6" },
  feeEstimated: { color: "#fbf3dc", attenuation: "#e7c77e", roughness: 0.5, distance: 2.5, transmission: 0.85, glow: "#000000", glowIntensity: 0, rim: "#fffaf0" },
  center: { color: "#f3f7fc", attenuation: "#8fb8f5", roughness: 0.02, distance: 3.5, transmission: 1, glow: "#000000", glowIntensity: 0, rim: "#ffffff" },
} satisfies Record<string, GlassTone>;

const DARK_TONES = {
  known: { color: "#9cc4ff", attenuation: "#3b82f6", roughness: 0.05, distance: 1.4, transmission: 0.55, glow: "#1d4ed8", glowIntensity: 0.45, rim: "#cfe1ff" },
  estimated: { color: "#cbd3de", attenuation: "#94a3b8", roughness: 0.5, distance: 3, transmission: 0.45, glow: "#334155", glowIntensity: 0.4, rim: "#e2e8f0" },
  fee: { color: "#ffd466", attenuation: "#f59e0b", roughness: 0.06, distance: 1, transmission: 0.5, glow: "#b45309", glowIntensity: 0.6, rim: "#fff1c2" },
  feeEstimated: { color: "#e9dcb8", attenuation: "#c9a860", roughness: 0.5, distance: 2.5, transmission: 0.45, glow: "#57451f", glowIntensity: 0.4, rim: "#f6ecd2" },
  center: { color: "#dbe5f3", attenuation: "#60a5fa", roughness: 0.03, distance: 4, transmission: 0.62, glow: "#1e293b", glowIntensity: 0.5, rim: "#e0ecff" },
} satisfies Record<string, GlassTone>;

function satin(color: string) {
  return new MeshPhysicalMaterial({
    color: new Color(color),
    metalness: 0.15,
    roughness: 0.32,
    clearcoat: 1,
    clearcoatRoughness: 0.12,
  });
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
  const renderer = new WebGLRenderer({
    canvas,
    antialias: true,
    powerPreference: "low-power",
  });
  try {
    return buildGlassScene(renderer, layout, look);
  } catch (error) {
    // Release the context at once; the caller shows the 2D graph instead.
    renderer.dispose();
    renderer.forceContextLoss();
    throw error;
  }
}

function buildGlassScene(
  renderer: WebGLRenderer,
  layout: RibbonLayout,
  look: GlassSceneLook,
): GlassScene {
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.toneMapping = NeutralToneMapping;

  const scene = new Scene();
  // Opaque, in the surface colour: transmission refracts what is behind the
  // glass, and a transparent clear would make every ribbon look dark.
  scene.background = new Color(look.background);
  const pmrem = new PMREMGenerator(renderer);
  const room = new RoomEnvironment();
  scene.environment = pmrem.fromScene(room, 0.04).texture;
  scene.environmentIntensity = look.dark ? 0.9 : 1.15;
  room.dispose();
  pmrem.dispose();
  scene.add(new HemisphereLight(0xffffff, look.dark ? 0x334155 : 0x8a94a6, look.dark ? 0.7 : 0.9));
  const key = new DirectionalLight(0xffffff, 2.2);
  key.position.set(-4, 7, 8);
  scene.add(key);

  const tones = look.dark ? DARK_TONES : LIGHT_TONES;
  const materials = {
    known: glass(tones.known),
    estimated: glass(tones.estimated),
    fee: glass(tones.fee),
    feeEstimated: glass(tones.feeEstimated),
    center: glass(tones.center, 1.4),
    // The dark line along each ribbon edge that gives the lab pieces their drawing.
    edge: satin(look.dark ? "#0b1220" : "#3f4a5c"),
    owned: satin("#2563eb"),
    external: satin(look.dark ? "#64748b" : "#7b8798"),
  } satisfies Record<string, Material>;

  const content = new Group();
  const ribbonGroups = {
    known: [] as BufferGeometry[],
    estimated: [] as BufferGeometry[],
    edge: [] as BufferGeometry[],
  };
  for (const ribbon of layout.ribbons) {
    ribbonGroups[ribbon.estimated ? "estimated" : "known"].push(
      ribbonGeometry(ribbon.from, ribbon.to),
    );
    // Only along the edges: a full backing would darken the glass it shows through.
    for (const side of [1, -1]) {
      ribbonGroups.edge.push(
        ribbonGeometry(
          ribbon.from,
          ribbon.to,
          EDGE * 2,
          RIBBON_DEPTH * 0.7,
          side * (RIBBON_HEIGHT / 2 + EDGE * 0.4),
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
  if (layout.fee) {
    content.add(
      new Mesh(
        ribbonGeometry(layout.fee.from, layout.fee.to, RIBBON_HEIGHT * 0.34, RIBBON_DEPTH * 0.8),
        layout.fee.estimated ? materials.feeEstimated : materials.fee,
      ),
    );
  }
  for (const leg of layout.legs) {
    const height = Math.max(0.08, leg.top - leg.bottom);
    const block = new Mesh(
      new RoundedBoxGeometry(BLOCK_WIDTH, height, BLOCK_DEPTH, 3, 0.05),
      leg.owned ? materials.owned : materials.external,
    );
    block.position.set(leg.x, (leg.top + leg.bottom) / 2, 0);
    content.add(block);
  }
  content.add(
    new Mesh(
      collarGeometry(layout.center.halfHeight, RIBBON_DEPTH / 2 + 0.07, 0.05),
      materials.center,
    ),
  );

  const pivot = new Group();
  const bounds = new Box3().setFromObject(content);
  content.position.sub(bounds.getCenter(new Vector3()));
  pivot.add(content);
  scene.add(pivot);

  // Far enough out that no turn can bring a corner past the camera, however
  // tall a 250-leg fan-in gets.
  const reach = new Box3().setFromObject(pivot).getBoundingSphere(new Sphere()).radius;
  const camera = new OrthographicCamera(-1, 1, 1, -1, 0.1, reach * 2 + 20);
  camera.position.set(0, 0, reach + 10);
  camera.lookAt(0, 0, 0);
  let frame = { halfWidth: 0, halfHeight: 0 };
  let size = { width: 1, height: 1 };

  // The camera looks at the pivot, so the frame is symmetric around it. It only
  // grows: turning never clips the piece, and never zooms back in.
  const fit = () => {
    pivot.updateMatrixWorld(true);
    const box = new Box3().setFromObject(pivot);
    frame = {
      halfWidth: Math.max(
        frame.halfWidth,
        MIN_HALF_WIDTH,
        Math.max(Math.abs(box.min.x), Math.abs(box.max.x)) * 1.06,
      ),
      halfHeight: Math.max(
        frame.halfHeight,
        MIN_HALF_HEIGHT,
        Math.max(Math.abs(box.min.y), Math.abs(box.max.y)) * 1.1,
      ),
    };
  };
  const applyFrame = () => {
    if (!frame.halfWidth || !frame.halfHeight) return;
    const aspect = size.width / Math.max(1, size.height);
    let { halfWidth, halfHeight } = frame;
    if (halfWidth / halfHeight > aspect) halfHeight = halfWidth / aspect;
    else halfWidth = halfHeight * aspect;
    camera.left = -halfWidth;
    camera.right = halfWidth;
    camera.top = halfHeight;
    camera.bottom = -halfHeight;
    camera.updateProjectionMatrix();
  };

  return {
    resize(width, height) {
      size = { width: Math.max(1, width), height: Math.max(1, height) };
      renderer.setSize(size.width, size.height, false);
      applyFrame();
    },
    setView(yaw, pitch) {
      pivot.rotation.set(pitch, yaw, 0);
      fit();
      applyFrame();
    },
    render() {
      renderer.render(scene, camera);
    },
    dispose() {
      scene.traverse((object) => {
        if (object instanceof Mesh) object.geometry.dispose();
      });
      Object.values(materials).forEach((material) => material.dispose());
      scene.environment?.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
    },
  };
}
