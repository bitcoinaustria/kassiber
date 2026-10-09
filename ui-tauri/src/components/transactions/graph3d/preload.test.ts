import { afterEach, describe, expect, it, vi } from "vitest";

const probe = vi.hoisted(() => ({ available: true, warm: vi.fn(async () => undefined) }));
vi.mock("@/components/kb/glass3d/webgl", () => ({ webglAvailable: () => probe.available }));
vi.mock("./glassScene", () => ({ warmGlassRenderer: probe.warm }));

import { preloadTransactionGraph3D } from "./preload";

afterEach(() => {
  probe.warm.mockClear();
  probe.available = true;
});

describe("warming the 3D graph ahead of its data", () => {
  it("loads the scene code and warms the renderer where WebGL is there", async () => {
    preloadTransactionGraph3D();
    await vi.waitFor(() => expect(probe.warm).toHaveBeenCalledOnce());
  });

  it("does nothing without WebGL", async () => {
    probe.available = false;
    preloadTransactionGraph3D();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(probe.warm).not.toHaveBeenCalled();
  });
});
