import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";

import { cn } from "@/lib/utils";

import type { GlassScene, GlassSceneLook } from "./stage";
import { webglAvailable } from "./webgl";

type Status = "loading" | "ready" | "unavailable";

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
 * shaders compiled before the first draw without blocking the page. The view
 * is fixed; it does not turn. The scene module
 * (and three.js with it) loads only through `load`, so callers keep it lazy.
 * Without WebGL, or when the scene fails or loses its context, `unavailable`
 * shows instead. Scenes with parts report the part under the pointer and a
 * click on it; the caller decides which part is lit.
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
  highlightedPart = null,
  onHoverPart,
  onSelectPart,
  selectable,
  overlay,
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
  /** The part drawn lit, e.g. the leg under the pointer or a hovered list row. */
  highlightedPart?: string | null;
  onHoverPart?: (part: string | null) => void;
  onSelectPart?: (part: string) => void;
  /** Which parts a click selects; all of them when left out. */
  selectable?: (part: string) => boolean;
  /** Laid over the drawing without taking the pointer, e.g. a hover card. */
  overlay?: ReactNode;
}) {
  const shellRef = useRef<HTMLDivElement | null>(null);
  const sceneRef = useRef<{ scene: GlassScene; draw: () => void } | null>(null);
  const loadRef = useRef(load);
  loadRef.current = load;
  // `pointer` is where the pointer last was over the drawing, in page pixels.
  const hoverRef = useRef<{
    part: string | null;
    frame: number;
    pointer: { x: number; y: number } | null;
  }>({ part: null, frame: 0, pointer: null });
  const onHoverRef = useRef(onHoverPart);
  onHoverRef.current = onHoverPart;
  const selectRef = useRef({ onSelectPart, selectable });
  selectRef.current = { onSelectPart, selectable };
  const [status, setStatus] = useState<Status>("loading");
  const [dark, setDark] = useState(isDark);

  // These read only refs, so the scene effect below can use them as well.
  const canSelect = useCallback((part: string | null): part is string => {
    const { onSelectPart: select, selectable: allowed } = selectRef.current;
    return Boolean(part && select && (!allowed || allowed(part)));
  }, []);

  const reportHover = useCallback((part: string | null) => {
    const hover = hoverRef.current;
    if (hover.part === part) return;
    hover.part = part;
    const shell = shellRef.current;
    if (shell) shell.style.cursor = canSelect(part) ? "pointer" : "";
    onHoverRef.current?.(part);
  }, [canSelect]);

  /** The part under a point on the page, picked now. */
  const partAt = useCallback((clientX: number, clientY: number) => {
    const shell = shellRef.current;
    const current = sceneRef.current;
    if (!shell || !current) return null;
    const rect = shell.getBoundingClientRect();
    return current.scene.pick(clientX - rect.left, clientY - rect.top);
  }, []);

  // One pick per frame at most, however fast the pointer moves, at where the
  // pointer is by then. Also after a resize moves the drawing under it.
  const scheduleHover = useCallback(() => {
    const hover = hoverRef.current;
    if (!onHoverRef.current || hover.frame || !hover.pointer) return;
    hover.frame = requestAnimationFrame(() => {
      hover.frame = 0;
      const pointer = hover.pointer;
      reportHover(pointer ? partAt(pointer.x, pointer.y) : null);
    });
  }, [partAt, reportHover]);

  const cancelHover = useCallback(() => {
    const hover = hoverRef.current;
    cancelAnimationFrame(hover.frame);
    hover.frame = 0;
  }, []);

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
    // A fresh canvas per scene; the scene draws onto it from the shared renderer.
    const canvas = document.createElement("canvas");
    canvas.className = "absolute inset-0 block h-full w-full";
    canvas.setAttribute("aria-hidden", "true");
    shell.prepend(canvas);
    let disposed = false;
    let scene: GlassScene | null = null;
    let frame = 0;
    let unsubscribeLost = () => {};
    const teardown = () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      frame = 0;
      // The lit part belonged to this drawing; the next one is picked afresh.
      cancelHover();
      reportHover(null);
      resize.disconnect();
      unsubscribeLost();
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
      scheduleHover();
    });
    // No restore: a lost context ends the 3D view and frees what it held.
    function onContextLost() {
      teardown();
      setStatus("unavailable");
    }
    loadRef
      .current(canvas, { background: surfaceColor(shell), dark })
      .then(async (created) => {
        if (disposed) {
          created.dispose();
          return;
        }
        scene = created;
        unsubscribeLost = created.onContextLost(onContextLost);
        await created.prepare();
        if (disposed) return;
        created.resize(shell.clientWidth, shell.clientHeight);
        sceneRef.current = { scene: created, draw };
        resize.observe(shell);
        draw();
        setStatus("ready");
        // A pointer already resting on the drawing points at what is now under it.
        scheduleHover();
      })
      .catch(() => {
        if (disposed) return;
        // The fallback unmounts the shell without rerunning this effect's cleanup.
        teardown();
        setStatus("unavailable");
      });
    return teardown;
  }, [sceneInput, dark, cancelHover, reportHover, scheduleHover]);

  useEffect(() => {
    const current = sceneRef.current;
    if (!current || status !== "ready") return;
    current.scene.highlight(highlightedPart);
    current.draw();
  }, [highlightedPart, status]);

  const hoverAt = (clientX: number, clientY: number) => {
    hoverRef.current.pointer = { x: clientX, y: clientY };
    scheduleHover();
  };

  const leave = () => {
    cancelHover();
    hoverRef.current.pointer = null;
    reportHover(null);
  };

  const onClick = (event: MouseEvent<HTMLDivElement>) => {
    if (!onSelectPart) return;
    // Picked where the click landed: a tap has no hover before it, and a
    // queued hover may still name the leg the pointer came from.
    cancelHover();
    const part = partAt(event.clientX, event.clientY);
    // A tap's click comes after its pointer has left; lighting the part then
    // would leave it lit with nothing to clear it.
    if (hoverRef.current.pointer) reportHover(part);
    if (canSelect(part)) onSelectPart(part);
  };

  if (status === "unavailable") return <>{unavailable}</>;

  return (
    <div className="space-y-2">
      <div className="relative">
        <div
          ref={shellRef}
          role="img"
          aria-label={ariaLabel}
          className={cn("relative w-full overflow-hidden rounded-md", className)}
          data-testid={testId}
          onPointerMove={(event) => hoverAt(event.clientX, event.clientY)}
          onPointerCancel={leave}
          onPointerLeave={leave}
          onClick={onClick}
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
        {status === "ready" && overlay ? (
          <div className="pointer-events-none absolute inset-0">{overlay}</div>
        ) : null}
      </div>
      {children}
    </div>
  );
}
