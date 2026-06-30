import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";

import {
  TransactionFlowDiagram,
  TransactionGraphPanel,
  compactGraphRows,
  nodeTooltipTitle,
  sensitiveGraphText,
  type TransactionGraphPayload,
} from "./TransactionGraphTab";

const graph: TransactionGraphPayload = {
  transaction: {
    id: "tx-graph",
    txid: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    inputCount: 6,
    outputCount: 2,
    vsize: 200,
    feeRateSatVb: 5,
  },
  supportLevel: "full",
  inputs: Array.from({ length: 6 }, (_, index) => ({
    id: `in-${index}`,
    outpoint: `${index.toString(16).repeat(64)}:${index}`,
    valueSats: 100_000 * (index + 1),
    valueBtc: (100_000 * (index + 1)) / 100_000_000,
    label: `Input ${index + 1}`,
    wallet: "Cold Storage",
    ownership: "owned",
    role: "input",
    annotations: [{ code: "owned_input", label: "Owned wallet" }],
  })),
  outputs: [
    {
      id: "out-0",
      outpoint:
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:0",
      address: "bc1qrecipient000000000000000000000000000000",
      valueSats: 500_000,
      valueBtc: 0.005,
      wallet: "Hot Wallet",
      ownership: "owned",
      role: "owned_destination",
      annotations: [{ code: "owned_destination", label: "Owned destination" }],
    },
  ],
  fee: {
    id: "fee",
    label: "Miner fee",
    valueSats: 1_000,
    valueBtc: 0.00001,
    role: "fee",
    ownership: "network_fee",
  },
  warnings: [],
  annotations: [
    { code: "ownership_derived", label: "Ownership-derived transfer" },
  ],
};

