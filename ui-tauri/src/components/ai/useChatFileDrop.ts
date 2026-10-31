import * as React from "react";

const isTauriRuntime =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function hasFiles(event: DragEvent): boolean {
  return Array.from(event.dataTransfer?.types ?? []).includes("Files");
}

/**
 * Files dragged onto `target`. In the desktop app the native window reports
 * the drag (the webview gets no file data) and a drop is staged by the shell;
 * `onNativeDrop` then asks for it. In the browser preview the drop carries
 * the files, which go to `onFiles`. Returns whether a file drag is over the
 * target, for the drop hint.
 */
export function useChatFileDrop(
  target: React.RefObject<HTMLElement | null>,
  {
    onFiles,
    onNativeDrop,
  }: {
    onFiles?: (files: File[]) => void;
    onNativeDrop?: () => void;
  },
): boolean {
  const [over, setOver] = React.useState(false);
  const handlers = React.useRef({ onFiles, onNativeDrop });
  React.useLayoutEffect(() => {
    handlers.current = { onFiles, onNativeDrop };
  });
  const enabled = isTauriRuntime ? Boolean(onNativeDrop) : Boolean(onFiles);

  React.useEffect(() => {
    if (!enabled || !isTauriRuntime) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const inside = (position: { x: number; y: number }) => {
      const element = target.current;
      if (!element) return false;
      // The window reports physical pixels; the layout is in CSS pixels.
      const scale = window.devicePixelRatio || 1;
      const x = position.x / scale;
      const y = position.y / scale;
      const rect = element.getBoundingClientRect();
      return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
    };
    void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
      getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "leave") {
          setOver(false);
        } else if (payload.type === "drop") {
          setOver(false);
          if (inside(payload.position)) handlers.current.onNativeDrop?.();
        } else {
          setOver(inside(payload.position));
        }
      }),
    ).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
      setOver(false);
    };
  }, [enabled, target]);

  React.useEffect(() => {
    const element = target.current;
    if (!enabled || isTauriRuntime || !element) return;
    // dragenter/dragleave fire for every child crossed; count them.
    let depth = 0;
    const enter = (event: DragEvent) => {
      if (!hasFiles(event)) return;
      event.preventDefault();
      depth += 1;
      setOver(true);
    };
    const overTarget = (event: DragEvent) => {
      if (!hasFiles(event)) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    };
    const leave = (event: DragEvent) => {
      if (!hasFiles(event)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) setOver(false);
    };
    const drop = (event: DragEvent) => {
      if (!hasFiles(event)) return;
      event.preventDefault();
      depth = 0;
      setOver(false);
      const files = Array.from(event.dataTransfer?.files ?? []);
      if (files.length) handlers.current.onFiles?.(files);
    };
    element.addEventListener("dragenter", enter);
    element.addEventListener("dragover", overTarget);
    element.addEventListener("dragleave", leave);
    element.addEventListener("drop", drop);
    return () => {
      element.removeEventListener("dragenter", enter);
      element.removeEventListener("dragover", overTarget);
      element.removeEventListener("dragleave", leave);
      element.removeEventListener("drop", drop);
      setOver(false);
    };
  }, [enabled, target]);

  return over;
}
