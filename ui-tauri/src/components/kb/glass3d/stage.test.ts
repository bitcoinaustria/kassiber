import { BoxGeometry, Mesh, MeshPhysicalMaterial, OrthographicCamera } from "three";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// No WebGL in Node: the renderer is a stub; scenes, cameras and rays are real.
const stubs = vi.hoisted(() => {
  const size = { x: 0, y: 0 };
  const renderer = {
    domElement: null as unknown as { width: number; height: number },
    setPixelRatio: vi.fn(),
    setSize: vi.fn((width: number, height: number) => {
      size.x = width;
      size.y = height;
      renderer.domElement.width = width;
      renderer.domElement.height = height;
    }),
    getSize: vi.fn((target: { x: number; y: number }) => {
      target.x = size.x;
      target.y = size.y;
      return target;
    }),
    render: vi.fn(),
    compileAsync: vi.fn(async () => undefined),
    dispose: vi.fn(),
    forceContextLoss: vi.fn(),
  };
  return {
    renderer,
    size,
    created: vi.fn(),
    environment: { texture: {}, dispose: vi.fn() },
    lost: [] as Array<() => void>,
  };
});

vi.mock("three", async (original) => ({
  ...(await original<typeof import("three")>()),
  WebGLRenderer: class {
    constructor({ canvas }: { canvas: { width: number; height: number } }) {
      stubs.created();
      stubs.renderer.domElement = canvas;
      return stubs.renderer;
    }
  },
  PMREMGenerator: class {
    fromScene() {
      return stubs.environment;
    }
    dispose() {}
  },
}));

type Stage = typeof import("./stage");
let createGlassStage: Stage["createGlassStage"];
let warmGlassRenderer: Stage["warmGlassRenderer"];

