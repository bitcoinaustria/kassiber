import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import "@/i18n";
import { router } from "@/routeTree";
import { assistantScreenContextFor } from "@/components/ai/assistantScreenContext";
import { InvestigationGraph } from "./InvestigationGraph";
import { SelectionInspector } from "./SelectionInspector";
import type { AnalysisEdge, AnalysisNode } from "@/lib/chainAnalysis";

describe("investigation graph presentation", () => {
  it.each(["stale", "conflicting"])(
    "makes %s custody evidence distinct even when its provenance was native verified",
    (status) => {
      const nodes: AnalysisNode[] = ["a", "b"].map((id) => ({
        id,
        kind: "record",
        chain: "bitcoin",
        network: "regtest",
        label: id,
        wallet_ids: [],
        amount_msat: null,
        asset: "BTC",
        evidence: [],
      }));
      const edge: AnalysisEdge = {
        id: "relation",
        source: "a",
        target: "b",
        kind: "custody",
        evidence_level: "native_verified",
        status,
        amount_msat: null,
        label: "Custody relation",
        evidence: [],
      };
      const html = renderToStaticMarkup(
        <InvestigationGraph
          nodes={nodes}
          edges={[edge]}
          selected={{ kind: "edge", id: edge.id }}
          highlightedIds={[]}
          onSelect={() => {}}
        />,
      );
      const edgeMarkup = html.slice(html.indexOf('class="ca-edge"'));
      const description =
        status === "stale" ? "Stale evidence" : "Conflicting evidence";
      expect(edgeMarkup).toContain(`data-status="${status}"`);
      expect(edgeMarkup).toContain(
        `aria-label="Custody relation · ${description}"`,
      );
      expect(edgeMarkup).toContain('stroke-dasharray="2 5"');
      expect(edgeMarkup).not.toContain('stroke-dasharray="9 3"');
      expect(edgeMarkup).toContain(
        `stroke="${status === "stale" ? "var(--ca-muted)" : "var(--destructive)"}"`,
      );
      const inspector = renderToStaticMarkup(
        <SelectionInspector
          edge={edge}
          busy={false}
          onTrace={() => {}}
          onTarget={() => {}}
          onSelect={() => {}}
          onError={() => {}}
        />,
      );
      expect(inspector).toContain(">Status</dt>");
      expect(inspector).toContain(`>${description}</dd>`);
      expect(inspector).toContain(">native_verified</dd>");
    },
  );
  it("renders selectable physical, custody and hypothesis edges with different line styles", () => {
    const nodes: AnalysisNode[] = ["a", "b"].map((id) => ({
      id,
      kind: "output",
      chain: "bitcoin",
      network: "regtest",
      label: `<script>${id}</script>`,
      outpoint: `${id}:0`,
      wallet_ids: [],
      amount_msat: "100000000001",
      asset: "BTC",
      evidence: [],
    }));
    const edges: AnalysisEdge[] = (
      ["spends", "custody", "hypothesis"] as const
    ).map((kind) => ({
      id: kind,
      source: "a",
      target: "b",
      kind,
      evidence_level: "observed",
      amount_msat: null,
      label: kind,
      evidence: [],
    }));
    const html = renderToStaticMarkup(
      <InvestigationGraph
        nodes={nodes}
        edges={edges}
        selected={{ kind: "node", id: "a" }}
        highlightedIds={[]}
        onSelect={() => {}}
      />,
    );
    expect(html).toContain('stroke-dasharray="9 3"');
    expect(html).toContain('stroke-dasharray="5 5"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain("1.00000000001 BTC");
    expect(html).toContain("&lt;script&gt;a&lt;/script&gt;");
    expect(html).not.toContain("<script>");
    expect(html).toContain('aria-label="Zoom in"');
    expect(html).toContain('aria-label="Focus selection"');
    expect(html).toContain('class="ca-graph-overview"');
    expect(html).toContain("nodes in view");
    expect(html).toContain(">Fit graph</span>");
  });
  it("exposes the power-user route without a developer gate and gives AI only allowlisted screen context", () => {
    expect(
      router.routesByPath["/chain-analysis"].options.beforeLoad,
    ).toBeUndefined();
    expect(
      assistantScreenContextFor(
        "/chain-analysis",
        "?subject=%2FUsers%2Fsecret&url=https://example.test",
      ),
    ).toEqual({
      route: "/chain-analysis",
      capabilities: ["privacy", "transactions", "transfers"],
    });
  });
});
