import {
  memo,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { Crosshair, Focus, Minus, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  formatAnalysisAmount,
  layoutAnalysisGraph,
  panAnalysisCamera,
  zoomAnalysisCamera,
  shortAnalysisId,
  type AnalysisEdge,
  type AnalysisNode,
} from "@/lib/chainAnalysis";
import {
  fitGraphCamera,
  focusGraphCamera,
  graphNodeInView,
  graphOverview,
  graphSelectionInView,
  graphSelectionPosition,
  initialGraphCamera,
  resizeGraphCamera,
  type GraphViewport,
} from "./graphViewport";

export interface GraphSelection {
  kind: "node" | "edge";
  id: string;
}

function edgeColor(kind: string): string {
  if (kind === "conflicting") return "var(--destructive)";
  if (kind === "stale") return "var(--ca-muted)";
  if (kind === "hypothesis") return "var(--ca-hypothesis)";
  if (kind === "custody") return "var(--ca-custody)";
  return "var(--ca-physical)";
}

const GraphMarks = memo(function GraphMarks({
  nodes,
  edges,
  positions,
  selected,
  highlighted,
  marker,
  onSelect,
}: {
  nodes: AnalysisNode[];
  edges: AnalysisEdge[];
  positions: Map<string, { x: number; y: number }>;
  selected: GraphSelection | null;
  highlighted: Set<string>;
  marker: string;
  onSelect: (selection: GraphSelection) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  return (
    <>
      {edges.map((edge) => {
        const from = positions.get(edge.source),
          to = positions.get(edge.target);
        if (!from || !to) return null;
        const curve = `M${from.x + 96},${from.y} C${from.x + 165},${from.y} ${to.x - 165},${to.y} ${to.x - 100},${to.y}`;
        const active =
          selected?.id === edge.id ||
          (highlighted.has(edge.source) && highlighted.has(edge.target));
        const impaired =
          edge.status === "stale" || edge.status === "conflicting";
        const appearance = impaired ? edge.status! : edge.kind;
        const status = edge.status
          ? t(`observationStatus.${edge.status}`, { defaultValue: edge.status })
          : "";
        const label = [edge.label || edge.kind, status]
          .filter(Boolean)
          .join(" · ");
        return (
          <g
            key={edge.id}
            className="ca-edge"
            data-edge={edge.id}
            data-status={edge.status}
            role="button"
            tabIndex={0}
            aria-label={label}
            onClick={(event) => {
              event.stopPropagation();
              onSelect({ kind: "edge", id: edge.id });
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                event.stopPropagation();
                onSelect({ kind: "edge", id: edge.id });
              }
            }}
          >
            <title>{`${label} · ${edge.evidence_level}`}</title>
            <path d={curve} fill="none" stroke="transparent" strokeWidth={15} />
            <path
              d={curve}
              fill="none"
              stroke={edgeColor(appearance)}
              strokeWidth={active ? 3 : 1.5}
              opacity={active ? 1 : impaired ? 0.7 : 0.5}
              strokeDasharray={
                impaired
                  ? "2 5"
                  : edge.kind === "hypothesis"
                    ? "5 5"
                    : edge.kind === "custody"
                      ? "9 3"
                      : undefined
              }
              markerEnd={`url(#${marker}-${appearance})`}
            />
          </g>
        );
      })}
      {nodes.map((node) => {
        const position = positions.get(node.id);
        if (!position) return null;
        const active = selected?.id === node.id;
        const isHighlighted = highlighted.has(node.id);
        return (
          <g
            key={node.id}
            data-node={node.id}
            data-status={node.status}
            transform={`translate(${position.x}, ${position.y})`}
            role="button"
            tabIndex={0}
            aria-label={`${node.label || node.kind}: ${node.outpoint || node.txid || node.id}`}
            aria-pressed={active}
            className={`ca-node ${active ? "is-selected" : ""} ${isHighlighted ? "is-highlighted" : ""}`}
            onClick={(event) => {
              event.stopPropagation();
              onSelect({ kind: "node", id: node.id });
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                event.stopPropagation();
                onSelect({ kind: "node", id: node.id });
              }
            }}
          >
            <title>{`${node.label}\n${node.outpoint || node.txid || node.id}\n${formatAnalysisAmount(node.amount_msat, node.asset)}`}</title>
            <rect
              x={-99}
              y={-29}
              width={198}
              height={58}
              rx={node.kind === "output" ? 26 : 8}
              className="ca-node-body"
            />
            <circle
              cx={-81}
              cy={-7}
              r={3}
              fill={
                node.wallet_ids?.length ? "var(--ca-owned)" : "var(--ca-muted)"
              }
            />
            <text x={-71} y={-3} className="ca-node-label">
              {shortAnalysisId(
                node.label || node.outpoint || node.txid || node.id,
                26,
              )}
            </text>
            <text x={-71} y={15} className="ca-node-detail">
              {node.kind === "output"
                ? formatAnalysisAmount(node.amount_msat, node.asset)
                : `${node.chain} · ${shortAnalysisId(node.txid || node.id, 18)}`}
            </text>
          </g>
        );
      })}
    </>
  );
});

