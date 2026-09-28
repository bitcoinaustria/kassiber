import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  DIALOG_FULL_WINDOW_CLASSNAME,
  DIALOG_VIEWPORT_CLASSNAME,
} from "@/components/ui/dialog";
import { WindowFrame } from "./WindowFrame";

const uiState = vi.hoisted(() => ({
  preAlphaBannerVisible: true,
  setPreAlphaBannerVisible: () => {},
}));

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

  it("keeps the alpha warning on screens outside the shell as a chip, not a row", () => {
    const html = renderFrame();

    expect(html).toContain('aria-label="Alpha software warning"');
    expect(html).toContain("h-(--kb-toolbar-height)");
    expect(html).not.toContain("--kb-warning-bar-height");
    expect(html).toContain("Setup or app content");
  });

  it("draws a draggable title bar strip only inside Tauri on macOS", () => {
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

    expect(macTauri).toContain("data-tauri-drag-region");
    expect(macBrowser).not.toContain("data-tauri-drag-region");
    expect(windowsTauri).not.toContain("data-tauri-drag-region");
  });

  it("hides the chip when the warning is disabled", () => {
    uiState.preAlphaBannerVisible = false;
    const html = renderFrame({
      tauri: true,
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    });

    expect(html).toContain("data-tauri-drag-region");
    expect(html).not.toContain("Alpha software warning");
  });

  it("frames screens outside the shell in the shell's own outline", () => {
    const html = renderFrame();

    // Title bar row on the chrome, then the page on the inset panel with the
    // dashboard panel's hairline and gutters.
    expect(html).toContain("h-(--kb-toolbar-height)");
    expect(html).toContain("md:px-1.5 md:pb-1.5");
    expect(html).toContain("md:rounded-xl md:border md:border-border/70");
  });

  it("keeps portalled dialogs below the title bar, so it stays draggable", () => {
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain(
      "top-[var(--kb-window-top-inset)]",
    );
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain("bottom-0");
    expect(DIALOG_VIEWPORT_CLASSNAME).toContain(
      "100dvh-var(--kb-window-top-inset)-2rem",
    );
    expect(DIALOG_FULL_WINDOW_CLASSNAME).toContain(
      "top-[var(--kb-window-top-inset)]",
    );
    expect(DIALOG_FULL_WINDOW_CLASSNAME).toContain("max-h-none");
  });
});