describe("TransactionFlowDiagram", () => {
  it("aggregates overflow rows into a pseudo-node", () => {
    const rows = compactGraphRows(graph.inputs, "input", 4);

    expect(rows).toHaveLength(4);
    expect(rows[3].overflow).toBe(true);
    expect(rows[3].overflowCount).toBe(3);
    expect(rows[3].valueSats).toBe(1_500_000);
  });

  it("renders hidden-sensitive labels without leaking addresses or outpoints", () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={graph} hideSensitive />
      </TooltipProvider>,
    );

    expect(html).toContain("Hidden");
    expect(html).toContain("sensitive");
    expect(html).not.toContain("bc1qrecipient");
    expect(html).not.toContain("abcdef0123456789abcdef");
  });

  it("renders copy controls for graph references", () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={graph} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).toContain('aria-label="Copy input outpoint"');
    expect(html).toContain('aria-label="Copy output reference"');
  });

  it("does not reserve a bordered hover dock inside the drawing area", () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={graph} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).not.toContain('data-testid="transaction-graph-hover-detail"');
    expect(html).not.toContain("h-16 border-t");
    expect(html).toContain("h-full overflow-auto");
  });

  it("keeps ordinary multi-input transactions expanded before overflow compaction", () => {
    const rows = compactGraphRows(
      Array.from({ length: 14 }, (_, index) => ({
        ...graph.inputs[0],
        id: `many-in-${index}`,
        valueSats: 10_000,
      })),
      "input",
    );

    expect(rows).toHaveLength(14);
    expect(rows.some((row) => row.overflow)).toBe(false);
  });

  it("keeps tooltips compact and avoids missing-source implementation text", () => {
    const tx = graph.transaction;
    if (!tx) throw new Error("test graph transaction missing");
    const partial: TransactionGraphPayload = {
      ...graph,
      transaction: tx,
      inputs: [
        {
          id: "in-missing",
          outpoint:
            "274ec9b2059a018599cd09605d70a8d7eeb35910834fcd4e93116a4cb2e3ea81:0",
          role: "input",
          ownership: "external",
        },
      ],
      outputs: graph.outputs,
      supportLevel: "partial",
      fee: null,
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={partial} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(nodeTooltipTitle(partial.inputs[0])).toBe("274ec9b205...ea81:0");
    expect(html).not.toContain("source did not include this amount");
  });

  it("renders a mempool-style bowtie flow with a conserved middle band", () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={graph} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).not.toContain('data-testid="transaction-junction"');
    expect(html).not.toContain("transaction-flow-middle-gradient");
    expect(html).not.toContain('data-testid="transaction-melt-trunk"');
    expect(html).toContain('data-testid="transaction-fee-strand"');
    expect(html).not.toContain('data-testid="transaction-flow-middle-band"');
    expect(html).toContain('id="transaction-flow-input-gradient"');
    expect(html).toContain('id="transaction-flow-output-gradient"');
    expect(html).toContain('id="transaction-flow-fee-gradient"');
    expect(html).toContain('id="transaction-flow-input-hover-gradient"');
    expect(html).toContain('id="transaction-flow-output-hover-gradient"');
    expect(html).toContain('id="transaction-flow-fee-hover-gradient"');
    expect(html).toContain('id="transaction-flow-hover-glow"');
    expect(html).toContain('aria-label="Fee graph leg"');
    expect(html).not.toContain("markerStart");
    expect(html).not.toContain("<marker");
    expect(html).not.toContain(">fee</text>");
    expect(html).not.toContain("<circle");
    expect(html).not.toContain('width="88" height="104" rx="12"');
  });

  it("treats fees as the first output-side bowtie strand", () => {
    const twoOutputGraph: TransactionGraphPayload = {
      ...graph,
      inputs: graph.inputs.slice(0, 1),
      outputs: [
        graph.outputs[0],
        {
          ...graph.outputs[0],
          id: "out-1",
          outpoint:
            "bbcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:1",
          role: "external_recipient",
          ownership: "external",
        },
      ],
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={twoOutputGraph} hideSensitive={false} />
      </TooltipProvider>,
    );

    const feeIndex = html.indexOf('data-testid="transaction-fee-strand"');
    const firstOutputIndex = html.indexOf('data-testid="transaction-output-strand"');
    expect(feeIndex).toBeGreaterThan(-1);
    expect(firstOutputIndex).toBeGreaterThan(-1);
    expect(feeIndex).toBeLessThan(firstOutputIndex);
    const visiblePaths = [...html.matchAll(/<path d="([^"]+)" data-testid="transaction-(fee|output)-strand"/g)];
    expect(new Set(visiblePaths.map((match) => match[1])).size).toBe(visiblePaths.length);
  });

  it("keeps known Liquid fees thinner than confidential output strands", () => {
    const confidentialLiquidGraph: TransactionGraphPayload = {
      ...graph,
      transaction: {
        ...graph.transaction,
        id: "liquid-confidential",
        inputCount: 72,
        outputCount: 2,
        feeRateSatVb: null,
      },
      supportLevel: "partial",
      inputs: Array.from({ length: 8 }, (_, index) => ({
        id: `conf-in-${index}`,
        outpoint: `${index.toString(16).repeat(64)}:${index}`,
        valueSats: null,
        valueBtc: null,
        valueState: "confidential" as const,
        role: "input",
        ownership: "owned",
      })),
      outputs: [
        {
          id: "conf-out-0",
          outpoint:
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:0",
          valueSats: 0,
          valueBtc: 0,
          valueState: "confidential" as const,
          role: "change",
          ownership: "owned",
        },
        {
          id: "conf-out-1",
          outpoint:
            "bbcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:1",
          valueSats: null,
          valueBtc: null,
          valueState: "confidential" as const,
          role: "external_recipient",
          ownership: "external",
        },
      ],
      fee: {
        id: "fee",
        label: "Fee",
        valueSats: 51,
        valueBtc: 0.00000051,
        role: "fee",
        ownership: "network_fee",
      },
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={confidentialLiquidGraph} hideSensitive={false} />
      </TooltipProvider>,
    );

    const feeWidth = Number(
      html.match(/data-testid="transaction-fee-strand"[^>]*stroke-width="([^"]+)"/)?.[1],
    );
    const outputWidths = [
      ...html.matchAll(
        /data-testid="transaction-output-strand"[^>]*stroke-width="([^"]+)"/g,
      ),
    ].map((match) => Number(match[1]));

    expect(outputWidths).toHaveLength(2);
    expect(Number.isFinite(feeWidth)).toBe(true);
    expect(feeWidth).toBeLessThan(Math.min(...outputWidths));
  });

  it("keeps different-value output endpoints aligned on one rail", () => {
    const variedOutputGraph: TransactionGraphPayload = {
      ...graph,
      inputs: graph.inputs.slice(0, 1),
      fee: null,
      outputs: [
        {
          ...graph.outputs[0],
          id: "large-out",
          valueSats: 1_900_000,
          valueBtc: 0.019,
        },
        {
          ...graph.outputs[0],
          id: "small-out",
          outpoint:
            "bbcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:1",
          valueSats: 100_000,
          valueBtc: 0.001,
        },
      ],
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={variedOutputGraph} hideSensitive={false} />
      </TooltipProvider>,
    );

    const outputPaths = [
      ...html.matchAll(/<path d="M ([0-9.]+) [^"]+" data-testid="transaction-output-strand"/g),
    ];
    expect(outputPaths).toHaveLength(2);
    expect(new Set(outputPaths.map((match) => match[1]))).toEqual(new Set(["896"]));
  });

  it("flattens graph geometry when sensitive values are hidden", () => {
    const variedOutputGraph: TransactionGraphPayload = {
      ...graph,
      inputs: graph.inputs.slice(0, 1),
      fee: null,
      outputs: [
        {
          ...graph.outputs[0],
          id: "large-hidden-out",
          valueSats: 1_900_000,
          valueBtc: 0.019,
        },
        {
          ...graph.outputs[0],
          id: "small-hidden-out",
          outpoint:
            "bbcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:1",
          valueSats: 100_000,
          valueBtc: 0.001,
        },
      ],
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={variedOutputGraph} hideSensitive />
      </TooltipProvider>,
    );

    const outputStrokeWidths = [
      ...html.matchAll(
        /data-testid="transaction-output-strand"[^>]*stroke-width="([^"]+)"/g,
      ),
    ].map((match) => match[1]);
    expect(outputStrokeWidths).toHaveLength(2);
    expect(new Set(outputStrokeWidths).size).toBe(1);
  });

  it("omits missing value placeholders and unknown fee stats", () => {
    const tx = graph.transaction;
    if (!tx) throw new Error("test graph transaction missing");
    const partial: TransactionGraphPayload = {
      ...graph,
      transaction: { ...tx, feeRateSatVb: null, size: null, vsize: null },
      inputs: [
        {
          id: "in-missing",
          outpoint:
            "1ce0f6d87e3b5a0a7fc116a4c2360a51203fe8877870de3d87d8ec351707a8b0:0",
          role: "input",
          ownership: "external",
        },
      ],
      supportLevel: "partial",
      fee: null,
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel graph={partial} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).not.toContain("Value not stored");
    expect(html).not.toContain("FEE RATE");
    expect(html).not.toContain("SIZE");
    expect(html).not.toContain("Inputs</div>");
    expect(html).not.toContain("Outputs</div>");
  });

  it("renders unknown and confidential values as bowtie strands", () => {
    const tx = graph.transaction;
    if (!tx) throw new Error("test graph transaction missing");
    const partial: TransactionGraphPayload = {
      ...graph,
      transaction: tx,
      inputs: [{ id: "unknown-input", role: "input", ownership: "external" }],
      outputs: [
        {
          id: "confidential-output",
          role: "external_recipient",
          ownership: "external",
          valueState: "confidential",
        },
        graph.outputs[0],
      ],
      fee: null,
      supportLevel: "partial",
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={partial} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html.match(/data-testid="transaction-input-strand"/g)).toHaveLength(1);
    expect(html.match(/data-testid="transaction-output-strand"/g)).toHaveLength(2);
    expect(html).not.toContain("Value not stored");
  });

  it("keeps informational prevout gaps out of warning banners", () => {
    const tx = graph.transaction;
    if (!tx) throw new Error("test graph transaction missing");
    const partial: TransactionGraphPayload = {
      ...graph,
      transaction: { ...tx, feeRateSatVb: null, size: null, vsize: null },
      supportLevel: "partial",
      unsupportedReason: "input_prevout_values_missing",
      warnings: [
        {
          code: "input_prevout_values_missing",
          level: "info",
          message:
            "Bitcoin outputs in this transaction have values. Some input values are not stored locally because an input amount comes from the spent previous output.",
        },
      ],
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel graph={partial} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).toContain("input amounts need spent previous-output data");
    expect(html).not.toContain("Some input values are not stored locally");
  });

  it("uses a stable non-scaling canvas for the diagram", () => {
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionFlowDiagram graph={graph} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).toContain('data-testid="transaction-flow-canvas"');
    expect(html).toContain("width:960px");
  });

  it("returns an explicit placeholder for hidden sensitive text", () => {
    expect(sensitiveGraphText("bc1qsecret", true)).toBe("Hidden");
    expect(sensitiveGraphText("bc1qsecret", false)).toBe("bc1qsecret");
  });
});

