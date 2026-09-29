import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { Glass3DView } from "@/components/kb/glass3d/Glass3DView";

import type { WalletUtxoRow } from "../UtxosInventoryPanel";
import { utxoBlocksLayout } from "./utxoBlocksLayout";

/** The wallet's main coin: bitcoin, else L-BTC, else whatever most coins hold. */
function primaryAsset(rows: readonly WalletUtxoRow[]) {
  const assets = rows.map((row) => row.asset);
  if (assets.includes("BTC")) return "BTC";
  const liquid = assets.find((asset) => asset === "LBTC" || asset === "L-BTC");
  if (liquid) return liquid;
  const counts = new Map<string, number>();
  for (const asset of assets) counts.set(asset, (counts.get(asset) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
}

/**
 * The wallet's unspent coins as glass blocks, one per coin, taller for larger
 * amounts, after the artwork lab's "a coin is a block". The table below stays
 * the exact view; without WebGL nothing replaces this one.
 */
export function UtxoBlocks3D({
  rows,
  hideSensitive,
}: {
  rows: readonly WalletUtxoRow[];
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("connections");
  const coins = useMemo(() => {
    const asset = primaryAsset(rows);
    return rows.filter((row) => row.asset === asset);
  }, [rows]);
  const layout = useMemo(
    () =>
      utxoBlocksLayout(
        coins.map((row) => ({
          outpoint: row.outpoint,
          confirmation_status: row.confirmation_status,
          asset: row.asset,
          amount_msat: row.amount_msat,
        })),
        hideSensitive,
      ),
    [coins, hideSensitive],
  );
  if (!coins.length) return null;
  return (
    <div className="border-b px-3 py-2" data-testid="utxo-blocks-3d">
      <Glass3DView
        scene={layout}
        load={(canvas, look) =>
          import("./utxoBlocksScene").then(({ createUtxoBlocksScene }) =>
            createUtxoBlocksScene(canvas, layout, look),
          )
        }
        ariaLabel={t("utxos.blocks3dAria", { count: coins.length })}
        loadingLabel={t("utxos.blocks3dLoading")}
        className="h-[220px]"
        unavailable={null}
      >
        <p className="text-xs text-muted-foreground">
          {hideSensitive ? t("utxos.blocks3dLegendHidden") : t("utxos.blocks3dLegend")}
        </p>
      </Glass3DView>
    </div>
  );
}
