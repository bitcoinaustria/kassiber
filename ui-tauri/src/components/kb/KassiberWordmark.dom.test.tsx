// @vitest-environment happy-dom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { KassiberWordmark, KassiberWorkingMark } from "./KassiberWordmark";
import { WORDMARK_K_PATH, WORDMARK_PATH } from "./kassiberWordmarkGrid";

let frames: Map<number, FrameRequestCallback>;
let nextFrame = 0;
let visibility: DocumentVisibilityState;
let reduced: boolean;

beforeEach(() => {
  frames = new Map();
  visibility = "visible";
  reduced = false;
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: reduced, media: query, onchange: null,
    addEventListener: vi.fn(), removeEventListener: vi.fn(),
    addListener: vi.fn(), removeListener: vi.fn(), dispatchEvent: vi.fn(),
  }));
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function setHidden(hidden: boolean) {
  visibility = hidden ? "hidden" : "visible";
  act(() => document.dispatchEvent(new Event("visibilitychange")));
}

it("finishes a reveal on hide and consumes its completion once", () => {
  const onEnd = vi.fn();
  const { container } = render(<KassiberWordmark animate onRevealEnd={onEnd} />);
  expect(frames.size).toBe(1);
  setHidden(true);
  expect(frames.size).toBe(0);
  expect(container.querySelector("path")?.getAttribute("d")).toBe(WORDMARK_PATH);
  expect(onEnd).toHaveBeenCalledTimes(1);
  setHidden(false);
  expect(frames.size).toBe(0);
  expect(onEnd).toHaveBeenCalledTimes(1);
});

it("does not replay a finished reveal when motion preferences change", () => {
  reduced = true;
  const onEnd = vi.fn();
  const { rerender, getByRole } = render(<KassiberWordmark animate onRevealEnd={onEnd} />);
  expect(getByRole("img", { name: "Kassiber" })).toBeTruthy();
  reduced = false;
  rerender(<KassiberWordmark animate onRevealEnd={onEnd} />);
  expect(frames.size).toBe(0);
  expect(onEnd).toHaveBeenCalledTimes(1);
  rerender(<KassiberWordmark animate={false} onRevealEnd={onEnd} />);
  rerender(<KassiberWordmark animate onRevealEnd={onEnd} />);
  expect(frames.size).toBe(1);
});

it("pauses the working loop while hidden and cancels it on unmount", () => {
  const { unmount } = render(<KassiberWorkingMark />);
  expect(frames.size).toBe(1);
  setHidden(true);
  expect(frames.size).toBe(0);
  setHidden(false);
  expect(frames.size).toBe(1);
  unmount();
  expect(frames.size).toBe(0);
});

it("shows static marks with reduced motion and consumes the reveal cue", () => {
  reduced = true;
  const onEnd = vi.fn();
  const { container } = render(<><KassiberWordmark animate onRevealEnd={onEnd} /><KassiberWorkingMark /></>);
  expect(frames.size).toBe(0);
  expect(onEnd).toHaveBeenCalledTimes(1);
  expect([...container.querySelectorAll("svg > path:first-child")].map((p) => p.getAttribute("d")))
    .toEqual([WORDMARK_PATH, WORDMARK_K_PATH]);
});

it("skips a reveal mounted in a hidden window", () => {
  visibility = "hidden";
  const onEnd = vi.fn();
  render(<KassiberWordmark animate onRevealEnd={onEnd} />);
  expect(frames.size).toBe(0);
  expect(onEnd).toHaveBeenCalledTimes(1);
});
