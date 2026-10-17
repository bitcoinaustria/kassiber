let available: boolean | undefined;

/**
 * Whether this browser can draw the glass views. Probed once: a probe costs a
 * WebGL context, and the answer does not change while the page lives.
 */
export function webglAvailable() {
  if (available !== undefined) return available;
  try {
    const context = document.createElement("canvas").getContext("webgl2");
    // Contexts are few; give the probe's back at once instead of waiting for GC.
    context?.getExtension("WEBGL_lose_context")?.loseContext();
    available = Boolean(context);
  } catch {
    available = false;
  }
  return available;
}

/** For tests that switch WebGL on and off between cases. */
export function forgetWebglProbe() {
  available = undefined;
}
