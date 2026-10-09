import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChainArtwork } from "./ChainArtwork";
import { BLOCK_LAYERS, advanceChain, type LoopState } from "./chainLayout";

describe("setup chain artwork view", () => {
  it("stays out of the accessibility tree and the tab order", () => {
    const html = renderToStaticMarkup(
      <ChainArtwork live mined={3} fill={0.5} className="h-56" />,
    );
    expect(html).toMatch(/^<svg aria-hidden="true"/);
    expect(html).not.toContain('role="img"');
    expect(html).not.toContain("tabindex");
  });

  it("outlines only the next block and shows the mempool above it", () => {
    const html = renderToStaticMarkup(
      <ChainArtwork mined={3} fill={0.5} className="h-56" />,
    );
    // Every block carries an outline; only the next block's is visible.
    expect(html.match(/kb-chain-shell/g)).toHaveLength(4);
    expect(html.match(/kb-chain-shell[^"]*opacity-0/g)).toHaveLength(3);
    expect(html).toContain("kb-chain-mempool");
  });

  it("clears the mempool once the block is full", () => {
    const html = renderToStaticMarkup(
      <ChainArtwork mined={0} fill={1} className="h-40" />,
    );
    expect(html).not.toContain("kb-chain-mempool");
  });

  it("fills the next block, finds it, then moves the chain back a slot", () => {
    let state: LoopState = { tip: 0, layers: 0, found: false };
    const beats: LoopState[] = [];
    for (let beat = 0; beat < BLOCK_LAYERS + 2; beat += 1) {
      state = advanceChain(state);
      beats.push(state);
    }
    expect(beats.slice(0, BLOCK_LAYERS).map((beat) => beat.layers)).toEqual(
      Array.from({ length: BLOCK_LAYERS }, (_, index) => index + 1),
    );
    expect(beats[BLOCK_LAYERS]).toEqual({ tip: 0, layers: BLOCK_LAYERS, found: true });
    expect(beats[BLOCK_LAYERS + 1]).toEqual({ tip: 1, layers: 0, found: false });
  });
});
