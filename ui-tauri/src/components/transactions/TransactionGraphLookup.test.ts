import { describe, expect, it } from "vitest";

import { publicLookupCanAddToGraph } from "./TransactionGraphLookup";
import type { TransactionGraphNode, TransactionGraphPayload } from "./TransactionGraphModel";

const node = (valueState: TransactionGraphNode["valueState"]): TransactionGraphNode => ({
  id: `in-${valueState}`,
  valueState,
  valueSats: valueState === "known" ? 1_000 : null,
});
const graph = (
  supportLevel: TransactionGraphPayload["supportLevel"],
  chain: string,
  inputs: TransactionGraphNode[],
): TransactionGraphPayload => ({
  transaction: { id: "tx", chain },
  supportLevel,
  inputs,
  outputs: [],
});

describe("public graph lookup offer", () => {
  it("is offered only where the daemon would look something up", () => {
    // No local graph at all, on either chain.
    expect(publicLookupCanAddToGraph(graph("graphless", "bitcoin", []))).toBe(true);
    expect(publicLookupCanAddToGraph(graph("graphless", "liquid", []))).toBe(true);
    // A Bitcoin graph whose spent outputs lack their amounts.
    expect(publicLookupCanAddToGraph(graph("partial", "bitcoin", [node("missing")]))).toBe(true);
    // Complete graphs, and local Liquid graphs: a lookup cannot add anything.
    expect(publicLookupCanAddToGraph(graph("full", "bitcoin", [node("known")]))).toBe(false);
    expect(publicLookupCanAddToGraph(graph("partial", "liquid", [node("confidential")]))).toBe(false);
    expect(publicLookupCanAddToGraph(graph("partial", "liquid", [node("missing")]))).toBe(false);
    expect(publicLookupCanAddToGraph(undefined)).toBe(false);
  });
});
