import { spawn } from "node:child_process";
import { access, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { afterEach, describe, expect, it } from "vitest";

import { cleanUpNow, trackChild, trackDirectory } from "./cleanup.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe("broker termination cleanup", () => {
  it("stops agents and removes per-turn directories", async () => {
    // What a signalled broker leaves behind otherwise: a working directory
    // whose agent home holds the session log.
    const cwd = await mkdtemp(join(tmpdir(), "kassiber-ai-cleanup-"));
    await writeFile(join(cwd, "events.jsonl"), "{}\n");
    trackDirectory(cwd);
    const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 60_000)"]);
    trackChild(child);
    const exited = new Promise<NodeJS.Signals | null>((resolve) =>
      child.once("exit", (_code, signal) => resolve(signal)),
    );

    cleanUpNow();

    expect(await exited).toBe("SIGKILL");
    await expect(access(cwd)).rejects.toThrow();
  });

  it("forgets a directory once its turn removed it", async () => {
    const cwd = await mkdtemp(join(tmpdir(), "kassiber-ai-cleanup-"));
    roots.push(cwd);
    const untrack = trackDirectory(cwd);
    untrack();
    cleanUpNow();
    await expect(access(cwd)).resolves.toBeUndefined();
  });
});
