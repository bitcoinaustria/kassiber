import { describe, expect, it } from "vitest";

import { buildBalanceReconciliation } from "./walletBalanceReconcile";

const inventory = (
  amountSat: number,
  extra: Partial<{ supported: boolean; lastSyncedAt: string }> = {},
) => ({
  totals: [{ asset: "BTC", amount_sat: amountSat }],
  support: { supported: extra.supported ?? true },
  freshness: { last_synced_at: extra.lastSyncedAt ?? "2026-06-15T10:00:00Z" },
});

describe("buildBalanceReconciliation", () => {
  it("reconciles when imported balance matches the UTXO total", () => {
    const r = buildBalanceReconciliation(1.24810472, inventory(124810472));
    expect(r.importedSat).toBe(124810472);
    expect(r.utxoSat).toBe(124810472);
    expect(r.deltaSat).toBe(0);
    expect(r.reconciled).toBe(true);
    expect(r.available).toBe(true);
    expect(r.lastSyncedAt).toBe("2026-06-15T10:00:00Z");
  });

  it("flags a mismatch with the signed delta", () => {
    const r = buildBalanceReconciliation(1.25, inventory(100000000));
    expect(r.deltaSat).toBe(25000000);
    expect(r.reconciled).toBe(false);
  });

  it("absorbs sub-satoshi float drift within tolerance", () => {
    // 0.1 + 0.2 style float noise should not register as a mismatch.
    const r = buildBalanceReconciliation(0.3, inventory(30000000));
    expect(r.reconciled).toBe(true);
  });

  it("sums BTC and L-BTC totals, ignoring other assets", () => {
    const r = buildBalanceReconciliation(0.0002, {
      totals: [
        { asset: "BTC", amount_sat: 10000 },
        { asset: "L-BTC", amount_sat: 10000 },
        { asset: "USDT", amount_sat: 999 },
      ],
      support: { supported: true },
    });
    expect(r.utxoSat).toBe(20000);
    expect(r.reconciled).toBe(true);
  });

  it("is unavailable for sources without a UTXO inventory", () => {
    expect(buildBalanceReconciliation(1, null).available).toBe(false);
    expect(
      buildBalanceReconciliation(1, {
        support: { supported: false },
        totals: [],
      }).available,
    ).toBe(false);
  });
});
