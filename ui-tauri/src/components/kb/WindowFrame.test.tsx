import { readFileSync } from "node:fs";

import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DIALOG_VIEWPORT_CLASSNAME } from "@/components/ui/dialog";
import { WindowFrame } from "./WindowFrame";

const uiState = vi.hoisted(() => ({ preAlphaBannerVisible: true }));

vi.mock("@/store/ui", () => ({
  useUiStore: (
    selector: (state: typeof uiState) => unknown,
  ) => selector(uiState),
}));

function renderFrame({
  tauri = false,
  userAgent = "Mozilla/5.0 (X11; Linux x86_64)",
}: {
  tauri?: boolean;
  userAgent?: string;
} = {}) {
  vi.stubGlobal("window", tauri ? { __TAURI_INTERNALS__: {} } : {});
  vi.stubGlobal("navigator", { userAgent });
  return renderToStaticMarkup(
    <WindowFrame>
      <main>Setup or app content</main>
    </WindowFrame>,
  );
}

describe("WindowFrame", () => {
  afterEach(() => {
    uiState.preAlphaBannerVisible = true;
    vi.unstubAllGlobals();
  });

  it("keeps the warning above every routed screen", () => {
    const html = renderFrame();

    expect(html).not.toContain("data-tauri-drag-region");
    expect(html).toContain("h-[var(--kb-warning-bar-height)]");
    expect(html).toContain("z-[60]");
    expect(html).toContain("You can disable this banner in Settings.");
    expect(html).toContain("Setup or app content");
  });

  it("reserves the native title bar only inside Tauri on macOS", () => {
    const macTauri = renderFrame({
      tauri: true,
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    });
    const macBrowser = renderFrame({
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    });
    const windowsTauri = renderFrame({
      tauri: true,
      userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    });

    expect(macTauri).toContain("--kb-native-titlebar-background");
    expect(macBrowser).not.toContain("--kb-native-titlebar-background");
    expect(windowsTauri).not.toContain("--kb-native-titlebar-background");
  });

  it("keeps the native title bar when the warning is disabled", () => {
    uiState.preAlphaBannerVisible = false;
    const html = renderFrame({
      tauri: true,
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    });

    expect(html).toContain("--kb-native-titlebar-background");
    expect(html).not.toContain("--kb-warning-bar-height");
    expect(html).not.toContain("Alpha software");
  });

  it("pins dark native chrome to RGB 18 18 18", () => {
    const css = readFileSync(
      new URL("../../styles/globals.css", import.meta.url),
      "utf8",
    );

    expect(css).toMatch(
      /\.dark\s*\{[\s\S]*--kb-native-titlebar-background:\s*#121212;/,
    );
  });

  it("keeps portalled dialogs below the shared window inset", () => {
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain(
      "top-[var(--kb-window-top-inset)]",
    );
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain("bottom-0");
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain(
      "100dvh-var(--kb-window-top-inset)-2rem",
    );
  });
});
