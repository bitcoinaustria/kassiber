import {
  Box3, BoxGeometry, Color, DirectionalLight, Group, HemisphereLight, Mesh,
  MeshBasicMaterial, NeutralToneMapping, OrthographicCamera, PMREMGenerator, Raycaster,
  Scene, Sphere, SRGBColorSpace, Vector2, Vector3, WebGLRenderer,
  type Material, type WebGLRenderTarget,
} from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";

import { glass, LIGHT_TONES, satin } from "./materials";

/** Import at runtime only from a lazy 3D entry point. No external assets or I/O. */
export type GlassSceneLook = { background: string; dark: boolean };
export type GlassScene = {
  resize: (width: number, height: number) => void;
  setView: (yaw: number, pitch: number) => void;
  /** Compiles the scene's shaders without blocking the page; draw after. */
  prepare: () => Promise<void>;
  /** Explicit draw; no animation loop or background work. */
  render: () => void;
  /** The part under a point, in CSS pixels from the canvas's top left. */
  pick: (x: number, y: number) => string | null;
  /** Light one part up, or none. Draw afterwards. */
  highlight: (part: string | null) => void;
  /** Called once if the GPU context is lost; the scene cannot draw again. */
  onContextLost: (listener: () => void) => () => void;
  dispose: () => void;
};

/** A pointer over the scene, with what a scene needs to find its part. */
export type GlassPointer = {
  /** Cast from the camera through the pointer. */
  raycaster: Raycaster;
  /** The pointer, in CSS pixels. */
  x: number;
  y: number;
  /** Where a point of the content lands on screen, in CSS pixels. */
  toScreen: (x: number, y: number) => [number, number];
  /** CSS pixels per scene unit at the current frame. */
  pixelsPerUnit: number;
};

/** A scene whose parts can be pointed at, e.g. the legs of a transaction. */
export type GlassPicking = {
  pick: (pointer: GlassPointer) => string | null;
  highlight: (part: string | null) => void;
};

/**
 * One WebGL renderer for every glass view. Compiled shaders and the room
 * environment belong to a renderer; with one shared renderer only the first
 * scene of a session pays for them, and several views never use up the
 * browser's few WebGL contexts. It also owns the one scene and camera every
 * view draws through, so three's per-scene, per-camera transmission buffer is
 * allocated once instead of once per graph. A view borrows the scene for its
 * draw and copies the result onto its own canvas.
 */
type SharedRenderer = {
  renderer: WebGLRenderer;
  environment: WebGLRenderTarget;
  scene: Scene;
  camera: OrthographicCamera;
  sky: HemisphereLight;
  /**
   * Never drawn, never disposed while the renderer lives: they hold the glass,
   * satin and glow shaders, which three would delete with the last material
   * using them. A later scene then finds its shaders compiled.
   */
  keepers: Group;
  warm: Promise<void> | null;
  users: number;
  idle: ReturnType<typeof setTimeout> | undefined;
  lost: boolean;
  listeners: Set<() => void>;
};

/** An idle renderer is released after this long without a view. */
const IDLE_RELEASE_MS = 30_000;

let shared: SharedRenderer | null = null;

function applyLook(target: SharedRenderer, dark: boolean, background: Color) {
  // Opaque, in the surface colour: transmission refracts what is behind the
  // glass, and a transparent clear would make every ribbon look dark.
  target.scene.background = background;
  target.scene.environmentIntensity = dark ? 0.9 : 1.15;
  target.sky.groundColor.set(dark ? 0x334155 : 0x8a94a6);
  target.sky.intensity = dark ? 0.7 : 0.9;
}

