import { describe, expect, it, vi } from "vitest";

// No WebGL in Node: the renderer is a stub, the scene graph and rays are real.
vi.mock("three", async (original) => ({
  ...(await original<typeof import("three")>()),
  WebGLRenderer: class {
    domElement = { width: 0, height: 0 };
    setPixelRatio() {}
    setSize() {}
    getSize(target: { x: number; y: number }) {
      return target;
    }
    render() {}
    async compileAsync() {}
    dispose() {}
    forceContextLoss() {}
  },
  PMREMGenerator: class {
    fromScene() {
      return { texture: {}, dispose() {} };
    }
    dispose() {}
  },
}));

import type { TransactionGraphPayload } from "../TransactionGraphModel";
import { createGlassScene } from "./glassScene";
import { ribbonLayout } from "./ribbonLayout";

const graph: TransactionGraphPayload = {
  transaction: { id: "tx", chain: "bitcoin" },
  supportLevel: "full",
  inputs: [{ id: "in", valueSats: 1_000_000, valueBtc: 0.01, valueState: "known", ownership: "owned" }],
  outputs: [{ id: "out", valueSats: 998_000, valueBtc: 0.00998, valueState: "known" }],
  // A fee this small draws as a hairline; it must still answer the pointer.
  fee: { id: "fee", valueSats: 2_000, valueBtc: 0.00002, role: "fee", ownership: "network_fee" },
};

function sweep(scene: ReturnType<typeof createGlassScene>, width: number, height: number) {
  const parts = new Set<string>();
  for (let y = 0; y < height; y += 4) {
    for (let x = 0; x < width; x += 4) {
      const part = scene.pick(x, y);
      if (part) parts.add(part);
    }
  }
  return parts;
}

describe("pointing at the glass transaction graph", () => {
  it("finds every leg, the hairline fee included, and nothing off the drawing", () => {
    vi.stubGlobal("window", { devicePixelRatio: 1 });
    vi.stubGlobal("document", { createElement: () => ({ addEventListener() {} }) });
    const canvas = { getContext: () => ({ drawImage() {} }) } as unknown as HTMLCanvasElement;
    const scene = createGlassScene(canvas, ribbonLayout(graph, false), {
      background: "#000000",
      dark: true,
    });
    scene.resize(900, 360);
    // Seen at the stage's fixed angle, every leg is found where it is drawn.
    expect(sweep(scene, 900, 360)).toEqual(new Set(["input:in", "output:out", "output:fee"]));
    expect(scene.pick(2, 2)).toBeNull();
    // Resized, the legs are found where they are drawn now, not where they were.
    scene.resize(500, 700);
    // The ribbons meet at the middle; before the resize, nothing was drawn here.
    expect(scene.pick(250, 350)).not.toBeNull();
    expect(sweep(scene, 500, 700)).toEqual(new Set(["input:in", "output:out", "output:fee"]));
    expect(scene.pick(2, 2)).toBeNull();
    scene.highlight("output:out");
    scene.highlight(null);
    scene.dispose();
    vi.unstubAllGlobals();
  });
});
