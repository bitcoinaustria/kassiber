import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";

/** The Bitcoin rail a leg moved on: the chain, Lightning or Liquid. */
export type SwapRail = "onchain" | "lightning" | "liquid";

export const RAIL_DETAILS: Record<
  SwapRail,
  {
    label: string;
    shortLabel: string;
    icon: string;
    className: string;
  }
> = {
  onchain: {
    label: "On-chain",
    shortLabel: "BTC",
    icon: bitcoinIcon,
    className:
      "border-orange-200 bg-orange-50 text-orange-800 dark:border-orange-400/30 dark:bg-orange-950/40 dark:text-orange-100",
  },
  lightning: {
    label: "Lightning",
    shortLabel: "LN",
    icon: lightningIcon,
    className:
      "border-violet-200 bg-violet-50 text-violet-800 dark:border-violet-400/30 dark:bg-violet-950/40 dark:text-violet-100",
  },
  liquid: {
    label: "Liquid",
    shortLabel: "Liquid",
    icon: liquidIcon,
    className:
      "border-cyan-200 bg-cyan-50 text-cyan-800 dark:border-cyan-400/30 dark:bg-cyan-950/40 dark:text-cyan-100",
  },
};

export function railForLeg(
  asset: string | null | undefined,
  walletKind: string | null | undefined,
): SwapRail {
  // Tolerate a daemon that predates the enriched pair payload (no wallet_kind):
  // fall through to the on-chain default instead of throwing on .toLowerCase().
  const assetKey = (asset ?? "").toUpperCase();
  const kindKey = (walletKind ?? "").toLowerCase();
  if (assetKey === "LBTC" || kindKey.includes("liquid")) return "liquid";
  if (
    kindKey.includes("phoenix") ||
    kindKey.includes("lightning") ||
    kindKey.includes("coreln") ||
    kindKey.includes("core-ln") ||
    kindKey === "lnd" ||
    kindKey === "nwc"
  ) {
    return "lightning";
  }
  return "onchain";
}

export function displayAssetLabel(asset: string) {
  const normalized = asset.toUpperCase();
  return normalized === "LBTC" || normalized === "L-BTC" ? "BTC" : normalized;
}
