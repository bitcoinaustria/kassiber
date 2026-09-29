import { BoxGeometry, Mesh, MeshPhysicalMaterial, OrthographicCamera } from "three";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const stubs = vi.hoisted(() => ({
  renderer: {
    setPixelRatio: vi.fn(), setSize: vi.fn(), render: vi.fn(),
    dispose: vi.fn(), forceContextLoss: vi.fn(),
  },
  environment: { texture: {}, dispose: vi.fn() },
}));

vi.mock("three", async (original) => ({
  ...await original<typeof import("three")>(),
  WebGLRenderer: class { constructor() { return stubs.renderer; } },
  PMREMGenerator: class {
    fromScene() { return stubs.environment; }
    dispose() {}
  },
}));

import { createGlassStage } from "./stage";

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("window", { devicePixelRatio: 1 });
});
afterEach(() => vi.unstubAllGlobals());

const canvas = {} as HTMLCanvasElement;
const look = { dark: false, background: "#ffffff" };

function lastCamera() {
  return stubs.renderer.render.mock.calls.at(-1)![1] as OrthographicCamera;
}

describe("shared glass stage", () => {
  it("preserves graph minimum framing, draws only on demand and never shrinks on rotation", () => {
    const stage = createGlassStage(canvas, look, (content, own) => {
      content.add(new Mesh(new BoxGeometry(2, 2, 20), own(new MeshPhysicalMaterial())));
    });
    stage.resize(880, 500);
    stage.setView(0, 0);
    expect(stubs.renderer.render).not.toHaveBeenCalled();
    stage.render();
    const camera = lastCamera();
    expect(camera.right).toBeCloseTo(4.4);
    expect(camera.top).toBeCloseTo(2.5);
    stage.setView(Math.PI / 2, 0);
    stage.render();
    const expanded = camera.right;
    expect(expanded).toBeGreaterThan(10);
    stage.setView(0, 0);
    expect(camera.right).toBe(expanded);
    expect(stubs.renderer.render).toHaveBeenCalledTimes(2);
    stage.dispose();
  });

  it("releases shared geometry, all owned materials and the environment once", () => {
    const geometry = new BoxGeometry();
    const used = new MeshPhysicalMaterial();
    const unused = new MeshPhysicalMaterial();
    const disposeGeometry = vi.spyOn(geometry, "dispose");
    const disposeUsed = vi.spyOn(used, "dispose");
    const disposeUnused = vi.spyOn(unused, "dispose");
    const stage = createGlassStage(canvas, look, (content, own) => {
      own(unused);
      content.add(new Mesh(geometry, own(used)), new Mesh(geometry, used));
    });
    stage.dispose();
    stage.dispose();
    for (const spy of [disposeGeometry, disposeUsed, disposeUnused, stubs.environment.dispose,
      stubs.renderer.dispose, stubs.renderer.forceContextLoss]) {
      expect(spy).toHaveBeenCalledTimes(1);
    }
  });

  it("cleans up a partially populated scene when construction fails", () => {
    const geometry = new BoxGeometry();
    const material = new MeshPhysicalMaterial();
    const disposeGeometry = vi.spyOn(geometry, "dispose");
    const disposeMaterial = vi.spyOn(material, "dispose");
    expect(() => createGlassStage(canvas, look, (content, own) => {
      content.add(new Mesh(geometry, own(material)));
      throw new Error("build failed");
    })).toThrow("build failed");
    expect(disposeGeometry).toHaveBeenCalledOnce();
    expect(disposeMaterial).toHaveBeenCalledOnce();
    expect(stubs.environment.dispose).toHaveBeenCalledOnce();
    expect(stubs.renderer.forceContextLoss).toHaveBeenCalledOnce();
  });

  it("frames an empty inventory without an infinite projection", () => {
    const stage = createGlassStage(canvas, look, () => {});
    stage.resize(880, 500);
    stage.setView(0.4, 0.2);
    stage.render();
    expect(lastCamera().right).toBeCloseTo(4.4);
    expect(lastCamera().projectionMatrix.elements.every(Number.isFinite)).toBe(true);
    stage.dispose();
  });
});
