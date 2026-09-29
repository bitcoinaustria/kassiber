import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";

import type { WalletUtxoRow } from "../UtxosInventoryPanel";
import { UtxoBlocks3D } from "./UtxoBlocks3D";

const row = (index: number, asset = "BTC", status = "confirmed"): WalletUtxoRow => ({
  id: `u${index}`,
  outpoint: `${index.toString(16).padStart(64, "0")}:0`,
  txid: index.toString(16).padStart(64, "0"),
  vout: 0,
  asset,
  amount: 0.001 * (index + 1),
  amount_sat: 100_000 * (index + 1),
  amount_msat: 100_000_000 * (index + 1),
  confirmation_status: status,
  source: {
    backend: "b",
    backend_kind: "electrum",
    chain: "bitcoin",
    network: "main",
    first_seen_at: "2026-09-01T00:00:00Z",
    last_seen_at: "2026-09-01T00:00:00Z",
  },
});

describe("UTXO glass blocks", () => {
  it("draws the wallet's coins above the table and explains the scale", () => {
    const html = renderToStaticMarkup(<UtxoBlocks3D rows={[row(0), row(1, "BTC", "mempool")]} hideSensitive={false} />);
    expect(html).toContain('data-testid="utxo-blocks-3d"');
    expect(html).toContain("2 unspent coins as glass blocks");
    expect(html).toContain("as tall as its amount");
  });

  it("says every block is the same height when values are hidden", () => {
    const html = renderToStaticMarkup(<UtxoBlocks3D rows={[row(0), row(1)]} hideSensitive />);
    expect(html).toContain("every block has the same height");
  });

  it("draws only the wallet's main coin and nothing without coins", () => {
    const html = renderToStaticMarkup(
      <UtxoBlocks3D rows={[row(0, "LBTC"), row(1, "LBTC"), row(2, "USDT")]} hideSensitive={false} />,
    );
    expect(html).toContain("2 unspent coins as glass blocks");
    expect(renderToStaticMarkup(<UtxoBlocks3D rows={[]} hideSensitive={false} />)).toBe("");
  });
});
