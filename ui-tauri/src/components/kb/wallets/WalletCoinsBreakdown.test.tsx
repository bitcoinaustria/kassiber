import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { WalletUtxoRow } from "./UtxosInventoryPanel";
import { WalletCoinsBreakdown } from "./WalletCoinsBreakdown";

function coin(id: string, amountSat: number, blockTime: string): WalletUtxoRow {
  return {
    id,
    outpoint: `${id}:0`,
    txid: id,
    vout: 0,
    asset: "BTC",
    amount: amountSat / 1e8,
    amount_sat: amountSat,
    amount_msat: amountSat * 1000,
    confirmation_status: "confirmed",
    confirmations: 100,
    block_height: 800_000,
    block_time: blockTime,
    source: {
      backend: "mempool",
      backend_kind: "esplora",
      chain: "bitcoin",
      network: "mainnet",
      first_seen_at: blockTime,
      last_seen_at: blockTime,
      spent_at: null,
    },
  };
}

// Both old, in different size bands: the age list shows "2 coins", each size
// band "1 coin".
const rows = [
  coin("a", 50_000_000, "2015-01-01T00:00:00Z"),
  coin("b", 200_000, "2016-01-01T00:00:00Z"),
];

describe("WalletCoinsBreakdown", () => {
  it("counts coins per size and age band", () => {
    const html = renderToStaticMarkup(
      <WalletCoinsBreakdown rows={rows} hideSensitive={false} />,
    );

    expect(html).toContain("1 coin");
    expect(html).toContain("2 coins");
    expect(html).toContain("₿ 0.50000000");
    expect(html).not.toContain("Covers the oldest");
  });

  it("shows no size-band counts or bars while values are hidden", () => {
    const html = renderToStaticMarkup(
      <WalletCoinsBreakdown rows={rows} hideSensitive />,
    );

    // The age bands say nothing about amounts and keep their counts.
    expect(html).toContain("2 coins");
    expect(html).not.toContain("1 coin");
    // Bars are drawn for the four age bands only.
    expect(html.match(/style="width:/g)).toHaveLength(4);
    expect(html).toMatch(/class="[^"]*\bsensitive\b[^"]*">₿ 0\.50200000/);
  });

  it("says how many coins a capped inventory covers", () => {
    const html = renderToStaticMarkup(
      <WalletCoinsBreakdown rows={rows} totalCount={900} hideSensitive={false} />,
    );

    expect(html).toContain("Covers the oldest 2 of 900 coins");
  });

  it("renders nothing without bitcoin coins", () => {
    const html = renderToStaticMarkup(
      <WalletCoinsBreakdown rows={[]} hideSensitive={false} />,
    );

    expect(html).toBe("");
  });
});
