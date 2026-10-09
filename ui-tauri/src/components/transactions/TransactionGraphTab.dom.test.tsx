// @vitest-environment happy-dom
//
// Mounted: the drawing and the list below it share one lit leg.
import type { ReactNode } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

// The UI store persists the currency switch; give it somewhere to write.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

import "@/i18n";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui";

import { TransactionGraphPanel } from "./TransactionGraphTab";
import type { TransactionGraphPayload } from "./TransactionGraphModel";

const picked = vi.hoisted(() => ({ part: "" }));

// No WebGL here: exercise the flat bowtie the 3D view falls back to, plus a
// stand-in for a click on a leg of the 3D drawing.
vi.mock("./graph3d/TransactionGraph3D", () => ({
  TransactionGraph3D: ({
    fallback,
    onSelectPart,
  }: {
    fallback: ReactNode;
    onSelectPart?: (part: string) => void;
  }) => (
    <div>
      {fallback}
      <button type="button" data-testid="pick-leg" onClick={() => onSelectPart?.(picked.part)} />
    </div>
  ),
}));

const graph: TransactionGraphPayload = {
  transaction: { id: "tx", chain: "bitcoin" },
  supportLevel: "full",
  inputs: [
    { id: "in-0", outpoint: `${"a".repeat(64)}:0`, valueSats: 600_000, valueBtc: 0.006, ownership: "owned", role: "input" },
  ],
  outputs: [
    { id: "out-0", outpoint: `${"b".repeat(64)}:0`, valueSats: 400_000, valueBtc: 0.004, ownership: "external", role: "external_recipient" },
    { id: "out-1", outpoint: `${"b".repeat(64)}:1`, valueSats: 199_000, valueBtc: 0.00199, ownership: "owned", role: "change" },
  ],
  fee: { id: "fee", valueSats: 1_000, valueBtc: 0.00001, role: "fee", ownership: "network_fee" },
};

afterEach(() => {
  cleanup();
  useUiStore.setState({ currency: "btc" });
  vi.restoreAllMocks();
});

function mount({
  graph: shown = graph,
  fiatPrice,
  hideSensitive = false,
}: { graph?: TransactionGraphPayload; fiatPrice?: number | null; hideSensitive?: boolean } = {}) {
  render(
    <TooltipProvider>
      <TransactionGraphPanel graph={shown} hideSensitive={hideSensitive} fiatPrice={fiatPrice} />
    </TooltipProvider>,
  );
}

const row = (part: string) => document.querySelector<HTMLElement>(`[data-graph-part="${part}"]`)!;
const outputStrands = () => screen.getAllByTestId("transaction-output-strand");

describe("transaction graph and its legs list", () => {
  it("lights a leg in the drawing while its row is pointed at", () => {
    mount();
    const [, change] = outputStrands();
    expect(change.getAttribute("stroke")).not.toContain("hover");
    fireEvent.pointerEnter(row("output:out-1"));
    expect(outputStrands()[1].getAttribute("stroke")).toContain("hover");
    expect(outputStrands()[0].getAttribute("stroke")).not.toContain("hover");
    fireEvent.pointerLeave(row("output:out-1"));
    expect(outputStrands()[1].getAttribute("stroke")).not.toContain("hover");
  });

  it("marks a leg's row while the leg is pointed at in the drawing", () => {
    mount();
    // Each strand is drawn twice: a wide invisible hit path, then the visible one.
    const hitPath = outputStrands()[0].previousElementSibling!;
    fireEvent.pointerEnter(hitPath);
    expect(row("output:out-0").dataset.active).toBe("true");
    expect(row("output:out-1").dataset.active).toBeUndefined();
    expect(screen.getByTestId("transaction-graph-hover-detail")).toBeTruthy();
    fireEvent.pointerLeave(hitPath);
    expect(row("output:out-0").dataset.active).toBeUndefined();
  });

  it("shows the legs in fiat at the transaction's price and switches on a click", () => {
    useUiStore.setState({ currency: "eur" });
    mount({ fiatPrice: 50_000 });
    const change = row("output:out-1");
    // 0.00199 BTC at € 50.000 per BTC.
    expect(change.textContent).toContain("99,50");
    fireEvent.click(screen.getAllByRole("button", { name: "Show amounts in bitcoin" })[0]);
    expect(useUiStore.getState().currency).toBe("btc");
    expect(row("output:out-1").textContent).toContain("₿ 0.00199000");
  });

  it("keeps the legs in bitcoin without a price, and never shows fiat for hidden values", () => {
    useUiStore.setState({ currency: "eur" });
    mount({ fiatPrice: null });
    expect(row("output:out-1").textContent).toContain("₿ 0.00199000");
    expect(screen.queryByRole("button", { name: "Show amounts in bitcoin" })).toBeNull();
    cleanup();
    mount({ fiatPrice: 50_000, hideSensitive: true });
    expect(row("output:out-1").textContent).not.toContain("€");
  });

  describe("a leg clicked in the drawing", () => {
    // More outputs than a folded column lists.
    const wide: TransactionGraphPayload = {
      ...graph,
      outputs: Array.from({ length: 10 }, (_, index) => ({
        id: `out-${index}`,
        outpoint: `${"b".repeat(64)}:${index}`,
        valueSats: 50_000,
        valueBtc: 0.0005,
        ownership: "external",
        role: "external_recipient",
      })),
    };
    const scrolls = () => {
      const scroll = vi.fn();
      vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(function (
        this: HTMLElement,
        options?: boolean | ScrollIntoViewOptions,
      ) {
        scroll(this.dataset.graphPart, options);
      });
      return scroll;
    };

    it("opens its folded column and scrolls to its row once mounted", () => {
      const scroll = scrolls();
      mount({ graph: wide });
      expect(document.querySelector('[data-graph-part="output:out-9"]')).toBeNull();
      picked.part = "output:out-9";
      fireEvent.click(screen.getByTestId("pick-leg"));
      expect(row("output:out-9")).toBeTruthy();
      expect(scroll).toHaveBeenCalledExactlyOnceWith("output:out-9", {
        block: "nearest",
        behavior: "smooth",
      });
      // The same leg again scrolls again.
      fireEvent.click(screen.getByTestId("pick-leg"));
      expect(scroll).toHaveBeenCalledTimes(2);
    });

    it("jumps instead of gliding when the user asked for less motion", () => {
      vi.spyOn(window, "matchMedia").mockImplementation(
        (query) => ({ matches: query === "(prefers-reduced-motion: reduce)" }) as MediaQueryList,
      );
      const scroll = scrolls();
      mount({ graph: wide });
      picked.part = "output:out-1";
      fireEvent.click(screen.getByTestId("pick-leg"));
      expect(scroll).toHaveBeenCalledExactlyOnceWith("output:out-1", {
        block: "nearest",
        behavior: "auto",
      });
    });
  });
});
