import { describe, expect, it, vi } from "vitest";

// No WebGL in Node: the renderer is a stub, the scene graph and rays are real.
vi.mock("three", async (original) => ({
  ...(await original<typeof import("three")>()),
  WebGLRenderer: class {
    setPixelRatio() {}
    setSize() {}
    render() {}
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
    const scene = createGlassScene({} as HTMLCanvasElement, ribbonLayout(graph, false), {
      background: "#000000",
      dark: true,
    });
    scene.resize(900, 360);
    scene.setView(0, 0);
    expect(sweep(scene, 900, 360)).toEqual(new Set(["input:in", "output:out", "output:fee"]));
    expect(scene.pick(2, 2)).toBeNull();
    // Turned, the same legs are still found where they now are.
    scene.setView(0.5, 0.3);
    expect(sweep(scene, 900, 360)).toEqual(new Set(["input:in", "output:out", "output:fee"]));
    scene.highlight("output:out");
    scene.highlight(null);
    scene.dispose();
    vi.unstubAllGlobals();
  });
});
