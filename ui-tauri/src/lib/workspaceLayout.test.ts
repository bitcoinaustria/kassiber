import { describe, expect, it } from "vitest";

import {
  addWorkspaceWidget,
  createWorkspaceLayout,
  moveWorkspaceItem,
  pageWorkspaceStorageKey,
  resizeWorkspaceItem,
  workspaceItemsOverlap,
  type WorkspaceWidgetDefinition,
} from "./workspaceLayout";

const widgets: WorkspaceWidgetDefinition[] = [
  {
    id: "summary",
    title: "Summary",
    description: "Book summary",
    minW: 3,
    minH: 2,
    defaultW: 6,
    defaultH: 2,
  },
  {
    id: "chart",
    title: "Chart",
    description: "Balance chart",
    minW: 4,
    minH: 3,
    defaultW: 8,
    defaultH: 5,
  },
];

describe("workspace layout", () => {
  it("keys persisted layouts by book identity and page", () => {
    const first = pageWorkspaceStorageKey("overview", {
      workspace: "Holding Co",
      profile: "Tax 2026",
      taxCountry: "at",
      fiatCurrency: "EUR",
    });
    const second = pageWorkspaceStorageKey("transactions", {
      workspace: "Holding Co",
      profile: "Tax 2026",
      taxCountry: "at",
      fiatCurrency: "EUR",
    });

    expect(first).toBe("Holding Co:Tax 2026:at:EUR::overview");
    expect(second).toBe("Holding Co:Tax 2026:at:EUR::transactions");
  });

  it("uses imported database path as the strongest book identity", () => {
    expect(
      pageWorkspaceStorageKey("overview", {
        workspace: "Renamed books",
        profile: "Renamed profile",
        importedProject: {
          database: "/Users/dev/.kassiber/data/kassiber.db",
        },
      }),
    ).toBe("/Users/dev/.kassiber/data/kassiber.db::overview");
  });

  it("snaps dragged widgets to the grid and pushes collisions down", () => {
    const layout = createWorkspaceLayout([
      { id: "a", widgetId: "summary", x: 0, y: 0, w: 6, h: 2, z: 1 },
      { id: "b", widgetId: "chart", x: 0, y: 3, w: 6, h: 4, z: 1 },
    ]);

    const moved = moveWorkspaceItem(layout, "b", 0, 0, widgets);
    const first = moved.items.find((item) => item.id === "a");
    const second = moved.items.find((item) => item.id === "b");

    expect(second).toMatchObject({ x: 0, y: 0, w: 6, h: 4 });
    expect(first).toMatchObject({ x: 0, y: 4, w: 6, h: 2 });
    expect(workspaceItemsOverlap(first!, second!)).toBe(false);
  });

  it("enforces minimum sizes while resizing from an edge", () => {
    const layout = createWorkspaceLayout([
      { id: "a", widgetId: "chart", x: 4, y: 2, w: 6, h: 5, z: 1 },
    ]);

    const resized = resizeWorkspaceItem(layout, "a", "w", 4, 0, widgets);

    expect(resized.items[0]).toMatchObject({ x: 6, y: 2, w: 4, h: 5 });
  });

  it("adds palette widgets below the current occupied workspace", () => {
    const layout = createWorkspaceLayout([
      { id: "a", widgetId: "summary", x: 0, y: 0, w: 6, h: 2, z: 1 },
      { id: "b", widgetId: "chart", x: 6, y: 0, w: 6, h: 5, z: 1 },
    ]);

    const next = addWorkspaceWidget(layout, widgets[0], "summary-copy");

    expect(next.items.find((item) => item.id === "summary-copy")).toMatchObject({
      x: 0,
      y: 5,
      w: 6,
      h: 2,
    });
  });
});
