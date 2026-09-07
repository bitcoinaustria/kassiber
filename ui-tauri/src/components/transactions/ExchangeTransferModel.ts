import { CONNECTION_SOURCES } from "@/lib/connectionCatalog";
import { classifyRouteKind, type TransactionSwapRoute } from "./TransactionGraphModel";

function custodyLocation(kind?: string | null) {
  const rawKind = kind?.trim().toLowerCase();
  const normalized = rawKind === "xpub" ? "descriptor" : rawKind;
  if (!normalized) return null;
  const categories = new Set(CONNECTION_SOURCES
    .filter(source => source.walletKind === normalized || source.id === normalized)
    .map(source => source.category));
  // Some importers serve both an exchange account and a wallet. Their kind
  // alone cannot distinguish custody, so retain the generic route for them.
  if (categories.has("wallets") && categories.has("exchanges")) return null;
  if (categories.has("exchanges")) return "exchange";
  if (categories.has("wallets")) return "wallet";
  return null;
}

/** Presentation of a known account/wallet boundary, not a new transfer claim. */
export function exchangeTransfer(route?: TransactionSwapRoute | null) {
  if (!route) return null;
  const kind = route.routeKind ?? classifyRouteKind({
    kind: route.kind, policy: route.policy, outAsset: route.out.asset, inAsset: route.in.asset,
  });
  if (kind !== "transfer" || !route.out.asset || route.out.asset !== route.in.asset) return null;
  const out = custodyLocation(route.out.wallet?.kind);
  const into = custodyLocation(route.in.wallet?.kind);
  if (out === "exchange" && into === "wallet") {
    return { direction: "withdrawal" as const, exchangeLeg: "out" as const, chainLeg: "in" as const };
  }
  if (out === "wallet" && into === "exchange") {
    return { direction: "deposit" as const, exchangeLeg: "in" as const, chainLeg: "out" as const };
  }
  return null;
}