function createShared(): SharedRenderer {
  const canvas = document.createElement("canvas");
  const renderer = new WebGLRenderer({
    canvas,
    antialias: true,
    powerPreference: "low-power",
    // Each view copies the drawing out right after it is made.
    preserveDrawingBuffer: true,
  });
  let environment: WebGLRenderTarget | undefined;
  try {
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.toneMapping = NeutralToneMapping;
    const pmrem = new PMREMGenerator(renderer);
    const room = new RoomEnvironment();
    try {
      environment = pmrem.fromScene(room, 0.04);
    } finally {
      room.dispose();
      pmrem.dispose();
    }
    const scene = new Scene();
    scene.environment = environment.texture;
    const sky = new HemisphereLight(0xffffff, 0x8a94a6, 0.9);
    const key = new DirectionalLight(0xffffff, 2.2);
    key.position.set(-4, 7, 8);
    scene.add(sky, key);
    const keepers = new Group();
    const block = new BoxGeometry(0.1, 0.1, 0.1);
    keepers.add(
      new Mesh(block, glass(LIGHT_TONES.known)),
      new Mesh(block, satin("#2563eb")),
      new Mesh(block, new MeshBasicMaterial({ transparent: true, opacity: 0.4, depthWrite: false })),
    );
    const entry: SharedRenderer = {
      renderer,
      environment,
      scene,
      camera: new OrthographicCamera(-1, 1, 1, -1, 0.1, 100),
      sky,
      keepers,
      warm: null,
      users: 0,
      idle: undefined,
      lost: false,
      listeners: new Set(),
    };
    canvas.addEventListener(
      "webglcontextlost",
      () => {
        entry.lost = true;
        if (shared === entry) shared = null;
        destroyShared(entry);
        entry.listeners.forEach((listener) => listener());
        entry.listeners.clear();
      },
      { once: true },
    );
    return entry;
  } catch (error) {
    environment?.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
    throw error;
  }
}

function destroyShared(entry: SharedRenderer) {
  clearTimeout(entry.idle);
  entry.keepers.traverse((object) => {
    if (object instanceof Mesh) {
      object.geometry.dispose();
      (object.material as Material).dispose();
    }
  });
  entry.environment.dispose();
  entry.renderer.dispose();
  if (!entry.lost) entry.renderer.forceContextLoss();
}

function acquireShared(): SharedRenderer {
  const entry = shared ?? (shared = createShared());
  clearTimeout(entry.idle);
  entry.idle = undefined;
  entry.users += 1;
  return entry;
}

function releaseShared(entry: SharedRenderer) {
  entry.users -= 1;
  if (entry.users > 0 || entry.lost) return;
  entry.idle = setTimeout(() => {
    if (entry.users > 0 || shared !== entry) return;
    shared = null;
    destroyShared(entry);
  }, IDLE_RELEASE_MS);
}

/**
 * Compiles every shader the glass scenes use. Run once, before the first scene
 * is needed: the page stays responsive while it compiles, and the scene that
 * follows draws at once. Resolves (never rejects) when done or impossible.
 */
function warmShared(entry: SharedRenderer): Promise<void> {
  entry.warm ??= (async () => {
    const holder = new Scene();
    holder.add(entry.keepers);
    const { renderer, scene, camera } = entry;
    camera.position.set(0, 0, 10);
    camera.lookAt(0, 0, 0);
    camera.updateProjectionMatrix();
    await renderer.compileAsync(holder, camera, scene);
    if (entry.lost) return;
    // Glass refracts a pass of the opaque parts, which draws them with their
    // own shader variant; one tiny draw compiles that too.
    applyLook(entry, false, new Color(0xffffff));
    scene.add(entry.keepers);
    renderer.setSize(1, 1, false);
    renderer.render(scene, camera);
    scene.remove(entry.keepers);
  })().catch(() => undefined);
  return entry.warm;
}

/**
 * Gets the shared renderer ready ahead of a glass view, e.g. while the data it
 * will draw is still loading. Local GPU work only.
 */
export function warmGlassRenderer(): Promise<void> {
  let entry: SharedRenderer;
  try {
    entry = acquireShared();
  } catch {
    return Promise.resolve();
  }
  return warmShared(entry).finally(() => releaseShared(entry));
}

/**
 * Owns all content geometry and registered materials, including on build failure.
 * Defaults preserve the transaction graph's framing. Set view before first draw.
 * Draws through the shared renderer onto `canvas`, a plain 2D canvas.
 */
