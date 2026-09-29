import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Glass3DView } from "@/components/kb/glass3d/Glass3DView";

import type { TransactionGraphPayload } from "../TransactionGraphModel";
import { ribbonLayout } from "./ribbonLayout";

/**
 * The transaction graph as glass ribbons, after the artwork lab's ribbon pieces.
 * Three.js loads only when this view opens; without WebGL the 2D graph shows.
 */
export function TransactionGraph3D({
  graph,
  hideSensitive,
  maxRows,
  size = "expanded",
  fallback,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  maxRows: number;
  /** Inline in the detail sheet, or the full expanded dialog. */
  size?: "compact" | "expanded";
  fallback: ReactNode;
}) {
  const { t } = useTranslation("transactions");
  const layout = useMemo(
    () => ribbonLayout(graph, hideSensitive, maxRows),
    [graph, hideSensitive, maxRows],
  );
  return (
    <Glass3DView
      scene={layout}
      load={(canvas, look) =>
        import("./glassScene").then(({ createGlassScene }) =>
          createGlassScene(canvas, layout, look),
        )
      }
      ariaLabel={t("graph.view3dAria")}
      loadingLabel={t("graph.view3dLoading")}
      className={size === "expanded" ? "h-[min(64vh,560px)]" : "h-[380px]"}
      testId="transaction-graph-3d"
      unavailable={
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground" role="status">
            {t("graph.view3dUnavailable")}
          </p>
          {fallback}
        </div>
      }
    >
      <p className="text-xs text-muted-foreground">{t("graph.view3dLegend")}</p>
    </Glass3DView>
  );
}
