// @vitest-environment happy-dom
//
// A setting-level search result opens its section, then the modal scrolls the
// row into view and washes it for a moment; a row rendered late (after its
// daemon query) is still found, and an unfolded disclosure is opened.
import * as React from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SettingsSectionId } from "../settingsSections";
import {
  requestSettingsAnchor,
  useSettingsAnchorHighlight,
  type SettingsAnchorId,
} from "./settingsSearch";

function Section({
  section,
  children,
}: {
  section: SettingsSectionId;
  children: React.ReactNode;
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  useSettingsAnchorHighlight(ref, section);
  return <div ref={ref}>{children}</div>;
}

const scrolled: Array<{ anchor: string | null; options: unknown }> = [];

beforeEach(() => {
  vi.useFakeTimers();
  scrolled.length = 0;
  HTMLElement.prototype.scrollIntoView = function (options?: unknown) {
    scrolled.push({ anchor: this.getAttribute("data-settings-anchor"), options });
  };
});

afterEach(() => {
  cleanup();
  requestSettingsAnchor(null);
  vi.useRealTimers();
});

const row = (anchor: SettingsAnchorId) => (
  <div data-settings-anchor={anchor}>{anchor}</div>
);

describe("useSettingsAnchorHighlight", () => {
  it("scrolls to the requested row and clears the highlight after a moment", () => {
    act(() => requestSettingsAnchor("clearClipboard"));
    const { container } = render(
      <Section section="security-privacy">
        {row("blurSensitive")}
        {row("clearClipboard")}
      </Section>,
    );
    const target = container.querySelector('[data-settings-anchor="clearClipboard"]')!;
    expect(scrolled.map((entry) => entry.anchor)).toEqual(["clearClipboard"]);
    expect(target.hasAttribute("data-settings-highlight")).toBe(true);
    act(() => vi.advanceTimersByTime(1600));
    expect(target.hasAttribute("data-settings-highlight")).toBe(false);
  });

  it("waits for its section and for a row that renders late", async () => {
    act(() => requestSettingsAnchor("autoLock"));
    const { rerender, container } = render(
      <Section section="security-privacy">{row("clearClipboard")}</Section>,
    );
    expect(scrolled).toEqual([]);
    rerender(<Section section="security-lock">{null}</Section>);
    expect(scrolled).toEqual([]);
    rerender(<Section section="security-lock">{row("autoLock")}</Section>);
    // MutationObserver callbacks are microtasks.
    await act(async () => {
      await Promise.resolve();
    });
    expect(scrolled.map((entry) => entry.anchor)).toEqual(["autoLock"]);
    expect(
      container
        .querySelector('[data-settings-anchor="autoLock"]')!
        .hasAttribute("data-settings-highlight"),
    ).toBe(true);
  });

  it("unfolds a closed disclosure around its target", () => {
    act(() => requestSettingsAnchor("analysisNetwork"));
    const { container } = render(
      <Section section="network-bitcoin">
        <details data-settings-anchor="analysisNetwork">
          <summary>Local analysis</summary>
        </details>
      </Section>,
    );
    expect(container.querySelector("details")!.open).toBe(true);
  });

  it("does not scroll smoothly when the user prefers reduced motion", () => {
    const matchMedia = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: query.includes("reduce"),
      media: query,
    })) as typeof window.matchMedia;
    try {
      act(() => requestSettingsAnchor("theme"));
      render(<Section section="general-appearance">{row("theme")}</Section>);
      expect(scrolled[0]?.options).toMatchObject({ behavior: "auto" });
    } finally {
      window.matchMedia = matchMedia;
    }
  });

  it("drops a request that never reached its section", () => {
    act(() => requestSettingsAnchor("autoLock"));
    act(() => vi.advanceTimersByTime(5000));
    render(<Section section="security-lock">{row("autoLock")}</Section>);
    expect(scrolled).toEqual([]);
  });
});
