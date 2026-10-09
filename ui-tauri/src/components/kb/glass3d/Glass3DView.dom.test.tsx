// @vitest-environment happy-dom
//
// Mounted: the scene loads in an effect and hover picks wait for a frame.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Glass3DView } from "./Glass3DView";
import type { GlassScene } from "./stage";

/** Frames run only when a test says so, like a browser between two paints. */
let frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;
function runFrames() {
  const pending = frames;
  frames = new Map();
  act(() => pending.forEach((callback) => callback(0)));
}

/** Left half of the drawing is one leg, right half another; turned, they swap. */
let turned = false;
function partAtX(x: number): string {
  return x < 200 !== turned ? "input:a" : "output:b";
}

function fakeScene() {
  return {
    resize: vi.fn(),
    setView: vi.fn((yaw: number) => {
      turned = yaw < 0.5;
    }),
    render: vi.fn(),
    pick: vi.fn((x: number) => partAtX(x)),
    highlight: vi.fn(),
    dispose: vi.fn(),
  } satisfies GlassScene;
}

beforeEach(() => {
  turned = false;
  frames = new Map();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    nextFrame += 1;
    frames.set(nextFrame, callback);
    return nextFrame;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const probe = { getExtension: () => ({ loseContext() {} }) };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    ((kind: string) => (kind === "webgl2" ? probe : null)) as never,
  );
  vi.spyOn(HTMLElement.prototype, "setPointerCapture").mockImplementation(() => {});
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function mount() {
  const scene = fakeScene();
  const onHoverPart = vi.fn();
  const onSelectPart = vi.fn();
  render(
    <Glass3DView
      scene={{}}
      load={async () => scene}
      ariaLabel="Graph"
      loadingLabel="Loading"
      className="h-10"
      unavailable={null}
      testId="view"
      onHoverPart={onHoverPart}
      onSelectPart={onSelectPart}
      selectable={(part) => part !== "output:fee"}
    />,
  );
  await vi.waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  runFrames();
  return { scene, onHoverPart, onSelectPart, view: screen.getByTestId("view") };
}

const at = (x: number) => ({ clientX: x, clientY: 10, pointerId: 1 });

describe("pointing at the glass view's parts", () => {
  it("selects the part under a tap that never hovered", async () => {
    const { view, onSelectPart } = await mount();
    fireEvent.pointerDown(view, at(300));
    fireEvent.pointerUp(view, at(300));
    expect(onSelectPart).toHaveBeenCalledExactlyOnceWith("output:b");
  });

  it("selects where a click lands while a hover from elsewhere is still queued", async () => {
    const { view, onSelectPart, onHoverPart } = await mount();
    fireEvent.pointerMove(view, at(100));
    runFrames();
    expect(onHoverPart).toHaveBeenLastCalledWith("input:a");
    fireEvent.pointerMove(view, at(300));
    fireEvent.pointerDown(view, at(300));
    fireEvent.pointerUp(view, at(300));
    expect(onSelectPart).toHaveBeenCalledExactlyOnceWith("output:b");
    expect(onHoverPart).toHaveBeenLastCalledWith("output:b");
    runFrames();
    expect(onHoverPart).toHaveBeenLastCalledWith("output:b");
  });

  it("leaves parts it may not select alone", async () => {
    const { view, scene, onSelectPart } = await mount();
    scene.pick.mockReturnValue("output:fee");
    fireEvent.pointerDown(view, at(300));
    fireEvent.pointerUp(view, at(300));
    expect(onSelectPart).not.toHaveBeenCalled();
    expect(view.style.cursor).toBe("");
  });

  it("hovers where the pointer last was when the frame comes, picking once", async () => {
    const { view, scene, onHoverPart } = await mount();
    fireEvent.pointerMove(view, at(100));
    fireEvent.pointerMove(view, at(150));
    fireEvent.pointerMove(view, at(300));
    expect(scene.pick).not.toHaveBeenCalled();
    runFrames();
    expect(scene.pick).toHaveBeenCalledOnce();
    expect(onHoverPart).toHaveBeenCalledExactlyOnceWith("output:b");
    expect(view.style.cursor).toBe("pointer");
  });

  it("picks again when a key turns the drawing under a still pointer", async () => {
    const { view, onHoverPart } = await mount();
    fireEvent.pointerMove(view, at(300));
    runFrames();
    expect(onHoverPart).toHaveBeenLastCalledWith("output:b");
    fireEvent.keyDown(view, { key: "ArrowLeft" });
    runFrames();
    expect(onHoverPart).toHaveBeenLastCalledWith("input:a");
  });

  it("drops a queued hover when the pointer is cancelled or leaves", async () => {
    const { view, onHoverPart } = await mount();
    fireEvent.pointerMove(view, at(300));
    fireEvent.pointerCancel(view, at(300));
    runFrames();
    fireEvent.pointerMove(view, at(100));
    fireEvent.pointerLeave(view, at(100));
    runFrames();
    expect(onHoverPart).not.toHaveBeenCalled();
  });

  it("clears the lit part when its drawing is torn down", async () => {
    const { view, onHoverPart } = await mount();
    fireEvent.pointerMove(view, at(300));
    runFrames();
    expect(onHoverPart).toHaveBeenLastCalledWith("output:b");
    cleanup();
    expect(onHoverPart).toHaveBeenLastCalledWith(null);
  });
});
