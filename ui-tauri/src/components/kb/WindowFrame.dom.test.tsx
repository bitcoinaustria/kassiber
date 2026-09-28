// @vitest-environment happy-dom
//
// Mounted, not static: the title-bar claim is a layout effect and the inert
// drag handler is a window listener, and neither runs under
// `renderToStaticMarkup`.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// The UI store persists through `localStorage`, which this DOM does not wire
// up as a global, and zustand resolves its storage while the store module is
// being evaluated. Hoisted so it precedes the imports below.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const appWindow = vi.hoisted(() => ({
  isFullscreen: vi.fn(),
  onResized: vi.fn(),
  startDragging: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => appWindow,
}));

import { WindowFrame } from "./WindowFrame";
import { useClaimWindowTitlebar } from "./windowChromeContext";

function Claimer({ active }: { active: boolean }) {
  useClaimWindowTitlebar(active);
  return <main>Screen</main>;
}

async function mount(active: boolean) {
  const view = render(
    <WindowFrame>
      <Claimer active={active} />
    </WindowFrame>,
  );
  // Let the lazily imported window API resolve.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
  return view;
}

function press(clientY: number) {
  const init = { button: 0, clientX: 600, clientY, detail: 1, bubbles: true };
  const root = document.documentElement;
  root.dispatchEvent(new PointerEvent("pointerdown", init));
  root.dispatchEvent(new MouseEvent("mousedown", init));
}

describe("WindowFrame on macOS", () => {
  beforeEach(() => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    );
    appWindow.isFullscreen.mockResolvedValue(false);
    appWindow.onResized.mockResolvedValue(() => {});
    appWindow.startDragging.mockResolvedValue(undefined);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    appWindow.startDragging.mockReset();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    document.querySelectorAll("[data-test-menu]").forEach((node) => node.remove());
  });

  it("drops its fallback drag strip and chip once a screen claims the title bar", async () => {
    const unclaimed = await mount(false);
    expect(document.querySelector("[data-tauri-drag-region]")).not.toBeNull();
    expect(
      document.querySelector('[aria-label="Alpha software warning"]'),
    ).not.toBeNull();
    unclaimed.unmount();

    await mount(true);
    expect(document.querySelector("[data-tauri-drag-region]")).toBeNull();
    expect(
      document.querySelector('[aria-label="Alpha software warning"]'),
    ).toBeNull();
    expect(
      document.documentElement.style.getPropertyValue(
        "--kb-native-titlebar-height",
      ),
    ).toBe("40px");
  });

  it("moves the window from the title bar while a dialog makes the page inert", async () => {
    await mount(true);
    // Stands in for Radix's dismiss-on-outside-press listener.
    const outsidePress = vi.fn();
    document.addEventListener("pointerdown", outsidePress);

    press(20);
    expect(appWindow.startDragging).toHaveBeenCalledTimes(1);
    expect(outsidePress).not.toHaveBeenCalled();

    press(120);
    expect(appWindow.startDragging).toHaveBeenCalledTimes(1);
    expect(outsidePress).toHaveBeenCalledTimes(1);

    document.removeEventListener("pointerdown", outsidePress);
  });

  it("lets a title-bar press close an open menu instead of dragging", async () => {
    await mount(true);
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    menu.setAttribute("data-state", "open");
    menu.setAttribute("data-test-menu", "");
    document.body.append(menu);
    const outsidePress = vi.fn();
    document.addEventListener("pointerdown", outsidePress);

    press(20);
    expect(appWindow.startDragging).not.toHaveBeenCalled();
    expect(outsidePress).toHaveBeenCalledTimes(1);

    document.removeEventListener("pointerdown", outsidePress);
  });

  it("drops the insets in full screen, where the traffic lights are hidden", async () => {
    appWindow.isFullscreen.mockResolvedValue(true);
    await mount(true);
    await act(async () => {
      await Promise.resolve();
    });

    const root = document.documentElement.style;
    expect(root.getPropertyValue("--kb-native-titlebar-height")).toBe("0px");
    expect(root.getPropertyValue("--kb-traffic-light-inset")).toBe("0px");
  });
});
