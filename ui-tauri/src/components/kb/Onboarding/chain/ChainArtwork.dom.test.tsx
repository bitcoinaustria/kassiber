// @vitest-environment happy-dom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { ChainArtwork } from "./ChainArtwork";

let reduced = true;
vi.mock("@/hooks/usePrefersReducedMotion", () => ({
  usePrefersReducedMotion: () => reduced,
}));

beforeEach(() => {
  reduced = true;
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it("renders reduced motion as a visible static frame and follows preference changes", () => {
  const { container, rerender } = render(<ChainArtwork live mined={3} fill={0.5} className="" />);
  const artwork = container.querySelector("svg")!;
  expect(artwork.hasAttribute("data-still")).toBe(true);
  // A paused decode would freeze its transparent first frame. Reduced
  // motion must remove the animation instead, showing its resting geometry.
  expect(artwork.hasAttribute("data-paused")).toBe(false);
  expect(artwork.querySelectorAll(".kb-chain-tx").length).toBeGreaterThan(0);
  const staticFrame = artwork.innerHTML;
  act(() => vi.advanceTimersByTime(1800));
  expect(artwork.innerHTML).toBe(staticFrame);

  reduced = false;
  rerender(<ChainArtwork live mined={3} fill={0.5} className="" />);
  expect(artwork.hasAttribute("data-still")).toBe(false);
  act(() => vi.advanceTimersByTime(900));
  expect(artwork.innerHTML).not.toBe(staticFrame);

  reduced = true;
  rerender(<ChainArtwork live mined={3} fill={0.5} className="" />);
  expect(artwork.hasAttribute("data-still")).toBe(true);
  expect(artwork.hasAttribute("data-paused")).toBe(false);
  expect(artwork.innerHTML).toBe(staticFrame);
});
