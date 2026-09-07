import { describe, expect, it } from "vitest";
import { provenanceGraph, type ProvenanceFlow } from "./provenanceGraph";

const flow: ProvenanceFlow = {
  levels: [
    { role: "source", nodes: [{ id: "source:a", label: "Purchase receipt", node_type: "source", amount: 0.006, asset: "BTC" }] },
    { role: "privacy_boundary", nodes: [{ id: "tx:b", label: "collaborative tx", wallet: "Intermediate wallet", node_type: "transaction", transaction_id: "b", deferred_privacy_hop: true, amount: 0.006, asset: "BTC" }] },
    { role: "target", nodes: [{ id: "tx:c", label: "receipt hash", wallet: "Own wallet", node_type: "transaction", transaction_id: "c", amount: 0.006, asset: "BTC" }] },
  ],
  edges: [{ id: "reviewed", from: "tx:b", to: "tx:c", link_type: "payjoin", deferred_privacy_hop: true, amount: 0.006, asset: "BTC" }],
};

describe("reviewed provenance graph adapter", () => {
  it("never invents connections from level adjacency or traverses a privacy boundary", () => {
    const graph = provenanceGraph(flow, role => role);
    expect(graph.edges.map(edge => [edge.source, edge.target])).toEqual([["tx:b", "tx:c"]]);
    expect(graph.edges[0]).toMatchObject({ appearance: "warning", dash: "dashed" });
    expect(graph.edges[0].title).toContain("privacy_boundary");
    expect(provenanceGraph({ ...flow, edges: [] }, role => role).edges).toEqual([]);
  });
  it("keeps source/transaction identity and allocated amounts while using readable wallet labels", () => {
    const graph = provenanceGraph(flow, role => role);
    expect(graph.subjectId).toBe("tx:c");
    expect(graph.nodes.find(node => node.id === "tx:c")).toMatchObject({ label: "Own wallet", detail: "0.00600000 BTC" });
    expect(graph.byId.get("tx:b")?.node.transaction_id).toBe("b");
    expect(graph.byId.get("source:a")?.node.transaction_id).toBeUndefined();
  });
  it("uses translated relationship names without changing the edge", () => {
    const graph = provenanceGraph(flow, role => role, direction => direction, () => "Geprüfte Verbindung");
    expect(graph.edges[0].label).toBe("Geprüfte Verbindung");
    expect(graph.edges[0].source).toBe("tx:b");
  });
  it("does not draw dangling links outside the disclosed graph", () => {
    const graph = provenanceGraph({ ...flow, edges: [{ id: "outside", from: "hidden", to: "tx:c" }] }, role => role);
    expect(graph.edges).toEqual([]);
  });
});
