import * as React from "react";
import {
  Check,
  GripHorizontal,
  Plus,
  RotateCcw,
  SlidersHorizontal,
  X,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import {
  addWorkspaceWidget,
  createWorkspaceLayout,
  focusWorkspaceItem,
  moveWorkspaceItem,
  normalizeWorkspaceLayout,
  pageWorkspaceStorageKey,
  removeWorkspaceItem,
  resizeWorkspaceItem,
  workspaceLayoutHeight,
  WORKSPACE_LAYOUT_COLUMNS,
  WORKSPACE_LAYOUT_ROW_HEIGHT,
  type WorkspaceLayoutItem,
  type WorkspacePageLayout,
  type WorkspaceResizeEdge,
  type WorkspaceWidgetDefinition,
} from "@/lib/workspaceLayout";
import { useUiStore } from "@/store/ui";

export type PageWorkspaceWidget = WorkspaceWidgetDefinition & {
  render: () => React.ReactNode;
};

interface PageWorkspaceProps {
  pageId: string;
  title: string;
  widgets: PageWorkspaceWidget[];
  defaultItems: WorkspaceLayoutItem[];
  className?: string;
}

type WorkspacePointerOperation =
  | {
      type: "move";
      pointerId: number;
      itemId: string;
      startClientX: number;
      startClientY: number;
      startItem: WorkspaceLayoutItem;
      startLayout: WorkspacePageLayout;
      cellWidth: number;
      rowHeight: number;
    }
  | {
      type: "resize";
      pointerId: number;
      itemId: string;
      edge: WorkspaceResizeEdge;
      startClientX: number;
      startClientY: number;
      startLayout: WorkspacePageLayout;
      cellWidth: number;
      rowHeight: number;
    };

interface WorkspacePreview {
  item: WorkspaceLayoutItem;
  layout: WorkspacePageLayout;
}

export function PageWorkspace({
  pageId,
  title,
  widgets,
  defaultItems,
  className,
}: PageWorkspaceProps) {
  const identity = useUiStore((state) => state.identity);
  const layoutKey = React.useMemo(
    () => pageWorkspaceStorageKey(pageId, identity),
    [identity, pageId],
  );
  const storedLayout = useUiStore(
    (state) => state.pageWorkspaceLayouts[layoutKey],
  );
  const setPageWorkspaceLayout = useUiStore(
    (state) => state.setPageWorkspaceLayout,
  );
  const clearPageWorkspaceLayout = useUiStore(
    (state) => state.clearPageWorkspaceLayout,
  );
  const [editing, setEditing] = React.useState(false);
  const [preview, setPreview] = React.useState<WorkspacePreview | null>(null);
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  const operationRef = React.useRef<WorkspacePointerOperation | null>(null);
  const widgetById = React.useMemo(
    () => new Map(widgets.map((widget) => [widget.id, widget])),
    [widgets],
  );
  const defaultLayout = React.useMemo(
    () => createWorkspaceLayout(defaultItems),
    [defaultItems],
  );
  const layout = React.useMemo(
    () => normalizeWorkspaceLayout(storedLayout ?? defaultLayout, widgets),
    [defaultLayout, storedLayout, widgets],
  );
  const previewLayoutRef = React.useRef<WorkspacePageLayout | null>(null);
  const activeWidgetIds = new Set(layout.items.map((item) => item.widgetId));

  const persistLayout = React.useCallback(
    (next: WorkspacePageLayout) => {
      setPageWorkspaceLayout(layoutKey, normalizeWorkspaceLayout(next, widgets));
    },
    [layoutKey, setPageWorkspaceLayout, widgets],
  );

  const measureGrid = React.useCallback(() => {
    const rect = containerRef.current?.getBoundingClientRect();
    return {
      cellWidth: rect ? rect.width / WORKSPACE_LAYOUT_COLUMNS : 96,
      rowHeight: WORKSPACE_LAYOUT_ROW_HEIGHT,
    };
  }, []);

  const updatePreview = React.useCallback(
    (next: WorkspacePageLayout, itemId: string) => {
      const item = next.items.find((candidate) => candidate.id === itemId);
      if (!item) return;
      previewLayoutRef.current = next;
      setPreview({ item, layout: next });
    },
    [],
  );

  const startMove = (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
  ) => {
    if (!editing || event.button !== 0) return;
    const { cellWidth, rowHeight } = measureGrid();
    const startLayout = focusWorkspaceItem(layout, item.id);
    operationRef.current = {
      type: "move",
      pointerId: event.pointerId,
      itemId: item.id,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startItem: item,
      startLayout,
      cellWidth,
      rowHeight,
    };
    previewLayoutRef.current = startLayout;
    updatePreview(startLayout, item.id);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const startResize = (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
    edge: WorkspaceResizeEdge,
  ) => {
    if (!editing || event.button !== 0) return;
    event.stopPropagation();
    const { cellWidth, rowHeight } = measureGrid();
    const startLayout = focusWorkspaceItem(layout, item.id);
    operationRef.current = {
      type: "resize",
      pointerId: event.pointerId,
      itemId: item.id,
      edge,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startLayout,
      cellWidth,
      rowHeight,
    };
    previewLayoutRef.current = startLayout;
    updatePreview(startLayout, item.id);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const updatePointerOperation = (event: React.PointerEvent<HTMLElement>) => {
    const operation = operationRef.current;
    if (!operation || operation.pointerId !== event.pointerId) return;
    const deltaX = Math.round(
      (event.clientX - operation.startClientX) / operation.cellWidth,
    );
    const deltaY = Math.round(
      (event.clientY - operation.startClientY) / operation.rowHeight,
    );
    const next =
      operation.type === "move"
        ? moveWorkspaceItem(
            operation.startLayout,
            operation.itemId,
            operation.startItem.x + deltaX,
            operation.startItem.y + deltaY,
            widgets,
          )
        : resizeWorkspaceItem(
            operation.startLayout,
            operation.itemId,
            operation.edge,
            deltaX,
            deltaY,
            widgets,
          );
    updatePreview(next, operation.itemId);
  };

  const finishPointerOperation = (event: React.PointerEvent<HTMLElement>) => {
    const operation = operationRef.current;
    if (!operation || operation.pointerId !== event.pointerId) return;
    operationRef.current = null;
    if (previewLayoutRef.current) {
      persistLayout(previewLayoutRef.current);
    }
    previewLayoutRef.current = null;
    setPreview(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const addWidget = (widget: PageWorkspaceWidget) => {
    persistLayout(addWorkspaceWidget(layout, widget));
  };

  const removeWidget = (itemId: string) => {
    persistLayout(removeWorkspaceItem(layout, itemId));
  };

  const resetLayout = () => {
    clearPageWorkspaceLayout(layoutKey);
    setPreview(null);
    previewLayoutRef.current = null;
  };

  const rows = Math.max(
    workspaceLayoutHeight(layout),
    preview ? workspaceLayoutHeight(preview.layout) : 0,
  );
  const availableWidgets = widgets.filter((widget) => !activeWidgetIds.has(widget.id));

  return (
    <section className={cn("space-y-3", className)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="font-medium text-foreground">{title}</span>
          {editing && <span>Editing layout</span>}
        </div>
        <div className="flex items-center gap-2">
          {editing && (
            <>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline" size="sm">
                    <Plus className="size-4" aria-hidden="true" />
                    Add widget
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-72">
                  <DropdownMenuLabel>Widget palette</DropdownMenuLabel>
                  <DropdownMenuSeparator />
                  {availableWidgets.length ? (
                    availableWidgets.map((widget) => (
                      <DropdownMenuItem
                        key={widget.id}
                        className="flex-col items-start gap-0.5"
                        onSelect={() => addWidget(widget)}
                      >
                        <span className="font-medium">{widget.title}</span>
                        <span className="text-xs text-muted-foreground">
                          {widget.description}
                        </span>
                      </DropdownMenuItem>
                    ))
                  ) : (
                    <DropdownMenuItem disabled>
                      All widgets are on the page
                    </DropdownMenuItem>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
              <Button variant="outline" size="sm" onClick={resetLayout}>
                <RotateCcw className="size-4" aria-hidden="true" />
                Reset
              </Button>
            </>
          )}
          <Button
            variant={editing ? "default" : "outline"}
            size="sm"
            onClick={() => {
              setEditing((value) => !value);
              setPreview(null);
              previewLayoutRef.current = null;
              operationRef.current = null;
            }}
          >
            {editing ? (
              <Check className="size-4" aria-hidden="true" />
            ) : (
              <SlidersHorizontal className="size-4" aria-hidden="true" />
            )}
            {editing ? "Done" : "Edit layout"}
          </Button>
        </div>
      </div>
      <div
        ref={containerRef}
        className={cn(
          "relative",
          editing &&
            "kb-workspace-edit rounded-lg border border-dashed border-border/80 bg-[linear-gradient(to_right,var(--border)_1px,transparent_1px),linear-gradient(to_bottom,var(--border)_1px,transparent_1px)]",
        )}
        style={{
          height: rows * WORKSPACE_LAYOUT_ROW_HEIGHT,
          backgroundSize: editing
            ? `${100 / WORKSPACE_LAYOUT_COLUMNS}% ${WORKSPACE_LAYOUT_ROW_HEIGHT}px`
            : undefined,
        }}
        data-testid={`${pageId}-page-workspace`}
      >
        {layout.items.map((item) => {
          const widget = widgetById.get(item.widgetId);
          if (!widget) return null;
          return (
            <WorkspaceItemFrame
              key={item.id}
              editing={editing}
              item={item}
              title={widget.title}
              placeholder={widget.placeholder}
              onMoveStart={startMove}
              onPointerMove={updatePointerOperation}
              onPointerUp={finishPointerOperation}
              onResizeStart={startResize}
              onRemove={removeWidget}
            >
              {widget.render()}
            </WorkspaceItemFrame>
          );
        })}
        {editing && preview && <WorkspacePreviewFrame item={preview.item} />}
      </div>
    </section>
  );
}

function WorkspaceItemFrame({
  editing,
  item,
  title,
  placeholder,
  children,
  onMoveStart,
  onPointerMove,
  onPointerUp,
  onResizeStart,
  onRemove,
}: {
  editing: boolean;
  item: WorkspaceLayoutItem;
  title: string;
  placeholder?: boolean;
  children: React.ReactNode;
  onMoveStart: (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
  ) => void;
  onPointerMove: (event: React.PointerEvent<HTMLElement>) => void;
  onPointerUp: (event: React.PointerEvent<HTMLElement>) => void;
  onResizeStart: (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
    edge: WorkspaceResizeEdge,
  ) => void;
  onRemove: (itemId: string) => void;
}) {
  return (
    <div
      className={cn("absolute p-1", editing && "kb-workspace-item")}
      style={{
        left: `${(item.x / WORKSPACE_LAYOUT_COLUMNS) * 100}%`,
        top: item.y * WORKSPACE_LAYOUT_ROW_HEIGHT,
        width: `${(item.w / WORKSPACE_LAYOUT_COLUMNS) * 100}%`,
        height: item.h * WORKSPACE_LAYOUT_ROW_HEIGHT,
        zIndex: item.z,
      }}
    >
      <section
        className={cn(
          "relative h-full min-h-0",
          editing &&
            "rounded-lg ring-1 ring-border/70 ring-offset-1 ring-offset-background",
          placeholder && editing && "ring-dashed",
        )}
        aria-label={title}
      >
        <div className="h-full min-h-0 overflow-auto [&>*]:min-h-full">
          {children}
        </div>
        {editing && (
          <>
            <div
              className="absolute left-2 top-2 z-30 flex max-w-[calc(100%-3rem)] cursor-grab touch-none select-none items-center gap-1.5 rounded-md border bg-background/55 px-2 py-1 text-xs text-muted-foreground shadow-sm backdrop-blur active:cursor-grabbing"
              onPointerDown={(event) => onMoveStart(event, item)}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
            >
              <GripHorizontal className="size-3.5 shrink-0" aria-hidden="true" />
              <span className="truncate">{title}</span>
            </div>
            <Button
              variant="secondary"
              size="icon"
              className="absolute right-2 top-2 z-30 size-7 border bg-background/55 shadow-sm backdrop-blur"
              title={`Remove ${title}`}
              onPointerDown={(event) => event.stopPropagation()}
              onClick={() => onRemove(item.id)}
            >
              <X className="size-3.5" aria-hidden="true" />
              <span className="sr-only">Remove {title}</span>
            </Button>
            <ResizeHandle
              edge="n"
              title={title}
              item={item}
              onStart={onResizeStart}
              onMove={onPointerMove}
              onEnd={onPointerUp}
            />
            <ResizeHandle
              edge="e"
              title={title}
              item={item}
              onStart={onResizeStart}
              onMove={onPointerMove}
              onEnd={onPointerUp}
            />
            <ResizeHandle
              edge="s"
              title={title}
              item={item}
              onStart={onResizeStart}
              onMove={onPointerMove}
              onEnd={onPointerUp}
            />
            <ResizeHandle
              edge="w"
              title={title}
              item={item}
              onStart={onResizeStart}
              onMove={onPointerMove}
              onEnd={onPointerUp}
            />
            <ResizeHandle
              edge="se"
              title={title}
              item={item}
              onStart={onResizeStart}
              onMove={onPointerMove}
              onEnd={onPointerUp}
            />
          </>
        )}
      </section>
    </div>
  );
}

function ResizeHandle({
  edge,
  title,
  item,
  onStart,
  onMove,
  onEnd,
}: {
  edge: WorkspaceResizeEdge;
  title: string;
  item: WorkspaceLayoutItem;
  onStart: (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
    edge: WorkspaceResizeEdge,
  ) => void;
  onMove: (event: React.PointerEvent<HTMLElement>) => void;
  onEnd: (event: React.PointerEvent<HTMLElement>) => void;
}) {
  const classes: Record<WorkspaceResizeEdge, string> = {
    n: "left-4 right-4 top-0 h-2 cursor-ns-resize",
    e: "bottom-4 right-0 top-4 w-2 cursor-ew-resize",
    s: "bottom-0 left-4 right-4 h-2 cursor-ns-resize",
    w: "bottom-4 left-0 top-4 w-2 cursor-ew-resize",
    ne: "right-0 top-0 size-4 cursor-nesw-resize",
    nw: "left-0 top-0 size-4 cursor-nwse-resize",
    se: "bottom-0 right-0 size-5 cursor-nwse-resize",
    sw: "bottom-0 left-0 size-4 cursor-nesw-resize",
  };
  return (
    <button
      type="button"
      aria-label={`Resize ${title}`}
      className={cn(
        "absolute z-20 rounded-sm bg-background/5 transition-colors hover:bg-primary/20",
        classes[edge],
      )}
      onPointerDown={(event) => onStart(event, item, edge)}
      onPointerMove={onMove}
      onPointerUp={onEnd}
      onPointerCancel={onEnd}
    />
  );
}

function WorkspacePreviewFrame({ item }: { item: WorkspaceLayoutItem }) {
  return (
    <div
      className="pointer-events-none absolute z-[999] p-1"
      style={{
        left: `${(item.x / WORKSPACE_LAYOUT_COLUMNS) * 100}%`,
        top: item.y * WORKSPACE_LAYOUT_ROW_HEIGHT,
        width: `${(item.w / WORKSPACE_LAYOUT_COLUMNS) * 100}%`,
        height: item.h * WORKSPACE_LAYOUT_ROW_HEIGHT,
      }}
    >
      <div className="h-full rounded-lg border-2 border-primary/70 bg-primary/10 shadow-[0_0_0_1px_rgb(255_255_255_/_0.35)_inset]" />
    </div>
  );
}
