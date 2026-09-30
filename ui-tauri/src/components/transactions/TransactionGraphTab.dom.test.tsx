// @vitest-environment happy-dom
//
// Mounted: the drawing and the list below it share one lit leg.
import type { ReactNode } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import { TooltipProvider } from "@/components/ui/tooltip";

import { TransactionGraphPanel } from "./TransactionGraphTab";
import type { TransactionGraphPayload } from "./TransactionGraphModel";

// No WebGL here: exercise the flat bowtie the 3D view falls back to.
vi.mock("./graph3d/TransactionGraph3D", () => ({
  TransactionGraph3D: ({ fallback }: { fallback: ReactNode }) => <div>{fallback}</div>,
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

afterEach(cleanup);

function mount() {
  render(
    <TooltipProvider>
      <TransactionGraphPanel graph={graph} hideSensitive={false} />
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
});