describe("TransactionGraphPanel", () => {
  it("renders paired swap route context above the single transaction graph", () => {
    const withRoute: TransactionGraphPayload = {
      ...graph,
      swapRoute: {
        id: "pair-1",
        kind: "swap",
        policy: "carrying-value",
        currentLeg: "out",
        swapFeeBtc: 0.00012977,
        out: {
          id: "swap-out",
          txid: "8f95646aaf1364f3fbcd1046ed1752b7e27189f5c30ce0d6633b32bd9cbc019c",
          asset: "LBTC",
          network: "Liquid",
          amountBtc: 0.12426275,
          wallet: { id: "wallet-liquid", label: "Satoshi-Liquid", kind: "liquid" },
          counterparty: "Swap LBTC -> BTC",
        },
        in: {
          id: "swap-in",
          txid: "afec51d0bc2dd514bc47406d11c8c750c12fa6382e845064b5e8bfb4f49779e",
          asset: "BTC",
          network: "Bitcoin",
          amountBtc: 0.12413298,
          wallet: { id: "wallet-btc", label: "Satoshi-Onchain-Multi", kind: "descriptor" },
          counterparty: "Swap LBTC -> BTC",
        },
      },
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel graph={withRoute} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).toContain('data-testid="swap-route-strip"');
    expect(html).toContain("Paired swap route");
    expect(html).toContain("Consolidation leg");
    expect(html).not.toContain("Spent leg");
    expect(html).toContain("LBTC -&gt; BTC");
    expect(html).toContain("Liquid");
    expect(html).toContain("Bitcoin");
    expect(html).toContain("Swap LBTC -&gt; BTC");
    expect(html).toContain("Selected");
  });

  it("renders paired swap legs as selectable toggles", () => {
    const withRoute: TransactionGraphPayload = {
      ...graph,
      swapRoute: {
        id: "pair-1",
        kind: "swap",
        policy: "carrying-value",
        currentLeg: "out",
        out: {
          id: "swap-out",
          txid: "8f95646aaf1364f3fbcd1046ed1752b7e27189f5c30ce0d6633b32bd9cbc019c",
          asset: "LBTC",
          network: "Liquid",
          amountBtc: 0.12426275,
          wallet: { id: "wallet-liquid", label: "Satoshi-Liquid", kind: "liquid" },
        },
        in: {
          id: "swap-in",
          txid: "afec51d0bc2dd514bc47406d11c8c750c12fa6382e845064b5e8bfb4f49779e",
          asset: "BTC",
          network: "Bitcoin",
          amountBtc: 0.12413298,
          wallet: { id: "wallet-btc", label: "Satoshi-Onchain-Multi", kind: "descriptor" },
        },
      },
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel
          graph={withRoute}
          hideSensitive={false}
          selectedSwapLeg="in"
          onSelectSwapLeg={() => undefined}
        />
      </TooltipProvider>,
    );

    expect(html).toContain("<button");
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-pressed="false"');
    expect(html).toContain("Received leg");
  });

  it("renders reviewed Coinjoin routes without swap wording", () => {
    const withRoute: TransactionGraphPayload = {
      ...graph,
      swapRoute: {
        id: "pair-coinjoin",
        kind: "coinjoin",
        routeKind: "coinjoin",
        policy: "carrying-value",
        currentLeg: "out",
        out: {
          id: "coinjoin-out",
          txid: "8f95646aaf1364f3fbcd1046ed1752b7e27189f5c30ce0d6633b32bd9cbc019c",
          asset: "BTC",
          network: "Bitcoin",
          amountBtc: 0.1,
          wallet: { id: "wallet-a", label: "Wallet A", kind: "descriptor" },
        },
        in: {
          id: "coinjoin-in",
          txid: "afec51d0bc2dd514bc47406d11c8c750c12fa6382e845064b5e8bfb4f49779e",
          asset: "BTC",
          network: "Bitcoin",
          amountBtc: 0.0995,
          wallet: { id: "wallet-c", label: "Wallet C", kind: "descriptor" },
        },
      },
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel graph={withRoute} hideSensitive={false} />
      </TooltipProvider>,
    );

    expect(html).toContain("Coinjoin route");
    expect(html).toContain("Coinjoin");
    expect(html).toContain("Spent leg");
    expect(html).toContain("Received leg");
    expect(html).not.toContain("Paired swap route");
    expect(html).not.toContain("Swap counterparty");
  });

  it("hides sensitive paired swap route values", () => {
    const withRoute: TransactionGraphPayload = {
      ...graph,
      swapRoute: {
        id: "pair-1",
        kind: "swap",
        policy: "carrying-value",
        currentLeg: "in",
        swapFeeBtc: 0.00012977,
        out: {
          id: "swap-out",
          txid: "8f95646aaf1364f3fbcd1046ed1752b7e27189f5c30ce0d6633b32bd9cbc019c",
          asset: "LBTC",
          network: "Liquid",
          amountBtc: 0.12426275,
          wallet: { id: "wallet-liquid", label: "Secret Liquid Wallet", kind: "liquid" },
          counterparty: "Private Swap Desk",
        },
        in: {
          id: "swap-in",
          txid: "afec51d0bc2dd514bc47406d11c8c750c12fa6382e845064b5e8bfb4f49779e",
          asset: "BTC",
          network: "Bitcoin",
          amountBtc: 0.12413298,
          wallet: { id: "wallet-btc", label: "Secret Bitcoin Wallet", kind: "descriptor" },
          counterparty: "Private Swap Desk",
        },
      },
    };
    const html = renderToStaticMarkup(
      <TooltipProvider>
        <TransactionGraphPanel graph={withRoute} hideSensitive />
      </TooltipProvider>,
    );

    expect(html).toContain("Hidden");
    expect(html).toContain("sensitive");
    expect(html).not.toContain("Secret Liquid Wallet");
    expect(html).not.toContain("Secret Bitcoin Wallet");
    expect(html).not.toContain("Private Swap Desk");
    expect(html).not.toContain("0.12426275");
  });

  it("renders a clear graphless empty state", () => {
    const graphless: TransactionGraphPayload = {
      transaction: { id: "csv-row" },
      supportLevel: "graphless",
      unsupportedReason: "graphless_import",
      inputs: [],
      outputs: [],
      fee: null,
      warnings: [],
      annotations: [],
    };
    const html = renderToStaticMarkup(
      <TransactionGraphPanel graph={graphless} hideSensitive={false} />,
    );

    expect(html).toContain("No graph for this source");
    expect(html).toContain("without valued Bitcoin vin/vout data");
  });
});
