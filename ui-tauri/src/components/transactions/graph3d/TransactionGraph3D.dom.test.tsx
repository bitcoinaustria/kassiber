// @vitest-environment happy-dom
//
// Mounted, not static: WebGL detection and the scene run in an effect.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { TransactionGraphPayload } from "../TransactionGraphModel";
import { TransactionGraph3D } from "./TransactionGraph3D";

const graph: TransactionGraphPayload = {
  transaction: { id: "tx" },
  supportLevel: "full",
  inputs: [{ id: "in", valueSats: 1_000, valueBtc: 0.00001, valueState: "known", ownership: "owned" }],
  outputs: [{ id: "out", valueSats: 900, valueBtc: 0.000009, valueState: "known" }],
  fee: { id: "fee", valueSats: 100, valueBtc: 0.000001 },
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("3D transaction graph", () => {
  it("falls back to the 2D graph where WebGL is not available", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    render(
      <TransactionGraph3D
        graph={graph}
        hideSensitive={false}
        maxRows={250}
        fallback={<div data-testid="flat-graph" />}
      />,
    );
    expect(screen.getByTestId("flat-graph")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("not available");
    expect(screen.queryByTestId("transaction-graph-3d")).toBeNull();
  });
});
