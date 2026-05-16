export const WORKSPACE_LAYOUT_VERSION = 1;
export const WORKSPACE_LAYOUT_COLUMNS = 12;
export const WORKSPACE_LAYOUT_ROW_HEIGHT = 72;

export type WorkspaceResizeEdge =
  | "n"
  | "e"
  | "s"
  | "w"
  | "ne"
  | "nw"
  | "se"
  | "sw";

export interface WorkspaceLayoutItem {
  id: string;
  widgetId: string;
  x: number;
  y: number;
  w: number;
  h: number;
  z: number;
}

export interface WorkspacePageLayout {
  version: typeof WORKSPACE_LAYOUT_VERSION;
  columns: number;
  rowHeight: number;
  items: WorkspaceLayoutItem[];
}

export interface WorkspaceWidgetDefinition {
  id: string;
  title: string;
  description: string;
  minW: number;
  minH: number;
  defaultW: number;
  defaultH: number;
  optional?: boolean;
  placeholder?: boolean;
}

export interface WorkspaceIdentityLike {
  name?: string;
  profile?: string;
  workspace?: string;
  taxCountry?: string;
  fiatCurrency?: string;
  importedProject?: {
    dataRoot?: string;
    database?: string;
    stateRoot?: string;
  };
}

export function pageWorkspaceStorageKey(
  pageId: string,
  identity: WorkspaceIdentityLike | null | undefined,
) {
  const bookKey =
    identity?.importedProject?.database ||
    identity?.importedProject?.dataRoot ||
    [
      identity?.workspace || "local-books",
      identity?.profile || identity?.name || "default-book",
      identity?.taxCountry || "generic",
      identity?.fiatCurrency || "EUR",
    ].join(":");

  return `${bookKey}::${pageId}`;
}

export function createWorkspaceLayout(
  items: WorkspaceLayoutItem[],
): WorkspacePageLayout {
  return normalizeWorkspaceLayout({
    version: WORKSPACE_LAYOUT_VERSION,
    columns: WORKSPACE_LAYOUT_COLUMNS,
    rowHeight: WORKSPACE_LAYOUT_ROW_HEIGHT,
    items,
  });
}

export function normalizeWorkspaceLayout(
  layout: WorkspacePageLayout,
  widgets: WorkspaceWidgetDefinition[] = [],
) {
  const widgetById = new Map(widgets.map((widget) => [widget.id, widget]));
  const seen = new Set<string>();
  const items = layout.items
    .filter((item) => {
      if (seen.has(item.id)) return false;
      seen.add(item.id);
      return widgets.length === 0 || widgetById.has(item.widgetId);
    })
    .map((item) =>
      clampWorkspaceItem(item, widgetById.get(item.widgetId), {
        columns: layout.columns || WORKSPACE_LAYOUT_COLUMNS,
      }),
    );

  return resolveWorkspaceCollisions({
    version: WORKSPACE_LAYOUT_VERSION,
    columns: layout.columns || WORKSPACE_LAYOUT_COLUMNS,
    rowHeight: WORKSPACE_LAYOUT_ROW_HEIGHT,
    items,
  });
}

export function clampWorkspaceItem(
  item: WorkspaceLayoutItem,
  widget?: WorkspaceWidgetDefinition,
  options: { columns?: number } = {},
): WorkspaceLayoutItem {
  const columns = options.columns ?? WORKSPACE_LAYOUT_COLUMNS;
  const minW = widget?.minW ?? 2;
  const minH = widget?.minH ?? 2;
  const w = Math.min(columns, Math.max(minW, Math.round(item.w)));
  const h = Math.max(minH, Math.round(item.h));
  return {
    ...item,
    x: Math.min(columns - w, Math.max(0, Math.round(item.x))),
    y: Math.max(0, Math.round(item.y)),
    w,
    h,
    z: Math.max(1, Math.round(item.z || 1)),
  };
}

export function workspaceItemsOverlap(
  a: Pick<WorkspaceLayoutItem, "x" | "y" | "w" | "h">,
  b: Pick<WorkspaceLayoutItem, "x" | "y" | "w" | "h">,
) {
  return (
    a.x < b.x + b.w &&
    a.x + a.w > b.x &&
    a.y < b.y + b.h &&
    a.y + a.h > b.y
  );
}

