import type { ChildProcess } from "node:child_process";
import { rmSync } from "node:fs";

/**
 * Per-turn state that must not outlive the broker.
 *
 * The daemon ends a broker by signalling its process group, and Node exits on
 * SIGTERM without running `finally` blocks. The working directory and tool
 * bridge would then stay in the temp folder, and an ACP agent's per-turn home
 * holds its session log with the conversation and tool results. The broker
 * therefore registers both here and clears them on termination.
 */
const directories = new Set<string>();
const children = new Set<ChildProcess>();

export function trackDirectory(path: string): () => void {
  directories.add(path);
  return () => directories.delete(path);
}

export function trackChild(child: ChildProcess): void {
  children.add(child);
  child.once("exit", () => children.delete(child));
}

/** Stop agents first so none can write into a directory being removed. */
export function cleanUpNow(): void {
  for (const child of children) {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
  children.clear();
  for (const path of directories) {
    try {
      rmSync(path, { recursive: true, force: true, maxRetries: 3, retryDelay: 20 });
    } catch {
      // Best effort: the process is exiting either way.
    }
  }
  directories.clear();
}

export function cleanUpOnTermination(): void {
  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"] as const) {
    process.once(signal, () => {
      cleanUpNow();
      process.exit(128 + (signal === "SIGTERM" ? 15 : signal === "SIGINT" ? 2 : 1));
    });
  }
}
