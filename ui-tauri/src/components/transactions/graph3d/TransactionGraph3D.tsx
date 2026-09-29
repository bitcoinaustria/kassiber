import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Glass3DView } from "@/components/kb/glass3d/Glass3DView";

import { InfoHint } from "../TransactionDetailSheetParts";

import type { TransactionGraphPayload } from "../TransactionGraphModel";
import { ribbonLayout } from "./ribbonLayout";

const LEGEND = [
  { key: "known", swatch: "bg-[#8fb4ff] ring-1 ring-inset ring-white/40" },
  { key: "unknown", swatch: "bg-slate-300/80 ring-1 ring-inset ring-white/60 dark:bg-slate-400/60" },
  { key: "own", swatch: "bg-[#2563eb]" },
  { key: "other", swatch: "bg-[#7b8798] dark:bg-[#64748b]" },
  { key: "fee", swatch: "bg-[#f5b544]" },
] as const;

/** Swatches instead of a paragraph; the reading note sits behind the hint. */
function GraphLegend() {
  const { t } = useTranslation("transactions");
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      {LEGEND.map((item) => (
        <span key={item.key} className="inline-flex items-center gap-1.5">
          <span className={`inline-block size-2.5 rounded-sm ${item.swatch}`} aria-hidden="true" />
          {t(`graph.legend.${item.key}`)}
        </span>
      ))}
      <InfoHint label={t("graph.legend.readingLabel")}>{t("graph.view3dLegend")}</InfoHint>
    </div>
  );
}

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
      <GraphLegend />
    </Glass3DView>
  );
}
