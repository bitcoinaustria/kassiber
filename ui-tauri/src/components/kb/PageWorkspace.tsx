import * as React from "react";
import {
  GripHorizontal,
  Plus,
  RotateCcw,
  SquareDashedMousePointer,
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

  const startMove = (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
  ) => {
    if (event.button !== 0) return;
    const { cellWidth, rowHeight } = measureGrid();
    operationRef.current = {
      type: "move",
      pointerId: event.pointerId,
      itemId: item.id,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startItem: item,
      startLayout: focusWorkspaceItem(layout, item.id),
      cellWidth,
      rowHeight,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    persistLayout(focusWorkspaceItem(layout, item.id));
  };

  const startResize = (
    event: React.PointerEvent<HTMLElement>,
    item: WorkspaceLayoutItem,
    edge: WorkspaceResizeEdge,
  ) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    const { cellWidth, rowHeight } = measureGrid();
    operationRef.current = {
      type: "resize",
      pointerId: event.pointerId,
      itemId: item.id,
      edge,
      startClientX: event.clientX,
      startClientY: event.clientY,
      startLayout: focusWorkspaceItem(layout, item.id),
      cellWidth,
      rowHeight,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    persistLayout(focusWorkspaceItem(layout, item.id));
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
    if (operation.type === "move") {
      persistLayout(
        moveWorkspaceItem(
          operation.startLayout,
          operation.itemId,
          operation.startItem.x + deltaX,
          operation.startItem.y + deltaY,
          widgets,
        ),
      );
      return;
    }
    persistLayout(
      resizeWorkspaceItem(
        operation.startLayout,
        operation.itemId,
        operation.edge,
        deltaX,
        deltaY,
        widgets,
      ),
    );
  };

  const finishPointerOperation = (event: React.PointerEvent<HTMLElement>) => {
    const operation = operationRef.current;
    if (!operation || operation.pointerId !== event.pointerId) return;
    operationRef.current = null;
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
  };

  const rows = workspaceLayoutHeight(layout);
  const availableWidgets = widgets.filter((widget) => !activeWidgetIds.has(widget.id));

  return (
    <section className={cn("space-y-3", className)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <SquareDashedMousePointer className="size-4" aria-hidden="true" />
          <span className="font-medium text-foreground">{title}</span>
        </div>
        <div className="flex items-center gap-2">
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
                <DropdownMenuItem disabled>All widgets are on the page</DropdownMenuItem>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
          <Button variant="outline" size="sm" onClick={resetLayout}>
            <RotateCcw className="size-4" aria-hidden="true" />
            Reset
          </Button>
        </div>
      </div>
      <div
        ref={containerRef}
        className="relative rounded-lg border border-dashed border-border/80 bg-[linear-gradient(to_right,var(--border)_1px,transparent_1px),linear-gradient(to_bottom,var(--border)_1px,transparent_1px)] bg-[length:8.333%_96px]"
        style={{ height: rows * WORKSPACE_LAYOUT_ROW_HEIGHT }}
        data-testid={`${pageId}-page-workspace`}
      >
        {layout.items.map((item) => {
          const widget = widgetById.get(item.widgetId);
          if (!widget) return null;
          return (
            <WorkspaceItemFrame
              key={item.id}
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
      </div>
    </section>
  );
}

function WorkspaceItemFrame({
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
  const edgeClassName =
    "absolute z-20 bg-transparent transition-colors hover:bg-primary/10";
  return (
    <div
      className="absolute p-1"
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
          "group relative flex h-full min-h-0 flex-col overflow-hidden rounded-lg border bg-background shadow-sm transition-shadow focus-within:ring-2 focus-within:ring-ring hover:shadow-md",
          placeholder && "border-dashed bg-muted/30",
        )}
        aria-label={title}
      >
        <div
          className="flex h-9 shrink-0 cursor-grab touch-none select-none items-center justify-between gap-2 border-b bg-muted/45 px-2 text-xs text-muted-foreground active:cursor-grabbing"
          onPointerDown={(event) => onMoveStart(event, item)}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        >
          <span className="flex min-w-0 items-center gap-1.5">
            <GripHorizontal className="size-4 shrink-0" aria-hidden="true" />
            <span className="truncate font-medium text-foreground">{title}</span>
          </span>
          <Button
            variant="ghost"
            size="icon"
            className="size-7"
            title={`Remove ${title}`}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={() => onRemove(item.id)}
          >
            <X className="size-3.5" aria-hidden="true" />
            <span className="sr-only">Remove {title}</span>
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-auto p-1 [&>*]:min-h-full">
          {children}
        </div>
        <button
          type="button"
          aria-label={`Resize ${title} from top`}
          className={cn(edgeClassName, "left-4 right-4 top-0 h-1 cursor-ns-resize")}
          onPointerDown={(event) => onResizeStart(event, item, "n")}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        <button
          type="button"
          aria-label={`Resize ${title} from right`}
          className={cn(edgeClassName, "bottom-4 right-0 top-4 w-1 cursor-ew-resize")}
          onPointerDown={(event) => onResizeStart(event, item, "e")}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        <button
          type="button"
          aria-label={`Resize ${title} from bottom`}
          className={cn(edgeClassName, "bottom-0 left-4 right-4 h-1 cursor-ns-resize")}
          onPointerDown={(event) => onResizeStart(event, item, "s")}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        <button
          type="button"
          aria-label={`Resize ${title} from left`}
          className={cn(edgeClassName, "bottom-4 left-0 top-4 w-1 cursor-ew-resize")}
          onPointerDown={(event) => onResizeStart(event, item, "w")}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
        <button
          type="button"
          aria-label={`Resize ${title} from corner`}
          className="absolute bottom-1 right-1 z-30 size-3 cursor-nwse-resize rounded-sm border border-border bg-background shadow-sm"
          onPointerDown={(event) => onResizeStart(event, item, "se")}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
      </section>
    </div>
  );
}