beforeEach(async () => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  stubs.lost.length = 0;
  stubs.size.x = 0;
  stubs.size.y = 0;
  vi.stubGlobal("window", { devicePixelRatio: 1 });
  vi.stubGlobal("document", {
    createElement: () => ({
      width: 0,
      height: 0,
      addEventListener: (type: string, listener: () => void) => {
        if (type === "webglcontextlost") stubs.lost.push(listener);
      },
    }),
  });
  // The shared renderer is module state: every test starts without one.
  vi.resetModules();
  ({ createGlassStage, warmGlassRenderer } = await import("./stage"));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const look = { dark: false, background: "#ffffff" };

function outputCanvas() {
  const drawImage = vi.fn();
  return {
    canvas: { width: 0, height: 0, getContext: () => ({ drawImage }) } as unknown as HTMLCanvasElement,
    drawImage,
  };
}

function lastCamera() {
  return stubs.renderer.render.mock.calls.at(-1)![1] as OrthographicCamera;
}

describe("shared glass stage", () => {
  it("preserves graph minimum framing, draws only on demand and never shrinks on rotation", () => {
    const { canvas, drawImage } = outputCanvas();
    const stage = createGlassStage(canvas, look, (content, own) => {
      content.add(new Mesh(new BoxGeometry(2, 2, 20), own(new MeshPhysicalMaterial())));
    });
    stage.resize(880, 500);
    stage.setView(0, 0);
    expect(stubs.renderer.render).not.toHaveBeenCalled();
    stage.render();
    expect(lastCamera().right).toBeCloseTo(4.4);
    expect(lastCamera().top).toBeCloseTo(2.5);
    stage.setView(Math.PI / 2, 0);
    stage.render();
    const expanded = lastCamera().right;
    expect(expanded).toBeGreaterThan(10);
    stage.setView(0, 0);
    stage.render();
    expect(lastCamera().right).toBe(expanded);
    expect(stubs.renderer.render).toHaveBeenCalledTimes(3);
    // Every draw lands on the view's own canvas, at the drawing's size.
    expect(drawImage).toHaveBeenCalledTimes(3);
    expect(canvas.width).toBe(880);
    expect(canvas.height).toBe(500);
    stage.dispose();
  });

  it("shares one renderer and its environment between views and releases it once idle", () => {
    const first = createGlassStage(outputCanvas().canvas, look, () => {});
    const second = createGlassStage(outputCanvas().canvas, { dark: true, background: "#000000" }, () => {});
    expect(stubs.created).toHaveBeenCalledTimes(1);
    first.dispose();
    second.dispose();
    // Kept a while for the next view: its shaders stay compiled.
    vi.advanceTimersByTime(10_000);
    const third = createGlassStage(outputCanvas().canvas, look, () => {});
    expect(stubs.created).toHaveBeenCalledTimes(1);
    third.dispose();
    expect(stubs.renderer.dispose).not.toHaveBeenCalled();
    vi.advanceTimersByTime(30_000);
    for (const spy of [stubs.environment.dispose, stubs.renderer.dispose, stubs.renderer.forceContextLoss]) {
      expect(spy).toHaveBeenCalledTimes(1);
    }
    createGlassStage(outputCanvas().canvas, look, () => {}).dispose();
    expect(stubs.created).toHaveBeenCalledTimes(2);
  });

  it("releases a view's own geometry and materials once, on dispose", () => {
    const geometry = new BoxGeometry();
    const used = new MeshPhysicalMaterial();
    const unused = new MeshPhysicalMaterial();
    const disposeGeometry = vi.spyOn(geometry, "dispose");
    const disposeUsed = vi.spyOn(used, "dispose");
    const disposeUnused = vi.spyOn(unused, "dispose");
    const stage = createGlassStage(outputCanvas().canvas, look, (content, own) => {
      own(unused);
      content.add(new Mesh(geometry, own(used)), new Mesh(geometry, used));
    });
    stage.dispose();
    stage.dispose();
    for (const spy of [disposeGeometry, disposeUsed, disposeUnused]) {
      expect(spy).toHaveBeenCalledTimes(1);
    }
  });

  it("cleans up a partially populated scene when construction fails", () => {
    const geometry = new BoxGeometry();
    const material = new MeshPhysicalMaterial();
    const disposeGeometry = vi.spyOn(geometry, "dispose");
    const disposeMaterial = vi.spyOn(material, "dispose");
    expect(() => createGlassStage(outputCanvas().canvas, look, (content, own) => {
      content.add(new Mesh(geometry, own(material)));
      throw new Error("build failed");
    })).toThrow("build failed");
    expect(disposeGeometry).toHaveBeenCalledOnce();
    expect(disposeMaterial).toHaveBeenCalledOnce();
    // The failed view gave its hold on the renderer back.
    vi.advanceTimersByTime(30_000);
    expect(stubs.renderer.forceContextLoss).toHaveBeenCalledOnce();
  });

  it("compiles the shared shaders once, then each scene before its first draw", async () => {
    await warmGlassRenderer();
    const stage = createGlassStage(outputCanvas().canvas, look, (content, own) => {
      content.add(new Mesh(new BoxGeometry(), own(new MeshPhysicalMaterial())));
    });
    stage.resize(880, 500);
    stage.setView(0, 0);
    await stage.prepare();
    // Keepers once (with one tiny draw for the refraction pass), then the scene.
    expect(stubs.renderer.compileAsync).toHaveBeenCalledTimes(2);
    expect(stubs.renderer.render).toHaveBeenCalledTimes(1);
    await createGlassStage(outputCanvas().canvas, look, () => {}).prepare();
    expect(stubs.renderer.compileAsync).toHaveBeenCalledTimes(3);
    stage.dispose();
  });

  it("tells every view when the shared context is lost, and starts over after", () => {
    const stage = createGlassStage(outputCanvas().canvas, look, () => {});
    const lost = vi.fn();
    stage.onContextLost(lost);
    stubs.lost.forEach((listener) => listener());
    expect(lost).toHaveBeenCalledOnce();
    stage.render();
    expect(stubs.renderer.render).not.toHaveBeenCalled();
    stage.dispose();
    createGlassStage(outputCanvas().canvas, look, () => {}).dispose();
    expect(stubs.created).toHaveBeenCalledTimes(2);
  });

  it("frames an empty inventory without an infinite projection", () => {
    const stage = createGlassStage(outputCanvas().canvas, look, () => {});
    stage.resize(880, 500);
    stage.setView(0.4, 0.2);
    stage.render();
    expect(lastCamera().right).toBeCloseTo(4.4);
    expect(lastCamera().projectionMatrix.elements.every(Number.isFinite)).toBe(true);
    stage.dispose();
  });

  it("hands a pointer to the scene's picker and forwards its highlight", () => {
    const highlight = vi.fn();
    let seen: { x: number; y: number; centre: [number, number] } | null = null;
    const stage = createGlassStage(outputCanvas().canvas, look, (content, own) => {
      const box = new Mesh(new BoxGeometry(1, 1, 1), own(new MeshPhysicalMaterial()));
      box.userData.part = "box";
      content.add(box);
      return {
        pick(pointer) {
          seen = { x: pointer.x, y: pointer.y, centre: pointer.toScreen(0, 0) };
          return (pointer.raycaster.intersectObject(box)[0]?.object.userData.part as string) ?? null;
        },
        highlight,
      };
    });
    stage.resize(880, 500);
    stage.setView(0, 0);
    expect(stage.pick(440, 250)).toBe("box");
    expect(seen!.centre[0]).toBeCloseTo(440);
    expect(seen!.centre[1]).toBeCloseTo(250);
    expect(stage.pick(10, 10)).toBeNull();
    stage.highlight("box");
    expect(highlight).toHaveBeenCalledWith("box");
    stage.dispose();
  });

  it("has nothing to pick in a scene without parts", () => {
    const stage = createGlassStage(outputCanvas().canvas, look, () => {});
    stage.resize(880, 500);
    stage.setView(0, 0);
    expect(stage.pick(440, 250)).toBeNull();
    stage.dispose();
  });
});
