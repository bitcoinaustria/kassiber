import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { TransactionGraphTechnicalDetails } from "./TransactionGraphTechnicalDetails";
import type { TransactionGraphPayload } from "./TransactionGraphModel";

const graph: TransactionGraphPayload = {
  transaction: { id: "exchange-sale", inputCount: 0, outputCount: 0 },
  supportLevel: "graphless", inputs: [], outputs: [], fee: null,
};

it("omits empty technical data for an accounting row without a graph", () => {
  expect(renderToStaticMarkup(<TransactionGraphTechnicalDetails graph={graph} hideSensitive={false} />)).toBe("");
});

it("shows observed zero fees and locktime instead of marking them unknown", () => {
  const observed: TransactionGraphPayload = { ...graph, supportLevel: "full",
    transaction: { id: "zero-fee", inputCount: 1, outputCount: 2, locktime: 0, feeRateSatVb: 0 },
    fee: { id: "fee", valueSats: 0 },
  };
  const html = renderToStaticMarkup(<TransactionGraphTechnicalDetails graph={observed} hideSensitive={false} />);
  expect(html).toContain("0 sat/vB");
  expect(html).toContain("Network fee");
  expect(html).toContain("Locktime");
  expect(html).not.toContain("Unknown");
  expect(html).not.toContain("Weight");
  const hidden = renderToStaticMarkup(<TransactionGraphTechnicalDetails graph={observed} hideSensitive />);
  expect(hidden).toContain("Hidden");
  expect(hidden).not.toContain("sat/vB");
});

it("does not infer transaction counts from capped display nodes", () => {
  const partial = { ...graph, supportLevel: "partial" as const,
    transaction: { id: "partial", size: 300 },
    inputs: [{ id: "overflow", overflow: true, overflowCount: 500 }],
  };
  const html = renderToStaticMarkup(<TransactionGraphTechnicalDetails graph={partial} hideSensitive={false} />);
  expect(html).toContain("300 B");
  expect(html).not.toContain("Inputs");
  expect(html).not.toContain("Outputs");
});
