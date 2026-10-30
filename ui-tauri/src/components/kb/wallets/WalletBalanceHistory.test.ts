import { describe, expect, it } from "vitest";

import { balancePointsFromRows } from "./WalletBalanceHistory";

const NOW = new Date("2024-06-10T12:00:00Z");

const row = (
  month: string,
  quantity: number,
  marketValue: number,
  asset = "BTC",
) => ({
  period_start: `${month}-01T00:00:00Z`,
  asset,
  quantity,
  market_value: marketValue,
});

describe("balancePointsFromRows", () => {
  it("fills the months the daemon leaves out, up to the current month", () => {
    // Emptied in April: the daemon sends nothing from then on.
    const { points, complete } = balancePointsFromRows(
      [row("2024-01", 1, 40_000), row("2024-03", 0.5, 30_000)],
      { now: NOW },
    );

    expect(complete).toBe(true);
    expect(points.map((point) => [point.bucket, point.quantity])).toEqual([
      ["2024-01", 1],
      ["2024-02", 0],
      ["2024-03", 0.5],
      ["2024-04", 0],
      ["2024-05", 0],
      ["2024-06", 0],
    ]);
    expect(points[3].value).toBe(0);
  });

  it("sums bitcoin and L-BTC and leaves other assets out", () => {
    const { points } = balancePointsFromRows(
      [
        row("2024-06", 1, 60_000),
        row("2024-06", 0.25, 15_000, "LBTC"),
        row("2024-06", 500, 500, "USDT"),
      ],
      { now: NOW },
    );

    expect(points).toEqual([
      { bucket: "2024-06", quantity: 1.25, value: 75_000 },
    ]);
  });

  it("treats a held month valued at zero as unpriced", () => {
    // The daemon multiplies by a rate that defaults to zero before any price.
    const { points } = balancePointsFromRows(
      [row("2024-05", 1, 0), row("2024-06", 1, 60_000)],
      { now: NOW },
    );

    expect(points.map((point) => point.value)).toEqual([null, 60_000]);
  });

  it("drops the oldest month of a truncated history", () => {
    // Row truncation can cut a month between its assets.
    const { points, complete } = balancePointsFromRows(
      [
        row("2024-04", 0.1, 4_000, "LBTC"),
        row("2024-05", 1, 50_000),
        row("2024-06", 1, 60_000),
      ],
      { truncated: true, now: NOW },
    );

    expect(complete).toBe(false);
    expect(points.map((point) => point.bucket)).toEqual(["2024-05", "2024-06"]);
  });
});