export function InvestigationGraph({
  nodes,
  edges,
  selected,
  highlightedIds,
  onSelect,
}: {
  nodes: AnalysisNode[];
  edges: AnalysisEdge[];
  selected: GraphSelection | null;
  highlightedIds: string[];
  onSelect: (selection: GraphSelection) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const container = useRef<HTMLDivElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const marker = useId().replace(/:/g, "");
  const [viewport, setViewport] = useState({ width: 1000, height: 540 });
  const drag = useRef<{
    pointer: number;
    x: number;
    y: number;
    cameraX: number;
    cameraY: number;
  } | null>(null);
  const layout = useMemo(
    () => layoutAnalysisGraph(nodes, edges),
    [nodes, edges],
  );
  const [camera, setCamera] = useState(() =>
    initialGraphCamera(layout, viewport, nodes, edges, selected),
  );
  const previousView = useRef<{
    layout: typeof layout;
    viewport: GraphViewport;
    selection: string;
  } | null>(null);
  const viewTouched = useRef(false);
  const overview = useMemo(
    () => graphOverview(layout, camera, viewport),
    [layout, camera, viewport],
  );
  const visibleCount = useMemo(
    () => [...layout.positions.values()].filter((position) =>
      graphNodeInView(position, camera, viewport),
    ).length,
    [layout, camera, viewport],
  );
  const highlighted = useMemo(() => new Set(highlightedIds), [highlightedIds]);
  const select = useCallback(
    (selection: GraphSelection) => onSelect(selection),
    [onSelect],
  );
  const fit = useCallback(() => {
    viewTouched.current = true;
    setCamera(fitGraphCamera(layout, viewport));
  }, [layout, viewport]);
  const focus = useCallback(() => {
    viewTouched.current = true;
    setCamera(initialGraphCamera(layout, viewport, nodes, edges, selected));
  }, [layout, viewport, nodes, edges, selected]);
  useEffect(() => {
    if (!container.current || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width > 0 && entry.contentRect.height > 0)
        setViewport({
          width: entry.contentRect.width,
          height: entry.contentRect.height,
        });
    });
    observer.observe(container.current);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    const previous = previousView.current;
    const selection = selected ? `${selected.kind}:${selected.id}` : "";
    previousView.current = { layout, viewport, selection };
    if (!previous || previous.layout !== layout) {
      viewTouched.current = false;
      setCamera(initialGraphCamera(layout, viewport, nodes, edges, selected));
      return;
    }
    if (previous.selection !== selection && selected) viewTouched.current = true;
    if (!viewTouched.current) {
      setCamera(initialGraphCamera(layout, viewport, nodes, edges, selected));
      return;
    }
    setCamera((current) => {
      const resized = resizeGraphCamera(current, previous.viewport, viewport);
      const position = graphSelectionPosition(layout, edges, selected);
      // A selection from findings or the inspector must be visible. Clicking an
      // already visible node leaves the user's position unchanged.
      if (
        previous.selection !== selection && position &&
        !graphSelectionInView(position, resized, viewport)
      ) {
        return focusGraphCamera(position, viewport);
      }
      return resized;
    });
  }, [layout, viewport, nodes, edges, selected]);
  const zoom = useCallback(
    (
      factor: number,
      point = { x: viewport.width / 2, y: viewport.height / 2 },
    ) => {
      viewTouched.current = true;
      setCamera((previous) => zoomAnalysisCamera(previous, factor, point));
    },
    [viewport.width, viewport.height],
  );
  useEffect(() => {
    const element = svg.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = element.getBoundingClientRect();
      zoom(Math.exp(-Math.max(-200, Math.min(200, event.deltaY)) * 0.003), {
        x: event.clientX - rect.left,
        y: event.clientY - rect.top,
      });
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [zoom]);
  return (
    <div className="ca-graph-container" ref={container}>
      <svg
        ref={svg}
        className="ca-graph"
        aria-label={t("graph.label")}
        aria-describedby={`${marker}-help`}
        tabIndex={0}
        role="group"
        viewBox={`0 0 ${viewport.width} ${viewport.height}`}
        onPointerDown={(event) => {
          if (
            event.button !== 0 ||
            drag.current ||
            (event.target as Element).closest("[data-node], .ca-edge")
          )
            return;
          viewTouched.current = true;
          drag.current = {
            pointer: event.pointerId,
            x: event.clientX,
            y: event.clientY,
            cameraX: camera.x,
            cameraY: camera.y,
          };
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }}
        onPointerMove={(event) => {
          const origin = drag.current;
          if (origin?.pointer === event.pointerId)
            setCamera((previous) => ({
              ...previous,
              x: origin.cameraX + event.clientX - origin.x,
              y: origin.cameraY + event.clientY - origin.y,
            }));
        }}
        onPointerUp={() => {
          drag.current = null;
        }}
        onPointerCancel={() => {
          drag.current = null;
        }}
        onKeyDown={(event) => {
          if (event.target !== event.currentTarget) return;
          if (event.key === "+" || event.key === "=") {
            event.preventDefault();
            zoom(1.3);
          } else if (event.key === "-") {
            event.preventDefault();
            zoom(1 / 1.3);
          } else if (event.key === "0" || event.key === "Home") {
            event.preventDefault();
            fit();
          } else if (event.key.toLowerCase() === "f") {
            event.preventDefault();
            focus();
          } else if (
            ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
              event.key,
            )
          ) {
            event.preventDefault();
            viewTouched.current = true;
            setCamera((previous) =>
              panAnalysisCamera(previous, {
                x:
                  event.key === "ArrowLeft"
                    ? 60
                    : event.key === "ArrowRight"
                      ? -60
                      : 0,
                y:
                  event.key === "ArrowUp"
                    ? 60
                    : event.key === "ArrowDown"
                      ? -60
                      : 0,
              }),
            );
          }
        }}
      >
        <defs>
          <pattern
            id={`${marker}-grid`}
            width={24}
            height={24}
            patternUnits="userSpaceOnUse"
          >
            <circle cx={1} cy={1} r={0.7} fill="var(--ca-grid)" />
          </pattern>
          {(
            [
              "creates",
              "spends",
              "custody",
              "hypothesis",
              "stale",
              "conflicting",
            ] as const
          ).map((kind) => (
            <marker
              key={kind}
              id={`${marker}-${kind}`}
              viewBox="0 0 10 10"
              refX={8}
              refY={5}
              markerWidth={5}
              markerHeight={5}
              orient="auto-start-reverse"
            >
              <path d="M 0 1 L 9 5 L 0 9 z" fill={edgeColor(kind)} />
            </marker>
          ))}
        </defs>
        <rect width="100%" height="100%" fill={`url(#${marker}-grid)`} />
        <g
          transform={`translate(${camera.x} ${camera.y}) scale(${camera.scale})`}
          data-testid="graph-camera"
        >
          <GraphMarks
            nodes={nodes}
            edges={edges}
            positions={layout.positions}
            selected={selected}
            highlighted={highlighted}
            marker={marker}
            onSelect={select}
          />
        </g>
      </svg>
      <div className="ca-graph-controls">
        <Button
          variant="outline"
          size="icon"
          aria-label={t("graph.zoomIn")}
          onClick={() => zoom(1.3)}
        >
          <Plus className="size-4" />
        </Button>
        <Button
          variant="outline"
          size="icon"
          aria-label={t("graph.zoomOut")}
          onClick={() => zoom(1 / 1.3)}
        >
          <Minus className="size-4" />
        </Button>
        <Button
          variant="outline"
          size="icon"
          aria-label={t("graph.fit")}
          title={t("graph.fit")}
          className="ca-fit-button"
          onClick={fit}
        >
          <Focus className="size-4" />
          <span>{t("graph.fit")}</span>
        </Button>
        <Button
          variant="outline"
          size="icon"
          aria-label={
            selected ? t("graph.focusSelection") : t("graph.readableView")
          }
          title={selected ? t("graph.focusSelection") : t("graph.readableView")}
          onClick={focus}
        >
          <Crosshair className="size-4" />
        </Button>
        <span className="min-w-10 text-center font-mono text-xs">
          {Math.round(camera.scale * 100)}%
        </span>
      </div>
      <div className="ca-graph-overview">
        <svg
          viewBox="0 0 164 60"
          role="button"
          tabIndex={0}
          aria-label={t("graph.overview")}
          onClick={(event) => {
            viewTouched.current = true;
            const rect = event.currentTarget.getBoundingClientRect();
            const position = {
              x: Math.max(0, Math.min(
                layout.width,
                ((event.clientX - rect.left) * 164 / rect.width - overview.x) / overview.scale,
              )),
              y: Math.max(0, Math.min(
                layout.height,
                ((event.clientY - rect.top) * 60 / rect.height - overview.y) / overview.scale,
              )),
            };
            setCamera(focusGraphCamera(position, viewport));
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              fit();
            }
          }}
        >
          <title>{t("graph.overview")}</title>
          {[...layout.positions].map(([id, position]) => (
            <circle
              key={id}
              cx={overview.x + position.x * overview.scale}
              cy={overview.y + position.y * overview.scale}
              r={selected?.id === id ? 2.5 : 1.5}
              fill={selected?.id === id ? "var(--ca-owned)" : "var(--ca-physical)"}
            />
          ))}
          <rect
            {...overview.viewport}
            fill="var(--ca-physical)"
            fillOpacity={0.12}
            stroke="var(--ca-physical)"
            strokeWidth={1}
            rx={1}
            pointerEvents="none"
          />
        </svg>
        <span>
          {t("graph.inView", { visible: visibleCount, total: nodes.length })}
        </span>
      </div>
      <p id={`${marker}-help`} className="ca-graph-help">
        {t("graph.help")}
      </p>
    </div>
  );
}
