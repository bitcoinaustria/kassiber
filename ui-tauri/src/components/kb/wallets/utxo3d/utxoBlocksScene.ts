import { Mesh, SphereGeometry } from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

import { DARK_TONES, LIGHT_TONES, coinMaterial, glass, satin } from "../../glass3d/materials";
import { createGlassStage, type GlassScene, type GlassSceneLook } from "../../glass3d/stage";
import type { UtxoBlocksLayout } from "./utxoBlocksLayout";

/** Prototype entry point: load dynamically if a product screen adopts it. */
export function createUtxoBlocksScene(
  canvas: HTMLCanvasElement,
  layout: UtxoBlocksLayout,
  look: GlassSceneLook,
): GlassScene {
  return createGlassStage(canvas, look, (content, own) => {
    const tones = look.dark ? DARK_TONES : LIGHT_TONES;
    // The wallet's own coins, as blue as its coins in the transaction graph;
    // an unconfirmed coin is frosted, the graph's cue for "not settled".
    const clear = own(coinMaterial(true, look.dark));
    const frosted = own(glass(tones.estimated));
    const marker = own(satin(look.dark ? "#e2e8f0" : "#3f4a5c"));
    for (const block of layout.blocks) {
      const mesh = new Mesh(
        new RoundedBoxGeometry(block.width, block.height, block.depth, 3, 0.05),
        block.frosted ? frosted : clear,
      );
      mesh.position.set(block.x, block.y, block.z);
      content.add(mesh);
      // A geometric ellipsis distinguishes the overflow sentinel from a coin.
      if (block.kind === "more") {
        for (const x of [-0.18, 0, 0.18]) {
          const dot = new Mesh(new SphereGeometry(0.045, 12, 8), marker);
          dot.position.set(block.x + x, block.y, block.z + block.depth / 2 + 0.02);
          content.add(dot);
        }
      }
    }
  });
}
