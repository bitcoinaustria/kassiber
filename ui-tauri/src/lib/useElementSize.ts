import { useEffect, useRef, useState } from "react";

/**
 * Track an element's content-box size via ResizeObserver.
 *
 * Returns a ref to attach to the target element and the most recently
 * measured `{ width, height }`. Defaults are used until the observer
 * fires for the first time (i.e. the first paint after mount).
 */
export function useElementSize<T extends HTMLElement>(
  defaultWidth = 0,
  defaultHeight = 0,
) {
  const ref = useRef<T | null>(null);
  const [size, setSize] = useState<{ width: number; height: number }>({
    width: defaultWidth,
    height: defaultHeight,
  });

  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        if (width > 0 && height > 0) {
          setSize({
            width: Math.round(width),
            height: Math.round(height),
          });
        }
      }
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  return [ref, size] as const;
}
