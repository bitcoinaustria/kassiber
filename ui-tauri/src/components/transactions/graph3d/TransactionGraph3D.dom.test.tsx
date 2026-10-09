// @vitest-environment happy-dom
//
// Mounted, not static: WebGL detection and the scene run in an effect.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import { TooltipProvider } from "@/components/ui/tooltip";

import type { TransactionGraphPayload } from "../TransactionGraphModel";
import { TransactionGraph3D } from "./TransactionGraph3D";

const graph: TransactionGraphPayload = {
  transaction: { id: "tx" },
  supportLevel: "full",
  inputs: [{ id: "in", valueSats: 1_000, valueBtc: 0.00001, valueState: "known", ownership: "owned" }],
  outputs: [{ id: "out", valueSats: 900, valueBtc: 0.000009, valueState: "known" }],
  fee: { id: "fee", valueSats: 100, valueBtc: 0.000001 },
};

/** A WebGL probe that succeeds; the 2D surface-colour probe gets nothing. */
function stubWebgl() {
  const probe = { getExtension: () => ({ loseContext() {} }) };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    ((kind: string) => (kind === "webgl2" ? probe : null)) as never,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("3D transaction graph", () => {
  it("falls back to the 2D graph where WebGL is not available", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    render(
      <TooltipProvider>
        <TransactionGraph3D
          graph={graph}
          hideSensitive={false}
          maxRows={250}
          fallback={<div data-testid="flat-graph" />}
        />
      </TooltipProvider>,
    );
    expect(screen.getByTestId("flat-graph")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("not available");
    expect(screen.queryByTestId("transaction-graph-3d")).toBeNull();
  });
});

describe("3D transaction graph scene", () => {
  it("keeps its scene when the payload is copied without changing its legs", async () => {
    // WebGL present, scene loads: count how often the scene is built.
    stubWebgl();
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    const scene = { resize: vi.fn(), setView: vi.fn(), render: vi.fn(), dispose: vi.fn() };
    const createGlassScene = vi.fn(() => scene);
    vi.doMock("./glassScene", () => ({ createGlassScene }));
    const { TransactionGraph3D: Fresh } = await import("./TransactionGraph3D");
    const view = (payload: TransactionGraphPayload) => (
      <TooltipProvider>
        <Fresh graph={payload} hideSensitive={false} maxRows={250} fallback={null} />
      </TooltipProvider>
    );
    const { rerender } = render(view(graph));
    await vi.waitFor(() => expect(createGlassScene).toHaveBeenCalledTimes(1));
    rerender(view({ ...graph, swapRoute: null }));
    rerender(view({ ...graph, annotations: [] }));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(createGlassScene).toHaveBeenCalledTimes(1);
    rerender(view({ ...graph, outputs: [...graph.outputs] }));
    await vi.waitFor(() => expect(createGlassScene).toHaveBeenCalledTimes(2));
    vi.unstubAllGlobals();
    vi.doUnmock("./glassScene");
  });

  it("lists only the legend entries the drawing uses", () => {
    stubWebgl();
    vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
    render(
      <TooltipProvider>
        <TransactionGraph3D graph={graph} hideSensitive={false} maxRows={250} fallback={null} />
      </TooltipProvider>,
    );
    const legend = document.body.textContent ?? "";
    expect(legend).toContain("Known amount");
    expect(legend).toContain("Fee");
    expect(legend).not.toContain("Amount unknown");
    vi.unstubAllGlobals();
  });
});
