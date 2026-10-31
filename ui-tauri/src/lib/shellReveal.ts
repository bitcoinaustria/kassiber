import type { WordmarkReveal } from "@/components/kb/kassiberWordmarkGrid";

/*
 * Which way the shell's wordmark arrives the next time it plays: assemble
 * right after setup creates new books, decrypt otherwise. Module state rather
 * than store state: it is a one-shot cue for a single animation, read during
 * render and cleared only once that animation has finished, so a remount
 * (StrictMode's included) replays the same reveal.
 */
let pending: WordmarkReveal | null = null;

export function celebrateBooksCreated() {
  pending = "assemble";
}

export function shellReveal(): WordmarkReveal {
  return pending ?? "decrypt";
}

export function shellRevealPlayed() {
  pending = null;
}
