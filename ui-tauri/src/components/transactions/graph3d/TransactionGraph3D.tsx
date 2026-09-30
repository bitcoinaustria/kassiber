import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Glass3DView } from "@/components/kb/glass3d/Glass3DView";

import { InfoHint } from "../TransactionDetailSheetParts";

import type { GraphRow, TransactionGraphPayload } from "../TransactionGraphModel";
import { legendKeys, ribbonLayout, type LegendKey, type RibbonLayout } from "./ribbonLayout";

const LEGEND: ReadonlyArray<{ key: LegendKey; swatch: string }> = [
  { key: "known", swatch: "bg-[#8fb4ff] ring-1 ring-inset ring-white/40" },
  { key: "unknown", swatch: "bg-slate-300/80 ring-1 ring-inset ring-white/60 dark:bg-slate-400/60" },
  { key: "own", swatch: "bg-[#2563eb]" },
  { key: "other", swatch: "bg-[#7b8798] dark:bg-[#64748b]" },
  { key: "fee", swatch: "bg-[#f5b544]" },
];

/** Swatches instead of a paragraph; the reading note sits behind the hint. */
function GraphLegend({ layout }: { layout: RibbonLayout }) {
  const { t } = useTranslation("transactions");
  const shown = legendKeys(layout);
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
      {LEGEND.filter((item) => shown.has(item.key)).map((item) => (
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
 * Pointing at a leg lights it and shows `renderLeg`'s card for it; parts are
 * named `input:<id>` and `output:<id>`, the fee among the outputs.
 */
export function TransactionGraph3D({
  graph,
  hideSensitive,
  maxRows,
  size = "expanded",
  fallback,
  activePart = null,
  onHoverPart,
  onSelectPart,
  renderLeg,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  maxRows: number;
  /** Inline in the detail sheet, or the full expanded dialog. */
  size?: "compact" | "expanded";
  fallback: ReactNode;
  /** A leg lit from outside the drawing, e.g. its row in the list below. */
  activePart?: string | null;
  onHoverPart?: (part: string | null) => void;
  onSelectPart?: (part: string) => void;
  renderLeg?: (row: GraphRow) => ReactNode;
}) {
  const { t } = useTranslation("transactions");
  // Keyed on the parts the drawing reads, not the payload wrapper: a copied
  // or refetched payload with the same legs must not rebuild the WebGL scene.
  const { inputs, outputs, fee } = graph;
  const chain = graph.transaction?.chain;
  const layout = useMemo(
    () =>
      ribbonLayout({ inputs, outputs, fee, transaction: { chain } }, hideSensitive, maxRows),
    [inputs, outputs, fee, chain, hideSensitive, maxRows],
  );
  const rows = useMemo(
    () => new Map(layout.ribbons.map((ribbon) => [ribbon.legId, ribbon.row])),
    [layout],
  );
  const [hovered, setHovered] = useState<string | null>(null);
  const hoveredRow = hovered ? rows.get(hovered) : undefined;
  return (
    <Glass3DView
      scene={layout}
      highlightedPart={hovered ?? activePart}
      onHoverPart={(part) => {
        setHovered(part);
        onHoverPart?.(part);
      }}
      onSelectPart={onSelectPart}
      overlay={hoveredRow && renderLeg ? renderLeg(hoveredRow) : null}
      load={(canvas, look) =>
        import("./glassScene").then(({ createGlassScene }) =>
          createGlassScene(canvas, layout, look),
        )
      }
      ariaLabel={t("graph.view3dAria")}
      loadingLabel={t("graph.view3dLoading")}
      className={size === "expanded" ? "h-[min(64vh,560px)]" : "h-[380px] xl:h-[440px]"}
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
      <GraphLegend layout={layout} />
    </Glass3DView>
  );
}
