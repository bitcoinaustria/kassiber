import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { TransactionRecordFlow } from "./TransactionRecordFlow";
import type { Transaction } from "./model";
import { graphlessTradeKind, type TransactionGraphPayload } from "./TransactionGraphModel";

const sale: Transaction = {
  id: "sale", txnId: "exchange:sale", amountBtc: -0.001, amount: -149,
  asset: "BTC", fiatCurrency: "EUR", pricingSourceKind: "exchange_execution", pricingQuality: "exact",
  kind: "sell", counterparty: "Private exchange", counterpartyInitials: "PE",
  direction: "Send", paymentMethod: "Exchange", date: "2025-11-09", status: "completed",
};
const graph: TransactionGraphPayload = {
  transaction: { id: "sale" }, supportLevel: "graphless", inputs: [], outputs: [],
};

it("shows a source-backed sale as a trade receipt instead of an empty chain graph", () => {
  expect(graphlessTradeKind(sale, graph)).toBe("sell");
  const html = renderToStaticMarkup(<TransactionRecordFlow transaction={sale} kind="sell" hideSensitive={false} />);
  expect(html).toContain("Sale");
  expect(html).toContain("0.00100000 BTC");
  expect(html).toContain("149,00");
  expect(html).toContain("Private exchange");
  expect(html).not.toContain("No graph");
});

it("never promotes a market valuation into trade proceeds", () => {
  const html = renderToStaticMarkup(<TransactionRecordFlow transaction={{ ...sale, pricingSourceKind: "fmv_provider" }} kind="sell" hideSensitive={false} />);
  expect(html).toContain("Not in the source record");
  expect(html).not.toContain("149");
});

it("keeps chain references and unknown transaction kinds on the graph path", () => {
  expect(graphlessTradeKind({ ...sale, explorerId: "a".repeat(64) }, graph)).toBeNull();
  expect(graphlessTradeKind(sale, { ...graph, supportLevel: "partial" })).toBeNull();
  expect(graphlessTradeKind({ ...sale, kindOverride: "withdrawal" }, graph)).toBeNull();
});

it("orders purchases fiat to crypto and preserves the asset", () => {
  const html = renderToStaticMarkup(<TransactionRecordFlow transaction={{ ...sale, kind: "buy", asset: "LBTC" }} kind="buy" hideSensitive={false} />);
  expect(html.indexOf("149,00")).toBeLessThan(html.indexOf("0.00100000 LBTC"));
});

it("hides source, trade values and fees in privacy mode", () => {
  const html = renderToStaticMarkup(<TransactionRecordFlow transaction={{ ...sale, feeBtc: 0.00001234 }} kind="sell" hideSensitive />);
  expect(html).toContain("Hidden");
  for (const sensitive of ["Private exchange", "149", "0.001", "1234"]) expect(html).not.toContain(sensitive);
});
