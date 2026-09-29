import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

import { cn } from "@/lib/utils";

import type { GlassScene, GlassSceneLook } from "./stage";

// Seen from the upper left, as in the lab renders: block tops and inner faces show.
const REST_VIEW = { yaw: 0.5, pitch: 0.3 };
const YAW_LIMIT = 0.95;
const PITCH_LIMIT = 0.6;
const DRAG_RADIANS_PER_PIXEL = 0.008;
const KEY_STEP = 0.08;

type Status = "loading" | "ready" | "unavailable";

function clamp(value: number, limit: number) {
  return Math.min(limit, Math.max(-limit, value));
}

function webglAvailable() {
  try {
    const context = document.createElement("canvas").getContext("webgl2");
    // Contexts are few; give the probe's back at once instead of waiting for GC.
    context?.getExtension("WEBGL_lose_context")?.loseContext();
    return Boolean(context);
  } catch {
    return false;
  }
}

function isDark() {
  // Static renders (tests, prerendering) have no document; effects set it later.
  return typeof document !== "undefined" && document.documentElement.classList.contains("dark");
}

/**
 * The painted surface behind the view, in sRGB. Theme colours are oklch and the
 * dialog surface is translucent, so the first painted ancestor is composited
 * over the theme base on a 1×1 canvas, which resolves any CSS colour.
 */
function surfaceColor(element: HTMLElement) {
  let color = "";
  for (let node: HTMLElement | null = element; node; node = node.parentElement) {
    const background = getComputedStyle(node).backgroundColor;
    if (background && background !== "transparent" && !/^rgba\(0, 0, 0, 0\)$/.test(background)) {
      color = background;
      break;
    }
  }
  const probe = document.createElement("canvas");
  probe.width = 1;
  probe.height = 1;
  const context = probe.getContext("2d", { willReadFrequently: true });
  if (!context) return isDark() ? "#262626" : "#ffffff";
  context.fillStyle = isDark() ? "#262626" : "#ffffff";
  context.fillRect(0, 0, 1, 1);
  if (color) {
    context.fillStyle = color;
    context.fillRect(0, 0, 1, 1);
  }
  const [red, green, blue] = context.getImageData(0, 0, 1, 1).data;
  return `rgb(${red}, ${green}, ${blue})`;
}

/**
 * A glass 3D view: WebGL check, one canvas per scene, render on change only,
 * drag or arrow keys to turn, Home or double-click to reset. The scene module
 * (and three.js with it) loads only through `load`, so callers keep it lazy.
 * Without WebGL, or when the scene fails or loses its context, `unavailable`
 * shows instead.
 */
