import {
  Box3, Color, DirectionalLight, Group, HemisphereLight, Mesh,
  NeutralToneMapping, OrthographicCamera, PMREMGenerator, Scene,
  Sphere, SRGBColorSpace, Vector3, WebGLRenderer,
  type Material, type WebGLRenderTarget,
} from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";

/** Import at runtime only from a lazy 3D entry point. No external assets or I/O. */
export type GlassSceneLook = { background: string; dark: boolean };
export type GlassScene = {
  resize: (width: number, height: number) => void;
  setView: (yaw: number, pitch: number) => void;
  /** Explicit draw; no animation loop or background work. */
  render: () => void;
  dispose: () => void;
};

/**
 * Owns all content geometry and registered materials, including on build failure.
 * Defaults preserve the transaction graph's framing. Set view before first draw.
 */
export function createGlassStage(
  canvas: HTMLCanvasElement,
  look: GlassSceneLook,
  populate: (content: Group, own: <T extends Material>(material: T) => T) => void,
  { minHalfWidth = 4.4, minHalfHeight = 2.5 } = {},
): GlassScene {
  const renderer = new WebGLRenderer({ canvas, antialias: true, powerPreference: "low-power" });
  const scene = new Scene();
  const content = new Group();
  const materials = new Set<Material>();
  let environment: WebGLRenderTarget | undefined;
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
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
    environment?.dispose();
    renderer.dispose();
    renderer.forceContextLoss();
  };
  try {
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.toneMapping = NeutralToneMapping;

    // Opaque, in the surface colour: transmission refracts what is behind the
    // glass, and a transparent clear would make every ribbon look dark.
    scene.background = new Color(look.background);
    const pmrem = new PMREMGenerator(renderer);
    const room = new RoomEnvironment();
    try {
      environment = pmrem.fromScene(room, 0.04);
      scene.environment = environment.texture;
    } finally {
      room.dispose();
      pmrem.dispose();
    }
    scene.environmentIntensity = look.dark ? 0.9 : 1.15;
    scene.add(new HemisphereLight(0xffffff, look.dark ? 0x334155 : 0x8a94a6, look.dark ? 0.7 : 0.9));
    const key = new DirectionalLight(0xffffff, 2.2);
    key.position.set(-4, 7, 8);
    scene.add(key);

    populate(content, (material) => {
      materials.add(material);
      return material;
    });
    const pivot = new Group();
    const bounds = new Box3().setFromObject(content);
    content.position.sub(bounds.getCenter(new Vector3()));
    pivot.add(content);
    scene.add(pivot);

    // Far enough out that no turn can bring a corner past the camera, however
    // tall a 250-leg fan-in gets.
    const reach = Math.max(0, new Box3().setFromObject(pivot).getBoundingSphere(new Sphere()).radius);
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
      dispose,
    };
  } catch (error) {
    dispose();
    throw error;
  }
}
