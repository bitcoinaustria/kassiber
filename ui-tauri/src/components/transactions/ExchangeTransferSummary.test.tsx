import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { ExchangeTransferSummary } from "./ExchangeTransferSummary";
import { exchangeTransfer } from "./ExchangeTransferModel";
import { preloadableSwapLegGraphLookupArgs } from "./TransactionGraphLookup";
import { TransactionGraphPanel } from "./TransactionGraphTab";
import type { TransactionSwapRoute } from "./TransactionGraphModel";

const route: TransactionSwapRoute = {
  id: "reviewed-withdrawal", routeKind: "transfer", policy: "carrying-value", currentLeg: "out",
  out: { id: "exchange-row", asset: "BTC", amountBtc: 0.006, feeBtc: 0.0001, externalId: "withdrawal-reference", occurredAt: "2025-11-09T17:48:17Z", txid: "a".repeat(64),
    wallet: { kind: "strike", label: "Strike savings" } },
  in: { id: "wallet-row", asset: "BTC", amountBtc: 0.006, txid: "a".repeat(64),
    wallet: { kind: "address", label: "Exchange self custody" } },
};

it("distinguishes account withdrawal from a chain-to-chain transfer", () => {
  expect(exchangeTransfer(route)).toEqual({ direction: "withdrawal", exchangeLeg: "out", chainLeg: "in" });
  const html = renderToStaticMarkup(<TransactionGraphPanel graph={{
    transaction: { id: "wallet-row" }, supportLevel: "graphless", inputs: [], outputs: [], swapRoute: route,
  }} hideSensitive={false} onOpenTransaction={() => {}} />);
  for (const label of ["Exchange withdrawal", "Exchange account", "On-chain wallet", "Strike savings", "Exchange self custody", "Reported exchange fee", "Exchange reference", "withdrawal-reference"]) expect(html).toContain(label);
  expect(html).not.toContain("<details");
  expect(html).not.toContain("swap-route-strip");
  expect(html).not.toContain("Carrying value");
});

it("reverses the account boundary for an exchange deposit", () => {
  const deposit = { ...route, out: route.in, in: route.out };
  expect(exchangeTransfer(deposit)).toEqual({ direction: "deposit", exchangeLeg: "in", chainLeg: "out" });
  const html = renderToStaticMarkup(<ExchangeTransferSummary route={deposit} hideSensitive={false} />);
  expect(html).toContain("Exchange deposit");
  expect(html.indexOf("Exchange self custody")).toBeLessThan(html.indexOf("Strike savings"));
});

it("does not infer an exchange from a wallet name or reshape a swap", () => {
  expect(exchangeTransfer({ ...route, out: { ...route.out, wallet: { kind: "descriptor", label: "Strike exchange" } } })).toBeNull();
  for (const kind of ["phoenix", "lnd", "coreln"]) {
    expect(exchangeTransfer({ ...route, in: { ...route.in, wallet: { kind } } })).toBeNull();
  }
  expect(exchangeTransfer({ ...route, routeKind: "swap" })).toBeNull();
  expect(exchangeTransfer({ ...route, in: { ...route.in, asset: "LBTC" } })).toBeNull();
  expect(exchangeTransfer({ ...route, out: { ...route.out, wallet: { kind: "bullbitcoin" } } })).toBeNull();
});

it("loads only the on-chain graph and retains explicit lookup consent", () => {
  expect(preloadableSwapLegGraphLookupArgs(route, "out", [], true)).toEqual({ transaction: "", allowPublicLookup: false });
  expect(preloadableSwapLegGraphLookupArgs(route, "in", [])).toEqual({ transaction: "wallet-row", allowPublicLookup: false });
  expect(preloadableSwapLegGraphLookupArgs(route, "in", [], true)).toEqual({ transaction: "wallet-row", allowPublicLookup: true });
});

it("masks wallet names, transaction identity, quantities and fees", () => {
  const html = renderToStaticMarkup(<ExchangeTransferSummary route={route} hideSensitive />);
  expect(html).toContain("Hidden");
  for (const secret of ["Strike savings", "Exchange self custody", "aaaa", "0.006", "0.0001", "withdrawal-reference", "2025-11-09"]) expect(html).not.toContain(secret);
});

it("omits duplicate blockchain references and an empty exchange disclosure", () => {
  const html = renderToStaticMarkup(<ExchangeTransferSummary route={{ ...route,
    out: { ...route.out, externalId: "a".repeat(64) },
  }} hideSensitive={false} />);
  expect(html).not.toContain("Exchange reference");
  expect(html).not.toContain("<details");
  expect(html).not.toContain("Recorded at");
});