export function Glass3DView({
  scene: sceneInput,
  load,
  ariaLabel,
  loadingLabel,
  className,
  unavailable,
  children,
  testId,
}: {
  /** What the scene is drawn from; a new value rebuilds the scene. */
  scene: unknown;
  load: (canvas: HTMLCanvasElement, look: GlassSceneLook) => Promise<GlassScene>;
  ariaLabel: string;
  loadingLabel: string;
  /** Size of the drawing area. */
  className: string;
  unavailable: ReactNode;
  /** Shown below the drawing, e.g. a legend. */
  children?: ReactNode;
  testId?: string;
}) {
  const shellRef = useRef<HTMLDivElement | null>(null);
  const sceneRef = useRef<{ scene: GlassScene; draw: () => void } | null>(null);
  const viewRef = useRef({ ...REST_VIEW });
  const dragRef = useRef<{ x: number; y: number; yaw: number; pitch: number } | null>(null);
  const loadRef = useRef(load);
  loadRef.current = load;
  const [status, setStatus] = useState<Status>("loading");
  const [dark, setDark] = useState(isDark);

  useEffect(() => {
    const observer = new MutationObserver(() => setDark(isDark()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);

  // A layout effect, so switching to hidden values removes the old canvas, and
  // the amounts its shape was drawn from, before the hidden state is painted.
  useLayoutEffect(() => {
    const shell = shellRef.current;
    if (!shell) return undefined;
    if (!webglAvailable()) {
      setStatus("unavailable");
      return undefined;
    }
    setStatus("loading");
    // A fresh canvas per scene: a context released on dispose cannot be reused.
    const canvas = document.createElement("canvas");
    canvas.className = "absolute inset-0 block h-full w-full";
    canvas.setAttribute("aria-hidden", "true");
    shell.prepend(canvas);
    let disposed = false;
    let scene: GlassScene | null = null;
    let frame = 0;
    const teardown = () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      resize.disconnect();
      canvas.removeEventListener("webglcontextlost", onContextLost);
      scene?.dispose();
      scene = null;
      sceneRef.current = null;
      canvas.remove();
    };
    const draw = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        scene?.render();
      });
    };
    const resize = new ResizeObserver(() => {
      if (!scene) return;
      scene.resize(shell.clientWidth, shell.clientHeight);
      draw();
    });
    // No restore: a lost context ends the 3D view and frees what it held.
    function onContextLost() {
      teardown();
      setStatus("unavailable");
    }
    canvas.addEventListener("webglcontextlost", onContextLost);
    loadRef
      .current(canvas, { background: surfaceColor(shell), dark })
      .then((created) => {
        if (disposed) {
          created.dispose();
          return;
        }
        scene = created;
        scene.resize(shell.clientWidth, shell.clientHeight);
        scene.setView(viewRef.current.yaw, viewRef.current.pitch);
        sceneRef.current = { scene, draw };
        resize.observe(shell);
        draw();
        setStatus("ready");
      })
      .catch(() => {
        if (disposed) return;
        // The fallback unmounts the shell without rerunning this effect's cleanup.
        teardown();
        setStatus("unavailable");
      });
    return teardown;
  }, [sceneInput, dark]);

  const turn = (yaw: number, pitch: number) => {
    viewRef.current = { yaw: clamp(yaw, YAW_LIMIT), pitch: clamp(pitch, PITCH_LIMIT) };
    const current = sceneRef.current;
    if (!current) return;
    current.scene.setView(viewRef.current.yaw, viewRef.current.pitch);
    current.draw();
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { x: event.clientX, y: event.clientY, ...viewRef.current };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    turn(
      drag.yaw + (event.clientX - drag.x) * DRAG_RADIANS_PER_PIXEL,
      drag.pitch + (event.clientY - drag.y) * DRAG_RADIANS_PER_PIXEL,
    );
  };
  const onPointerUp = () => {
    dragRef.current = null;
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const { yaw, pitch } = viewRef.current;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-KEY_STEP, 0],
      ArrowRight: [KEY_STEP, 0],
      ArrowUp: [0, -KEY_STEP],
      ArrowDown: [0, KEY_STEP],
    };
    if (event.key === "Home") {
      event.preventDefault();
      turn(REST_VIEW.yaw, REST_VIEW.pitch);
      return;
    }
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    turn(yaw + move[0], pitch + move[1]);
  };

  if (status === "unavailable") return <>{unavailable}</>;

  return (
    <div className="space-y-2">
      <div className="relative">
        <div
          ref={shellRef}
          role="img"
          aria-label={ariaLabel}
          tabIndex={0}
          className={cn(
            "relative w-full cursor-grab touch-none overflow-hidden rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring active:cursor-grabbing",
            className,
          )}
          data-testid={testId}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onDoubleClick={() => turn(REST_VIEW.yaw, REST_VIEW.pitch)}
          onKeyDown={onKeyDown}
        />
        {/* Beside the image, not inside it: an image role hides its descendants. */}
        {status === "loading" ? (
          <div
            className="pointer-events-none absolute inset-0 grid place-items-center text-xs text-muted-foreground"
            role="status"
          >
            {loadingLabel}
          </div>
        ) : null}
      </div>
      {children}
    </div>
  );
}