export function resolveWorkspaceCollisions(
  layout: WorkspacePageLayout,
  activeItemId?: string,
) {
  const items = layout.items.map((item) => ({ ...item }));
  const maxPasses = Math.max(4, items.length * items.length);

  for (let pass = 0; pass < maxPasses; pass += 1) {
    let changed = false;
    const ordered = [...items].sort(
      (a, b) => a.y - b.y || a.x - b.x || b.z - a.z,
    );

    for (let index = 0; index < ordered.length; index += 1) {
      const current = ordered[index];
      for (let nextIndex = index + 1; nextIndex < ordered.length; nextIndex += 1) {
        const next = ordered[nextIndex];
        if (!workspaceItemsOverlap(current, next)) continue;

        const itemToPush =
          current.id === activeItemId
            ? next
            : next.id === activeItemId
              ? current
              : current.y <= next.y
                ? next
                : current;
        const blocker = itemToPush.id === current.id ? next : current;
        const newY = blocker.y + blocker.h;

        if (newY !== itemToPush.y) {
          const stored = items.find((item) => item.id === itemToPush.id);
          if (stored) stored.y = newY;
          itemToPush.y = newY;
          changed = true;
        }
      }
    }

    if (!changed) break;
  }

  return {
    ...layout,
    items: items.sort((a, b) => a.y - b.y || a.x - b.x || a.id.localeCompare(b.id)),
  };
}

export function moveWorkspaceItem(
  layout: WorkspacePageLayout,
  itemId: string,
  x: number,
  y: number,
  widgets: WorkspaceWidgetDefinition[] = [],
) {
  const widgetById = new Map(widgets.map((widget) => [widget.id, widget]));
  const nextZ = nextWorkspaceZ(layout);
  const items = layout.items.map((item) =>
    item.id === itemId
      ? clampWorkspaceItem(
          { ...item, x, y, z: nextZ },
          widgetById.get(item.widgetId),
          { columns: layout.columns },
        )
      : item,
  );
  return resolveWorkspaceCollisions({ ...layout, items }, itemId);
}

export function resizeWorkspaceItem(
  layout: WorkspacePageLayout,
  itemId: string,
  edge: WorkspaceResizeEdge,
  deltaX: number,
  deltaY: number,
  widgets: WorkspaceWidgetDefinition[] = [],
) {
  const widgetById = new Map(widgets.map((widget) => [widget.id, widget]));
  const nextZ = nextWorkspaceZ(layout);
  const items = layout.items.map((item) => {
    if (item.id !== itemId) return item;
    const widget = widgetById.get(item.widgetId);
    const minW = widget?.minW ?? 2;
    const minH = widget?.minH ?? 2;
    let { x, y, w, h } = item;

    if (edge.includes("e")) w += deltaX;
    if (edge.includes("s")) h += deltaY;
    if (edge.includes("w")) {
      x += deltaX;
      w -= deltaX;
    }
    if (edge.includes("n")) {
      y += deltaY;
      h -= deltaY;
    }

    if (w < minW) {
      if (edge.includes("w")) x -= minW - w;
      w = minW;
    }
    if (h < minH) {
      if (edge.includes("n")) y -= minH - h;
      h = minH;
    }

    return clampWorkspaceItem(
      { ...item, x, y, w, h, z: nextZ },
      widget,
      { columns: layout.columns },
    );
  });
  return resolveWorkspaceCollisions({ ...layout, items }, itemId);
}

export function focusWorkspaceItem(layout: WorkspacePageLayout, itemId: string) {
  const nextZ = nextWorkspaceZ(layout);
  return {
    ...layout,
    items: layout.items.map((item) =>
      item.id === itemId ? { ...item, z: nextZ } : item,
    ),
  };
}

export function removeWorkspaceItem(
  layout: WorkspacePageLayout,
  itemId: string,
) {
  return {
    ...layout,
    items: layout.items.filter((item) => item.id !== itemId),
  };
}

export function createWorkspaceItemFromWidget(
  layout: WorkspacePageLayout,
  widget: WorkspaceWidgetDefinition,
  id = `${widget.id}-${Date.now().toString(36)}`,
) {
  const y = layout.items.reduce(
    (bottom, item) => Math.max(bottom, item.y + item.h),
    0,
  );
  return clampWorkspaceItem(
    {
      id,
      widgetId: widget.id,
      x: 0,
      y,
      w: widget.defaultW,
      h: widget.defaultH,
      z: nextWorkspaceZ(layout),
    },
    widget,
    { columns: layout.columns },
  );
}

export function addWorkspaceWidget(
  layout: WorkspacePageLayout,
  widget: WorkspaceWidgetDefinition,
  id?: string,
) {
  return resolveWorkspaceCollisions({
    ...layout,
    items: [...layout.items, createWorkspaceItemFromWidget(layout, widget, id)],
  });
}

export function workspaceLayoutHeight(layout: WorkspacePageLayout) {
  const rows = layout.items.reduce(
    (bottom, item) => Math.max(bottom, item.y + item.h),
    0,
  );
  return Math.max(4, rows);
}

function nextWorkspaceZ(layout: WorkspacePageLayout) {
  return layout.items.reduce((max, item) => Math.max(max, item.z), 0) + 1;
}
