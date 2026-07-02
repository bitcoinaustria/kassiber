import { describe, expect, it } from "vitest";

import { nextAssistantDockCollapsed } from "./assistantDockLayout";

describe("nextAssistantDockCollapsed", () => {
  it("does not collapse when removing dock padding would erase the scroll range", () => {
    expect(
      nextAssistantDockCollapsed({
        collapsed: false,
        scrollTop: 120,
        scrollHeight: 940,
        clientHeight: 720,
      }),
    ).toBe(false);
  });

  it("does not collapse while the user is already in the bottom range that would be removed", () => {
    expect(
      nextAssistantDockCollapsed({
        collapsed: false,
        scrollTop: 280,
        scrollHeight: 1010,
        clientHeight: 720,
      }),
    ).toBe(false);
  });

  it("collapses on long pages after the user scrolls past the dock threshold", () => {
    expect(
      nextAssistantDockCollapsed({
        collapsed: false,
        scrollTop: 140,
        scrollHeight: 2000,
        clientHeight: 720,
      }),
    ).toBe(true);
  });

  it("keeps the collapsed dock stable until the user returns near the top", () => {
    expect(
      nextAssistantDockCollapsed({
        collapsed: true,
        scrollTop: 80,
        scrollHeight: 1824,
        clientHeight: 720,
      }),
    ).toBe(true);
    expect(
      nextAssistantDockCollapsed({
        collapsed: true,
        scrollTop: 20,
        scrollHeight: 1824,
        clientHeight: 720,
      }),
    ).toBe(false);
  });
});
