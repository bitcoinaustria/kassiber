import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  explorerButtonTitle,
  explorerTargetForUtxo,
  sortUtxosForDisplay,
  UTXO_SORT_OPTIONS,
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
      explorerSettings={{ bitcoinBaseUrl: "", liquidBaseUrl: "" }}
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
    expect(html).toContain("mempool.bitcoin-austria.at");
    expect(html).toContain("Open UTXO transaction on mempool.bitcoin-austria.at");
    expect(html).not.toContain(`Open ${baseInventory.utxos[0].txid}`);
    expect(html).not.toContain("Sort UTXOs");
  });

  it("shows sorting controls when multiple UTXOs are present", () => {
    const html = renderPanel({
      ...baseInventory,
      utxos: [
        ...baseInventory.utxos,
        {
          ...baseInventory.utxos[0],
          id: "coin-2",
          outpoint:
            "1e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766:1",
          txid: "1e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766",
          vout: 1,
          amount: 0.025,
          amount_sat: 2_500_000,
          amount_msat: 2_500_000_000,
          confirmations: 1,
          block_height: 800_010,
          block_time: "2026-01-02T00:00:00Z",
          source: {
            ...baseInventory.utxos[0].source,
            last_seen_at: "2026-01-02T00:00:00Z",
          },
        },
      ],
      summary: { count: 2 },
    });

    expect(html).toContain("Sort UTXOs");
    expect(UTXO_SORT_OPTIONS.map((option) => option.label)).toContain(
      "Chain date: newest first",
    );
  });

  it("sorts UTXOs by value, chain date, confirmations, and outpoint", () => {
    const rows = [
      {
        ...baseInventory.utxos[0],
        id: "small-new",
        outpoint:
          "2e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766:0",
        txid: "2e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766",
        amount_sat: 2_000,
        amount_msat: 2_000_000,
        confirmations: 3,
        block_time: "2026-01-03T00:00:00Z",
      },
      {
        ...baseInventory.utxos[0],
        id: "large-old",
        outpoint:
          "1e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766:0",
        txid: "1e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766",
        amount_sat: 9_000,
        amount_msat: 9_000_000,
        confirmations: 8,
        block_time: "2026-01-01T00:00:00Z",
      },
      {
        ...baseInventory.utxos[0],
        id: "medium-mempool",
        outpoint:
          "3e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766:0",
        txid: "3e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766",
        amount_sat: 5_000,
        amount_msat: 5_000_000,
        confirmation_status: "mempool",
        confirmations: 0,
        block_time: null,
      },
    ];

    expect(sortUtxosForDisplay(rows, "size-desc").map((row) => row.id)).toEqual([
      "large-old",
      "medium-mempool",
      "small-new",
    ]);
    expect(sortUtxosForDisplay(rows, "date-desc").map((row) => row.id)).toEqual([
      "small-new",
      "large-old",
      "medium-mempool",
    ]);
    expect(
      sortUtxosForDisplay(rows, "confirmations-asc").map((row) => row.id),
    ).toEqual(["medium-mempool", "small-new", "large-old"]);
    expect(sortUtxosForDisplay(rows, "outpoint-asc").map((row) => row.id)).toEqual([
      "large-old",
      "small-new",
      "medium-mempool",
    ]);
  });

  it("builds explorer targets without leaking txids into hover titles", () => {
    const target = explorerTargetForUtxo(baseInventory.utxos[0], {
      bitcoinBaseUrl: "",
      liquidBaseUrl: "",
    });

    expect(target).toMatchObject({
      label: "mempool.bitcoin-austria.at",
      url: expect.stringContaining(baseInventory.utxos[0].txid),
    });
    expect(explorerButtonTitle(target!)).toBe(
      "Open UTXO transaction on mempool.bitcoin-austria.at",
    );
    expect(explorerButtonTitle(target!)).not.toContain(baseInventory.utxos[0].txid);
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
