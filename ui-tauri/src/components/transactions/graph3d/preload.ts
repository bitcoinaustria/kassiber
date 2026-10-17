import { webglAvailable } from "@/components/kb/glass3d/webgl";

/**
 * Loads the 3D graph's code and gets its renderer ready (environment light,
 * compiled shaders) while the graph itself is still being fetched, so the
 * drawing appears as soon as its data does. Local work only; a no-op without
 * WebGL, and cheap to call again.
 */
export function preloadTransactionGraph3D() {
  if (!webglAvailable()) return;
  void import("./glassScene")
    .then(({ warmGlassRenderer }) => warmGlassRenderer())
    .catch(() => undefined);
}