export function createGlassStage(
  canvas: HTMLCanvasElement,
  look: GlassSceneLook,
  populate: (
    content: Group,
    own: <T extends Material>(material: T) => T,
  ) => GlassPicking | void,
  { minHalfWidth = 4.4, minHalfHeight = 2.5 } = {},
): GlassScene {
  const entry = acquireShared();
  const content = new Group();
  const pivot = new Group();
  const materials = new Set<Material>();
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    pivot.removeFromParent();
    const geometries = new Set<Mesh["geometry"]>();
    content.traverse((object) => {
      if (object instanceof Mesh) {
        geometries.add(object.geometry);
        for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
          materials.add(material);
        }
      }
    });
    geometries.forEach((geometry) => geometry.dispose());
    materials.forEach((material) => material.dispose());
    releaseShared(entry);
  };
  try {
    const output = canvas.getContext("2d", { alpha: false });
    if (!output) throw new Error("No 2D context for the glass view");
    const background = new Color(look.background);
    const picking = populate(content, (material) => {
      materials.add(material);
      return material;
    });
    const bounds = new Box3().setFromObject(content);
    content.position.sub(bounds.getCenter(new Vector3()));
    pivot.add(content);

    // Far enough out that no turn can bring a corner past the camera, however
    // tall a 250-leg fan-in gets.
    const reach = Math.max(0, new Box3().setFromObject(pivot).getBoundingSphere(new Sphere()).radius);
    const camera = new OrthographicCamera(-1, 1, 1, -1, 0.1, reach * 2 + 20);
    camera.position.set(0, 0, reach + 10);
    camera.lookAt(0, 0, 0);
    // Never rendered itself (the shared camera copies it), so its world matrix
    // is kept current here for picking.
    camera.updateMatrixWorld();
    let frame = { halfWidth: 0, halfHeight: 0 };
    let size = { width: 1, height: 1 };
    const raycaster = new Raycaster();
    const ndc = new Vector2();
    const probe = new Vector3();
    const drawn = new Vector2();

    // The camera looks at the pivot, so the frame is symmetric around it. It only
    // grows: turning never clips the piece, and never zooms back in.
    const fit = () => {
      pivot.updateMatrixWorld(true);
      const box = new Box3().setFromObject(pivot);
      if (box.isEmpty()) box.set(new Vector3(), new Vector3());
      frame = {
        halfWidth: Math.max(
          frame.halfWidth,
          minHalfWidth,
          Math.max(Math.abs(box.min.x), Math.abs(box.max.x)) * 1.06,
        ),
        halfHeight: Math.max(
          frame.halfHeight,
          minHalfHeight,
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
    // Borrow the shared scene and camera for one draw or compile.
    const borrow = <T>(draw: () => T): T => {
      applyLook(entry, look.dark, background);
      entry.camera.copy(camera);
      entry.scene.add(pivot);
      try {
        return draw();
      } finally {
        entry.scene.remove(pivot);
      }
    };

    return {
      resize(width, height) {
        size = { width: Math.max(1, width), height: Math.max(1, height) };
        applyFrame();
      },
      setView(yaw, pitch) {
        pivot.rotation.set(pitch, yaw, 0);
        fit();
        applyFrame();
      },
      async prepare() {
        await warmShared(entry);
        if (disposed || entry.lost) return;
        // Compiled off to the side, then drawn through the shared scene.
        const holder = new Scene();
        holder.add(pivot);
        applyLook(entry, look.dark, background);
        entry.camera.copy(camera);
        try {
          await entry.renderer.compileAsync(holder, entry.camera, entry.scene);
        } finally {
          holder.remove(pivot);
        }
      },
      render() {
        if (disposed || entry.lost) return;
        const { renderer } = entry;
        renderer.getSize(drawn);
        if (drawn.x !== size.width || drawn.y !== size.height) {
          renderer.setSize(size.width, size.height, false);
        }
        borrow(() => renderer.render(entry.scene, entry.camera));
        const { width, height } = renderer.domElement;
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        output.drawImage(renderer.domElement, 0, 0);
      },
      pick(x, y) {
        if (!picking || !frame.halfWidth) return null;
        ndc.set((x / size.width) * 2 - 1, 1 - (y / size.height) * 2);
        raycaster.setFromCamera(ndc, camera);
        content.updateMatrixWorld(true);
        return picking.pick({
          raycaster,
          x,
          y,
          toScreen(px, py) {
            probe.set(px, py, 0).applyMatrix4(content.matrixWorld).project(camera);
            return [((probe.x + 1) / 2) * size.width, ((1 - probe.y) / 2) * size.height];
          },
          pixelsPerUnit: size.width / (camera.right - camera.left),
        });
      },
      highlight(part) {
        picking?.highlight(part);
      },
      onContextLost(listener) {
        if (entry.lost) {
          queueMicrotask(listener);
          return () => undefined;
        }
        entry.listeners.add(listener);
        return () => {
          entry.listeners.delete(listener);
        };
      },
      dispose,
    };
  } catch (error) {
    dispose();
    throw error;
  }
}
