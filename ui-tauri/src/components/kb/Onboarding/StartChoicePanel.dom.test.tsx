// @vitest-environment happy-dom
//
// Mounted, not static: the select shows its chosen language only once Radix
// has mounted its items, and choosing one writes the UI store.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  // The language lives in the persisted UI store. Provide its storage before
  // the module imports, as other mounted store tests do.
  vi.stubGlobal("localStorage", {
    getItem: vi.fn(() => null),
    setItem: vi.fn(),
    removeItem: vi.fn(),
  });
});

import { useUiStore } from "@/store/ui";

import { StartChoicePanel } from "./StartChoicePanel";

const renderPanel = () =>
  render(
    <StartChoicePanel
      importAvailable
      importing={false}
      onSetup={vi.fn()}
      onImport={vi.fn()}
      onQuickStart={vi.fn()}
    />,
  );

afterEach(() => {
  cleanup();
  useUiStore.getState().setLang("en");
});

describe("StartChoicePanel language", () => {
  it("offers the UI language before setup and stores the choice", () => {
    renderPanel();
    const trigger = screen.getByRole("combobox", { name: "Language" });
    expect(trigger.textContent).toContain("English");

    fireEvent.keyDown(trigger, { key: "Enter" });
    fireEvent.click(screen.getByRole("option", { name: "Deutsch" }));

    expect(useUiStore.getState().lang).toBe("de");
  });
});
