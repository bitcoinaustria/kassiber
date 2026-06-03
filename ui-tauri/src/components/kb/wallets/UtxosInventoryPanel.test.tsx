import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  UtxosInventoryPanel,
  type WalletUtxosData,
} from "./UtxosInventoryPanel";

const baseInventory: WalletUtxosData = {
  wallet: {
    id: "wallet-1",
    label: "Vault",
  },
  utxos: [
    {
      id: "coin-1",
      outpoint:
        "4e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766:0",
      txid: "4e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766",
      vout: 0,
      asset: "BTC",
      amount: 0.125,
      amount_sat: 12_500_000,
      amount_msat: 12_500_000_000,
      confirmation_status: "confirmed",
      confirmations: 12,
      block_height: 800_000,
      block_time: "2026-01-01T00:00:00Z",
      address: "bc1qcoin",
      address_label: "receive #3",
      branch_label: "receive",
      branch_index: 0,
      address_index: 3,
      source: {
        backend: "mempool",
        backend_kind: "esplora",
        chain: "bitcoin",
        network: "mainnet",
        first_seen_at: "2026-01-01T00:00:00Z",
        last_seen_at: "2026-01-01T00:00:00Z",
        spent_at: null,
      },
    },
  ],
  totals: [
    {
      asset: "BTC",
      amount: 0.125,
      amount_sat: 12_500_000,
      amount_msat: 12_500_000_000,
    },
  ],
  support: {
    supported: true,
    status: "supported",
    reason: "",
    message: "",
  },
  freshness: {
    status: "current",
    stale: false,
    active_count: 1,
    last_seen_at: "2026-01-01T00:00:00Z",
    last_synced_at: "2026-01-01T00:00:00Z",
  },
};

const renderPanel = (inventory: WalletUtxosData | null = baseInventory) =>
  renderToStaticMarkup(
    <UtxosInventoryPanel
      inventory={inventory}
      hideSensitive={false}
      isRefreshing={false}
      onRefresh={vi.fn()}
    />,
  );

describe("UtxosInventoryPanel", () => {
  it("renders known UTXO rows with UTXO wording", () => {
    const html = renderPanel();

    expect(html).toContain("UTXOs");
    expect(html).toContain("Currently unspent transaction outputs");
    expect(html).toContain("₿ 0.12500000");
    expect(html).toContain("receive #3");
    expect(html).toContain("12 conf");
  });

  it("renders stale, empty, unsupported, and Liquid blocker states", () => {
    const staleHtml = renderPanel({
      ...baseInventory,
      freshness: { ...baseInventory.freshness, stale: true, status: "stale" },
    });
    expect(staleHtml).toContain("Refresh this source to update the UTXO inventory.");

    const emptyHtml = renderPanel({
      ...baseInventory,
      utxos: [],
      totals: [],
      summary: { count: 0 },
    });
    expect(emptyHtml).toContain("No UTXOs known");

    const unsupportedHtml = renderPanel({
      ...baseInventory,
      utxos: [],
      totals: [],
      support: {
        supported: false,
        status: "unsupported_source",
        reason: "not_chain_backed",
        message: "This source is not a chain-backed watch-only wallet.",
      },
      freshness: { status: "unsupported_source", stale: false },
    });
    expect(unsupportedHtml).toContain("UTXO inventory unavailable");
    expect(unsupportedHtml).toContain("not a chain-backed watch-only wallet");

    const liquidHtml = renderPanel({
      ...baseInventory,
      utxos: [],
      totals: [],
      support: {
        supported: false,
        status: "liquid_unblind_blocked",
        reason: "missing_blinding_keys",
        message: "Liquid UTXO inventory needs private blinding keys.",
      },
      freshness: { status: "liquid_unblind_blocked", stale: false },
    });
    expect(liquidHtml).toContain("Liquid UTXOs need unblinding");
    expect(liquidHtml).toContain("private blinding keys");
  });
});
