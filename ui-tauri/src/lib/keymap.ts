/**
 * Tiny keyboard-shortcut binder.
 *
 * Lives outside any one component so future review surfaces (quarantine,
 * source-funds suggestions, etc.) can reuse the same wiring. Each binding
 * names the keys it listens for, an optional matcher for modifier shape,
 * a human-readable label that the help overlay shows, and the handler.
 *
 * The binder is intentionally minimal:
 *  - one `useKeymap` hook installs all bindings while the component is
 *    mounted and removes them on unmount;
 *  - typing in an `input`, `textarea`, or `contenteditable` always wins
 *    over a binding so the user can type "P" into a filter field without
 *    triggering the Pair action;
 *  - `?` opens whatever overlay the host component decides — the helper
 *    just exposes the bindings for the overlay to render.
 */

import { useEffect, useMemo } from "react";

export interface Keybinding {
  /** Keys that trigger this binding. Compared against ``KeyboardEvent.key``. */
  keys: string | string[];
  /** Human-readable description for the help overlay. */
  description: string;
  /** Optional category label so the overlay can group bindings. */
  category?: string;
  /** Optional matcher for the event — return true to handle. */
  match?: (event: KeyboardEvent) => boolean;
  /** Whether the binding should fire while the focus is in an input. */
  allowInInput?: boolean;
  /** Handler invoked when the binding fires. */
  handler: (event: KeyboardEvent) => void;
}

const TYPABLE_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);

export function isTypingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (TYPABLE_TAGS.has(target.tagName)) return true;
  if (target.isContentEditable) return true;
  return false;
}

export function matchesKey(event: Pick<KeyboardEvent, "key">, keys: string | string[]): boolean {
  const list = Array.isArray(keys) ? keys : [keys];
  for (const key of list) {
    if (event.key === key) return true;
    if (event.key.toLowerCase() === key.toLowerCase()) return true;
  }
  return false;
}

export function pickBinding(
  bindings: Keybinding[],
  event: Pick<KeyboardEvent, "key" | "target">,
  typing: boolean,
): Keybinding | null {
  for (const binding of bindings) {
    if (typing && !binding.allowInInput) continue;
    if (!matchesKey(event, binding.keys)) continue;
    if (binding.match && !binding.match(event as KeyboardEvent)) continue;
    return binding;
  }
  return null;
}

export function useKeymap(bindings: Keybinding[]): void {
  const stable = useMemo(() => bindings, [bindings]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const typing = isTypingTarget(event.target);
      const binding = pickBinding(stable, event, typing);
      if (!binding) return;
      event.preventDefault();
      binding.handler(event);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [stable]);
}
